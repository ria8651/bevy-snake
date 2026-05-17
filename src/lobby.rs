//! Client-side lobby browser + matchmaking glue.
//!
//! Owns a single WebSocket connection to the lobby service
//! (`/lobbies` on the static-file HTTP server). Exposes three resources:
//!
//! - [`LobbyClient`] — connection state + helpers the UI calls
//!   (`create`, `join`, `start`, `leave`, `solo`).
//! - [`LobbyList`] — current snapshot of all open lobbies, rendered by the
//!   browser screen.
//! - [`CurrentLobby`] — describes what kind of session we're trying to
//!   build right now. [src/net.rs](src/net.rs)'s `start_session` /
//!   `wait_for_players` read this to know which matchbox room to open and
//!   when the host has pressed Start.
//!
//! The plugin's `Update` system pumps the WS in both directions: incoming
//! `ServerMsg`s get dispatched into resource mutations + state transitions,
//! outgoing `ClientMsg`s queued by helper methods get flushed each tick.

use crate::ClientState;
use bevy::prelude::*;
use bevy_matchbox::prelude::PeerId;
use bevy_snake::lobby_proto::{ClientMsg, Lobby, LobbyId, LobbyState, ServerMsg};
use bevy_snake::settings::GameSettings;
use ewebsock::{Options, WsEvent, WsMessage, WsReceiver, WsSender};

/// WebSocket URL of the lobby service. Compile-time override at the same
/// build step you'd set `MATCHBOX_ROOM_URL`. Defaults to the colocated
/// static server.
fn lobby_ws_url() -> String {
    option_env!("LOBBY_WS_URL")
        .unwrap_or("ws://localhost:1234/lobbies")
        .to_string()
}

/// Heartbeat cadence — server times out at 10 s, so 3 s gives three
/// in-flight chances before we get GC'd.
const HEARTBEAT_INTERVAL_SECS: f32 = 3.0;

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    #[default]
    None,
    Solo,
    Host,
    Joiner,
}

/// What [`crate::net::start_session`] should build next, plus what state
/// the lobby plugin has accumulated from the server. Read from net.rs,
/// written by both the lobby plugin and the UI.
#[derive(Resource, Default)]
pub struct CurrentLobby {
    /// `Some` when we're in a networked lobby (host or joiner). `None` for
    /// solo play and for "no lobby right now".
    pub id: Option<LobbyId>,
    /// Matchbox room name to connect to (`format!("lobby-{id}")`). Set
    /// alongside `id`. Kept separate so net.rs doesn't need to know the
    /// id-to-room-name convention.
    pub room_name: Option<String>,
    pub role: Role,
    /// Frozen peer ordering from the server's `LobbyStarting` broadcast.
    /// `wait_for_players` waits until `socket.players()` matches this set
    /// (including the local peer), then builds the GGRS session in roster
    /// order.
    pub start_roster: Option<Vec<PeerId>>,
}

impl CurrentLobby {
    pub fn clear(&mut self) {
        self.id = None;
        self.room_name = None;
        self.role = Role::None;
        self.start_roster = None;
    }
}

/// Live mirror of the server's lobby table. Rebuilt on every
/// `ServerMsg::LobbyList`.
#[derive(Resource, Default)]
pub struct LobbyList {
    pub lobbies: Vec<Lobby>,
    /// Bumped on every refresh so UI can use `Changed<>` cheaply (the
    /// Vec itself doesn't compare).
    pub rev: u64,
}

/// The WS sender/receiver wrap `Rc<WebSocket>` on wasm and
/// `std::sync::mpsc` channels on native — both are `!Send + !Sync`. Bevy
/// supports this via `NonSend` resources: systems that take
/// `NonSendMut<LobbyClient>` are pinned to the main thread.
pub struct LobbyClient {
    sender: Option<WsSender>,
    receiver: Option<WsReceiver>,
    connected: bool,
    /// Outbound messages buffered if the connection isn't open yet.
    /// Drained on `WsEvent::Opened` and on every successful flush.
    pending: Vec<ClientMsg>,
    heartbeat_acc: f32,
    /// Cached most-recent local matchbox peer id, refreshed each Update
    /// from the live socket. Sent in every heartbeat once known.
    pub last_peer_id: Option<String>,
}

impl Default for LobbyClient {
    fn default() -> Self {
        Self {
            sender: None,
            receiver: None,
            connected: false,
            pending: Vec::new(),
            heartbeat_acc: 0.0,
            last_peer_id: None,
        }
    }
}

impl LobbyClient {
    fn try_connect(&mut self) {
        if self.sender.is_some() {
            return;
        }
        let url = lobby_ws_url();
        match ewebsock::connect(url.clone(), Options::default()) {
            Ok((sender, receiver)) => {
                info!("lobby ws connecting to {}", url);
                self.sender = Some(sender);
                self.receiver = Some(receiver);
                self.connected = false;
            }
            Err(e) => {
                warn!("lobby ws connect failed: {}", e);
            }
        }
    }

    fn enqueue(&mut self, msg: ClientMsg) {
        self.pending.push(msg);
    }

    fn flush(&mut self) {
        if !self.connected {
            return;
        }
        let Some(sender) = self.sender.as_mut() else {
            return;
        };
        for msg in self.pending.drain(..) {
            match serde_json::to_string(&msg) {
                Ok(json) => sender.send(WsMessage::Text(json)),
                Err(e) => warn!("encode error: {}", e),
            }
        }
    }

    fn drain_events(&mut self) -> Vec<WsEvent> {
        let Some(receiver) = self.receiver.as_ref() else {
            return Vec::new();
        };
        std::iter::from_fn(|| receiver.try_recv()).collect()
    }

    fn mark_opened(&mut self) {
        self.connected = true;
    }

