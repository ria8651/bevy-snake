use apples::{AppleEv, Apples};
use bevy::{
    prelude::*,
    render::{camera::ScalingMode, mesh::PrimitiveTopology},
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};
use bevy_kira_audio::{Audio, AudioPlugin, AudioSource};
use effects::ExplosionEv;
use guns::{Bullet, SpawnBulletEv};
use meshing::*;
use rand::Rng;
use snake::{DamageSnakeEv, InputMap, Snake};
use std::collections::{HashMap, VecDeque};
use walls::{WallEv, Walls};

mod apples;
mod effects;
mod fps_counter;
mod guns;
mod meshing;
mod snake;
mod ui;
mod walls;

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub enum GameState {
    Menu,
    Playing,
    Paused,
    GameOver,
}

pub struct Settings {
    pub interpolation: bool,
    pub tps: f32,
    pub snake_count: u32,
    pub walls: bool,
    pub walls_debug: bool,
    // pub board_size: IVec2,
    pub boom_texture_atlas_handle: Option<Handle<TextureAtlas>>,
    pub boom_sound_handle: Option<Handle<AudioSource>>,
}

pub struct Board {
    width: i32,
    height: i32,
    colour1: Color,
    colour2: Color,
}

pub struct MovmentTimer(Timer);
pub struct BulletTimer(Timer);
pub struct GameTimer(Timer);
#[derive(Component, Deref, DerefMut)]
pub struct AnimationTimer(Timer);

struct Colours {
    colours: Vec<Color>,
}

fn main() {
    let movment_timer = Timer::from_seconds(1.0 / 4.0, true);

    let mut app = App::new();

    #[cfg(target_arch = "wasm32")]
    {
        app.add_plugin(bevy_web_resizer::Plugin);
    }

    app.add_plugins(DefaultPlugins)
        .add_plugin(AudioPlugin)
        .insert_resource(WindowDescriptor {
            title: "Snake".to_string(),
            ..default()
        })
        .insert_resource(ClearColor(Color::rgb(0.1, 0.1, 0.1)))
        .insert_resource(Board {
            width: 17,
            height: 15,
            colour1: Color::rgb(0.3, 0.5, 0.3),
            colour2: Color::rgb(0.25, 0.45, 0.25),
        })
        .insert_resource(Settings {
            interpolation: true,
            tps: 5.0,
            snake_count: 4,
            walls: false,
            walls_debug: false,
            boom_texture_atlas_handle: None,
            boom_sound_handle: None,
        })
        .insert_resource(MovmentTimer(movment_timer.clone()))
        .insert_resource(BulletTimer(movment_timer))
        .insert_resource(GameTimer(Timer::from_seconds(99999.0, false)))
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
        .add_plugin(fps_counter::FpsCounter)
        .add_plugin(effects::EffectsPlugin)
        .add_plugin(ui::UiPlugin)
        .add_plugin(snake::SnakePlugin)
        .add_plugin(walls::WallPlugin)
        .add_plugin(guns::GunPlugin)
        .add_plugin(apples::ApplePlugin)
        .add_system(bevy::input::system::exit_on_esc_system)
        .add_state(GameState::Menu)
        .add_event::<ExplosionEv>()
        .add_event::<DamageSnakeEv>()
        .add_event::<SpawnBulletEv>()
        .add_event::<AppleEv>()
        .add_event::<WallEv>()
        .add_startup_system(scene_setup)
        .add_system(game_state)
        .add_system_set(SystemSet::on_update(GameState::Playing).with_system(settings_system))
        .add_system_set(SystemSet::on_enter(GameState::Playing).with_system(reset_game))
        .run();
}

fn game_state(
    mut game_state: ResMut<State<GameState>>,
    keys: Res<Input<KeyCode>>,
    snake_query: Query<&Snake>,
) {
    match game_state.current() {
        GameState::Menu => game_state.set(GameState::Playing).unwrap(),
        GameState::Playing => {
            if snake_query.iter().count() < 1 {
                game_state.set(GameState::GameOver).unwrap();
            }
            if keys.just_pressed(KeyCode::P) {
                game_state.push(GameState::Paused).unwrap();
            }
        }
        GameState::Paused => {
            if keys.just_pressed(KeyCode::P) {
                game_state.pop().unwrap();
            }
        }
        GameState::GameOver => {
            if keys.just_pressed(KeyCode::Space) {
                game_state.set(GameState::Playing).unwrap();
            }
        }
    }
}

