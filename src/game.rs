use crate::{
    client::{ClientConnection, NetworkUpdate},
    ClientState, ConnectionError, ErrorKind, GizmoSetting, Settings,
};
use bevy::{prelude::*, utils::HashMap};
use bevy_snake::{
    ai::{cycle_basis, AIGizmos, SnakeAI, TreeSearch},
    board::{Board, BoardEvent, Cell, Direction},
    GameCommands, GameUpdates,
};
use rand::{rngs::StdRng, SeedableRng};
use std::{collections::VecDeque, time::Duration};
use web_time::Instant;

/// Default tick period the client assumes until the first server snapshot
/// tells it otherwise. Matches the server's `DEFAULT_TICK_INTERVAL_MS`.
const DEFAULT_TICK_PERIOD: Duration = Duration::from_millis(133);

/// Proportional gain of the phase-locked loop. Each snapshot nudges
/// `next_tick_at` by `PLL_ALPHA * phase_error`. Stay well below 1.0 to absorb
/// per-packet jitter; we still trust the server-sent `tick_interval_ms` for
/// frequency so we don't need a high gain to track it.
const PLL_ALPHA: f32 = 0.1;

/// EMA gain for one-way trip smoothing. `beta = 0.2` gives a ~5-sample window
/// — enough to smooth single packet jitter, fast enough to follow a real
/// route change within a couple seconds.
const RTT_EMA_BETA: f32 = 0.2;

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClientEpoch(Instant::now()))
            .insert_resource(RttEstimator::default())
            .insert_resource(TickClock::new(Instant::now()))
            .insert_resource(Board::empty(0, 0))
            .insert_resource(AuthoritativeBoard(Board::empty(0, 0)))
            .insert_resource(AuthoritativeTick(0))
            .insert_resource(PredictedTick(0))
            .insert_resource(LastAppliedInputs(Vec::new()))
            .insert_resource(LocalInputLog(VecDeque::new()))
            .insert_resource(LocalSnakeId(0))
            .insert_resource(Rng(StdRng::from_os_rng()))
            .insert_resource(Points(vec![0; 4]))
            .insert_resource(SnakeInputs(vec![
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyW,
                        down: KeyCode::KeyS,
                        left: KeyCode::KeyA,
                        right: KeyCode::KeyD,
                        shoot: KeyCode::Space,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::ArrowUp,
                        down: KeyCode::ArrowDown,
                        left: KeyCode::ArrowLeft,
                        right: KeyCode::ArrowRight,
                        shoot: KeyCode::AltRight,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyP,
                        down: KeyCode::Semicolon,
                        left: KeyCode::KeyL,
                        right: KeyCode::Quote,
                        shoot: KeyCode::Backslash,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyY,
                        down: KeyCode::KeyH,
                        left: KeyCode::KeyG,
                        right: KeyCode::KeyJ,
                        shoot: KeyCode::KeyB,
                    },
                    input_queue: VecDeque::new(),
                },
            ]))
            .add_systems(Startup, create_client)
            .add_systems(Update, reset_game)
            .add_systems(Update, update_game);
    }
}

/// Local monotonic reference point used to tag `Input` commands with a
/// `client_send_ms` delta. Set once at startup; the server echoes the value
/// back on the next snapshot for skew-free RTT measurement.
#[derive(Resource, Deref, DerefMut)]
pub struct ClientEpoch(pub Instant);

impl ClientEpoch {
    pub fn now_ms(&self) -> u32 {
        Instant::now().saturating_duration_since(self.0).as_millis() as u32
    }
}

/// Smoothed estimate of the one-way trip between client and server. Driven by
/// the `echo_client_send_ms` field the server bounces back in each snapshot.
#[derive(Resource, Default)]
pub struct RttEstimator {
    /// Smoothed one-way trip in milliseconds. `None` until the first echo
    /// lands (i.e., until the player has sent at least one input).
    pub one_way_ms: Option<f32>,
    /// Most recent raw RTT measurement; for diagnostics only.
    pub last_rtt_ms: Option<u32>,
    /// The `client_send_ms` we most recently consumed an echo for. The
    /// server keeps echoing its last-processed value snapshot after
    /// snapshot, so without this we'd feed the same input's RTT into the
    /// EMA every period and the estimate would climb by a period each time
    /// while the player is idle.
    pub last_consumed_echo: Option<u32>,
}

impl RttEstimator {
    /// Fold an echo into the EMA, skipping duplicates. Returns true iff a
    /// new sample was actually consumed.
    pub fn observe_echo(&mut self, echoed_ms: u32, now_ms: u32) -> bool {
        if self.last_consumed_echo == Some(echoed_ms) {
            return false;
        }
        self.last_consumed_echo = Some(echoed_ms);
        let rtt_ms = now_ms.saturating_sub(echoed_ms);
        self.last_rtt_ms = Some(rtt_ms);
        let owt = rtt_ms as f32 / 2.0;
        self.one_way_ms = Some(match self.one_way_ms {
            None => owt,
            Some(prev) => (1.0 - RTT_EMA_BETA) * prev + RTT_EMA_BETA * owt,
        });
        true
    }
}

