use bevy::prelude::*;
use board::{AppleCount, Board, BoardSettings, BoardSize, PlayerCount};
use effects::ExplosionEv;

mod board;
mod effects;
mod game;
mod render;
mod ui;
mod web;

#[derive(States, Default, Debug, Hash, PartialEq, Eq, Clone)]
pub enum GameState {
    #[default]
    Setup,
    Start,
    InGame,
    GameOver,
}

#[derive(PartialEq, Eq)]
pub enum Speed {
    Slow,
    Medium,
    Fast,
}

#[derive(Resource, Reflect)]
pub struct Settings {
    pub interpolation: bool,
    pub do_game_tick: bool,
    pub tps: f32,
    pub tps_ramp: bool,
    pub board_settings: BoardSettings,
    pub walls: bool,
    pub walls_debug: bool,
}

#[derive(Resource, Default)]
pub struct GameTime(f32);
#[derive(Component, Deref, DerefMut)]
pub struct AnimationTimer(Timer);

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Snake, WITH GUNS!".to_string(),
                    canvas: Some("#bevy".to_string()),
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            }),
            // effects::EffectsPlugin,
            ui::UiPlugin,
            game::GamePlugin,
            render::BoardRenderPlugin,
            web::WebPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.1, 0.1, 0.1)))
        .insert_resource(Settings {
            interpolation: true,
            do_game_tick: true,
            tps: 7.5,
            tps_ramp: false,
            board_settings: BoardSettings {
                board_size: BoardSize::Medium,
                apples: AppleCount::One,
                players: PlayerCount::One,
            },
            walls: false,
            walls_debug: false,
        })
        .insert_resource(GameTime::default())
        .init_state::<GameState>()
        .add_event::<ExplosionEv>()
        .add_systems(Update, game_state.after(game::update_game))
        .add_systems(Update, settings_system.run_if(in_state(GameState::InGame)))
        .run();
}

fn game_state(
    mut next_game_state: ResMut<NextState<GameState>>,
    game_state: Res<State<GameState>>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    board: Res<Board>,
) {
    match game_state.get() {
        GameState::Setup => next_game_state.set(GameState::Start),
        GameState::Start => next_game_state.set(GameState::InGame),
        GameState::InGame => {
            let snakes = board.count_snakes();
            if snakes <= (settings.board_settings.players as usize != 1) as usize {
                next_game_state.set(GameState::GameOver);
            }
        }
        GameState::GameOver => {
            if keys.just_pressed(KeyCode::Space) {
                next_game_state.set(GameState::Start);
            }
        }
    }
}

fn settings_system(
    mut settings: ResMut<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mut game_time: ResMut<GameTime>,
    time: Res<Time>,
) {
    if keys.just_pressed(KeyCode::KeyI) {
        settings.interpolation = !settings.interpolation;
    }

    game_time.0 += time.delta_seconds();
    if settings.tps_ramp {
        settings.tps = (game_time.0 * 0.1 + 5.0).clamp(5.0, 7.0);
    }
}

// fn reset_game(
//     snake_query: Query<Entity, With<Snake>>,
//     // bullet_query: Query<Entity, With<Bullet>>,
//     board_query: Query<Entity, With<BoardTile>>,
//     mut camera_query: Query<&mut OrthographicProjection, With<MainCamera>>,
//     mut commands: Commands,
//     mut apples: ResMut<Apples>,
//     mut walls: ResMut<Walls>,
//     mut game_time: ResMut<GameTime>,
//     mut materials: ResMut<Assets<ColorMaterial>>,
//     mut b: ResMut<Board>,
//     mut apple_ev: EventWriter<AppleEv>,
//     colours: Res<Colours>,
//     settings: Res<Settings>,
// ) {
//     for tile in board_query.iter() {
//         commands.entity(tile).despawn();
//     }

//     *b = match settings.board_size {
//         BoardSize::Small => Board::small(1),
//         BoardSize::Medium => unimplemented!("Medium board size"),
//         BoardSize::Large => unimplemented!("Large board size"),
//     };

//     let mut camera_projection = camera_query.single_mut();
//     camera_projection.scaling_mode = ScalingMode::AutoMin {
//         min_height: b.height() as f32,
//         min_width: b.width() as f32,
//     };

