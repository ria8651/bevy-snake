use crate::{
    board::{Board, Direction},
    GameState, Settings,
};
use bevy::prelude::*;
use std::{collections::VecDeque, time::Duration};

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TickTimer(Timer::from_seconds(1.0, TimerMode::Repeating)))
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
            .add_systems(Startup, setup_game)
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

#[derive(Clone, Copy)]
pub struct InputMap {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub shoot: KeyCode,
}

pub fn setup_game(mut commands: Commands) {
    commands.spawn(Board::small(1));
}

pub fn update_game(
    mut input_queues: ResMut<SnakeInputs>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut timer: ResMut<TickTimer>,
    mut board_query: Query<&mut Board>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
) {
    timer.set_duration(Duration::from_secs_f32(1.0 / settings.tps));
    timer.tick(time.delta());

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

    // if timer.just_finished() {
    //     let inputs: Vec<Option<Direction>> = input_queues
    //         .iter_mut()
    //         .map(|i| i.input_queue.pop_front())
    //         .collect();

    //     for mut board in board_query.iter_mut() {
    //         let snakes = board.snakes();
    //         board.tick_board(&inputs).unwrap();
    //     }
    // }
}
