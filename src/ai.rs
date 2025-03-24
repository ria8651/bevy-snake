#![allow(dead_code)]

use crate::board::{Board, BoardEvent, Cell, Direction};
use bevy::prelude::*;
use rand::prelude::SliceRandom;
use std::collections::{HashMap, HashSet, VecDeque};
use web_time::{Duration, Instant};

pub trait SnakeAI {
    fn chose_move(
        &self,
        board: &Board,
        gizmos: &mut Option<&mut AIGizmos>,
    ) -> Result<Direction, ()>;
}

pub struct RandomWalk;

impl SnakeAI for RandomWalk {
    fn chose_move(
        &self,
        board: &Board,
        _gizmos: &mut Option<&mut AIGizmos>,
    ) -> Result<Direction, ()> {
        let snakes = board.snakes();
        let snake = snakes.get(&0).ok_or(())?;

        let mut dir = Direction::ALL;
        dir.shuffle(&mut rand::rng());

        for dir in dir {
            let pos = snake.head + dir.as_vec2();
            if let Ok(Cell::Empty | Cell::Apple { .. }) = board.get(pos) {
                return Ok(dir);
            }
        }

        Err(())
    }
}

pub struct TreeSearch {
    pub max_depth: usize,
    pub max_time: Duration,
}

pub fn cycle_basis(graph: &Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let mut cycles: Vec<Vec<usize>> = Vec::new();
    let root_index = 0;
    // Stack (ie "pushdown list") of vertices already in the spanning tree
    let mut stack: Vec<usize> = vec![root_index];
    // Map of node index to predecessor node index
    let mut pred: HashMap<usize, usize> = HashMap::new();
    pred.insert(root_index, root_index);
    // Set of examined nodes during this iteration
    let mut used: HashMap<usize, HashSet<usize>> = HashMap::new();
    used.insert(root_index, HashSet::new());
    // Walk the spanning tree
    while !stack.is_empty() {
        // Use the last element added so that cycles are easier to find
        let z = stack.pop().unwrap();
        for neighbor in graph[z].iter().copied() {
            // A new node was encountered:
            if !used.contains_key(&neighbor) {
                pred.insert(neighbor, z);
                stack.push(neighbor);
                let mut temp_set: HashSet<usize> = HashSet::new();
                temp_set.insert(z);
                used.insert(neighbor, temp_set);
            // A self loop:
            } else if z == neighbor {
                let cycle: Vec<usize> = vec![z];
                cycles.push(cycle);
            // A cycle was found:
            } else if !used.get(&z).unwrap().contains(&neighbor) {
                let pn = used.get(&neighbor).unwrap();
                let mut cycle: Vec<usize> = vec![neighbor, z];
                let mut p = pred.get(&z).unwrap();
                while !pn.contains(p) {
                    cycle.push(*p);
                    p = pred.get(p).unwrap();
                }
                cycle.push(*p);
                cycles.push(cycle);
                let neighbor_set = used.get_mut(&neighbor).unwrap();
                neighbor_set.insert(z);
            }
        }
    }

    cycles
}

impl SnakeAI for TreeSearch {
    fn chose_move(
        &self,
        board: &Board,
        gizmos: &mut Option<&mut AIGizmos>,
    ) -> Result<Direction, ()> {
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

        let mut rng = rand::rng();
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
                let events = board
                    .tick_board(&[Some(dir), None, None, None], &mut rng)
                    .unwrap();

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

            if start_time.elapsed() > self.max_time {
                final_boards.extend(queue);
                break;
            }
        }

        for board in final_boards.iter_mut() {
            board.score = self.eval_board(&board.board, board.score, gizmos)?;

            if let Some(gizmos) = gizmos {
                // show path in red
                if board.score > 0.0 {
                    let red = Color::srgb(1.0, 0.0, 0.0);
                    let mut head = snake.head;
                    for dir in board.history.iter() {
                        gizmos.lines.push((head, head + dir.as_vec2(), red));
                        head += dir.as_vec2();
                    }
                }
            }
        }

        let max_board = final_boards
            .into_iter()
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
            .ok_or(())?;

        let dir = *max_board.history.first().unwrap();

        if let Some(gizmos) = gizmos {
            // show best path in green
            let green = Color::srgb(0.0, 1.0, 0.0);
            let mut head = snake.head;
            for dir in max_board.history {
                gizmos.lines.push((head, head + dir.as_vec2(), green));
                head += dir.as_vec2();
            }
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
        gizmos: &mut Option<&mut AIGizmos>,
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
                        if let Some(gizmos) = gizmos {
                            gizmos
                                .points
                                .push((next_pos, Color::srgba(0.0, 0.0, 0.0, 0.4)));
                        }
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

        let mut hole_score = 1.0;
        for (pos, cell) in board.cells() {
            if let Cell::Apple { .. } | Cell::Empty = cell {
                let mut hole = true;
                for dir in Direction::ALL {
                    let next_pos = pos + dir.as_vec2();
                    if let Ok(Cell::Empty | Cell::Apple { .. }) = board.get(next_pos) {
                        hole = false;
                        break;
                    }
                }
                if hole {
                    hole_score /= 2.0;
                }
            }
        }

        // return Ok(apple_score);
        return Ok(flood_fill + apple_score * 2.0 + hole_score * 2.0);
    }
}

#[derive(Default)]
pub struct AIGizmos {
    pub lines: Vec<(IVec2, IVec2, Color)>,
    pub arrows: Vec<(Vec2, Vec2, Color)>,
    pub points: Vec<(IVec2, Color)>,
}