//     for x in 0..b.width() {
//         for y in 0..b.height() {
//             let color = if (x + y) % 2 == 0 {
//                 Color::srgb(0.3, 0.5, 0.3)
//             } else {
//                 Color::srgb(0.25, 0.45, 0.25)
//             };

//             commands.spawn((
//                 SpriteBundle {
//                     sprite: Sprite { color, ..default() },
//                     transform: Transform::from_xyz(
//                         x as f32 - b.width() as f32 / 2.0 + 0.5,
//                         y as f32 - b.height() as f32 / 2.0 + 0.5,
//                         -1.0,
//                     ),
//                     ..default()
//                 },
//                 BoardTile,
//             ));
//         }
//     }

//     for snake_entity in snake_query.iter() {
//         commands.entity(snake_entity).despawn();
//     }
//     // for bullet_entity in bullet_query.iter() {
//     //     commands.entity(bullet_entity).despawn();
//     // }

//     for apple in apples.list.iter().clone() {
//         commands.entity(*apple.1).despawn();
//     }
//     apples.list = HashMap::new();

//     for apple in walls.list.iter().clone() {
//         commands.entity(*apple.1).despawn();
//     }
//     walls.list = HashMap::new();

//     for _ in 0..settings.apple_count {
//         apple_ev.send(AppleEv::SpawnRandom);
//     }

//     game_time.0 = 0.0;

//     // spawn in new snakes
//     let snake_controls = vec![
//         InputMap {
//             up: KeyCode::KeyW,
//             down: KeyCode::KeyS,
//             left: KeyCode::KeyA,
//             right: KeyCode::KeyD,
//             shoot: KeyCode::ShiftLeft,
//         },
//         InputMap {
//             up: KeyCode::ArrowUp,
//             down: KeyCode::ArrowDown,
//             left: KeyCode::ArrowLeft,
//             right: KeyCode::ArrowRight,
//             shoot: KeyCode::AltRight,
//         },
//         InputMap {
//             up: KeyCode::KeyP,
//             down: KeyCode::Semicolon,
//             left: KeyCode::KeyL,
//             right: KeyCode::Quote,
//             shoot: KeyCode::Backslash,
//         },
//         InputMap {
//             up: KeyCode::KeyY,
//             down: KeyCode::KeyH,
//             left: KeyCode::KeyG,
//             right: KeyCode::KeyJ,
//             shoot: KeyCode::KeyB,
//         },
//     ];
//     let positions = vec![
//         vec![
//             IVec2::new(4, b.height() as i32 - 2),
//             IVec2::new(3, b.height() as i32 - 2),
//             IVec2::new(2, b.height() as i32 - 2),
//             IVec2::new(1, b.height() as i32 - 2),
//         ],
//         vec![
//             IVec2::new(b.width() as i32 - 5, 1),
//             IVec2::new(b.width() as i32 - 4, 1),
//             IVec2::new(b.width() as i32 - 3, 1),
//             IVec2::new(b.width() as i32 - 2, 1),
//         ],
//         vec![
//             IVec2::new(b.width() as i32 - 2, b.height() as i32 - 5),
//             IVec2::new(b.width() as i32 - 2, b.height() as i32 - 4),
//             IVec2::new(b.width() as i32 - 2, b.height() as i32 - 3),
//             IVec2::new(b.width() as i32 - 2, b.height() as i32 - 2),
//         ],
//         vec![
//             IVec2::new(1, 4),
//             IVec2::new(1, 3),
//             IVec2::new(1, 2),
//             IVec2::new(1, 1),
//         ],
//     ];

//     let transform = Transform::from_xyz(-(b.width() as f32) / 2.0, -(b.height() as f32) / 2.0, 0.0);

//     for i in 0..settings.snake_count as usize {
//         commands.spawn((
//             MaterialMesh2dBundle {
//                 material: materials.add(ColorMaterial::from(colours.colours[i])),
//                 transform,
//                 ..default()
//             },
//             Snake {
//                 id: i as u32,
//                 body: positions[i].clone(),
//                 input_map: snake_controls[i],
//                 ..Default::default()
//             },
//         ));
//     }
// }
