use apples::{AppleEv, Apples};
use bevy::{
    prelude::*,
    render::{camera::ScalingMode, mesh::PrimitiveTopology},
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};
use effects::ExplosionEv;
use guns::{Bullet, SpawnBulletEv};
use meshing::*;
use rand::Rng;
use snake::{DamageSnakeEv, InputMap, Snake};
use std::collections::{HashMap, VecDeque};
use walls::{WallEv, Walls};

mod apples;
mod effects;
mod guns;
mod meshing;
mod snake;
mod ui;
mod walls;

#[derive(PartialEq, Eq, Hash, Default, Copy, Clone, Debug, States)]
pub enum GameState {
    #[default]
    Menu,
    Playing,
    Paused,
    GameOver,
}

#[derive(PartialEq, Eq)]
pub enum BoardSize {
    Small,
    Medium,
    Large,
}

#[derive(PartialEq, Eq)]
pub enum Speed {
    Slow,
    Medium,
    Fast,
}

#[derive(Resource)]
pub struct Settings {
    pub interpolation: bool,
    pub tps: f32,
    pub tps_ramp: bool,
    pub snake_count: u32,
    pub apple_count: u32,
    pub board_size: BoardSize,
    pub walls: bool,
    pub walls_debug: bool,
}

#[derive(Resource)]
pub struct Board {
    width: i32,
    height: i32,
    colour1: Color,
    colour2: Color,
}

#[derive(Resource)]
pub struct MovmentTimer(Timer);
#[derive(Resource)]
pub struct BulletTimer(Timer);
#[derive(Resource, Default)]
pub struct GameTime(f32);
#[derive(Component, Deref, DerefMut)]
pub struct AnimationTimer(Timer);

#[derive(Resource)]
struct Colours {
    colours: Vec<Color>,
}

#[derive(Component)]
struct BoardTile;
#[derive(Component)]
struct MainCamera;

fn main() {
    let movment_timer = Timer::from_seconds(1.0 / 4.0, TimerMode::Repeating);

    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Snake, WITH GUNS!".to_string(),
                    ..default()
                }),
                ..default()
            }),
            effects::EffectsPlugin,
            ui::UiPlugin,
            snake::SnakePlugin,
            walls::WallPlugin,
            guns::GunPlugin,
            apples::ApplePlugin,
        ))
        .insert_resource(ClearColor(Color::rgb(0.1, 0.1, 0.1)))
        .insert_resource(Board {
            width: 10,
            height: 9,
            colour1: Color::rgb(0.3, 0.5, 0.3),
            colour2: Color::rgb(0.25, 0.45, 0.25),
        })
        .insert_resource(Settings {
            interpolation: true,
            tps: 7.5,
            tps_ramp: false,
            snake_count: 1,
            apple_count: 3,
            board_size: BoardSize::Medium,
            walls: false,
            walls_debug: false,
        })
        .insert_resource(MovmentTimer(movment_timer.clone()))
        .insert_resource(BulletTimer(movment_timer))
        .insert_resource(GameTime::default())
        .insert_resource(Apples {
            list: HashMap::new(),
            sprite: None,
        })
        .insert_resource(Walls {
            list: HashMap::new(),
        })
        .insert_resource(Colours {
            colours: vec![
                Color::rgb(0.0, 0.7, 0.25),
                Color::rgb(0.3, 0.4, 0.7),
                Color::rgb(0.7, 0.4, 0.3),
                Color::rgb(0.7, 0.7, 0.7),
            ],
        })
        .init_state::<GameState>()
        .add_event::<ExplosionEv>()
        .add_event::<DamageSnakeEv>()
        .add_event::<SpawnBulletEv>()
        .add_event::<AppleEv>()
        .add_event::<WallEv>()
        .add_systems(Startup, scene_setup)
        .add_systems(Update, game_state)
        .add_systems(OnEnter(GameState::Playing), reset_game)
        .add_systems(Update, settings_system.run_if(in_state(GameState::Playing)))
        .run();
}

fn game_state(
    game_state: Res<State<GameState>>,
    mut next_game_state: ResMut<NextState<GameState>>,
    keys: Res<ButtonInput<KeyCode>>,
    snake_query: Query<&Snake>,
    settings: Res<Settings>,
) {
    match game_state.get() {
        GameState::Menu => next_game_state.set(GameState::Playing),
        GameState::Playing => {
            if snake_query.iter().count() <= (settings.snake_count != 1) as usize {
                next_game_state.set(GameState::GameOver);
            }
        }
        GameState::GameOver => {
            if keys.just_pressed(KeyCode::Space) {
                next_game_state.set(GameState::Playing);
            }
        }
        _ => {}
    }
}

fn scene_setup(
    mut commands: Commands,
    mut apples: ResMut<Apples>,
    asset_server: Res<AssetServer>,
    b: Res<Board>,
) {
    apples.sprite = Some(asset_server.load("images/apple.png"));

    commands.spawn((
        Camera2dBundle {
            projection: OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical(b.height as f32),
                ..default()
            },
            transform: Transform::from_xyz(0.0, 0.0, 500.0),
            ..default()
        },
        MainCamera,
    ));
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

