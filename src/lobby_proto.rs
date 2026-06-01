//! Wire types for the lobby WebSocket protocol, shared between the lobby
//! service (`src/bin/server.rs`) and the Bevy client plugin
//! (`src/lobby.rs`).
//!
//! The lobby layer is **metadata only**. It tells clients where to point
//! their Lightyear client (each lobby has its own netcode private key) and
//! when to actually connect (host clicks Start → server broadcasts the
//! per-client credentials).

use crate::settings::GameSettings;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Short URL-safe identifier.
pub type LobbyId = String;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyState {
    Waiting,
    /// Round in progress. New connections still allowed — they join as
    /// spectators and get promoted on the next round.
    InProgress,
    Finished,
}

/// Public view of a lobby — what every connected client sees in the
/// browser list.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lobby {
    pub id: LobbyId,
    pub settings: GameSettings,
    pub state: LobbyState,
    pub players_present: u8,
    pub spectators_present: u8,
}

/// Hard cap on players per lobby. Matches the snake-spawn position table in
/// `board::Board::new`.
pub const MAX_PLAYERS: u8 = 4;

/// Per-client credentials for connecting to the Lightyear game server. The
/// lobby service hands these to each member when the host hits Start.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameSessionCreds {
    /// Where to connect the WebSocket. Either an absolute URL
    /// (`ws://host:port/path` / `wss://...`) or a path starting with `/`,
    /// in which case the client resolves it relative to the page origin.
    /// Same-origin path keeps the deploy single-port.
    pub endpoint: String,
    /// SocketAddr the netcode auth token is bound to — must match the game
    /// server's `LocalAddr`. Differs from the connect URL when the WS is
    /// proxied: client connects via the front door (e.g. `/game` on the
    /// HTTP port) but netcode validates against the backend's bind addr.
    pub netcode_server_addr: SocketAddr,
    /// Netcode client id — unique within the session.
    pub client_id: u64,
    /// Shared 32-byte netcode private key for this session.
    pub private_key: [u8; 32],
    /// Netcode protocol id for this session.
    pub protocol_id: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClientMsg {
    /// Host a new lobby with the caller's chosen settings.
    CreateLobby { settings: GameSettings },
    /// Try to join an existing lobby. If the lobby is in-progress the client
    /// joins as a spectator (server signals this in the credentials handout).
    JoinLobby { id: LobbyId },
    /// Host-only: start the game (or next round). The server spins up a
    /// Lightyear game session (if not already running) and broadcasts
    /// `GameSessionReady` to all members with their per-client credentials.
    StartLobby { id: LobbyId },
    /// Host-only: mark the lobby Finished (post-game). The server GC removes
    /// the record after a short grace period.
    UpdateState { id: LobbyId, state: LobbyState },
    /// Sent every few seconds. Refreshes the server's view of who is alive.
    Heartbeat { id: LobbyId },
    /// Graceful exit. If sent by the host, the lobby is torn down.
    LeaveLobby { id: LobbyId },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ServerMsg {
    /// Full snapshot of all lobbies.
    LobbyList { lobbies: Vec<Lobby> },
    /// Ack to the host of a successful `CreateLobby`.
    LobbyCreated { id: LobbyId },
    /// Sent to a single member when the game server is ready to accept
    /// their connection. Each recipient gets unique netcode credentials.
    GameSessionReady {
        id: LobbyId,
        creds: GameSessionCreds,
    },
    /// The server rejected a `JoinLobby` (full, gone).
    JoinDenied { id: LobbyId, reason: String },
    /// The server removed us from a lobby (host left, GC, etc).
    Kicked { id: LobbyId, reason: String },
    /// Generic error, for protocol-level problems.
    Error { msg: String },
}
