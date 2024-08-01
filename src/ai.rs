use crate::{
    board::{Board, BoardEvent, Cell, Direction},
    game::{update_game, SnakeInputs, TickTimer},
    Settings,
};
use bevy::prelude::*;
use rand::prelude::SliceRandom;
use std::{
    collections::{HashSet, VecDeque},
    ops::{Index, IndexMut},
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
        let ai = TreeSearch { max_depth: 7 };

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

        let scores = self.recursive_eval(board.clone(), 0, 0.0, snake.dir, gizmos);
        
        let mut dir_scores: Vec<_> = scores.dir_values().collect();
        dir_scores.shuffle(&mut rand::thread_rng());

        let (dir, _score) = dir_scores
            .into_iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap();

        Ok(dir)
    }
}

const BAD_SCORE: f32 = -1000.0;

impl TreeSearch {
    fn recursive_eval(
        &self,
        board: Board,
        depth: usize,
        apple_score: f32,
        last: Direction,
        gizmos: &mut AIGizmos,
    ) -> Dir<f32> {
        let head = board.snakes().get(&0).unwrap().head;

        let mut scores = Dir::all(BAD_SCORE);
        for dir in Direction::ALL {
            if dir == last.opposite() {
                continue;
            }

            let mut apple_score = apple_score;
            let mut board = board.clone();
            let events = board.tick_board(&[Some(dir)]).unwrap();
            if events.contains(&BoardEvent::GameOver) {
                scores[dir] = BAD_SCORE + depth as f32; // survive as long as possible when faced with death
                continue;
            }
            for event in events {
                match event {
                    BoardEvent::AppleEaten { snake } => {
                        if snake == 0 {
                            apple_score += 1.0 / (depth as f32 + 1.0);
                        }
                    }
                    _ => {}
                }
            }

            if depth == self.max_depth {
                let eval = self
                    .eval_board(&board, apple_score, gizmos)
                    .unwrap_or(BAD_SCORE);
                scores[dir] = eval;
            } else {
                let child_scores = self.recursive_eval(board, depth + 1, apple_score, dir, gizmos);
                let (_dir, score) = child_scores.max();
                scores[dir] = score;
            }

            let col = Color::srgb(scores[dir] as f32, 0.0, 0.0);
            gizmos.lines.push((head, head + dir.as_vec2(), col));
        }

        scores
    }

    fn eval_board(
        &self,
        board: &Board,
        apple_score: f32,
        gizmos: &mut AIGizmos,
    ) -> Result<f32, ()> {
        let (_, snake) = board.snakes().into_iter().next().ok_or(())?;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dir<T: Copy>([T; 4]);

impl<T: Copy> Dir<T> {
    fn all(value: T) -> Self {
        Self([value; 4])
    }

    fn values(&self) -> impl Iterator<Item = T> + '_ {
        self.0.iter().copied()
    }

    fn dir_values(&self) -> impl Iterator<Item = (Direction, T)> + '_ {
        Direction::ALL.into_iter().zip(self.values())
    }

    fn max(&self) -> (Direction, T)
    where
        T: PartialOrd,
    {
        self.dir_values()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
    }
}

impl<T: Copy> Index<Direction> for Dir<T> {
    type Output = T;

    fn index(&self, index: Direction) -> &Self::Output {
        &self.0[index as usize]
    }
}

impl<T: Copy> IndexMut<Direction> for Dir<T> {
    fn index_mut(&mut self, index: Direction) -> &mut Self::Output {
        &mut self.0[index as usize]
    }
}

#[derive(Default)]
struct AIGizmos {
    lines: Vec<(IVec2, IVec2, Color)>,
    points: Vec<(IVec2, Color)>,
}
