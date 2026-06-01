//! Lightyear-based client networking.
//!
//! - The server runs the authoritative `Board::tick_movement` at the
//!   movement tick rate and broadcasts `TickConfirmed { tick, inputs,
//!   spawns, events }`.
//! - Clients re-run `tick_movement` with the broadcast inputs and
//!   `apply_spawns` with the broadcast positions. No client-side RNG, no
//!   rollback — visual responsiveness comes from the renderer's existing
//!   head-lean against the local input queue.
//!
//! State exposed to the rest of the app:
//! - [`Board`] resource — updated on every `TickConfirmed`.
//! - [`RenderClock`] resource — wall-clock interpolation factor used by
//!   the renderer.
//! - [`InputQueues`] resource — client-local; head-lean preview only.
//! - [`NetStatus`] / [`ConnectStage`] — UI status indicator.

use crate::ClientState;
use crate::lobby::{CurrentLobby, Role};
use crate::notice::Notice;
use bevy::prelude::*;
use bevy_snake::board::{Board, Direction, PlayerCount};
use bevy_snake::game_server::{GameSimPlugin, SERVER_TICK_HZ, SESSION_KEY, SessionState};
use bevy_snake::lobby_proto::GameSessionCreds;
use bevy_snake::net_proto::{
    GameProtocolPlugin, InputMsg, PROTOCOL_ID, ReliableChannel, RequestStartRound, RoundEnded,
    RoundStarting, TickConfirmed, UnreliableChannel, Welcome,
};
use bevy_snake::settings::GameSettings;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::server::{LinkOf, NetcodeServer, ServerPlugins, Start, Started, Stop};
use lightyear::websocket::client::{WebSocketClientIo, WebSocketTarget};
use lightyear::websocket::prelude::client::ClientConfig as WsClientConfig;
use std::collections::VecDeque;
use std::time::Duration;

/// Tick duration used by both client and server Lightyear plugins. The
/// actual game-sim tick rate (movement rate) is driven by `GameSettings.speed`
/// and runs on top of this base Bevy fixed-timestep.
pub const TICK_HZ: f64 = 30.0;

/// Wall-clock interpolation factor used by the renderer. Replaces the old
/// `MovementFrame::movement_progress`. Goes 0→1 over one movement tick.
#[derive(Resource, Debug, Clone)]
pub struct RenderClock {
    /// Time the most recent `TickConfirmed` was applied.
    pub last_tick_at: f64,
    /// Expected interval between movement ticks (seconds).
    pub tick_period: f64,
}

impl Default for RenderClock {
    fn default() -> Self {
        Self {
            last_tick_at: 0.0,
            tick_period: 8.0 / 60.0,
        }
    }
}

impl RenderClock {
    /// 0..1 fraction of the way through the current movement tick, based on
    /// wall clock since the last `TickConfirmed` arrived.
    pub fn movement_progress(&self, now: f64) -> f32 {
        if self.tick_period <= 0.0 {
            return 0.0;
        }
        ((now - self.last_tick_at) / self.tick_period).clamp(0.0, 1.0) as f32
    }
}

/// Per-player queue of upcoming turn directions. Client-local; used by the
/// renderer for head-lean preview. The authoritative input that actually
/// moves a snake is the one the server includes in `TickConfirmed`.
#[derive(Resource, Default, Clone, Debug)]
pub struct InputQueues(pub Vec<Vec<Direction>>);

impl InputQueues {
    pub fn front(&self, player: usize) -> Option<Direction> {
        self.0.get(player).and_then(|q| q.first().copied())
    }
}

/// Local-only buffer that captures key presses between frames so a press
/// survives the small gap before the next input-send tick.
#[derive(Resource, Default)]
pub struct PendingInput {
    pub direction: Option<Direction>,
    pub restart: bool,
}

pub const MAX_QUEUE_LEN: usize = 3;

/// Visual-only knob for the head-lean crossover in the renderer.
#[derive(Resource, Clone, Copy, Debug)]
pub struct InterpolationPhase(pub f32);

impl Default for InterpolationPhase {
    fn default() -> Self {
        Self(0.3)
    }
}

/// High-level connection-lifecycle status, displayed in the UI.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct NetStatus {
    pub stage: ConnectStage,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum ConnectStage {
    #[default]
    Idle,
    LobbyConnecting,
    LobbyConnected,
    /// Game-server credentials received from lobby; opening Lightyear link.
    ConnectingToGameServer,
    /// Connected to the game server and playing.
    Playing,
}

