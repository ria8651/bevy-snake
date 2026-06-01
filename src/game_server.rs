//! Authoritative game simulation, shared between the standalone server
//! binary and the in-process server used for solo play.
//!
//! [`GameSimPlugin`] is transport-agnostic: it owns the [`SessionState`],
//! runs `Board::tick_movement` per movement tick, and broadcasts
//! `TickConfirmed` deltas to whatever clients are connected — over a real
//! WebSocket (multiplayer, via [`GameServerPlugin`]) or an in-memory
//! crossbeam link (solo, wired up in `net.rs`).
//!
//! The host App is responsible for adding `ServerPlugins` + the
//! [`GameProtocolPlugin`]; `GameSimPlugin` assumes they are present so it
//! doesn't double-register messages/channels.

use crate::board::{Board, Direction, PlayerCount};
use crate::net_proto::{
    InputMsg, Role as PlayerRole, ReliableChannel, RequestJoinNextRound, RequestStartRound,
    RoundEnded, RoundStarting, TickConfirmed, UnreliableChannel, Welcome,
};
use crate::settings::GameSettings;
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::collections::HashMap;

/// Shared netcode private key. The lobby service uses the same value when
/// allocating credentials, and solo mode reuses it for its loopback link.
pub const SESSION_KEY: [u8; 32] = [
    0x42, 0x9a, 0x11, 0x73, 0xfe, 0x07, 0xb1, 0x55, 0xc7, 0x80, 0x14, 0x32, 0x6d, 0x83, 0xaa, 0x4c,
    0x9e, 0xf1, 0x05, 0x6b, 0x33, 0xd2, 0x90, 0x8f, 0x71, 0x18, 0x4e, 0x29, 0x77, 0x0d, 0xbc, 0xee,
];

/// Tick rate used by the Lightyear server's fixed-step.
pub const SERVER_TICK_HZ: f64 = 30.0;

/// Transport-agnostic authoritative simulation. Add this to an App that
/// already has `ServerPlugins` + `GameProtocolPlugin`. The transport (which
/// server link entities exist) is set up by the host — `GameServerPlugin`
/// for WebSocket, or the solo bootstrap in `net.rs` for crossbeam.
pub struct GameSimPlugin;

impl Plugin for GameSimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SessionState::default())
            .add_systems(Startup, init_session)
            .add_observer(handle_new_client)
            .add_observer(handle_connected)
            .add_systems(Update, (receive_inputs, receive_join_next, receive_start))
            .add_systems(FixedUpdate, tick_world);
    }
}

#[derive(Resource, Default)]
pub struct SessionState {
    board: Option<Board>,
    settings: GameSettings,
    tick: u32,
    round: u32,
    /// player_id → (PeerId, latest_input)
    players: HashMap<u8, PlayerSlot>,
    /// Spectator peer ids who've asked to be promoted at the next round.
    spectators_wanting_in: Vec<PeerId>,
    /// All currently-connected peers + their role.
    peers: HashMap<PeerId, PlayerRole>,
    /// Wall ticks accumulated; movement tick when `frame_counter % frames_per_movement == 0`.
    frame_counter: u32,
    rng_seed: u64,
}

impl SessionState {
    /// Reset the session for a fresh game with the given settings. Used by
    /// the solo bootstrap so the embedded server honors the player's chosen
    /// board/speed before the local client connects.
    pub fn reset_for(&mut self, settings: GameSettings) {
        self.settings = settings;
        self.board = None;
        self.tick = 0;
        self.round = 0;
        self.frame_counter = 0;
        self.players.clear();
        self.spectators_wanting_in.clear();
        self.peers.clear();
        self.rng_seed = rand::random::<u64>();
    }

    /// Tear the session back down to an idle, board-less state so a later
    /// game starts clean. Called when leaving a solo session.
    pub fn clear(&mut self) {
        self.reset_for(GameSettings::default());
    }
}

#[derive(Debug, Clone, Default)]
struct PlayerSlot {
    peer: Option<PeerId>,
    latest_input: Option<Direction>,
}

/// Initialize the session to an idle, waiting state. The actual game starts
/// when the first client connects (`handle_connected` builds the board) and
/// `tick_world` begins advancing it.
fn init_session(mut session: ResMut<SessionState>) {
    session.settings = GameSettings::default();
    session.board = None;
    session.tick = 0;
    session.round = 0;
    session.rng_seed = rand::random::<u64>();
}

/// Attach per-client receivers + sender when a new client link is added.
fn handle_new_client(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands.entity(trigger.entity).insert((
        MessageReceiver::<InputMsg>::default(),
        MessageReceiver::<RequestJoinNextRound>::default(),
        MessageReceiver::<RequestStartRound>::default(),
        MessageSender::<Welcome>::default(),
        MessageSender::<TickConfirmed>::default(),
        MessageSender::<RoundEnded>::default(),
        MessageSender::<RoundStarting>::default(),
    ));
}

