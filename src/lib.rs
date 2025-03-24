use board::{Board, BoardEvent, BoardSettings, Direction};
use serde::{Deserialize, Serialize};

pub mod ai;
pub mod board;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameCommands {
    Input { tick: u64, direction: Direction },
    RestartGame { board_settings: BoardSettings },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameUpdates {
    Ticked {
        tick: u64,
        board: Board,
        events: Vec<BoardEvent>,
    },
}