/// Credentials handed by the lobby service for the current session, awaiting
/// the netcode/Lightyear client to be spun up against them.
#[derive(Resource, Clone, Debug)]
pub struct PendingSession {
    pub creds: GameSessionCreds,
}

/// Marker on the Lightyear client Link entity.
#[derive(Component)]
pub struct GameClient;

/// Marker on the in-process server entity used for solo play. The embedded
/// `GameSimPlugin` runs the authoritative sim; the local client connects to
/// it as a host-client (no socket, no netcode handshake) so solo works
/// identically on native and wasm.
#[derive(Component)]
pub struct SoloServer;

/// Buffer of recent `TickConfirmed` messages, kept for the
/// render-behind interpolation buffer. We apply them in tick order in
/// `apply_confirmed_ticks` and trim once applied.
#[derive(Resource, Default)]
pub struct TickInbox {
    pub queue: VecDeque<TickConfirmed>,
}

/// Server-assigned identity for this client, set on `Welcome`. `None` until
/// the first Welcome arrives.
#[derive(Resource, Default, Debug, Clone)]
pub struct SessionIdentity {
    pub player_id: Option<u8>,
    pub is_spectator: bool,
    pub round: u32,
    pub last_tick: u32,
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        let default_settings = GameSettings::default();
        let default_players = default_settings.board.players as usize;

        app.add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / TICK_HZ),
        })
        // The embedded authoritative server. It stays dormant during
        // multiplayer (no local server entity is ever Started, so
        // `tick_world` early-returns and no `ClientOf` observers fire) and
        // is driven over an in-memory crossbeam link for solo play.
        .add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / SERVER_TICK_HZ),
        })
        .add_plugins(GameProtocolPlugin)
        .add_plugins(GameSimPlugin)
        .insert_resource(Board::new(default_settings.board))
        .insert_resource(InputQueues(vec![Vec::new(); default_players]))
        .insert_resource(PendingInput::default())
        .insert_resource(InterpolationPhase::default())
        .insert_resource(RenderClock::default())
        .insert_resource(default_settings)
        .init_resource::<NetStatus>()
        .init_resource::<SessionIdentity>()
        .init_resource::<TickInbox>()
        .add_observer(connect_solo_client)
        .add_systems(OnEnter(ClientState::WaitingForOpponent), enter_waiting)
        .add_systems(
            Update,
            connect_to_game_server.run_if(bevy::ecs::schedule::common_conditions::resource_added::<PendingSession>),
        )
        .add_systems(OnExit(ClientState::Playing), close_session)
        .add_systems(OnEnter(ClientState::Browsing), reset_to_browsing_stage)
        .add_systems(
            Update,
            (
                buffer_local_input,
                send_local_input,
                receive_welcome,
                receive_tick_confirmed,
                receive_round_ended,
                receive_round_starting,
                apply_confirmed_ticks,
            ),
        );
    }
}

/// Lobby server location for the lobby WebSocket. wasm derives this from
/// the page origin; native reads `SERVER_URL` at compile time.
pub(crate) fn server_url(path: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        same_origin_ws_url(path).unwrap_or_else(|| format!("ws://localhost:1234{}", path))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let base = option_env!("SERVER_URL")
            .unwrap_or("ws://localhost:1234")
            .trim_end_matches('/');
        format!("{}{}", base, path)
    }
}

#[cfg(target_arch = "wasm32")]
fn same_origin_ws_url(path: &str) -> Option<String> {
    let location = web_sys::window()?.location();
    let protocol = location.protocol().ok()?;
    let host = location.host().ok()?;
    if host.is_empty() {
        return None;
    }
    let scheme = if protocol == "https:" { "wss" } else { "ws" };
    Some(format!("{}://{}{}", scheme, host, path))
}

/// `OnEnter(WaitingForOpponent)` — reset per-session client state. Does
/// **not** open the Lightyear link: credentials arrive asynchronously via
/// `ServerMsg::GameSessionReady`, which inserts a `PendingSession` resource
/// and triggers [`connect_to_game_server`].
fn enter_waiting(
    mut commands: Commands,
    mut settings: ResMut<GameSettings>,
    mut board: ResMut<Board>,
    mut queues: ResMut<InputQueues>,
    mut identity: ResMut<SessionIdentity>,
    mut inbox: ResMut<TickInbox>,
    mut session: ResMut<SessionState>,
    mut status: ResMut<NetStatus>,
    current: Res<CurrentLobby>,
) {
    *identity = SessionIdentity::default();
    inbox.queue.clear();
    settings.board.players = PlayerCount::One;
    *board = Board::new(settings.board);
    *queues = InputQueues(vec![Vec::new(); 1]);

    if current.role == Role::Solo {
        start_solo_session(&mut commands, &mut session, &mut status, *settings);
        return;
    }
    // Network roles: wait for GameSessionReady → PendingSession, which
    // triggers `connect_to_game_server`.
}