    fn mark_closed(&mut self) {
        self.connected = false;
        self.sender = None;
        self.receiver = None;
    }

    pub fn create_lobby(&mut self, settings: GameSettings) {
        self.enqueue(ClientMsg::CreateLobby { settings });
    }

    pub fn join_lobby(&mut self, id: LobbyId) {
        self.enqueue(ClientMsg::JoinLobby { id });
    }

    pub fn start_lobby(&mut self, id: LobbyId) {
        self.enqueue(ClientMsg::StartLobby { id });
    }

    pub fn leave_lobby(&mut self, id: LobbyId) {
        self.enqueue(ClientMsg::LeaveLobby { id });
    }

    pub fn mark_finished(&mut self, id: LobbyId) {
        self.enqueue(ClientMsg::UpdateState {
            id,
            state: LobbyState::Finished,
        });
    }
}

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_non_send_resource::<LobbyClient>()
            .init_resource::<LobbyList>()
            .init_resource::<CurrentLobby>()
            .add_systems(Startup, connect_lobby_ws)
            .add_systems(Update, (pump_lobby_ws, heartbeat));
    }
}

fn connect_lobby_ws(mut client: NonSendMut<LobbyClient>) {
    client.try_connect();
}

/// Drains the WS receiver, dispatches messages, then flushes outbound. Also
/// tracks the local matchbox peer id from the live `MatchboxSocket` so the
/// next heartbeat includes it.
fn pump_lobby_ws(
    mut client: NonSendMut<LobbyClient>,
    mut list: ResMut<LobbyList>,
    mut current: ResMut<CurrentLobby>,
    mut next: ResMut<NextState<ClientState>>,
    state: Res<State<ClientState>>,
    mut socket: Option<ResMut<bevy_matchbox::prelude::MatchboxSocket>>,
) {
    // Refresh known peer id from matchbox. `id()` needs `&mut self` on the
    // underlying socket — once known it's stable, but the call still
    // requires mutable access to lazily resolve it.
    client.last_peer_id = socket
        .as_mut()
        .and_then(|s| s.id().map(|p| p.0.to_string()));

    for ev in client.drain_events() {
        match ev {
            WsEvent::Opened => {
                info!("lobby ws opened");
                client.mark_opened();
            }
            WsEvent::Closed => {
                warn!("lobby ws closed");
                client.mark_closed();
                if current.id.is_some() {
                    current.clear();
                    if state.get() != &ClientState::Browsing {
                        next.set(ClientState::Browsing);
                    }
                }
            }
            WsEvent::Error(e) => warn!("lobby ws error: {}", e),
            WsEvent::Message(WsMessage::Text(text)) => {
                handle_server_text(
                    &text,
                    &mut client,
                    &mut list,
                    &mut current,
                    &mut next,
                    state.get(),
                );
            }
            WsEvent::Message(_) => {}
        }
    }

    client.flush();
}

fn handle_server_text(
    text: &str,
    client: &mut LobbyClient,
    list: &mut LobbyList,
    current: &mut CurrentLobby,
    next: &mut NextState<ClientState>,
    state: &ClientState,
) {
    let Ok(msg) = serde_json::from_str::<ServerMsg>(text) else {
        warn!("bad server msg: {}", text);
        return;
    };
    match msg {
        ServerMsg::LobbyList { lobbies } => {
            list.lobbies = lobbies;
            list.rev = list.rev.wrapping_add(1);
        }
        ServerMsg::LobbyCreated { id } => {
            info!("lobby created: {}", id);
            current.id = Some(id.clone());
            current.room_name = Some(format!("lobby-{}", id));
            current.role = Role::Host;
            current.start_roster = None;
            next.set(ClientState::WaitingForOpponent);
        }
        ServerMsg::LobbyStarting { id, roster } => {
            if current.id.as_deref() != Some(&id) {
                return;
            }
            let parsed: Vec<PeerId> = roster
                .iter()
                .filter_map(|s| {
                    uuid::Uuid::parse_str(s).ok().map(PeerId)
                })
                .collect();
            // If we're not in the roster ourselves, the server filtered us
            // out (likely because our matchbox PeerId hadn't been
            // heartbeated yet). Bounce back rather than hang.
            let my_pid = client
                .last_peer_id
                .as_deref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(PeerId);
            if let Some(me) = my_pid
                && !parsed.contains(&me)
            {
                warn!("not in start roster, returning to browser");
                current.clear();
                next.set(ClientState::Browsing);
                return;
            }
            info!("start roster: {} players", parsed.len());
            current.start_roster = Some(parsed);
        }
        ServerMsg::JoinDenied { id, reason } => {
            warn!("join denied for {}: {}", id, reason);
            if current.id.as_deref() == Some(&id) {
                current.clear();
                if state != &ClientState::Browsing {
                    next.set(ClientState::Browsing);
                }
            }
        }
        ServerMsg::Kicked { id, reason } => {
            warn!("kicked from {}: {}", id, reason);
            if current.id.as_deref() == Some(&id) {
                current.clear();
                next.set(ClientState::Browsing);
            }
        }
        ServerMsg::Error { msg } => warn!("lobby server: {}", msg),
    }
}

fn heartbeat(
    mut client: NonSendMut<LobbyClient>,
    current: Res<CurrentLobby>,
    time: Res<Time<Real>>,
) {
    client.heartbeat_acc += time.delta_secs();
    if client.heartbeat_acc < HEARTBEAT_INTERVAL_SECS {
        return;
    }
    client.heartbeat_acc = 0.0;
    let Some(id) = current.id.clone() else {
        return;
    };
    let peer_id = client.last_peer_id.clone();
    client.enqueue(ClientMsg::Heartbeat { id, peer_id });
}

