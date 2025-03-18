use crate::{client::PlayerConnections, GameState, GizmoSetting, Settings};
use bevy::{prelude::*, utils::HashMap};
use bevy_snake::{
    ai::{cycle_basis, AIGizmos, SnakeAI, TreeSearch},
    board::{Board, BoardEvent, Cell, Direction},
    GameCommands, GameUpdates,
};
use rand::{rngs::StdRng, SeedableRng};
use std::{collections::VecDeque, time::Duration};

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TickTimer(Timer::from_seconds(1.0 / 7.5, TimerMode::Once)))
            .insert_resource(Board::empty(0, 0))
            .insert_resource(Rng(StdRng::from_os_rng()))
            .insert_resource(Points(vec![0; 4]))
            .insert_resource(SnakeInputs(vec![
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyW,
                        down: KeyCode::KeyS,
                        left: KeyCode::KeyA,
                        right: KeyCode::KeyD,
                        shoot: KeyCode::Space,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::ArrowUp,
                        down: KeyCode::ArrowDown,
                        left: KeyCode::ArrowLeft,
                        right: KeyCode::ArrowRight,
                        shoot: KeyCode::AltRight,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyP,
                        down: KeyCode::Semicolon,
                        left: KeyCode::KeyL,
                        right: KeyCode::Quote,
                        shoot: KeyCode::Backslash,
                    },
                    input_queue: VecDeque::new(),
                },
                SnakeInput {
                    input_map: InputMap {
                        up: KeyCode::KeyY,
                        down: KeyCode::KeyH,
                        left: KeyCode::KeyG,
                        right: KeyCode::KeyJ,
                        shoot: KeyCode::KeyB,
                    },
                    input_queue: VecDeque::new(),
                },
            ]))
            .add_systems(OnEnter(GameState::InGame), reset_game)
            .add_systems(Update, update_game);
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct TickTimer(Timer);

#[derive(Resource, Deref, DerefMut)]
pub struct SnakeInputs(Vec<SnakeInput>);

#[derive(Resource, Deref, DerefMut)]
pub struct Points(Vec<usize>);

pub struct SnakeInput {
    pub input_map: InputMap,
    pub input_queue: VecDeque<Direction>,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct InputMap {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub shoot: KeyCode,
}

pub fn reset_game(
    mut board: ResMut<Board>,
    mut input_queues: ResMut<SnakeInputs>,
    player_connections: Res<PlayerConnections>,
    settings: Res<Settings>,
) {
    *board = Board::new(settings.board_settings);

    for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
        input_queue.clear();
    }

    let (_game_updates, game_commands) = &player_connections[0];
    game_commands
        .send(GameCommands::RestartGame {
            board_settings: settings.board_settings.clone(),
        })
        .unwrap();
}

#[derive(Resource, Deref, DerefMut)]
pub struct Rng(StdRng);