/// Local-clock state driving the predicted tick fire instants. Replaces the
/// old `TickTimer`: instead of free-running on `Time::delta`, the next tick
/// instant is steered by a PLL whose setpoint is "server wall-clock for the
/// next tick, in our local frame," derived from snapshot arrivals and the
/// `RttEstimator`'s one-way estimate.
#[derive(Resource)]
pub struct TickClock {
    /// Local-clock instant at which the next predicted tick should fire.
    pub next_tick_at: Instant,
    /// Local-clock instant of the most recent board advance — either a
    /// per-frame fire or a snapshot reconcile that bumped `predicted_tick`.
    /// Renderer interpolates from here to `next_tick_at` so animations
    /// always start at 0 progress when the board ticks.
    pub last_advance_at: Instant,
    /// Current estimate of the server's tick period. Updated from
    /// `tick_interval_ms` on every snapshot; we trust the server for cadence.
    pub tick_period: Duration,
    /// Set true on the Bevy frame in which we crossed `next_tick_at`. Read by
    /// other systems (renderer, AI) so they don't each duplicate the timing
    /// check.
    pub just_fired: bool,
    /// True until the first snapshot has been folded in. Forces a snap on
    /// that first snapshot regardless of phase error so the clock locks to
    /// the server's actual cadence even if our initial `next_tick_at` was
    /// off by a long time.
    pub locked: bool,
    /// Signed phase error from the most recent snapshot, in nanoseconds.
    /// Positive = our `next_tick_at` was too early (we'd predict before the
    /// server fires the same tick). Diagnostic only.
    pub last_phase_error_ns: i64,
    /// Whether the most recent snapshot snapped (true) or low-pass-filtered
    /// (false) the phase update. Diagnostic only.
    pub last_snapped: bool,
    /// Monotonically-increasing counter for per-frame logging correlation.
    /// Bumped at the top of `update_game`. Diagnostic only.
    pub frame_idx: u64,
    /// True after a `BoardEvent::GameOver` lands and before a restart. While
    /// set, the local clock stops firing predicted ticks: with no snakes on
    /// the board, fires don't change anything visually but ratchet
    /// `predicted_tick` forward (the local clock has no idea the server has
    /// paused), which makes any input you press tag a tick the server will
    /// never fire. Cleared on a server reset (next snapshot with
    /// `tick < auth_tick`) and on local `reset_game`.
    pub game_over: bool,
}

impl TickClock {
    pub fn new(now: Instant) -> Self {
        Self {
            next_tick_at: now + DEFAULT_TICK_PERIOD,
            last_advance_at: now,
            tick_period: DEFAULT_TICK_PERIOD,
            just_fired: false,
            locked: false,
            last_phase_error_ns: 0,
            last_snapped: false,
            frame_idx: 0,
            game_over: false,
        }
    }

