//! Wire types for the lobby WebSocket protocol, shared between the lobby
//! service (`src/bin/server.rs`) and the Bevy client plugin
//! (`src/lobby.rs`).
//!
//! The lobby layer is **metadata only**. It tells clients where to point
//! their matchbox sockets (each lobby is its own matchbox room) and when to
//! actually build the GGRS session (host clicks Start → server broadcasts
//! the roster). The gameplay path is unchanged from there on.

use crate::settings::GameSettings;
use serde::{Deserialize, Serialize};

/// Short URL-safe identifier. Also used verbatim as the matchbox room
/// suffix, e.g. `ws://host:3536/lobby-{id}`.
pub type LobbyId = String;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyState {
    Waiting,
    Playing,
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
}

/// Hard cap on players per lobby. Matches the snake-spawn position table in
/// `board::Board::new`; any higher and the board has nowhere to put the
/// extras.
pub const MAX_PLAYERS: u8 = 4;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClientMsg {
    /// Host a new lobby with the caller's chosen settings.
    CreateLobby { settings: GameSettings },
    /// Try to join an existing lobby. Server rejects with `JoinDenied` if
    /// the lobby is non-Waiting, full, or unknown.
    JoinLobby { id: LobbyId },
    /// Host-only: freeze the current roster and broadcast `LobbyStarting`.
    StartLobby { id: LobbyId },
    /// Host-only: mark the lobby Finished (post-game). The server's GC
    /// removes the record after a short grace period.
    UpdateState { id: LobbyId, state: LobbyState },
    /// Sent every few seconds. Refreshes the server's view of who is still
    /// alive in a lobby and carries the matchbox `PeerId` once the local
    /// socket knows it (server needs the PeerIds to build the Start
    /// roster).
    Heartbeat {
        id: LobbyId,
        peer_id: Option<String>,
    },
    /// Graceful exit. If sent by the host, the lobby is torn down; other
    /// members get `Kicked`.
    LeaveLobby { id: LobbyId },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ServerMsg {
    /// Full snapshot of all lobbies, pushed on connect and on every
    /// mutation. Cheap — there will never be many lobbies in practice.
    LobbyList { lobbies: Vec<Lobby> },
    /// Ack to the host of a successful `CreateLobby`.
    LobbyCreated { id: LobbyId },
    /// Sent to *every* member of a lobby when the host hits Start. The
    /// roster is the authoritative, ordered list of matchbox PeerIds that
    /// the GGRS session will be built around — clients wait until matchbox
    /// shows the same set before building their session.
    LobbyStarting {
        id: LobbyId,
        roster: Vec<String>,
    },
    /// The server rejected a `JoinLobby` (full, gone, already playing).
    JoinDenied { id: LobbyId, reason: String },
    /// The server removed us from a lobby (host left, GC, etc).
    Kicked { id: LobbyId, reason: String },
    /// Generic error, for protocol-level problems.
    Error { msg: String },
}