/// Spin up the embedded authoritative server for solo play. The host-client
/// is spawned later by [`connect_solo_client`], once the server is `Started`
/// — the host-server `connect` observer only promotes a client to
/// `Connected + ClientOf + HostClient` if its target server is already
/// started, and `Started` lands a frame after the `Start` trigger.
///
/// No socket and no netcode handshake: messages loop back in-process via
/// `HostClient`, so from `Welcome` onward the existing client systems
/// (`receive_welcome`, `send_local_input`, `apply_confirmed_ticks`,
/// `receive_round_*`) drive the game exactly as in multiplayer.
fn start_solo_session(
    commands: &mut Commands,
    session: &mut SessionState,
    status: &mut NetStatus,
    settings: GameSettings,
) {
    // Seed the embedded server with the player's chosen settings before the
    // local client connects and `handle_connected` builds the board.
    session.reset_for(settings);

    // `NetcodeServer` requires `Server`, and `Start` inserts `Started` — all
    // host-server needs (netcode itself is skipped for the host-client). No
    // IO component, so nothing binds a socket.
    let server = commands
        .spawn((
            // Fully-qualified: the `client::*` glob in scope brings a
            // different `NetcodeConfig` (used by `NetcodeClient` elsewhere in
            // this file), so name the server one explicitly.
            NetcodeServer::new(lightyear::prelude::server::NetcodeConfig {
                protocol_id: PROTOCOL_ID,
                private_key: SESSION_KEY,
                ..default()
            }),
            SoloServer,
            Name::from("SoloServer"),
        ))
        .id();
    commands.trigger(Start { entity: server });

    status.stage = ConnectStage::ConnectingToGameServer;
    info!("solo session started (embedded host-server)");
}

/// Observer: when the solo server finishes starting, spawn the host-client.
/// `LinkOf { server }` + `Client` is what the host-server `connect` observer
/// keys on; `GameClient` lets the existing client systems find it.
fn connect_solo_client(
    trigger: On<Add, Started>,
    servers: Query<(), With<SoloServer>>,
    mut commands: Commands,
) {
    let server = trigger.entity;
    if servers.get(server).is_err() {
        return;
    }
    let client = commands
        .spawn((
            Client::default(),
            LinkOf { server },
            GameClient,
            Name::from("SoloClient"),
        ))
        .id();
    commands.trigger(Connect { entity: client });
    info!("solo host-client connecting to embedded server");
}

/// Runs once when `PendingSession` is inserted. Spawns the Lightyear client
/// link and triggers Connect. Fires for hosts after Start, and for joiners
/// as soon as they Join (the lobby server emits `GameSessionReady` for both).
fn connect_to_game_server(
    mut commands: Commands,
    mut status: ResMut<NetStatus>,
    pending: Res<PendingSession>,
) {
    let creds = pending.creds.clone();
    // `endpoint` may be an absolute URL or a path like `/game` (resolved
    // against the page origin so the deploy stays single-port).
    let connect_url = if creds.endpoint.starts_with("ws://") || creds.endpoint.starts_with("wss://")
    {
        creds.endpoint.clone()
    } else {
        server_url(&creds.endpoint)
    };
    info!(
        "connecting Lightyear client to {} (client_id={}, netcode_server={})",
        connect_url, creds.client_id, creds.netcode_server_addr
    );
    status.stage = ConnectStage::ConnectingToGameServer;

    let auth = Authentication::Manual {
        server_addr: creds.netcode_server_addr,
        client_id: creds.client_id,
        private_key: creds.private_key,
        protocol_id: creds.protocol_id,
    };
    let target = WebSocketTarget::Url(connect_url);
    // On native, skip cert validation so dev against the server's self-signed
    // cert works. On wasm `ClientConfig` is a unit-struct stub — the browser
    // owns TLS and will reject self-signed certs regardless, so prod needs a
    // real cert and dev uses ws://.
    #[cfg(not(target_family = "wasm"))]
    let ws_config = WsClientConfig::builder().with_no_cert_validation();
    #[cfg(target_family = "wasm")]
    let ws_config = WsClientConfig::default();

    let netcode = match NetcodeClient::new(auth, NetcodeConfig::default()) {
        Ok(c) => c,
        Err(e) => {
            warn!("NetcodeClient::new failed: {:?}", e);
            return;
        }
    };
    let entity = commands
        .spawn((
            Client::default(),
            Link::new(None),
            netcode,
            WebSocketClientIo { config: ws_config, target },
            GameClient,
            Name::from("GameClient"),
        ))
        .id();
    commands.trigger(Connect { entity });
}

