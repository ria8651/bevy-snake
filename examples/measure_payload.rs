use bevy_snake::{GameUpdates, board::{Board, BoardSettings, BoardSize, AppleCount, PlayerCount}};

fn main() {
    for &size in &[BoardSize::Small, BoardSize::Medium, BoardSize::Large] {
        let settings = BoardSettings {
            board_size: size,
            apples: AppleCount::Five,
            players: PlayerCount::One,
        };
        let board = Board::new(settings);
        let update = GameUpdates::Ticked {
            tick: 999,
            board,
            events: Vec::new(),
            applied_inputs: Vec::new(),
            tick_interval_ms: 133,
            echo_client_send_ms: Some(1_234_567_890),
        };
        let json = serde_json::to_string(&update).unwrap();
        let bincoded = bincode::serialize(&update).unwrap();
        println!(
            "{:?}: {} bytes JSON, {} bytes bincode",
            size,
            json.len(),
            bincoded.len()
        );
    }
}
