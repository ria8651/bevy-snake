use crate::{
    board::{Board, BoardEvent, Cell, Direction},
    game::{update_game, SnakeInputs, TickTimer},
    Settings,
};
use bevy::prelude::*;
use rand::{prelude::SliceRandom, rngs::StdRng, SeedableRng};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
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
        let ai = TreeSearch {
            max_depth: 100,
            max_time: Duration::from_millis(5),
        };

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
        let board_pos = |pos: Vec2| {
            Vec2::new(
                pos.x as f32 - board.width() as f32 / 2.0 + 0.5,
                pos.y as f32 - board.height() as f32 / 2.0 + 0.5,
            )
        };
        for (start, end, color) in ai_gizmos.lines.iter() {
            gizmos.line_2d(board_pos(start.as_vec2()), board_pos(end.as_vec2()), *color);
        }
        for (start, end, color) in ai_gizmos.arrows.iter() {
            gizmos.arrow_2d(board_pos(*start), board_pos(*end), *color);
        }
        for (pos, color) in ai_gizmos.points.iter() {
            gizmos.circle_2d(board_pos(pos.as_vec2()), 0.3, *color);
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
    max_time: Duration,
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
    fn chose_move(&self, board: &Board, gizmos: &mut AIGizmos) -> Result<Direction, ()> {
        // find cycle basis of the board
        let mut nodes = HashMap::new();
        let mut graph = Vec::new();
        for (pos, cell) in board.cells() {
            if !matches!(cell, Cell::Wall) {
                nodes.insert(pos, nodes.len());
                graph.push(Vec::new());
            }
        }
        for (node, index) in nodes.iter() {
            for dir in Direction::ALL {
                let next_node = *node + dir.as_vec2();
                if let Some(next_index) = nodes.get(&next_node) {
                    graph[*index].push(*next_index);
                }
            }
        }
        let cycles = cycle_basis(&graph);

        // find the edges of the graph
        let mut edges = Vec::new();
        for (index, neighbors) in graph.iter().enumerate() {
            for &neighbor in neighbors {
                if neighbor < index {
                    edges.push((neighbor, index));
                }
            }
        }
        let find_edge_index = |a: usize, b: usize| {
            edges
                .iter()
                .position(|&(x, y)| (x == a && y == b) || (x == b && y == a))
        };
        let mut cycle_edge_masks: Vec<Vec<bool>> = cycles
            .iter()
            .map(|cycle| {
                let mut mask = vec![false; edges.len()];
                for i in 0..cycle.len() {
                    let mut a = cycle[i];
                    let mut b = cycle[(i + 1) % cycle.len()];
                    if a > b {
                        std::mem::swap(&mut a, &mut b);
                    }
                    if let Some(index) = find_edge_index(a, b) {
                        mask[index] = true;
                    }
                }
                mask
            })
            .collect();

        let print_edge_mask = |mask: &[bool]| {
            for y in 0..board.height() {
                if y > 0 {
                    for x in 0..board.width() {
                        if x > 0 {
                            print!(" ");
                        }
                        let pos = IVec2::new(x as i32, y as i32);
                        if let Some(node) = nodes.get(&pos) {
                            let pos_last = IVec2::new(x as i32, y as i32 - 1);
                            if let Some(node_last) = nodes.get(&pos_last) {
                                let edge_index = find_edge_index(*node, *node_last);
                                if let Some(index) = edge_index {
                                    if mask[index] {
                                        print!("|");
                                    } else {
                                        print!(" ");
                                    }
                                } else {
                                    print!(" ");
                                }
                            } else {
                                print!(" ");
                            }
                        } else {
                            print!(" ");
                        }
                    }
                }
                println!();
                for x in 0..board.width() {
                    let pos = IVec2::new(x as i32, y as i32);
                    if let Some(node) = nodes.get(&pos) {
                        if x > 0 {
                            let pos_last = IVec2::new(x as i32 - 1, y as i32);
                            if let Some(node_last) = nodes.get(&pos_last) {
                                let edge_index = find_edge_index(*node, *node_last);
                                if let Some(index) = edge_index {
                                    if mask[index] {
                                        print!("-");
                                    } else {
                                        print!(" ");
                                    }
                                } else {
                                    print!(" ");
                                }
                            } else {
                                print!(" ");
                            }
                        }

                        print!(".");
                    } else {
                        if x > 0 {
                            print!(" ");
                        }
                        print!("#");
                    }
                }
                println!();
            }
        };

        // find how edges are connected through vertices
        let mut edge_connections: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &(a, b)) in edges.iter().enumerate() {
            for (j, &(k, l)) in edges.iter().enumerate() {
                if i != j && (a == k || b == k || a == l || b == l) {
                    edge_connections.entry(i).or_default().push(j);
                }
            }
        }

        // combine all cycles
        let mut combined_cycle = cycle_edge_masks.pop().unwrap();
        println!("Initial combined cycles:");
        print_edge_mask(&combined_cycle);
        loop {
            let mut some_valid = false;
            for i in 0..cycle_edge_masks.len() {
                let mut valid = false;
                let mut temp = vec![false; edges.len()];
                for j in 0..edges.len() {
                    temp[j] = cycle_edge_masks[i][j] != combined_cycle[j];

                    // must overlap with combined cycles
                    if cycle_edge_masks[i][j] && combined_cycle[j] {
                        valid = true;
                    }
                }

                if !valid {
                    continue;
                }

                // cycle is valid if all parts are connected
                let mut visited = vec![false; edges.len()];
                let first = temp.iter().position(|&x| x);
                if let Some(first) = first {
                    let mut stack = vec![first];
                    visited[first] = true;
                    while let Some(index) = stack.pop() {
                        let mut neighbors = 0;
                        for &neighbor in edge_connections.get(&index).unwrap_or(&Vec::new()) {
                            if !visited[neighbor] && temp[neighbor] {
                                visited[neighbor] = true;
                                stack.push(neighbor);
                            }
                            if temp[neighbor] {
                                neighbors += 1;
                            }
                        }
                        if neighbors != 2 {
                            valid = false;
                            break;
                        }
                    }
                }

                if valid && visited == temp {
                    some_valid = true;
                    combined_cycle = temp;
                    cycle_edge_masks.remove(i);
                    break;
                }
            }
            if !some_valid {
                break;
            }
        }
        // display combined cycle
        for (i, &value) in combined_cycle.iter().enumerate() {
            if value {
                let (a, b) = edges[i];
                let start = nodes.iter().find(|&(_, &index)| index == a).unwrap().0;
                let end = nodes.iter().find(|&(_, &index)| index == b).unwrap().0;
                gizmos
                    .arrows
                    .push((start.as_vec2(), end.as_vec2(), Color::srgb(1.0, 0.0, 0.0)));
            }
        }

        // show cycles
        // let points: HashMap<_, _> = nodes.iter().map(|(pos, index)| (*index, *pos)).collect();
        // for (index, cycle) in cycles.iter().enumerate() {
        //     let color = Color::srgb(
        //         (index as f32 / cycles.len() as f32).min(1.0),
        //         0.0,
        //         1.0 - (index as f32 / cycles.len() as f32).min(1.0),
        //     );
        //     let com = cycle
        //         .iter()
        //         .fold(Vec2::ZERO, |acc, &index| acc + points[&index].as_vec2())
        //         / cycle.len() as f32;
        //     let points = cycle
        //         .iter()
        //         .map(|&index| points[&index].as_vec2() - (points[&index].as_vec2() - com) * 0.1)
        //         .collect::<Vec<_>>();
        //     for i in 0..cycle.len() {
        //         let start = points[i];
        //         let end = points[(i + 1) % cycle.len()];
        //         gizmos.arrows.push((start, end, color));
        //     }
        // }

        // acutual game logic
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

            if start_time.elapsed() > self.max_time {
                final_boards.extend(queue);
                break;
            }
        }

        for board in final_boards.iter_mut() {
            board.score = self.eval_board(&board.board, board.score, gizmos)?;

            // show path in red
            // if board.score > 0.0 {
            //     let red = Color::srgb(1.0, 0.0, 0.0);
            //     let mut head = snake.head;
            //     for dir in board.history.iter() {
            //         gizmos.lines.push((head, head + dir.as_vec2(), red));
            //         head += dir.as_vec2();
            //     }
            // }
        }

        let max_board = final_boards
            .into_iter()
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
            .ok_or(())?;

        let dir = *max_board.history.first().unwrap();

        // show best path in green
        // let green = Color::srgb(0.0, 1.0, 0.0);
        // let mut head = snake.head;
        // for dir in max_board.history {
        //     gizmos.lines.push((head, head + dir.as_vec2(), green));
        //     head += dir.as_vec2();
        // }

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
struct AIGizmos {
    steps: u32,
    lines: Vec<(IVec2, IVec2, Color)>,
    arrows: Vec<(Vec2, Vec2, Color)>,
    points: Vec<(IVec2, Color)>,
}

mod cycles {
    use crate::board::Board;

    pub struct Graph {
        pub graph: Vec<Vec<usize>>,
    }

    impl Graph {
        pub fn from_board(board: &Board) -> Self {
            let mut nodes = std::collections::HashMap::new();
            let mut graph = Vec::new();
            for (pos, cell) in board.cells() {
                if !matches!(cell, crate::board::Cell::Wall) {
                    nodes.insert(pos, nodes.len());
                    graph.push(Vec::new());
                }
            }
            for (node, index) in nodes.iter() {
                for dir in crate::board::Direction::ALL {
                    let next_node = *node + dir.as_vec2();
                    if let Some(next_index) = nodes.get(&next_node) {
                        graph[*index].push(*next_index);
                    }
                }
            }
            Graph { graph }
        }
    }
}