    /// 0.0..=1.0 measure of how far we are between the last board advance
    /// and the next predicted fire. Used by the renderer for smooth
    /// between-tick interpolation. Always 0 right after `last_advance_at`,
    /// climbs to 1 at `next_tick_at`.
    pub fn interpolation(&self, now: Instant) -> f32 {
        let span = self.next_tick_at.saturating_duration_since(self.last_advance_at);
        let span_ns = span.as_nanos() as f32;
        if span_ns <= 0.0 {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.last_advance_at);
        (elapsed.as_nanos() as f32 / span_ns).clamp(0.0, 1.0)
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct SnakeInputs(Vec<SnakeInput>);

#[derive(Resource, Deref, DerefMut)]
pub struct Points(Vec<usize>);

pub struct SnakeInput {
    pub input_map: InputMap,
    pub input_queue: VecDeque<Direction>,
}

/// The most recent fully-authoritative server snapshot. Replays of the
/// local input log start from here.
#[derive(Resource, Deref, DerefMut)]
pub struct AuthoritativeBoard(Board);

/// The tick number the authoritative board was at when the server sent it.
#[derive(Resource, Deref, DerefMut)]
pub struct AuthoritativeTick(u64);

/// How far the predicted `Board` has been advanced past the authoritative
/// snapshot. Equal to `AuthoritativeTick + log.len()` after a reconcile.
#[derive(Resource, Deref, DerefMut)]
pub struct PredictedTick(u64);

/// Per-snake direction the server applied on the most recent authoritative
/// tick. Used for opponents when we predict forward — we don't see their
/// inputs, so we assume they keep doing what the server last saw them do.
#[derive(Resource, Deref, DerefMut)]
pub struct LastAppliedInputs(Vec<Option<Direction>>);

/// Local inputs sent but not yet acknowledged by a server snapshot, in
/// submission order. Each entry is the (tick, direction) it was meant for.
/// Replayed against the authoritative board on reconcile.
#[derive(Resource, Deref, DerefMut)]
pub struct LocalInputLog(VecDeque<(u64, Direction)>);

/// Which snake id we control on this client. Today the convention is "the
/// first registered client controls snake 0," and we have no other source
/// of truth — server doesn't tell us our id. For a single player this is 0.
#[derive(Resource, Deref, DerefMut)]
pub struct LocalSnakeId(u8);

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct InputMap {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub shoot: KeyCode,
}

pub fn create_client(mut commands: Commands) {
    // In tests the harness spawns its own pre-wired `ClientConnection` after
    // the App is built. Skipping here avoids racing with that and avoids
    // pulling in the real WT URL env var.
    #[cfg(test)]
    {
        let _ = commands;
        return;
    }
    #[cfg(not(test))]
    commands.spawn(ClientConnection::new(get_wt_url()));
}

#[cfg(target_arch = "wasm32")]
pub fn get_wt_url() -> String {
    use wasm_bindgen::JsValue;
    let win = web_sys::window().expect("no window");
    let val = js_sys::Reflect::get(&win, &JsValue::from_str("WT_URL"))
        .expect("failed to read window.WT_URL");
    val.as_string().expect("window.WT_URL must be a string")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_wt_url() -> String {
    std::env::var("WT_URL").unwrap_or_else(|_| "https://localhost:1234".to_string())
}

pub fn reset_game(
    mut board: ResMut<Board>,
    mut auth_board: ResMut<AuthoritativeBoard>,
    mut auth_tick: ResMut<AuthoritativeTick>,
    mut predicted_tick: ResMut<PredictedTick>,
    mut last_applied: ResMut<LastAppliedInputs>,
    mut input_log: ResMut<LocalInputLog>,
    mut input_queues: ResMut<SnakeInputs>,
    mut tick_clock: ResMut<TickClock>,
    mut client_connections: Query<&mut ClientConnection>,
    settings: Res<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        *board = Board::new(settings.board_settings);
        **auth_board = board.clone();
        **auth_tick = 0;
        **predicted_tick = 0;
        last_applied.clear();
        input_log.clear();
        tick_clock.game_over = false;

        for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
            input_queue.clear();
        }

        if !client_connections.is_empty() {
            client_connections
                .single_mut()
                .send_command(GameCommands::RestartGame {
                    board_settings: settings.board_settings.clone(),
                });
        }
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct Rng(StdRng);

pub fn update_game(
    mut input_queues: ResMut<SnakeInputs>,
    mut tick_clock: ResMut<TickClock>,
    mut rtt: ResMut<RttEstimator>,
    client_epoch: Res<ClientEpoch>,
    mut board: ResMut<Board>,
    mut auth_board: ResMut<AuthoritativeBoard>,
    mut auth_tick: ResMut<AuthoritativeTick>,
    mut predicted_tick: ResMut<PredictedTick>,
    mut last_applied: ResMut<LastAppliedInputs>,
    mut input_log: ResMut<LocalInputLog>,
    local_snake: Res<LocalSnakeId>,
    mut points: ResMut<Points>,
    mut client_connections: Query<&mut ClientConnection>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<ClientState>>,
    mut connection_error: ResMut<ConnectionError>,
) {
    tick_clock.just_fired = false;
    tick_clock.frame_idx = tick_clock.frame_idx.wrapping_add(1);
    // Per-frame trace of the PLL state. `interp` is what the renderer uses
    // for between-tick animation; `next_in_ms` is how long until the local
    // clock fires its next predicted tick; `since_advance_ms` is how long
    // since the last board advance (fire OR snapshot reconcile). Jitter in
    // these is what visual stutter looks like in code.
    {
        let now = Instant::now();
        let next_in_ms = tick_clock
            .next_tick_at
            .saturating_duration_since(now)
            .as_secs_f64()
            * 1000.0;
        let since_advance_ms = now
            .saturating_duration_since(tick_clock.last_advance_at)
            .as_secs_f64()
            * 1000.0;
        let interp = tick_clock.interpolation(now);
        info!(
            "FRAME[{}] interp={:.3} since_advance_ms={:.1} next_in_ms={:.1} phase_err_ms={:+.1} period_ms={} pred={} auth={} log={}",
            tick_clock.frame_idx,
            interp,
            since_advance_ms,
            next_in_ms,
            tick_clock.last_phase_error_ns as f64 / 1e6,
            tick_clock.tick_period.as_millis(),
            **predicted_tick,
            **auth_tick,
            input_log.len(),
        );
    }

    // No connection entity right now (e.g. between Retry click and respawn).
    // Skip the network section entirely — predicted board still ticks below.
    let Ok(mut client_connection) = client_connections.get_single_mut() else {
        return;
    };

    // 1. Consume server snapshots: trust authoritative state, drop ack'd
    //    inputs from the log, and re-predict forward from the new baseline.
    if let Some(game_updates) = client_connection.receive_update() {
        match game_updates {
            NetworkUpdate::Update(GameUpdates::Ticked {
                board: new_board,
                events,
                tick,
                tick_interval_ms,
                echo_client_send_ms,
                applied_inputs,
            }) => {
                // Note: we don't guard against `tick < auth_tick` here.
                // RestartGame resets the server's tick counter to 0, and we
                // must accept that snapshot even though it goes "backwards"
                // numerically. The reliable framed stream doesn't reorder
                // anyway, so there's no real duplicate-snapshot scenario.
                //
                // Detect a server-side reset (RestartGame) — the tick
                // counter jumps backwards. Force the PLL to snap rather
                // than smooth-filter; otherwise stale `next_tick_at` from
                // before the reset takes ~10 snapshots to correct out.
                // Also clears `game_over` so the local clock starts firing
                // again for the new round.
                if tick < **auth_tick {
                    info!("server reset detected (tick {} < auth {}), unlocking PLL", tick, **auth_tick);
                    tick_clock.locked = false;
                    tick_clock.game_over = false;
                }

                let arrival = Instant::now();

                // Fold the RTT echo into the estimator first so the PLL has
                // the freshest OWT estimate when it computes phase error.
                // `observe_echo` drops duplicates — the server keeps echoing
                // its last-processed value snapshot after snapshot, and if
                // we accepted those, an idle client's RTT would climb by a
                // period per snapshot.
                let now_ms = client_epoch.now_ms();
                if let Some(echoed) = echo_client_send_ms {
                    let consumed = rtt.observe_echo(echoed, now_ms);
                    if consumed {
                        info!(
                            "snapshot tick={} now_ms={} echoed={} rtt_ms={} (fresh)",
                            tick,
                            now_ms,
                            echoed,
                            now_ms.saturating_sub(echoed),
                        );
                    }
                }

                **auth_board = new_board.clone();
                **auth_tick = tick;
                **last_applied = applied_inputs;

                // Drop any local inputs the server has now seen.
                while matches!(input_log.front(), Some((t, _)) if *t <= tick) {
                    input_log.pop_front();
                }

                // Reconcile: start from the authoritative snapshot and replay
                // any still-unack'd local inputs forward, ticking the board
                // without RNG (apples/walls only come from the server).
                let mut next = new_board.clone();
                let mut t = tick;
                for &(_input_tick, dir) in input_log.iter() {
                    if !apply_tick(&mut next, dir, &last_applied, **local_snake) {
                        break;
                    }
                    t += 1;
                }
                *board = next;
                **predicted_tick = t;

                for event in events {
                    match event {
                        BoardEvent::GameOver => {
                            info!("game over");
                            // Freeze the local clock until restart. With no
                            // snakes on the board, local fires don't change
                            // anything visually but ratchet predicted_tick
                            // forward, which makes any input you press tag a
                            // tick the server will never fire.
                            tick_clock.game_over = true;
                        }
                        BoardEvent::SnakeDamaged { .. } => {
                            for (snake_id, _) in board.snakes().into_iter() {
                                points[snake_id as usize] += 1;
                            }
                        }
                        _ => {}
                    }
                }

                // PLL update: re-aim next_tick_at at the local-clock instant
                // the server will fire its next tick. Pure function so it's
                // directly unit-testable.
                let pre_next = tick_clock.next_tick_at;
                apply_snapshot_to_clock(
                    &mut tick_clock,
                    tick,
                    arrival,
                    tick_interval_ms,
                    rtt.one_way_ms,
                );
                info!(
                    "PLL: tick={} auth={} pred={} log={} owt_ms={:?} snapped={} phase_err_ms={:.1} next_in_ms={:.1}",
                    tick,
                    **auth_tick,
                    **predicted_tick,
                    input_log.len(),
                    rtt.one_way_ms,
                    tick_clock.last_snapped,
                    tick_clock.last_phase_error_ns as f64 / 1e6,
                    tick_clock
                        .next_tick_at
                        .saturating_duration_since(arrival)
                        .as_secs_f64()
                        * 1000.0,
                );
                let _ = pre_next;
                // Reconcile updated the predicted board, so anchor the
                // renderer's interpolation to "now". Without this, the
                // animation would carry over fractional progress from the
                // previous tick window and pop visually.
                tick_clock.last_advance_at = arrival;

                // Now that the server ack'd the head of the queue, send the
                // next queued input (visual queue, separate from input_log).
                // Tag with the server's next tick (auth_tick + 1) — the
                // visible queue is meant to drain one-per-server-tick.
                for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
                    input_queue.pop_front();
                    if let Some(&direction) = input_queue.front() {
                        let server_tick_tag = tick + 1;
                        let input = GameCommands::Input {
                            direction,
                            tick: server_tick_tag,
                            client_send_ms: client_epoch.now_ms(),
                        };
                        info!("sending {:?} (server_tag={})", input, server_tick_tag);
                        client_connection.send_command(input);
                    }
                }
            }
            NetworkUpdate::Connected => {
                info!("connected");
                next_state.set(ClientState::Connected);
            }
            NetworkUpdate::Disconnected => {
                info!("disconnected");
                *connection_error = ConnectionError {
                    kind: ErrorKind::Disconnected,
                    detail: String::new(),
                };
                next_state.set(ClientState::Error);
            }
            NetworkUpdate::ConnectFailed(kind, detail) => {
                info!("connect failed: {:?} {}", kind, detail);
                *connection_error = ConnectionError { kind, detail };
                next_state.set(ClientState::Error);
            }
        }
    }

    // 2. Local prediction tick: PLL-driven. Fire at most one tick per Bevy
    //    frame; if we're behind, the next snapshot will snap us forward.
    // Skipped entirely while `game_over` is set: with no snakes on the
    // board, firing would advance predicted_tick past where the (paused)
    // server actually is, and any input you tag based on that tick is dead
    // on arrival. The flag is cleared when the next server-reset snapshot
    // lands.
    let now = Instant::now();
    if !tick_clock.game_over && now >= tick_clock.next_tick_at {
        let period = tick_clock.tick_period;
        let overdue_ms = now
            .saturating_duration_since(tick_clock.next_tick_at)
            .as_secs_f64()
            * 1000.0;
        info!(
            "FIRE: auth={} pred={} log={} overdue_ms={:.1}",
            **auth_tick,
            **predicted_tick,
            input_log.len(),
            overdue_ms,
        );
        tick_clock.just_fired = true;
        tick_clock.last_advance_at = tick_clock.next_tick_at;
        tick_clock.next_tick_at += period;
    }

    if tick_clock.just_fired && board.width() > 0 {
        let next_tick = **predicted_tick + 1;
        let local_dir = input_log
            .iter()
            .find(|(t, _)| *t == next_tick)
            .map(|(_, d)| *d);
        let mut next = board.clone();
        if apply_tick(
            &mut next,
            local_dir.unwrap_or_else(|| local_snake_dir(&board, **local_snake)),
            &last_applied,
            **local_snake,
        ) {
            *board = next;
            **predicted_tick = next_tick;
        }
    }

    // 3. Collect local key presses, push into the visual input_queue (which
    //    gates "send next input on snapshot ack"), and into the canonical
    //    LocalInputLog used for replay. Also fire the immediate send for the
    //    head of the queue.
    for (snake_idx, SnakeInput { input_map, input_queue }) in
        input_queues.iter_mut().enumerate()
    {
        if input_queue.len() >= 3 {
            continue;
        }
        let pressed = if keys.just_pressed(input_map.up) {
            Some(Direction::Up)
        } else if keys.just_pressed(input_map.down) {
            Some(Direction::Down)
        } else if keys.just_pressed(input_map.left) {
            Some(Direction::Left)
        } else if keys.just_pressed(input_map.right) {
            Some(Direction::Right)
        } else {
            None
        };
        let Some(input) = pressed else { continue };

        let last_in_queue = input_queue.back();
        if last_in_queue.is_some_and(|&last| input == last || input == last.opposite()) {
            continue;
        }

        // Only the local snake's inputs are predicted/replayed. For
        // multi-player on the same machine the other snakes' keys still
        // queue + send; we just don't predict their snake locally.
        let is_local = snake_idx as u8 == **local_snake;

        if input_queue.is_empty() {
            // Tag with the next server tick we expect to land on, not the
            // client's predicted_tick (which races ahead by however many
            // unacked inputs are in flight). Locally we still store the
            // input keyed by the client's next predicted tick so replay
            // ordering stays correct.
            let server_tick_tag = **auth_tick + 1;
            let local_tick_tag = **predicted_tick + 1;
            let cmd = GameCommands::Input {
                direction: input,
                tick: server_tick_tag,
                client_send_ms: client_epoch.now_ms(),
            };
            info!(
                "sending {:?} (server_tag={}, local_tag={})",
                cmd, server_tick_tag, local_tick_tag
            );
            client_connection.send_command(cmd);
            if is_local {
                input_log.push_back((local_tick_tag, input));
            }
        }
        input_queue.push_back(input);
    }
}

/// Apply one tick of deterministic simulation to `board`, treating
/// `local_dir` as the local snake's input and `last_applied[i]` as the most
/// recently-server-seen direction for other snakes. Returns `false` if the
/// tick errored (e.g., not enough inputs for the board's snake count), in
/// which case `board` is left untouched.
fn apply_tick(
    board: &mut Board,
    local_dir: Direction,
    last_applied: &[Option<Direction>],
    local_snake: u8,
) -> bool {
    // Build the inputs vec sized to the highest snake id present. We need at
    // least max(snake_id, local_snake) + 1 slots.
    let snake_count = board
        .snakes()
        .keys()
        .copied()
        .max()
        .map(|m| m as usize + 1)
        .unwrap_or(0)
        .max(local_snake as usize + 1)
        .max(last_applied.len());
    let mut inputs: Vec<Option<Direction>> = vec![None; snake_count];
    for (i, slot) in inputs.iter_mut().enumerate() {
        // For non-local snakes, reuse the server's most recently applied
        // direction. This is the standard "constant velocity" prediction —
        // it's wrong when the opponent turns, and the next snapshot will fix
        // that.
        if i as u8 == local_snake {
            *slot = Some(local_dir);
        } else if i < last_applied.len() {
            *slot = last_applied[i];
        }
    }
    let snapshot = board.clone();
    match board.tick_board_no_spawn(&inputs) {
        Ok(_) => true,
        Err(e) => {
            warn!("predicted tick failed: {}", e);
            *board = snapshot;
            false
        }
    }
}

/// What direction is the local snake heading on the displayed board? Used as
/// a fallback when the input log doesn't have anything queued for the next
/// predicted tick — predict it'll keep moving forward.
fn local_snake_dir(board: &Board, local_snake: u8) -> Direction {
    board
        .snakes()
        .get(&local_snake)
        .map(|s| s.dir)
        .unwrap_or(Direction::Right)
}

/// Update the local `TickClock` from a server snapshot. Pure (no Bevy access)
/// so the PLL math can be unit-tested directly.
///
/// `arrival` is the local-clock instant the snapshot was received.
/// `server_tick` is the tick that the snapshot represents.
/// `tick_interval_ms` is the server's current advertised period.
/// `one_way_ms` is the smoothed one-way trip; `None` cold-starts as zero.
///
/// Setpoint derivation: the server fires tick `server_tick` at server-time
/// `t_s`. That snapshot reaches us at `arrival = t_s + OWT`, i.e.
/// `t_s ≈ arrival - OWT`. The server fires its next tick (server_tick + 1)
/// `period` later — at local-time `arrival - OWT + period`. We want the
/// client's next predicted tick to fire at that same instant so prediction
/// runs in lockstep with the server's wall-clock cadence.
///
/// This does NOT depend on the client's `predicted_tick`. The client may be
/// running several predicted ticks ahead of the server (unacked inputs in
/// flight); that affects which board state we render, not when our local
/// clock fires next. The PLL only aligns the *phase* of the local tick
/// schedule with the server's tick schedule.
pub fn apply_snapshot_to_clock(
    clock: &mut TickClock,
    _server_tick: u64,
    arrival: Instant,
    tick_interval_ms: u32,
    one_way_ms: Option<f32>,
) {
    let new_period = Duration::from_millis(tick_interval_ms.max(1) as u64);
    let period_changed = new_period != clock.tick_period;
    clock.tick_period = new_period;

    let period_ns = clock.tick_period.as_nanos() as i64;
    let owt_ns = (one_way_ms.unwrap_or(0.0) * 1_000_000.0) as i64;

    // Desired local-clock instant for the server's NEXT tick: arrival was
    // when server tick `server_tick` happened (give or take OWT downstream),
    // so the next server tick fires one `period` later, minus the one-way
    // trip to get the message back to us.
    //
    // Snapping to "the next server tick" means the local fire instant lands
    // when the server actually fires; inputs we tag for our predicted_tick + 1
    // and send immediately have the full (period − OWT_up) to reach the
    // server before it processes that tick.
    let offset_ns = period_ns.saturating_sub(owt_ns);
    let desired_next_tick_at = add_signed_ns(arrival, offset_ns);

    // Signed phase error: positive means our current `next_tick_at` is earlier
    // than desired (we're firing too soon and need to push later).
    let phase_error_ns = signed_ns_between(desired_next_tick_at, clock.next_tick_at);

    let half_period_ns = period_ns / 2;
    let force_snap = !clock.locked || period_changed || phase_error_ns.abs() > half_period_ns;
    if force_snap {
        clock.next_tick_at = desired_next_tick_at;
        clock.last_snapped = true;
    } else {
        let nudge_ns = (phase_error_ns as f64 * PLL_ALPHA as f64) as i64;
        clock.next_tick_at = add_signed_ns(clock.next_tick_at, nudge_ns);
        clock.last_snapped = false;
    }
    clock.last_phase_error_ns = phase_error_ns;
    clock.locked = true;
}

/// `Instant + i64 nanoseconds`, handling negative offsets without panicking.
fn add_signed_ns(base: Instant, ns: i64) -> Instant {
    if ns >= 0 {
        base + Duration::from_nanos(ns as u64)
    } else {
        base.checked_sub(Duration::from_nanos((-ns) as u64))
            .unwrap_or(base)
    }
}

/// `target - current` in signed nanoseconds. Positive iff `target > current`.
fn signed_ns_between(target: Instant, current: Instant) -> i64 {
    if target >= current {
        target.duration_since(current).as_nanos() as i64
    } else {
        -(current.duration_since(target).as_nanos() as i64)
    }
}

pub struct AIPlugin;

impl Plugin for AIPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, ai_system.after(update_game));
    }
}

