//! Headless integration tests: real Bevy client App + real server GameLoop
//! + in-process MockTransport. Drives multi-second scenarios and asserts
//! invariants the unit tests can't see because they don't have a clock that
//! advances.
//!
//! Run with: cargo test --bin bevy-snake integration_tests::

use bevy::prelude::*;
use bevy::MinimalPlugins;
use bevy_snake::{
    board::{BoardSettings, Direction},
    server,
    transport::mock::{pair, LossConfig},
    GameCommands, GameUpdates,
};
use std::time::Duration;
use tokio::sync::mpsc::{channel, Sender};
use tokio::task::JoinHandle;
use web_time::Instant;

use crate::{
    client::{ClientConnection, ClientPlugin, NetworkUpdate},
    game::{
        AuthoritativeTick, GamePlugin, LastAppliedInputs, LocalInputLog, PredictedTick,
        RttEstimator, TickClock,
    },
    ClientState, ConnectionError, GizmoSetting, Settings,
};

/// Bevy update cadence inside the harness. 16 ms ≈ 60 Hz, matches a typical
/// frame budget. Each `advance` loop iteration runs one `app.update()` then
/// sleeps so async tasks (pumps, server tick loop, mock wires) can run.
const FRAME_DT: Duration = Duration::from_millis(16);

// ---------------------------------------------------------------------------
// InterpolationSpy: records (instant, interp, just_fired, predicted_tick)
// once per Bevy frame. Used by the smoothness scenario.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct InterpolationSample {
    pub interp: f32,
    pub just_fired: bool,
    pub predicted_tick: u64,
}

#[derive(Resource, Default)]
pub struct InterpolationSpy {
    pub samples: Vec<InterpolationSample>,
}

