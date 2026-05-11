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
        timestamp: u64,
    },
    RestartGame {
        board_settings: BoardSettings,
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
        timestamp: u64,
    },
}