fn ai_system(
    mut input_queues: ResMut<SnakeInputs>,
    mut gizmos: Gizmos,
    mut ai_gizmos: Local<AIGizmos>,
    settings: Res<Settings>,
    board: Res<Board>,
    tick_clock: Res<TickClock>,
) {
    if tick_clock.just_fired || settings.tick_interval_ms.is_none() {
        // let ai = RandomWalk;
        let ai = TreeSearch {
            max_depth: 100,
            max_time: Duration::from_millis(5),
        };

        let mut new_ai_gizmos = AIGizmos::default();

        if let Ok(dir) = ai.chose_move(board.as_ref(), &mut Some(&mut new_ai_gizmos)) {
            let input_queue = &mut input_queues[0].input_queue;
            if settings.ai && input_queue.is_empty() {
                input_queue.push_back(dir);
            }
        }

        *ai_gizmos = new_ai_gizmos;
    }

    if let GizmoSetting::CycleBasis = settings.gizmos {
        // find cycle basis of the board
        let mut nodes = HashMap::new();
        let mut graph = Vec::new();
        for (pos, cell) in board.cells() {
            if !matches!(cell, Cell::Wall) {
                nodes.insert(pos, nodes.len());
                graph.push(Vec::new());
            }
        }
        for (node, index) in nodes.iter() {
            for dir in Direction::ALL {
                let next_node = *node + dir.as_vec2();
                if let Some(next_index) = nodes.get(&next_node) {
                    graph[*index].push(*next_index);
                }
            }
        }
        let cycles = cycle_basis(&graph);

        // show cycles
        let points: HashMap<_, _> = nodes.iter().map(|(pos, index)| (*index, *pos)).collect();
        for (index, cycle) in cycles.iter().enumerate() {
            let color = Color::srgb(
                (index as f32 / cycles.len() as f32).min(1.0),
                0.0,
                1.0 - (index as f32 / cycles.len() as f32).min(1.0),
            );
            let com = cycle
                .iter()
                .fold(Vec2::ZERO, |acc, &index| acc + points[&index].as_vec2())
                / cycle.len() as f32;
            let points = cycle
                .iter()
                .map(|&index| points[&index].as_vec2() - (points[&index].as_vec2() - com) * 0.1)
                .collect::<Vec<_>>();
            for i in 0..cycle.len() {
                let start = points[i];
                let end = points[(i + 1) % cycle.len()];
                ai_gizmos.arrows.push((start, end, color));
            }
        }

        // combine cycles
        let mut edges = HashMap::new();
        let mut index = 0;
        for (cell, neighbors) in graph.iter().enumerate() {
            for &neighbor in neighbors {
                if neighbor < cell {
                    edges.insert((neighbor, cell), index);
                    index += 1;
                }
            }
        }
        let mut edge_cycles = Vec::new();
        for cycle in cycles.iter() {
            let mut edge_cycle = Vec::new();
            for i in 0..cycle.len() {
                let mut a = cycle[i];
                let mut b = cycle[(i + 1) % cycle.len()];
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                edge_cycle.push(*edges.get(&(a.min(b), a.max(b))).unwrap());
            }
            edge_cycles.push(edge_cycle);
        }
    }

    if !matches!(settings.gizmos, GizmoSetting::None) {
        let board_pos = |pos: Vec2| {
            Vec2::new(
                pos.x as f32 - board.width() as f32 / 2.0 + 0.5,
                pos.y as f32 - board.height() as f32 / 2.0 + 0.5,
            )
        };
        for (start, end, color) in ai_gizmos.lines.iter() {
            gizmos.line_2d(board_pos(start.as_vec2()), board_pos(end.as_vec2()), *color);
        }
        for (start, end, color) in ai_gizmos.arrows.iter() {
            gizmos.arrow_2d(board_pos(*start), board_pos(*end), *color);
        }
        for (pos, color) in ai_gizmos.points.iter() {
            gizmos.circle_2d(board_pos(pos.as_vec2()), 0.3, *color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- RttEstimator ----

    #[test]
    fn rtt_estimator_first_echo_initializes_owt() {
        let mut e = RttEstimator::default();
        // First echo: client sent at 0, now is 100 → RTT 100, OWT 50.
        let consumed = e.observe_echo(0, 100);
        assert!(consumed);
        assert_eq!(e.last_rtt_ms, Some(100));
        assert_eq!(e.one_way_ms, Some(50.0));
    }

    #[test]
    fn rtt_estimator_skips_duplicate_echo() {
        // Server echoes the same client_send_ms across multiple snapshots
        // when the client is idle. Each later snapshot would compute a
        // larger RTT (now is growing), but we must not feed the EMA.
        let mut e = RttEstimator::default();
        assert!(e.observe_echo(1000, 1133));
        let owt_after_first = e.one_way_ms;
        // Same echo arrives again, much later in wall time.
        assert!(!e.observe_echo(1000, 5000));
        assert_eq!(e.one_way_ms, owt_after_first, "duplicate must not move OWT");
    }

    #[test]
    fn rtt_estimator_ema_smooths_spike() {
        let mut e = RttEstimator::default();
        // 20 distinct echoes, each RTT = 50ms (OWT = 25ms).
        for i in 0..20u32 {
            assert!(e.observe_echo(i * 1000, i * 1000 + 50));
        }
        let stable = e.one_way_ms.unwrap();
        assert!((stable - 25.0).abs() < 0.01, "should settle at 25ms");
        // One spike: RTT = 500ms, OWT = 250ms.
        assert!(e.observe_echo(20_000, 20_500));
        let after_spike = e.one_way_ms.unwrap();
        // beta = 0.2, so the new value is 0.8 * 25 + 0.2 * 250 = 70.
        assert!(
            (after_spike - 70.0).abs() < 0.5,
            "expected ~70ms after one spike, got {}",
            after_spike
        );
    }

    // ---- PLL ----

    /// Sanity helper: pretend the local clock is at `now`, feed a snapshot,
    /// and read the resulting state back out.
    fn run_snapshot(
        clock: &mut TickClock,
        server_tick: u64,
        arrival: Instant,
        period_ms: u32,
        _predicted_tick: u64,
        owt: Option<f32>,
    ) {
        apply_snapshot_to_clock(clock, server_tick, arrival, period_ms, owt);
    }

    #[test]
    fn pll_first_snapshot_with_no_owt_snaps_to_arrival_plus_period() {
        // Fresh, unlocked clock. First snapshot always snaps so a stale
        // initial `next_tick_at` (constructor default) doesn't poison the
        // lock.
        let now = Instant::now();
        let mut clock = TickClock::new(now);
        // Pretend a few seconds elapsed before the first snapshot.
        let arrival = now + Duration::from_secs(5);
        run_snapshot(&mut clock, 0, arrival, 133, 0, None);
        assert!(clock.last_snapped, "first snapshot must snap");
        assert!(clock.locked, "after first snapshot, clock is locked");
        let expected = arrival + Duration::from_millis(133);
        let diff_ns = signed_ns_between(clock.next_tick_at, expected).abs();
        assert!(diff_ns < 1_000_000, "diff {} ns too large", diff_ns);
    }

    #[test]
    fn pll_steady_state_with_owt_converges_near_zero_phase_error() {
        // 50 snapshots exactly 133 ms apart, fixed OWT = 40 ms. Between each
        // pair of snapshots the per-frame fire would advance
        // `next_tick_at += period` once, which we simulate here so the test
        // exercises the real steady-state geometry.
        let mut clock = TickClock::new(Instant::now());
        let mut arrival = Instant::now() + Duration::from_secs(1);
        for tick in 0..50u64 {
            run_snapshot(&mut clock, tick, arrival, 133, tick, Some(40.0));
            // Simulate the in-game per-frame advance: one tick fires between
            // each pair of snapshots once locked.
            clock.next_tick_at += clock.tick_period;
            arrival += Duration::from_millis(133);
        }
        let err_ms = clock.last_phase_error_ns as f64 / 1e6;
        assert!(
            err_ms.abs() < 0.5,
            "expected steady-state phase error near zero, got {:.3} ms",
            err_ms
        );
        assert!(!clock.last_snapped, "should be smooth-locked at steady-state");
    }

    #[test]
    fn pll_period_change_snaps() {
        let mut clock = TickClock::new(Instant::now());
        let mut arrival = Instant::now() + Duration::from_secs(1);
        for tick in 0..10u64 {
            run_snapshot(&mut clock, tick, arrival, 133, tick, Some(40.0));
            clock.next_tick_at += clock.tick_period;
            arrival += Duration::from_millis(133);
        }
        assert!(!clock.last_snapped, "expected lock before period change");
        // Now the server changes to 266 ms.
        run_snapshot(&mut clock, 10, arrival, 266, 10, Some(40.0));
        assert!(clock.last_snapped, "period change must force a snap");
        assert_eq!(clock.tick_period, Duration::from_millis(266));
    }

    #[test]
    fn pll_snaps_on_big_step() {
        let mut clock = TickClock::new(Instant::now());
        let mut arrival = Instant::now() + Duration::from_secs(1);
        for tick in 0..10u64 {
            run_snapshot(&mut clock, tick, arrival, 133, tick, Some(40.0));
            clock.next_tick_at += clock.tick_period;
            arrival += Duration::from_millis(133);
        }
        assert!(!clock.last_snapped, "expected lock");
        // Inject a snapshot 200 ms late (phase error > 66 ms = half period).
        arrival += Duration::from_millis(200);
        run_snapshot(&mut clock, 10, arrival, 133, 10, Some(40.0));
        assert!(
            clock.last_snapped,
            "200ms-late snapshot must trigger a snap, not a smooth filter"
        );
    }

    #[test]
    fn pll_handles_owt_larger_than_period() {
        // OWT > period: desired_next_tick_at = arrival + period - OWT is
        // negative-offset (in the past). Must not panic and must still snap.
        let mut clock = TickClock::new(Instant::now());
        let now = Instant::now() + Duration::from_secs(1);
        run_snapshot(&mut clock, 5, now, 100, 5, Some(250.0));
        // Period 100ms, OWT 250ms → desired = arrival - 150ms. Snap-only,
        // never panics. The per-frame loop will fire and advance once on the
        // next frame.
        assert!(clock.last_snapped);
        assert!(clock.next_tick_at < now);
    }

    #[test]
    fn pll_no_owt_means_lock_to_arrival() {
        // With no RTT echo yet, OWT = 0 — desired next_tick is exactly
        // `arrival + period`. Confirms the cold-start fallback behaves like
        // the pre-RTT plan.
        let mut clock = TickClock::new(Instant::now());
        let t = Instant::now();
        run_snapshot(&mut clock, 5, t, 100, 5, None);
        let expected = t + Duration::from_millis(100);
        let diff_ns = signed_ns_between(clock.next_tick_at, expected).abs();
        assert!(diff_ns < 1_000_000, "diff {} ns too large", diff_ns);
    }

    #[test]
    fn pll_with_owt_runs_ahead_of_arrival_by_period_minus_owt() {
        // With OWT > 0 the local fire instant lands `period - OWT` after
        // arrival, so we predict in lockstep with the server's wall-clock
        // instead of running OWT behind.
        let mut clock = TickClock::new(Instant::now());
        let t = Instant::now();
        run_snapshot(&mut clock, 5, t, 133, 5, Some(40.0));
        let expected = t + Duration::from_millis(133 - 40);
        let diff_ns = signed_ns_between(clock.next_tick_at, expected).abs();
        assert!(
            diff_ns < 1_000_000,
            "with OWT, next_tick_at should be arrival + (period - OWT); diff {} ns",
            diff_ns
        );
    }

    // ---- TickClock::interpolation ----

    #[test]
    fn interpolation_zero_right_after_advance() {
        let mut clock = TickClock::new(Instant::now());
        let now = Instant::now();
        clock.last_advance_at = now;
        clock.next_tick_at = now + Duration::from_millis(133);
        assert!(
            clock.interpolation(now) < 0.01,
            "right after a board advance interpolation should be ~0"
        );
    }

    #[test]
    fn interpolation_one_at_next_tick() {
        let mut clock = TickClock::new(Instant::now());
        let now = Instant::now();
        clock.last_advance_at = now - Duration::from_millis(133);
        clock.next_tick_at = now;
        assert!(
            clock.interpolation(now) > 0.99,
            "at the next-tick instant interpolation should be ~1"
        );
    }

    #[test]
    fn interpolation_half_at_midpoint() {
        let mut clock = TickClock::new(Instant::now());
        let now = Instant::now();
        clock.last_advance_at = now - Duration::from_millis(50);
        clock.next_tick_at = now + Duration::from_millis(50);
        let v = clock.interpolation(now);
        assert!(
            (v - 0.5).abs() < 0.01,
            "halfway between advance and next tick should be ~0.5, got {}",
            v
        );
    }
}