pub fn update_game(
    mut input_queues: ResMut<SnakeInputs>,
    mut timer: ResMut<TickTimer>,
    mut board: ResMut<Board>,
    mut points: ResMut<Points>,
    mut next_game_state: ResMut<NextState<GameState>>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    player_connections: Res<PlayerConnections>,
    mut server_tick: Local<u64>,
) {
    timer.tick(time.delta());

    let (game_updates, game_commands) = &player_connections[0];

    if let Ok(game_updates) = game_updates.try_recv() {
        match game_updates {
            GameUpdates::Ticked {
                board: new_board,
                events,
                tick,
            } => {
                info!("received {}", tick);

                *board = new_board;
                for event in events {
                    match event {
                        BoardEvent::GameOver => {
                            info!("game over");
                            next_game_state.set(GameState::GameOver);
                            return;
                        }
                        BoardEvent::SnakeDamaged { .. } => {
                            for (snake_id, _) in board.snakes().into_iter() {
                                points[snake_id as usize] += 1;
                            }
                        }
                        _ => {}
                    }
                }

                *server_tick = tick;
                timer.reset();

                for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
                    // first input is sent immediately, next is sent at the start of the next tick
                    input_queue.pop_front();

                    // send next input in queue *without* popping it
                    if let Some(&direction) = input_queue.front() {
                        let input = GameCommands::Input { direction, tick };
                        game_commands.send(input).unwrap();
                    }
                }

                // let ai = TreeSearch {
                //     max_depth: 100,
                //     max_time: Duration::from_millis(5),
                // };
                // if let Ok(dir) = ai.chose_move(board.as_ref(), &mut None) {
                //     let (_game_updates, game_commands) = &player_connections[0];
                //     let input = GameCommands::Input {
                //         direction: dir,
                //         tick,
                //     };
                //     game_commands.send(input).unwrap();
                // }
            }
        }
    }

    for SnakeInput {
        input_map,
        input_queue,
    } in input_queues.iter_mut()
    {
        if input_queue.len() < 3 {
            if let Some(input) = if keys.just_pressed(input_map.up) {
                Some(Direction::Up)
            } else if keys.just_pressed(input_map.down) {
                Some(Direction::Down)
            } else if keys.just_pressed(input_map.left) {
                Some(Direction::Left)
            } else if keys.just_pressed(input_map.right) {
                Some(Direction::Right)
            } else {
                None
            } {
                let last_in_queue = input_queue.back();

                // dont put same direction in queue twice
                if last_in_queue.is_some_and(|&last| input == last || input == last.opposite()) {
                    continue;
                }

                // if queue is empty, send input immediately
                if input_queue.is_empty() {
                    let input = GameCommands::Input {
                        direction: input,
                        tick: *server_tick,
                    };
                    game_commands.send(input).unwrap();
                }

                // still add input to queue to mark that an input has been sent
                input_queue.push_back(input);
            }
        }
    }
}

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

        if let Ok(dir) = ai.chose_move(board.as_ref(), &mut Some(&mut new_ai_gizmos)) {
            *ai_gizmos = new_ai_gizmos;

            let input_queue = &mut input_queues[0].input_queue;
            if settings.ai && input_queue.is_empty() {
                input_queue.push_back(dir);
            }
        }
    }

    if let GizmoSetting::CycleBasis = settings.gizmos {
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

        // show cycles
        let points: HashMap<_, _> = nodes.iter().map(|(pos, index)| (*index, *pos)).collect();
        for (index, cycle) in cycles.iter().enumerate() {
            let color = Color::srgb(
                (index as f32 / cycles.len() as f32).min(1.0),
                0.0,
                1.0 - (index as f32 / cycles.len() as f32).min(1.0),
            );
            let com = cycle
                .iter()
                .fold(Vec2::ZERO, |acc, &index| acc + points[&index].as_vec2())
                / cycle.len() as f32;
            let points = cycle
                .iter()
                .map(|&index| points[&index].as_vec2() - (points[&index].as_vec2() - com) * 0.1)
                .collect::<Vec<_>>();
            for i in 0..cycle.len() {
                let start = points[i];
                let end = points[(i + 1) % cycle.len()];
                ai_gizmos.arrows.push((start, end, color));
            }
        }

        // combine cycles
        let mut edges = HashMap::new();
        let mut index = 0;
        for (cell, neighbors) in graph.iter().enumerate() {
            for &neighbor in neighbors {
                if neighbor < cell {
                    edges.insert((neighbor, cell), index);
                    index += 1;
                }
            }
        }
        let mut edge_cycles = Vec::new();
        for cycle in cycles.iter() {
            let mut edge_cycle = Vec::new();
            for i in 0..cycle.len() {
                let mut a = cycle[i];
                let mut b = cycle[(i + 1) % cycle.len()];
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                edge_cycle.push(*edges.get(&(a.min(b), a.max(b))).unwrap());
            }
            edge_cycles.push(edge_cycle);
        }
    }

    if !matches!(settings.gizmos, GizmoSetting::None) {
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