fn interpolation_spy_system(
    mut spy: ResMut<InterpolationSpy>,
    tick_clock: Res<TickClock>,
    predicted: Res<PredictedTick>,
) {
    spy.samples.push(InterpolationSample {
        interp: tick_clock.interpolation(Instant::now()),
        just_fired: tick_clock.just_fired,
        predicted_tick: **predicted,
    });
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

pub struct Harness {
    pub app: App,
    server_task: JoinHandle<()>,
    pumps: Vec<JoinHandle<()>>,
    /// Direct injection channel into the client's command pipeline. The
    /// component holds a clone of the same `Sender`, so a `try_send` here
    /// reaches the client-send pump.
    bevy_cmd_tx: Sender<GameCommands>,
    /// Keepalive: dropping this would close `register_client` on the server
    /// and exit the game_loop. We don't need it functionally after the
    /// initial registration, but its drop semantics matter.
    _register_tx_keepalive: Sender<server::Client>,
}

impl Harness {
    pub async fn new(loss: LossConfig) -> Self {
        // 1. Spawn the server's game_loop task.
        let (register_tx, register_rx) = channel::<server::Client>(1);
        let server_task = tokio::spawn(server::game_loop(register_rx));

        // 2. Build MockTransport pair.
        let (client_side, server_side) = pair(loss);

        // 3. Register a single server-side Client over the mock.
        let (server_client, server_cmd_in, server_upd_out) = server::Client::new();
        register_tx
            .send(server_client)
            .await
            .expect("server accepts client registration");

        // 4. Build the headless Bevy app.
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // MinimalPlugins doesn't include InputPlugin, but reset_game reads
        // `ButtonInput<KeyCode>`. The plugin inserts the resource even when
        // no winit window exists; just no input events will arrive.
        app.add_plugins(bevy::input::InputPlugin);
        // MinimalPlugins also omits StatesPlugin, which `init_state` requires.
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins((ClientPlugin, GamePlugin));
        app.init_state::<ClientState>();
        app.init_resource::<ConnectionError>();
        app.insert_resource(Settings {
            interpolation: true,
            tick_interval_ms: Some(133),
            board_settings: BoardSettings::default(),
            ai: false,
            gizmos: GizmoSetting::None,
            walls: false,
            walls_debug: false,
        });
        app.init_resource::<InterpolationSpy>();
        app.add_systems(Update, interpolation_spy_system.after(crate::game::update_game));

        // 5. Pre-wired ClientConnection. The harness keeps the OTHER ends of
        // the channels and pumps them itself.
        let (conn, bevy_cmd_rx, bevy_upd_tx, bevy_cmd_tx) = ClientConnection::with_mock_channels();
        app.world_mut().spawn(conn);

        // 6. Tell the client it's "connected" so it leaves the Connecting
        // state. The real WT task would normally do this on session open.
        bevy_upd_tx
            .send(NetworkUpdate::Connected)
            .await
            .expect("bevy app's update channel accepts initial Connected");

        // 7. Spawn the four pumps that bridge channels across the mock.
        let mut pumps = Vec::with_capacity(4);

        // client-send pump: GameCommands from the Bevy app → MockClientSide,
        // routed reliable vs datagram to match the real client's wire choice.
        let client_reliable_out = client_side.reliable_out.clone();
        let client_datagram_out = client_side.datagram_out.clone();
        let mut bevy_cmd_rx_owned = bevy_cmd_rx;
        pumps.push(tokio::spawn(async move {
            while let Some(cmd) = bevy_cmd_rx_owned.recv().await {
                let sink = match &cmd {
                    GameCommands::Input { .. } => &client_datagram_out,
                    _ => &client_reliable_out,
                };
                if sink.send(cmd).await.is_err() {
                    break;
                }
            }
        }));

        // client-recv pump: GameUpdates from MockClientSide → bevy_upd_tx.
        // Reliable and datagram are coalesced into a single ordered stream
        // (mirrors how the bevy app sees `NetworkUpdate::Update` via
        // ClientConnection::receive_update).
        let mut client_reliable_in = client_side.reliable_in;
        let mut client_datagram_in = client_side.datagram_in;
        let bevy_upd_tx_for_recv = bevy_upd_tx.clone();
        pumps.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = client_reliable_in.recv() => {
                        match msg {
                            Some(u) => {
                                if bevy_upd_tx_for_recv.send(NetworkUpdate::Update(u)).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    msg = client_datagram_in.recv() => {
                        match msg {
                            Some(u) => {
                                if bevy_upd_tx_for_recv.send(NetworkUpdate::Update(u)).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        }));

        // server-recv pump: GameCommands from MockServerSide → server's Client.
        let mut server_reliable_in = server_side.reliable_in;
        let mut server_datagram_in = server_side.datagram_in;
        pumps.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = server_reliable_in.recv() => {
                        match msg {
                            Some(c) => if server_cmd_in.send(c).await.is_err() { break; },
                            None => break,
                        }
                    }
                    msg = server_datagram_in.recv() => {
                        match msg {
                            Some(c) => if server_cmd_in.send(c).await.is_err() { break; },
                            None => break,
                        }
                    }
                }
            }
        }));

        // server-send pump: GameUpdates from server's Client → MockServerSide
        // (reliable only — the server never emits datagrams). We move BOTH
        // `reliable_out` and `datagram_out` into the pump even though we
        // never write to datagram_out: dropping the unused sender would close
        // the s2c_datagram wire task, then close `client_side.datagram_in`,
        // and the client-recv pump's select! arm on that receiver would
        // become a hot-spin returning None forever.
        let server_reliable_out = server_side.reliable_out;
        let server_datagram_out_keepalive = server_side.datagram_out;
        let mut server_upd_out_owned = server_upd_out;
        pumps.push(tokio::spawn(async move {
            // Keep the unused datagram sender alive for the lifetime of this
            // task. Dropping it would close the s2c_datagram wire task on
            // the mock's other side, which would close
            // `client_side.datagram_in`, and the client-recv pump's
            // `select!` arm on that receiver would hot-spin returning None.
            let _keepalive = server_datagram_out_keepalive;
            while let Some(u) = server_upd_out_owned.recv().await {
                if server_reliable_out.send(u).await.is_err() {
                    break;
                }
            }
        }));

        Harness {
            app,
            server_task,
            pumps,
            bevy_cmd_tx,
            _register_tx_keepalive: register_tx,
        }
    }

    /// Advance for `duration` wall-clock by alternating `app.update()` and
    /// `tokio::time::sleep(FRAME_DT)`. The sleep yields so async tasks
    /// (pumps, server, mock wires) can run between Bevy frames.
    pub async fn advance(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.app.update();
            tokio::time::sleep(FRAME_DT).await;
        }
    }

    /// Inject a command directly into the client's command pipeline. Bypasses
    /// `update_game`'s input-collection path, which is fine because in tests
    /// no key presses fire anyway.
    pub fn send(&self, cmd: GameCommands) {
        self.bevy_cmd_tx
            .try_send(cmd)
            .expect("harness command channel full or closed");
    }

    pub fn tick_clock(&self) -> &TickClock {
        self.app.world().resource::<TickClock>()
    }
    pub fn rtt(&self) -> &RttEstimator {
        self.app.world().resource::<RttEstimator>()
    }
    pub fn predicted_tick(&self) -> u64 {
        **self.app.world().resource::<PredictedTick>()
    }
    pub fn auth_tick(&self) -> u64 {
        **self.app.world().resource::<AuthoritativeTick>()
    }
    pub fn client_state(&self) -> ClientState {
        self.app.world().resource::<State<ClientState>>().get().clone()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server_task.abort();
        for h in self.pumps.drain(..) {
            h.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// 3.1 — Idle RTT stability. The duplicate-echo bug would have climbed
/// `last_rtt_ms` by a tick period per snapshot while the player was idle.
#[tokio::test(flavor = "multi_thread")]
async fn idle_rtt_stays_bounded() {
    let mut h = Harness::new(LossConfig::default()).await;
    // Let connection/initial-snapshot settle.
    h.advance(Duration::from_millis(300)).await;
    assert_eq!(
        h.client_state(),
        ClientState::Connected,
        "client should transition to Connected within 300ms"
    );

    // Sit idle for 3 s with no input. Server has nothing to echo (or echoes
    // None); the bug would have produced thousands of ms here.
    h.advance(Duration::from_secs(3)).await;

    let rtt = h.rtt();
    if let Some(last) = rtt.last_rtt_ms {
        assert!(
            last < 100,
            "last_rtt_ms must stay low when idle, got {} ms (duplicate-echo regression)",
            last,
        );
    }
    if let Some(owt) = rtt.one_way_ms {
        assert!(
            owt < 50.0,
            "one_way_ms must stay near zero when idle, got {} ms",
            owt,
        );
    }
}

/// 3.2 — After a `RestartGame`, the first server snapshot has tick=0 while
/// the client's `auth_tick` is whatever it had reached. The PLL should snap
/// (not smooth-filter) and `predicted_tick` should land back at `auth_tick`
/// once the in-flight inputs (none, in this test) have drained.
#[tokio::test(flavor = "multi_thread")]
async fn restart_converges_within_one_snapshot() {
    let mut h = Harness::new(LossConfig::default()).await;
    h.advance(Duration::from_millis(800)).await; // ~5 ticks elapse
    assert!(
        h.predicted_tick() >= 2,
        "should have ticked at least a couple of times pre-restart, got pred={}",
        h.predicted_tick(),
    );

    h.send(GameCommands::RestartGame {
        board_settings: BoardSettings::default(),
    });
    h.advance(Duration::from_millis(400)).await; // 2-3 ticks for restart snapshot + sync

    let tc = h.tick_clock();
    assert!(
        tc.last_snapped,
        "first post-restart snapshot must snap (tick went backwards)",
    );
    assert_eq!(
        h.auth_tick(),
        h.predicted_tick(),
        "after restart converges and no inputs in flight, pred == auth",
    );
}

/// 3.3 — An input sent from the client should end up applied on the server
/// and reflected back as `LastAppliedInputs[0] == Some(Up)`.
#[tokio::test(flavor = "multi_thread")]
async fn input_round_trips_to_applied_inputs() {
    let mut h = Harness::new(LossConfig::default()).await;
    h.advance(Duration::from_millis(300)).await;

    let target = h.auth_tick() + 1;
    h.send(GameCommands::Input {
        tick: target,
        direction: Direction::Up,
        client_send_ms: 0,
    });

    // Poll up to 400 ms (2-3 server ticks) for the server to apply the input.
    let start = Instant::now();
    let mut saw_up = false;
    while Instant::now().duration_since(start) < Duration::from_millis(400) {
        h.advance(Duration::from_millis(20)).await;
        let last = h.app.world().resource::<LastAppliedInputs>();
        if last.first().copied().flatten() == Some(Direction::Up) {
            saw_up = true;
            break;
        }
    }
    assert!(saw_up, "server should apply Up within 400ms");

    // After applying, the input should have drained from the local log on
    // the next reconcile.
    h.advance(Duration::from_millis(200)).await;
    let log = h.app.world().resource::<LocalInputLog>();
    assert!(log.is_empty(), "input log should drain after server ack");
}

/// 3.5 — After a `BoardEvent::GameOver` lands, the local clock should
/// freeze so `predicted_tick` doesn't race ahead of the now-paused server.
/// Without this, inputs sent in the dead window get tagged with ticks the
/// server will never fire — and the eventual restart has to undo a big
/// `predicted_tick` drift, masking real bugs.
#[tokio::test(flavor = "multi_thread")]
async fn predicted_tick_freezes_on_game_over() {
    let mut h = Harness::new(LossConfig::default()).await;
    h.advance(Duration::from_millis(300)).await;

    // Default snake faces right; without inputs it walks straight into the
    // right wall. The Small board is 10 wide; the snake's head is roughly
    // mid-board, so death lands within ~6-8 ticks (~1 s).
    h.advance(Duration::from_millis(2500)).await;

    let auth_after_death = h.auth_tick();
    let pred_after_death = h.predicted_tick();
    // Server pauses on GameOver; auth_tick stops advancing.
    // Without the freeze, predicted_tick would still climb at ~7.5 Hz.
    assert!(
        pred_after_death <= auth_after_death + 1,
        "predicted_tick should not run ahead of auth_tick post-GameOver \
         (auth={}, pred={}); local clock failed to freeze",
        auth_after_death,
        pred_after_death,
    );
    assert!(
        h.app.world().resource::<TickClock>().game_over,
        "TickClock.game_over should be set after the GameOver snapshot"
    );

    // Sit idle for another 1.5 s — pred should *still* not climb.
    h.advance(Duration::from_millis(1500)).await;
    assert!(
        h.predicted_tick() <= auth_after_death + 1,
        "predicted_tick must stay frozen across an extended GameOver pause \
         (auth_at_death={}, pred_now={})",
        auth_after_death,
        h.predicted_tick(),
    );

    // Restart and let one snapshot land. game_over should clear and pred
    // should track the fresh auth_tick.
    h.send(GameCommands::RestartGame {
        board_settings: BoardSettings::default(),
    });
    h.advance(Duration::from_millis(400)).await;
    assert!(
        !h.app.world().resource::<TickClock>().game_over,
        "game_over should clear on the server-reset snapshot"
    );
    assert_eq!(
        h.auth_tick(),
        h.predicted_tick(),
        "after restart, pred should be in lockstep with auth"
    );
}

/// 3.4 — The renderer's `tick_clock.interpolation()` value should rise
/// monotonically from ~0 to ~1 within each tick window and reset on
/// `just_fired`. Big jumps would indicate the PLL is nudging the timer in a
/// way that produces visual stutter.
#[tokio::test(flavor = "multi_thread")]
async fn interpolation_is_smooth_within_tick_windows() {
    let mut h = Harness::new(LossConfig::default()).await;
    h.advance(Duration::from_millis(300)).await;
    // Discard warm-up samples (initial Connected push + PLL lock) so we
    // measure steady-state behaviour.
    h.app
        .world_mut()
        .resource_mut::<InterpolationSpy>()
        .samples
        .clear();

    h.advance(Duration::from_secs(1)).await; // ~7-8 tick windows at 133ms

    let spy = h.app.world().resource::<InterpolationSpy>();
    assert!(
        spy.samples.len() > 30,
        "expected at least ~30 frames in 1s, got {}",
        spy.samples.len(),
    );

    // Walk samples within "tick windows" delimited by `predicted_tick`
    // changes. The board can advance via either a local fire (just_fired=true)
    // or a snapshot reconcile (just_fired=false); both should reset interp
    // toward 0 because the renderer's `last_advance_at` is bumped in either
    // path. So we use `predicted_tick` changes — not `just_fired` — to mark
    // window boundaries.
    let mut tick_advances = 0;
    let mut last: Option<InterpolationSample> = None;
    for &s in &spy.samples {
        let crossed = last.map_or(false, |prev| s.predicted_tick != prev.predicted_tick);
        if crossed {
            tick_advances += 1;
            // First sample of a new window: interp should reset toward 0.
            assert!(
                s.interp < 0.3,
                "first sample of new tick window: interp should reset to ~0, got {}",
                s.interp,
            );
        } else if let Some(prev) = last {
            let delta = s.interp - prev.interp;
            // Lower bound -0.05 tolerates the per-snapshot reanchor
            // (last_advance_at = arrival) stepping interp slightly backwards
            // between predicted ticks.
            assert!(
                delta >= -0.05,
                "non-monotone in window: {} -> {} (Δ={})",
                prev.interp,
                s.interp,
                delta,
            );
            // 16ms frame / 133ms period = ~12% per frame; 0.4 = ~3× headroom.
            assert!(
                delta.abs() < 0.4,
                "interp per-frame jump too big: {} -> {} (Δ={})",
                prev.interp,
                s.interp,
                delta,
            );
        }
        last = Some(s);
    }
    assert!(
        tick_advances >= 5,
        "expected ≥ 5 tick advances in 1s @ 133ms, got {}",
        tick_advances,
    );
}