fn scene_setup(
    mut commands: Commands,
    b: Res<Board>,
    mut apples: ResMut<Apples>,
    asset_server: Res<AssetServer>,
    mut texture_atlases: ResMut<Assets<TextureAtlas>>,
    mut settings: ResMut<Settings>,
) {
    apples.sprite = Some(asset_server.load("images/apple.png"));

    commands.spawn_bundle(UiCameraBundle::default());
    commands.spawn_bundle(OrthographicCameraBundle {
        orthographic_projection: OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical,
            scale: b.height as f32 / 2.0,
            ..Default::default()
        },
        transform: Transform::from_xyz(0.0, 0.0, 500.0),
        ..OrthographicCameraBundle::new_2d()
    });

    for x in 0..b.width {
        for y in 0..b.height {
            let color = if (x + y) % 2 == 0 {
                b.colour1
            } else {
                b.colour2
            };

            commands.spawn_bundle(SpriteBundle {
                sprite: Sprite { color, ..default() },
                transform: Transform::from_xyz(
                    x as f32 - b.width as f32 / 2.0 + 0.5,
                    y as f32 - b.height as f32 / 2.0 + 0.5,
                    -1.0,
                ),
                ..default()
            });
        }
    }

    let texture_handle = asset_server.load("images/spritesheet.png");
    let texture_atlas = TextureAtlas::from_grid(texture_handle, Vec2::new(512.0, 512.0), 31, 1);
    let texture_atlas_handle = texture_atlases.add(texture_atlas);
    settings.boom_texture_atlas_handle = Some(texture_atlas_handle);

    settings.boom_sound_handle = Some(asset_server.load("sounds/boom.ogg"));

    // song
    // let music = asset_server.load("sounds/song.ogg");
    // audio.play_looped(music);
}

fn settings_system(
    mut settings: ResMut<Settings>,
    keys: Res<Input<KeyCode>>,
    mut game_timer: ResMut<GameTimer>,
    time: Res<Time>,
) {
    if keys.just_pressed(KeyCode::I) {
        settings.interpolation = !settings.interpolation;
    }

    game_timer.0.tick(time.delta());
    settings.tps = (game_timer.0.elapsed_secs() * 0.1 + 5.0).clamp(5.0, 8.0);
}

fn reset_game(
    snake_query: Query<Entity, With<Snake>>,
    bullet_query: Query<Entity, With<Bullet>>,
    mut commands: Commands,
    mut apples: ResMut<Apples>,
    mut walls: ResMut<Walls>,
    mut game_timer: ResMut<GameTimer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    b: Res<Board>,
    mut apple_ev: EventWriter<AppleEv>,
    colours: Res<Colours>,
    settings: Res<Settings>,
) {
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

    for _ in 0..3 {
        apple_ev.send(AppleEv::SpawnRandom);
    }

    game_timer.0.reset();

    // spawn in new snakes
    let snake_controls = vec![
        InputMap {
            up: KeyCode::W,
            down: KeyCode::S,
            left: KeyCode::A,
            right: KeyCode::D,
            shoot: KeyCode::LShift,
        },
        InputMap {
            up: KeyCode::Up,
            down: KeyCode::Down,
            left: KeyCode::Left,
            right: KeyCode::Right,
            shoot: KeyCode::RAlt,
        },
        InputMap {
            up: KeyCode::P,
            down: KeyCode::Semicolon,
            left: KeyCode::L,
            right: KeyCode::Apostrophe,
            shoot: KeyCode::Backslash,
        },
        InputMap {
            up: KeyCode::Y,
            down: KeyCode::H,
            left: KeyCode::G,
            right: KeyCode::J,
            shoot: KeyCode::B,
        },
    ];
    let positions = vec![
        vec![
            IVec2::new(12, 13),
            IVec2::new(13, 13),
            IVec2::new(14, 13),
            IVec2::new(15, 13),
        ],
        vec![
            IVec2::new(4, 1),
            IVec2::new(3, 1),
            IVec2::new(2, 1),
            IVec2::new(1, 1),
        ],
        vec![
            IVec2::new(12, 10),
            IVec2::new(13, 10),
            IVec2::new(14, 10),
            IVec2::new(15, 10),
        ],
        vec![
            IVec2::new(4, 4),
            IVec2::new(3, 4),
            IVec2::new(2, 4),
            IVec2::new(1, 4),
        ],
    ];

    let transform = Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 0.0);

    for i in 0..settings.snake_count as usize {
        commands
            .spawn_bundle(MaterialMesh2dBundle {
                material: materials.add(ColorMaterial::from(colours.colours[i])),
                transform,
                ..default()
            })
            .insert(Snake {
                id: i as u32,
                body: positions[i].clone(),
                input_map: snake_controls[i],
                ..Default::default()
            });
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
