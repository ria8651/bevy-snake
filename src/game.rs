use crate::{
    board::{Board, BoardEvent, Direction},
    web::{WebCommands, WebResources, WebUpdates},
    GameState, Settings,
};
use bevy::prelude::*;
use std::{collections::VecDeque, time::Duration};

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TickTimer(Timer::from_seconds(1.0, TimerMode::Repeating)))
            .insert_resource(Board::empty(0, 0))
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
            .add_systems(OnEnter(GameState::Start), reset_game)
            .add_systems(Update, update_game.run_if(in_state(GameState::InGame)));
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
    settings: Res<Settings>,
) {
    *board = Board::new(settings.board_settings);

    for SnakeInput { input_queue, .. } in input_queues.iter_mut() {
        input_queue.clear();
    }
}

pub fn update_game(
    mut input_queues: ResMut<SnakeInputs>,
    mut timer: ResMut<TickTimer>,
    mut board: ResMut<Board>,
    mut next_game_state: ResMut<NextState<GameState>>,
    mut web_resources: ResMut<WebResources>,
    mut points: ResMut<Points>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
) {
    if settings.do_game_tick {
        timer.set_duration(Duration::from_secs_f32(1.0 / settings.tps));
        timer.tick(time.delta());
    } else {
        timer.reset();
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

                if let Some(&last_in_queue) = last_in_queue {
                    if input != last_in_queue && input != last_in_queue.opposite() {
                        input_queue.push_back(input);
                    }
                } else {
                    input_queue.push_back(input);
                }
            }
        }
    }

    if timer.just_finished() || !settings.do_game_tick {
        let inputs: Vec<Option<Direction>> = input_queues
            .iter_mut()
            .map(|i| {
                i.input_queue.pop_front().or_else(|| {
                    web_resources
                        .web_commands
                        .try_recv()
                        .ok()
                        .and_then(|c| match c {
                            WebCommands::SendInput { direction } => Some(direction),
                        })
                })
            })
            .collect();

        let snakes = board.snakes();
        if inputs[0..snakes.len()].iter().any(|i| i.is_some()) || settings.do_game_tick {
            match board.tick_board(&inputs) {
                Ok(events) => {
                    for event in events {
                        match event {
                            BoardEvent::GameOver => {
                                next_game_state.set(GameState::GameOver);
                            }
                            BoardEvent::SnakeDamaged { .. } => {
                                for (snake_id, snake) in board.snakes().into_iter().enumerate() {
                                    if snake.len() > 0 {
                                        points[snake_id as usize] += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Board tick error: {:?}", e);
                    next_game_state.set(GameState::GameOver);
                }
            }

            web_resources
                .web_updates
                .send(WebUpdates::UpdateBoard {
                    board: board.clone(),
                })
                .unwrap();
        }
    }
}
