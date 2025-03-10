use std::time::Duration;

use bevy_snake::{
    ai::{SnakeAI, TreeSearch},
    board::{Board, BoardEvent, BoardSettings},
};

fn main() {
    let ai = TreeSearch {
        max_depth: 100,
        max_time: Duration::from_millis(5),
    };
    let mut scores = Vec::new();
    for i in 0..100 {
        let mut board = Board::new(BoardSettings::default());
        let mut rng = rand::rng();
        let mut score = 0;
        for _ in 0..500 {
            score = score.max(board.snakes().values().next().unwrap().parts.len() - 4);

            let direction = ai.chose_move(&board, &mut None).unwrap();
            let events = board.tick_board(&[Some(direction)], &mut rng).unwrap();
            if events.contains(&BoardEvent::GameOver) {
                break;
            }
        }

        println!("Game {}: score {}", i, score);
        println!("{:?}", board);

        scores.push(score);
    }

    let output = serde_json::to_string(&scores).unwrap();
    println!("{}", output);
}