fn close_session(
    mut commands: Commands,
    clients: Query<Entity, With<GameClient>>,
    solo_servers: Query<Entity, With<SoloServer>>,
    mut inbox: ResMut<TickInbox>,
    mut session: ResMut<SessionState>,
) {
    teardown_session(&mut commands, &clients, &solo_servers, &mut session);
    inbox.queue.clear();
    commands.remove_resource::<PendingSession>();
}

fn reset_to_browsing_stage(
    mut commands: Commands,
    mut status: ResMut<NetStatus>,
    clients: Query<Entity, With<GameClient>>,
    solo_servers: Query<Entity, With<SoloServer>>,
    mut session: ResMut<SessionState>,
) {
    teardown_session(&mut commands, &clients, &solo_servers, &mut session);
    commands.remove_resource::<PendingSession>();
    if !matches!(
        status.stage,
        ConnectStage::LobbyConnected | ConnectStage::LobbyConnecting | ConnectStage::Idle
    ) {
        status.stage = ConnectStage::LobbyConnected;
    }
}

/// Despawn the local client + (for solo) the embedded server, and reset the
/// embedded session so a later solo game starts clean.
fn teardown_session(
    commands: &mut Commands,
    clients: &Query<Entity, With<GameClient>>,
    solo_servers: &Query<Entity, With<SoloServer>>,
    session: &mut SessionState,
) {
    for e in clients.iter() {
        commands.entity(e).despawn();
    }
    for e in solo_servers.iter() {
        commands.trigger(Stop { entity: e });
        commands.entity(e).despawn();
    }
    session.clear();
}

/// Capture local key presses into PendingInput each Update.
fn buffer_local_input(keys: Res<ButtonInput<KeyCode>>, mut pending: ResMut<PendingInput>) {
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        pending.direction = Some(Direction::Up);
    } else if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        pending.direction = Some(Direction::Down);
    } else if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::KeyA) {
        pending.direction = Some(Direction::Left);
    } else if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::KeyD) {
        pending.direction = Some(Direction::Right);
    }
    if keys.just_pressed(KeyCode::Space) {
        pending.restart = true;
    }
}

/// Send the buffered direction to the server. Also push it onto the local
/// InputQueue for the renderer's head-lean preview.
fn send_local_input(
    mut pending: ResMut<PendingInput>,
    mut queues: ResMut<InputQueues>,
    identity: Res<SessionIdentity>,
    mut sender_q: Query<&mut MessageSender<InputMsg>, With<GameClient>>,
    mut restart_q: Query<&mut MessageSender<RequestStartRound>, With<GameClient>>,
) {
    let Some(dir) = pending.direction.take() else {
        // No direction; still might need to send a restart.
        if pending.restart {
            pending.restart = false;
            if let Ok(mut s) = restart_q.single_mut() {
                let _ = s.send::<ReliableChannel>(RequestStartRound);
            }
        }
        return;
    };

    // Head-lean preview: push onto local queue for our player id.
    if let Some(pid) = identity.player_id {
        let idx = pid as usize;
        if queues.0.len() <= idx {
            queues.0.resize(idx + 1, Vec::new());
        }
        let q = &mut queues.0[idx];
        if q.last().copied() != Some(dir) && q.len() < MAX_QUEUE_LEN {
            q.push(dir);
        }
    }

    if let Ok(mut s) = sender_q.single_mut() {
        let target_tick = identity.last_tick.saturating_add(2);
        let _ = s.send::<UnreliableChannel>(InputMsg {
            target_tick,
            dir: Some(dir),
        });
    }

    if pending.restart {
        pending.restart = false;
        if let Ok(mut s) = restart_q.single_mut() {
            let _ = s.send::<ReliableChannel>(RequestStartRound);
        }
    }
}

