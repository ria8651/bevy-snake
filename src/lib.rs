use board::{Board, BoardEvent, BoardSettings, Direction};
use serde::{Deserialize, Serialize};

pub mod ai;
pub mod board;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod transport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameCommands {
    Input {
        tick: u64,
        direction: Direction,
        /// Millisecond delta from a client-local monotonic epoch. The server
        /// only echoes this back in `GameUpdates::Ticked.echo_client_send_ms`;
        /// the client subtracts its own current value to compute RTT
        /// skew-free.
        client_send_ms: u32,
    },
    RestartGame {
        board_settings: BoardSettings,
    },
    SetTickRate {
        tick_interval_ms: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameUpdates {
    Ticked {
        tick: u64,
        board: Board,
        events: Vec<BoardEvent>,
        /// One slot per snake id. `Some(dir)` is the input the server actually
        /// applied for this tick; `None` means the snake kept its current
        /// direction (no input or rejected reverse). Lets clients distinguish
        /// "server didn't see my input yet" from "I predicted the wrong
        /// direction" during reconcile.
        applied_inputs: Vec<Option<Direction>>,
        /// Current server tick period in milliseconds. Client uses this as the
        /// PLL frequency setpoint and adapts immediately when it changes.
        tick_interval_ms: u32,
        /// Echo of the most recently-processed `Input.client_send_ms` from
        /// this recipient client. `None` until the client has sent any input.
        /// Skew-free RTT source.
        echo_client_send_ms: Option<u32>,
    },
}
