//! Shared client/server network protocol for the snake game.
//!
//! Lightyear-based authoritative server: the server runs the simulation,
//! picks apple/wall spawn positions, and broadcasts the inputs + spawn
//! positions per tick. Clients re-run `Board::tick_movement` locally with
//! the broadcast inputs and `apply_spawns` with the broadcast positions —
//! so a connected client receives a tiny per-tick delta, not a full board.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::board::{Board, BoardEvent, Direction, SpawnPositions};
use crate::settings::GameSettings;

/// Per-lobby shared secret embedded in the netcode handshake. The lobby
/// server allocates a unique 32-byte private key per game session and hands
/// it (plus a `client_id`) to each joining client over the lobby websocket.
pub type SessionKey = [u8; 32];

/// Default Lightyear protocol id — must match between client and server.
pub const PROTOCOL_ID: u64 = 0x534e_414b_4500_0001;

/// Role of a connected client in the current round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Player { id: u8 },
    Spectator,
}

// ---------- Client → Server ----------

/// Player input for an upcoming tick. Sent on the unreliable channel,
/// latest-wins per (client, target_tick).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InputMsg {
    /// The server tick this input should apply to, computed from the
    /// client's estimated server clock.
    pub target_tick: u32,
    pub dir: Option<Direction>,
}

/// Spectator → server: please slot me into the next round if there's room.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RequestJoinNextRound;

/// Host → server: start the next round now (server is the authority on
/// whether the request is honored).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RequestStartRound;

// ---------- Server → Client ----------

/// Sent once on connection. Includes a full board snapshot — the only full
/// state message on the wire.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Welcome {
    pub role: Role,
    pub settings: GameSettings,
    pub board: Board,
    pub tick: u32,
    pub round: u32,
    /// Server's wall-clock tick rate in Hz — clients use this for
    /// interpolation timing.
    pub tick_hz: f32,
}

/// One-per-tick authoritative delta. Clients re-run `tick_movement` with
/// `inputs` and `apply_spawns` with `spawns`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TickConfirmed {
    pub tick: u32,
    pub inputs: Vec<Option<Direction>>,
    pub spawns: SpawnPositions,
    pub events: Vec<BoardEvent>,
}

/// Sent when the current round ends (all snakes dead). Clients show the
/// game-over screen and may request to join the next round.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoundEnded {
    pub round: u32,
}

/// Sent when a new round is starting. Carries each receiving client's new
/// role (spectators may be promoted to Player) plus a fresh board snapshot
/// so everyone resets in sync.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoundStarting {
    pub round: u32,
    pub your_role: Role,
    pub board: Board,
    pub settings: GameSettings,
    pub tick: u32,
}

// ---------- Channels ----------

/// Reliable, ordered. Used for Welcome / RoundStarting / RoundEnded.
pub struct ReliableChannel;

/// Unreliable, latest-wins. Used for inputs and TickConfirmed.
pub struct UnreliableChannel;

// ---------- Protocol plugin ----------

pub struct GameProtocolPlugin;

impl Plugin for GameProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Messages
        app.register_message::<InputMsg>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<RequestJoinNextRound>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<RequestStartRound>()
            .add_direction(NetworkDirection::ClientToServer);

        app.register_message::<Welcome>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<TickConfirmed>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<RoundEnded>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<RoundStarting>()
            .add_direction(NetworkDirection::ServerToClient);

        // Channels
        app.add_channel::<ReliableChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.add_channel::<UnreliableChannel>(ChannelSettings {
            mode: ChannelMode::SequencedUnreliable,
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}
