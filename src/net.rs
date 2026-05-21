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
use bevy_snake::lobby_proto::GameSessionCreds;
use bevy_snake::net_proto::{
    GameProtocolPlugin, InputMsg, ReliableChannel, RequestStartRound, RoundEnded,
    RoundStarting, TickConfirmed, UnreliableChannel, Welcome,
};
use bevy_snake::settings::GameSettings;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
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
    /// Link open, waiting on Welcome.
    AwaitingWelcome,
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
        .add_plugins(GameProtocolPlugin)
        .insert_resource(Board::new(default_settings.board))
        .insert_resource(InputQueues(vec![Vec::new(); default_players]))
        .insert_resource(PendingInput::default())
        .insert_resource(InterpolationPhase::default())
        .insert_resource(RenderClock::default())
        .insert_resource(default_settings)
        .init_resource::<NetStatus>()
        .init_resource::<SessionIdentity>()
        .init_resource::<TickInbox>()
        .add_systems(OnEnter(ClientState::WaitingForOpponent), open_session)
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

/// Enter WaitingForOpponent: spin up a Lightyear client against the lobby's
/// game server. Solo runs an in-process server (see `solo_server` module),
/// connecting via the same Lightyear client.
fn open_session(
    mut commands: Commands,
    mut settings: ResMut<GameSettings>,
    mut board: ResMut<Board>,
    mut queues: ResMut<InputQueues>,
    mut status: ResMut<NetStatus>,
    mut identity: ResMut<SessionIdentity>,
    mut inbox: ResMut<TickInbox>,
    current: Res<CurrentLobby>,
    pending: Option<Res<PendingSession>>,
) {
    *identity = SessionIdentity::default();
    inbox.queue.clear();

    if current.role == Role::Solo {
        // For now treat solo as "not yet wired" — fall through to needing
        // a session. The host server runs in-process for solo.
        settings.board.players = PlayerCount::One;
        *board = Board::new(settings.board);
        *queues = InputQueues(vec![Vec::new(); 1]);
        status.stage = ConnectStage::ConnectingToGameServer;
        info!("solo session — server runs in-process");
        // TODO: spawn embedded server App for solo mode.
        return;
    }

    let Some(pending) = pending.as_deref() else {
        warn!("open_session entered without PendingSession credentials");
        status.stage = ConnectStage::ConnectingToGameServer;
        return;
    };
    let creds = pending.creds.clone();
    settings.board.players = PlayerCount::One;
    *board = Board::new(settings.board);
    *queues = InputQueues(vec![Vec::new(); 1]);

    let endpoint = creds.endpoint.clone();
    info!(
        "connecting Lightyear client to {} (client_id={})",
        endpoint, creds.client_id
    );
    status.stage = ConnectStage::ConnectingToGameServer;

    let auth = Authentication::Manual {
        server_addr: parse_ws_addr(&endpoint),
        client_id: creds.client_id,
        private_key: creds.private_key,
        protocol_id: creds.protocol_id,
    };
    let target = WebSocketTarget::Url(endpoint);
    // Dev: don't validate certs on the WebSocket TLS handshake — the
    // server's self-signed cert wouldn't validate against any root CA.
    let ws_config = WsClientConfig::builder().with_no_cert_validation();

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
    let _ = entity;
    commands.trigger(Connect { entity });
}

/// Best-effort: parse `ws[s]://host:port` into a SocketAddr used by the
/// Lightyear netcode `server_addr` field. We don't actually open a UDP
/// socket here — Lightyear's WebSocket IO does the connecting — but
/// netcode still wants a SocketAddr in its auth token.
fn parse_ws_addr(url: &str) -> std::net::SocketAddr {
    let stripped = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    let host_port = stripped.split('/').next().unwrap_or(stripped);
    host_port
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap())
}

fn close_session(
    mut commands: Commands,
    clients: Query<Entity, With<GameClient>>,
    mut inbox: ResMut<TickInbox>,
) {
    for e in clients.iter() {
        commands.entity(e).despawn();
    }
    inbox.queue.clear();
    commands.remove_resource::<PendingSession>();
}

fn reset_to_browsing_stage(
    mut commands: Commands,
    mut status: ResMut<NetStatus>,
    clients: Query<Entity, With<GameClient>>,
) {
    for e in clients.iter() {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<PendingSession>();
    if !matches!(
        status.stage,
        ConnectStage::LobbyConnected | ConnectStage::LobbyConnecting | ConnectStage::Idle
    ) {
        status.stage = ConnectStage::LobbyConnected;
    }
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