fn receive_welcome(
    mut welcome_q: Query<&mut MessageReceiver<Welcome>, With<GameClient>>,
    mut settings: ResMut<GameSettings>,
    mut board: ResMut<Board>,
    mut queues: ResMut<InputQueues>,
    mut identity: ResMut<SessionIdentity>,
    mut clock: ResMut<RenderClock>,
    mut status: ResMut<NetStatus>,
    time: Res<Time>,
) {
    let Ok(mut rx) = welcome_q.single_mut() else {
        return;
    };
    for msg in rx.receive() {
        info!(
            "welcome: role={:?} tick={} round={} tick_hz={}",
            msg.role, msg.tick, msg.round, msg.tick_hz
        );
        *settings = msg.settings;
        let player_count = settings.board.players as usize;
        *board = msg.board.clone();
        queues.0.resize(player_count, Vec::new());
        identity.player_id = match msg.role {
            bevy_snake::net_proto::Role::Player { id } => Some(id),
            bevy_snake::net_proto::Role::Spectator => None,
        };
        identity.is_spectator = matches!(msg.role, bevy_snake::net_proto::Role::Spectator);
        identity.round = msg.round;
        identity.last_tick = msg.tick;
        clock.tick_period = 1.0 / msg.tick_hz as f64;
        clock.last_tick_at = time.elapsed_secs_f64();
        status.stage = ConnectStage::Playing;
    }
}

fn receive_tick_confirmed(
    mut q: Query<&mut MessageReceiver<TickConfirmed>, With<GameClient>>,
    mut inbox: ResMut<TickInbox>,
) {
    let Ok(mut rx) = q.single_mut() else {
        return;
    };
    for msg in rx.receive() {
        inbox.queue.push_back(msg);
    }
}

/// Apply queued `TickConfirmed`s in order. Render-behind buffer is one tick
/// — we apply the oldest message as soon as we have it, and let the next
/// arrival interpolate against it. (Phase-sync logic from the plan is
/// deferred to a follow-up; this gets the basic flow working.)
fn apply_confirmed_ticks(
    mut inbox: ResMut<TickInbox>,
    mut board: ResMut<Board>,
    mut queues: ResMut<InputQueues>,
    mut identity: ResMut<SessionIdentity>,
    mut clock: ResMut<RenderClock>,
    time: Res<Time>,
) {
    while let Some(msg) = inbox.queue.pop_front() {
        if msg.tick <= identity.last_tick {
            continue;
        }
        let outcome = match board.tick_movement(&msg.inputs) {
            Ok(o) => o,
            Err(e) => {
                warn!("tick_movement error: {}", e);
                continue;
            }
        };
        // Server-decided event count should match; we trust the server's
        // events list rather than the locally-derived one (renderer reads
        // board state, not events).
        let _ = outcome;
        board.apply_spawns(
            &bevy_snake::board::TickOutcome {
                events: msg.events.clone(),
                apples_to_spawn: msg.spawns.apples.len(),
            },
            &msg.spawns,
        );
        // Pop the front of *our* local input queue so the head-lean preview
        // advances to the next press.
        if let Some(pid) = identity.player_id {
            if let Some(q) = queues.0.get_mut(pid as usize) {
                if !q.is_empty() {
                    q.remove(0);
                }
            }
        }
        identity.last_tick = msg.tick;
        clock.last_tick_at = time.elapsed_secs_f64();
    }
}

fn receive_round_ended(
    mut q: Query<&mut MessageReceiver<RoundEnded>, With<GameClient>>,
    mut notice: ResMut<Notice>,
    time: Res<Time<Real>>,
) {
    let Ok(mut rx) = q.single_mut() else { return };
    for msg in rx.receive() {
        info!("round {} ended", msg.round);
        notice.warn(&time, "Round over");
    }
}

fn receive_round_starting(
    mut q: Query<&mut MessageReceiver<RoundStarting>, With<GameClient>>,
    mut board: ResMut<Board>,
    mut queues: ResMut<InputQueues>,
    mut settings: ResMut<GameSettings>,
    mut identity: ResMut<SessionIdentity>,
    mut inbox: ResMut<TickInbox>,
    mut clock: ResMut<RenderClock>,
    time: Res<Time>,
) {
    let Ok(mut rx) = q.single_mut() else { return };
    for msg in rx.receive() {
        info!("round {} starting (role={:?})", msg.round, msg.your_role);
        *settings = msg.settings;
        let player_count = settings.board.players as usize;
        *board = msg.board.clone();
        queues.0.clear();
        queues.0.resize(player_count, Vec::new());
        identity.player_id = match msg.your_role {
            bevy_snake::net_proto::Role::Player { id } => Some(id),
            bevy_snake::net_proto::Role::Spectator => None,
        };
        identity.is_spectator = matches!(msg.your_role, bevy_snake::net_proto::Role::Spectator);
        identity.round = msg.round;
        identity.last_tick = msg.tick;
        inbox.queue.clear();
        clock.last_tick_at = time.elapsed_secs_f64();
    }
}
