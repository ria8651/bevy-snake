use crate::{
    board::{Board, BoardEvent, Cell, Direction},
    game::{update_game, SnakeInputs, TickTimer},
    Settings,
};
use bevy::prelude::*;
use rand::{prelude::SliceRandom, rngs::StdRng, SeedableRng};
use std::{
    collections::{HashSet, VecDeque},
    time::Instant,
};

pub struct AIPlugin;

impl Plugin for AIPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, ai_system.after(update_game));
    }
}

fn ai_system(
    mut input_queues: ResMut<SnakeInputs>,
    mut gizmos: Gizmos,
    mut ai_gizmos: Local<AIGizmos>,
    settings: Res<Settings>,
    board: Res<Board>,
    tick_timer: Res<TickTimer>,
) {
    if tick_timer.just_finished() || !settings.do_game_tick {
        // let ai = RandomWalk;
        let ai = TreeSearch { max_depth: 70 };

        let mut new_ai_gizmos = AIGizmos::default();

        if let Ok(dir) = ai.chose_move(board.as_ref(), &mut new_ai_gizmos) {
            *ai_gizmos = new_ai_gizmos;

            let input_queue = &mut input_queues[0].input_queue;
            if settings.ai && input_queue.is_empty() {
                input_queue.push_back(dir);
            }
        }
    }

    if settings.gizmos {
        let board_pos = |pos: IVec2| {
            Vec2::new(
                pos.x as f32 - board.width() as f32 / 2.0 + 0.5,
                pos.y as f32 - board.height() as f32 / 2.0 + 0.5,
            )
        };
        for (start, end, color) in ai_gizmos.lines.iter() {
            gizmos.line_2d(board_pos(*start), board_pos(*end), *color);
        }
        for (pos, color) in ai_gizmos.points.iter() {
            gizmos.circle_2d(board_pos(*pos), 0.3, *color);
        }
    }
}

trait SnakeAI {
    fn chose_move(&self, board: &Board, gizmos: &mut AIGizmos) -> Result<Direction, ()>;
}

struct RandomWalk;

impl SnakeAI for RandomWalk {
    fn chose_move(&self, board: &Board, _gizmos: &mut AIGizmos) -> Result<Direction, ()> {
        let snakes = board.snakes();
        let snake = snakes.get(&0).ok_or(())?;

        let mut dir = Direction::ALL;
        dir.shuffle(&mut rand::thread_rng());

        for dir in dir {
            let pos = snake.head + dir.as_vec2();
            if let Ok(Cell::Empty | Cell::Apple { .. }) = board.get(pos) {
                return Ok(dir);
            }
        }

        Err(())
    }
}

struct TreeSearch {
    max_depth: usize,
}

impl SnakeAI for TreeSearch {
    fn chose_move(&self, board: &Board, gizmos: &mut AIGizmos) -> Result<Direction, ()> {
        let snakes = board.snakes();
        let snake = snakes.get(&0).ok_or(())?;

        struct BoardEval {
            board: Board,
            score: f32,
            depth: usize,
            history: Vec<Direction>,
        }

        let mut queue = VecDeque::from([BoardEval {
            board: board.clone(),
            score: 0.0,
            depth: 0,
            history: Vec::new(),
        }]);

        let mut final_boards = Vec::new();

        let mut rng = rand::thread_rng();
        let start_time = Instant::now();
        while let Some(board_eval) = queue.pop_front() {
            let BoardEval {
                board,
                score,
                depth,
                history,
            } = board_eval;

            let snakes = board.snakes();
            let snake = match snakes.get(&0) {
                Some(snake) => snake,
                None => continue,
            };

            for dir in Direction::ALL {
                if dir == snake.dir.opposite() {
                    continue;
                }

                let mut history = history.clone();
                history.push(dir);

                let mut board = board.clone();
                board.rng = StdRng::from_rng(&mut rng).unwrap();
                let events = board.tick_board(&[Some(dir), None, None, None]).unwrap();

                let mut score = score;
                let mut game_over = false;
                for event in events {
                    match event {
                        BoardEvent::AppleEaten { snake } => {
                            if snake == 0 {
                                score += 1.0 / (depth as f32 + 1.0);
                            }
                        }
                        BoardEvent::GameOver => {
                            game_over = true;
                        }
                        _ => {}
                    }
                }

                let board_eval = BoardEval {
                    board,
                    score,
                    depth: depth + 1,
                    history,
                };

                if game_over || depth == self.max_depth {
                    final_boards.push(board_eval);
                } else {
                    queue.push_back(board_eval);
                }
            }

            if start_time.elapsed().as_millis() > 5 {
                final_boards.extend(queue);
                break;
            }
        }

        for board in final_boards.iter_mut() {
            board.score = self.eval_board(&board.board, board.score, gizmos)?;

            if board.score > 0.0 {
                let mut head = snake.head;
                for dir in board.history.iter() {
                    gizmos
                        .lines
                        .push((head, head + dir.as_vec2(), Color::srgb(1.0, 0.0, 0.0)));
                    head += dir.as_vec2();
                }
            }
        }

        let max_board = final_boards
            .into_iter()
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
            .ok_or(())?;

        let dir = *max_board.history.first().unwrap();
        let mut head = snake.head;
        for dir in max_board.history {
            gizmos
                .lines
                .push((head, head + dir.as_vec2(), Color::srgb(0.0, 1.0, 0.0)));
            head += dir.as_vec2();
        }

        Ok(dir)
    }
}

const BAD_SCORE: f32 = -1000.0;

impl TreeSearch {
    fn eval_board(
        &self,
        board: &Board,
        apple_score: f32,
        gizmos: &mut AIGizmos,
    ) -> Result<f32, ()> {
        let snakes = board.snakes();
        if snakes.len() == 0 {
            return Ok(BAD_SCORE + apple_score);
        }

        let (_, snake) = snakes.into_iter().next().unwrap();

        let max_search = snake.parts.len() * 2;
        let mut queue = VecDeque::from([snake.head]);
        let mut visited = HashSet::from([snake.head]);
        let mut found_tail = false;
        while let Some(pos) = queue.pop_front() {
            for dir in Direction::ALL {
                let next_pos = pos + dir.as_vec2();
                if visited.contains(&next_pos) {
                    continue;
                }
                visited.insert(next_pos);
                match board.get(next_pos) {
                    Ok(Cell::Empty | Cell::Apple { .. }) => {
                        queue.push_back(next_pos);
                        gizmos
                            .points
                            .push((next_pos, Color::srgba(0.0, 0.0, 0.0, 0.4)));
                    }
                    Ok(Cell::Snake { id: 0, part: 0 }) => {
                        found_tail = true;
                    }
                    _ => {}
                }
            }

            if visited.len() >= max_search {
                found_tail = true;
                break;
            }
        }
        let flood_fill = visited.len().min(max_search) as f32 / max_search as f32;

        if !found_tail {
            return Ok(BAD_SCORE + apple_score);
        }

        // return Ok(apple_score);
        return Ok(flood_fill + apple_score * 2.0);
    }
}

#[derive(Default)]
struct AIGizmos {
    lines: Vec<(IVec2, IVec2, Color)>,
    points: Vec<(IVec2, Color)>,
}