/// Send Welcome on `Connected` event for a client link.
fn handle_connected(
    trigger: On<Add, Connected>,
    mut q: Query<(&RemoteId, &mut MessageSender<Welcome>), With<ClientOf>>,
    mut session: ResMut<SessionState>,
) {
    let Ok((RemoteId(peer_id), mut sender)) = q.get_mut(trigger.entity) else {
        return;
    };
    // Decide role: assign Player if we have room and the session isn't
    // mid-round (or round 0); otherwise Spectator.
    let role = assign_role(&mut session, *peer_id);
    let board = match &session.board {
        Some(b) => b.clone(),
        None => {
            session.board = Some(Board::new(session.settings.board));
            session.board.as_ref().unwrap().clone()
        }
    };
    info!("welcome peer {:?} as {:?}", peer_id, role);
    let _ = sender.send::<ReliableChannel>(Welcome {
        role,
        settings: session.settings,
        board,
        tick: session.tick,
        round: session.round,
        tick_hz: tick_hz(&session.settings) as f32,
    });
    session.peers.insert(*peer_id, role);
}

fn assign_role(session: &mut SessionState, peer: PeerId) -> PlayerRole {
    if session.round == 0 {
        let max = session.settings.board.players as u8;
        for id in 0..max {
            if !session.players.contains_key(&id) {
                session.players.insert(
                    id,
                    PlayerSlot {
                        peer: Some(peer),
                        latest_input: None,
                    },
                );
                return PlayerRole::Player { id };
            }
        }
        PlayerRole::Spectator
    } else {
        PlayerRole::Spectator
    }
}

fn receive_inputs(
    mut q: Query<(&RemoteId, &mut MessageReceiver<InputMsg>), With<ClientOf>>,
    mut session: ResMut<SessionState>,
) {
    for (RemoteId(peer), mut rx) in q.iter_mut() {
        for msg in rx.receive() {
            // Find the player_id owned by this peer.
            for (pid, slot) in session.players.iter_mut() {
                if slot.peer == Some(*peer) {
                    slot.latest_input = msg.dir;
                    let _ = pid;
                    break;
                }
            }
        }
    }
}

fn receive_join_next(
    mut q: Query<(&RemoteId, &mut MessageReceiver<RequestJoinNextRound>), With<ClientOf>>,
    mut session: ResMut<SessionState>,
) {
    for (RemoteId(peer), mut rx) in q.iter_mut() {
        for _ in rx.receive() {
            if !session.spectators_wanting_in.contains(peer) {
                session.spectators_wanting_in.push(*peer);
            }
        }
    }
}

fn receive_start(
    mut q: Query<&mut MessageReceiver<RequestStartRound>, With<ClientOf>>,
    mut session: ResMut<SessionState>,
    mut welcomers: Query<(&RemoteId, &mut MessageSender<RoundStarting>), With<ClientOf>>,
) {
    let mut any_start = false;
    for mut rx in q.iter_mut() {
        for _ in rx.receive() {
            any_start = true;
        }
    }
    if !any_start {
        return;
    }
    start_new_round(&mut session, &mut welcomers);
}

fn start_new_round(
    session: &mut SessionState,
    welcomers: &mut Query<(&RemoteId, &mut MessageSender<RoundStarting>), With<ClientOf>>,
) {
    session.round = session.round.saturating_add(1);
    session.tick = 0;
    session.frame_counter = 0;
    // Promote spectators (cap at MAX_PLAYERS).
    let max = session.settings.board.players as u8;
    let promotions: Vec<PeerId> = session.spectators_wanting_in.drain(..).collect();
    let mut new_players: HashMap<u8, PlayerSlot> = HashMap::new();
    let mut next_id: u8 = 0;
    // Re-assign incumbent players first
    let incumbents: Vec<PeerId> = session.players.values().filter_map(|s| s.peer).collect();
    for peer in incumbents {
        if next_id >= max {
            break;
        }
        new_players.insert(
            next_id,
            PlayerSlot {
                peer: Some(peer),
                latest_input: None,
            },
        );
        next_id += 1;
    }
    for peer in promotions {
        if next_id >= max {
            break;
        }
        new_players.insert(
            next_id,
            PlayerSlot {
                peer: Some(peer),
                latest_input: None,
            },
        );
        next_id += 1;
    }
    session.players = new_players;
    // Compute player_count from actual slots filled (1..=4)
    let n = session.players.len().max(1).min(4);
    session.settings.board.players = PlayerCount::from_count(n);
    let board = Board::new(session.settings.board);
    session.board = Some(board.clone());
    // Update each peer's role and send RoundStarting.
    let mut roles: HashMap<PeerId, PlayerRole> = HashMap::new();
    for (id, slot) in session.players.iter() {
        if let Some(peer) = slot.peer {
            roles.insert(peer, PlayerRole::Player { id: *id });
        }
    }
    for (peer, role) in session.peers.iter_mut() {
        *role = *roles.get(peer).unwrap_or(&PlayerRole::Spectator);
    }
    for (RemoteId(peer), mut sender) in welcomers.iter_mut() {
        let your_role = *session.peers.get(peer).unwrap_or(&PlayerRole::Spectator);
        let _ = sender.send::<ReliableChannel>(RoundStarting {
            round: session.round,
            your_role,
            board: board.clone(),
            settings: session.settings,
            tick: session.tick,
        });
    }
    info!("round {} starting, {} players", session.round, n);
}