fn reset_game(
    snake_query: Query<Entity, With<Snake>>,
    bullet_query: Query<Entity, With<Bullet>>,
    board_query: Query<Entity, With<BoardTile>>,
    mut camera_query: Query<&mut OrthographicProjection, With<MainCamera>>,
    mut commands: Commands,
    mut apples: ResMut<Apples>,
    mut walls: ResMut<Walls>,
    mut game_time: ResMut<GameTime>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut b: ResMut<Board>,
    mut apple_ev: EventWriter<AppleEv>,
    colours: Res<Colours>,
    settings: Res<Settings>,
) {
    for tile in board_query.iter() {
        commands.entity(tile).despawn();
    }

    match settings.board_size {
        BoardSize::Small => {
            b.width = 10;
            b.height = 9;
        }
        BoardSize::Medium => {
            b.width = 17;
            b.height = 15;
        }
        BoardSize::Large => {
            b.width = 24;
            b.height = 21;
        }
    }

    let mut camera_projection = camera_query.single_mut();
    camera_projection.scaling_mode = ScalingMode::FixedVertical(b.height as f32);

    for x in 0..b.width {
        for y in 0..b.height {
            let color = if (x + y) % 2 == 0 {
                b.colour1
            } else {
                b.colour2
            };

            commands.spawn((
                SpriteBundle {
                    sprite: Sprite { color, ..default() },
                    transform: Transform::from_xyz(
                        x as f32 - b.width as f32 / 2.0 + 0.5,
                        y as f32 - b.height as f32 / 2.0 + 0.5,
                        -1.0,
                    ),
                    ..default()
                },
                BoardTile,
            ));
        }
    }

    for snake_entity in snake_query.iter() {
        commands.entity(snake_entity).despawn();
    }
    for bullet_entity in bullet_query.iter() {
        commands.entity(bullet_entity).despawn();
    }

    for apple in apples.list.iter().clone() {
        commands.entity(*apple.1).despawn();
    }
    apples.list = HashMap::new();

    for apple in walls.list.iter().clone() {
        commands.entity(*apple.1).despawn();
    }
    walls.list = HashMap::new();

    for _ in 0..settings.apple_count {
        apple_ev.send(AppleEv::SpawnRandom);
    }

    game_time.0 = 0.0;

    // spawn in new snakes
    let snake_controls = vec![
        InputMap {
            up: KeyCode::KeyW,
            down: KeyCode::KeyS,
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
            shoot: KeyCode::ShiftLeft,
        },
        InputMap {
            up: KeyCode::ArrowUp,
            down: KeyCode::ArrowDown,
            left: KeyCode::ArrowLeft,
            right: KeyCode::ArrowRight,
            shoot: KeyCode::AltRight,
        },
        InputMap {
            up: KeyCode::KeyP,
            down: KeyCode::Semicolon,
            left: KeyCode::KeyL,
            right: KeyCode::Quote,
            shoot: KeyCode::Backslash,
        },
        InputMap {
            up: KeyCode::KeyY,
            down: KeyCode::KeyH,
            left: KeyCode::KeyG,
            right: KeyCode::KeyJ,
            shoot: KeyCode::KeyB,
        },
    ];
    let positions = vec![
        vec![
            IVec2::new(4, b.height - 2),
            IVec2::new(3, b.height - 2),
            IVec2::new(2, b.height - 2),
            IVec2::new(1, b.height - 2),
        ],
        vec![
            IVec2::new(b.width - 5, 1),
            IVec2::new(b.width - 4, 1),
            IVec2::new(b.width - 3, 1),
            IVec2::new(b.width - 2, 1),
        ],
        vec![
            IVec2::new(b.width - 2, b.height - 5),
            IVec2::new(b.width - 2, b.height - 4),
            IVec2::new(b.width - 2, b.height - 3),
            IVec2::new(b.width - 2, b.height - 2),
        ],
        vec![
            IVec2::new(1, 4),
            IVec2::new(1, 3),
            IVec2::new(1, 2),
            IVec2::new(1, 1),
        ],
    ];

    let transform = Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 0.0);

    for i in 0..settings.snake_count as usize {
        commands.spawn((
            MaterialMesh2dBundle {
                material: materials.add(ColorMaterial::from(colours.colours[i])),
                transform,
                ..default()
            },
            Snake {
                id: i as u32,
                body: positions[i].clone(),
                input_map: snake_controls[i],
                ..Default::default()
            },
        ));
    }
}

fn in_bounds(pos: IVec2, b: &Board) -> bool {
    pos.x >= 0 && pos.x < b.width && pos.y >= 0 && pos.y < b.height
}

fn calculate_flip(dir: IVec2) -> IVec2 {
    match dir.to_array() {
        [0, 1] => IVec2::new(1, 0),
        [0, -1] => IVec2::new(-1, 0),
        [1, 0] => IVec2::new(1, 1),
        [-1, 0] => IVec2::new(-1, 1),
        _ => IVec2::new(1, 1),
    }
}