/// Advance the simulation. Movement frames fire every `frames_per_movement`
/// fixed ticks; on each, broadcast `TickConfirmed`.
fn tick_world(
    mut session: ResMut<SessionState>,
    mut senders: Query<(&RemoteId, &mut MessageSender<TickConfirmed>), With<ClientOf>>,
) {
    if session.board.is_none() {
        return;
    }
    let fpm = session.settings.speed.frames_per_movement();
    session.frame_counter = session.frame_counter.wrapping_add(1);
    if session.frame_counter % fpm != 0 {
        return;
    }
    if session.players.is_empty() {
        return;
    }

    // Snapshot all the immutable state we need.
    let n = session.players.len();
    let mut inputs: Vec<Option<Direction>> = vec![None; n];
    for (id, slot) in session.players.iter() {
        if (*id as usize) < n {
            inputs[*id as usize] = slot.latest_input;
        }
    }
    let seed = session
        .rng_seed
        .wrapping_add(session.tick as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);

    // Now take a single mutable borrow of board.
    let board = session.board.as_mut().unwrap();
    let outcome = match board.tick_movement(&inputs) {
        Ok(o) => o,
        Err(e) => {
            warn!("tick error: {}", e);
            return;
        }
    };
    let mut rng = StdRng::seed_from_u64(seed);
    let spawns = board.pick_spawns(&outcome, &mut rng);

    session.tick = session.tick.saturating_add(1);
    let confirmed = TickConfirmed {
        tick: session.tick,
        inputs,
        spawns,
        events: outcome.events,
    };
    for (_, mut sender) in senders.iter_mut() {
        let _ = sender.send::<UnreliableChannel>(confirmed.clone());
    }
}

fn tick_hz(settings: &GameSettings) -> f64 {
    SERVER_TICK_HZ / settings.speed.frames_per_movement() as f64
}

/// Standalone WebSocket game server: [`GameSimPlugin`] plus a real WebSocket
/// listener bound to `bind`. Native-only — the WebSocket *server* transport
/// isn't available on wasm (and solo uses crossbeam there anyway).
#[cfg(not(target_arch = "wasm32"))]
pub struct GameServerPlugin {
    pub bind: std::net::SocketAddr,
}

#[cfg(not(target_arch = "wasm32"))]
impl Plugin for GameServerPlugin {
    fn build(&self, app: &mut App) {
        use crate::net_proto::GameProtocolPlugin;
        use std::time::Duration;

        app.add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / SERVER_TICK_HZ),
        })
        .add_plugins(GameProtocolPlugin)
        .add_plugins(GameSimPlugin)
        .insert_resource(BindAddr(self.bind))
        .add_systems(Startup, start_websocket_listener);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
struct BindAddr(std::net::SocketAddr);

/// Spawn the WebSocket server link entity and Start it. The session itself is
/// initialized by `GameSimPlugin`'s `init_session`.
#[cfg(not(target_arch = "wasm32"))]
fn start_websocket_listener(mut commands: Commands, bind: Res<BindAddr>) {
    use crate::net_proto::PROTOCOL_ID;
    use lightyear::websocket::prelude::server::ServerConfig;
    use lightyear::websocket::server::WebSocketServerIo;

    // Plain ws:// (no in-process TLS). Browsers won't accept the self-signed
    // cert we used to mint here, so for dev the operator visits ws:// directly
    // and for prod TLS is terminated by a reverse proxy that forwards to this
    // plain ws:// listener.
    let cfg = ServerConfig::builder()
        .with_bind_address(bind.0)
        .with_no_encryption();

    let netcode = NetcodeServer::new(NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: SESSION_KEY,
        ..default()
    });

    let server = commands
        .spawn((
            netcode,
            LocalAddr(bind.0),
            WebSocketServerIo { config: cfg },
            Name::from("GameServer"),
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!("game server listening on {}", bind.0);
}
