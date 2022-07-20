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
use snake::{DamageSnakeEv, Snake};
use std::collections::{HashMap, VecDeque};

mod effects;
mod fps_counter;
mod guns;
mod meshing;
mod snake;

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub enum GameState {
    Menu,
    Playing,
    Paused,
    GameOver,
}

pub struct Settings {
    interpolation: bool,
    tps: f32,
    boom_texture_atlas_handle: Option<Handle<TextureAtlas>>,
    boom_sound_handle: Option<Handle<AudioSource>>,
}

#[derive(Clone, Copy)]
pub struct InputMap {
    up: KeyCode,
    down: KeyCode,
    left: KeyCode,
    right: KeyCode,
    shoot: KeyCode,
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

pub struct Apples {
    list: HashMap<IVec2, Entity>,
    apples_to_spawn: Vec<AppleSpawn>,
    sprite: Option<Handle<Image>>,
}

#[derive(Copy, Clone)]
enum AppleSpawn {
    Random,
    Pos(IVec2),
}

fn main() {
    let movment_timer = Timer::from_seconds(1.0 / 4.0, true);

    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugin(AudioPlugin)
        .insert_resource(Board {
            width: 17,
            height: 15,
            colour1: Color::rgb(0.3, 0.3, 0.3),
            colour2: Color::rgb(0.2, 0.2, 0.2),
        })
        .insert_resource(Settings {
            interpolation: true,
            tps: 8.0,
            boom_texture_atlas_handle: None,
            boom_sound_handle: None,
        })
        .insert_resource(MovmentTimer(movment_timer.clone()))
        .insert_resource(BulletTimer(movment_timer))
        .insert_resource(GameTimer(Timer::from_seconds(99999.0, false)))
        .insert_resource(Apples {
            list: HashMap::new(),
            apples_to_spawn: Vec::new(),
            sprite: None,
        })
        .add_plugin(fps_counter::FpsCounter)
        .add_system(bevy::input::system::exit_on_esc_system)
        .add_plugin(effects::EffectsPlugin)
        .add_plugin(snake::SnakePlugin)
        .add_plugin(guns::GunPlugin)
        .add_event::<ExplosionEv>()
        .add_event::<DamageSnakeEv>()
        .add_event::<SpawnBulletEv>()
        .add_startup_system(scene_setup)
        .add_startup_system(reset_game)
        .add_system(game_state)
        .add_system(settings_system)
        .add_state(GameState::Menu)
        .add_system_set(SystemSet::on_update(GameState::Playing).with_system(spawn_apples))
        .add_system_set(SystemSet::on_enter(GameState::Playing).with_system(reset_game))
        .run();
}

fn game_state(
    mut game_state: ResMut<State<GameState>>,
    keys: Res<Input<KeyCode>>,
    mut settings: ResMut<Settings>,
    time: Res<Time>,
    mut game_timer: ResMut<GameTimer>,
    snake_query: Query<&Snake>,
) {
    match game_state.current() {
        GameState::Menu => game_state.set(GameState::Playing).unwrap(),
        GameState::Playing => {
            if snake_query.is_empty() {
                game_state.set(GameState::GameOver).unwrap();
            }
            if keys.just_pressed(KeyCode::P) {
                game_state.push(GameState::Paused).unwrap()
            }
        }
        GameState::Paused => {
            if keys.just_pressed(KeyCode::P) {
                game_state.pop().unwrap()
            }
        }
        GameState::GameOver => {
            if keys.just_pressed(KeyCode::Space) {
                game_state.set(GameState::Playing).unwrap()
            }
        }
    }

    game_timer.0.tick(time.delta());
    settings.tps = (game_timer.0.elapsed_secs() * 0.1 + 5.0).clamp(5.0, 8.0);
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
        transform: Transform::from_xyz(0.0, 0.0, 999.9),
        ..OrthographicCameraBundle::new_2d()
    });

    commands.spawn_bundle(SpriteBundle {
        sprite: Sprite {
            color: Color::rgb(0.1, 0.1, 0.1),
            custom_size: Some(Vec2::new(1000.0, 1000.0)),
            ..default()
        },
        ..default()
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
                transform: Transform {
                    translation: Vec3::new(
                        x as f32 - b.width as f32 / 2.0 + 0.5,
                        y as f32 - b.height as f32 / 2.0 + 0.5,
                        0.0,
                    ),
                    ..default()
                },
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

fn settings_system(mut settings: ResMut<Settings>, keys: Res<Input<KeyCode>>) {
    if keys.just_pressed(KeyCode::I) {
        settings.interpolation = !settings.interpolation;
    }
}

fn reset_game(
    snake_query: Query<Entity, With<Snake>>,
    bullet_query: Query<Entity, With<Bullet>>,
    mut commands: Commands,
    mut apples: ResMut<Apples>,
    mut game_timer: ResMut<GameTimer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    b: Res<Board>,
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
    apples.apples_to_spawn = vec![AppleSpawn::Random; 3];

    game_timer.0.reset();

    // spawn in new snakes
    let snake_colours = [
        Color::rgb(0.0, 0.7, 0.25),
        Color::rgb(0.3, 0.4, 0.7),
        Color::rgb(0.7, 0.4, 0.3),
        Color::rgb(0.7, 0.7, 0.7),
    ];
    let snake_controls = [
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
    let positions = [
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

    let transform = Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 10.0);

    for i in 0..4 {
        commands
            .spawn_bundle(MaterialMesh2dBundle {
                material: materials.add(ColorMaterial::from(snake_colours[i])),
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

fn spawn_apples(
    mut apples: ResMut<Apples>,
    snake_query: Query<&Snake>,
    mut commands: Commands,
    b: Res<Board>,
) {
    let mut rng = rand::thread_rng();

    while let Some(apple) = apples.apples_to_spawn.pop() {
        let mut pos;
        let mut count = 0;
        'inner: loop {
            pos = if let AppleSpawn::Pos(pos) = apple {
                pos
            } else {
                IVec2::new(rng.gen_range(0..b.width), rng.gen_range(0..b.height))
            };

            count += 1;
            if count > 1000 {
                return;
            }

            if apples.list.contains_key(&pos) {
                continue 'inner;
            }

            for snake in snake_query.iter() {
                if snake.body.contains(&pos) {
                    continue 'inner;
                }
            }

            break;
        }

        let texture = apples.sprite.as_ref().unwrap().clone();
        apples.list.insert(
            pos,
            commands
                .spawn_bundle(SpriteBundle {
                    texture: texture,
                    transform: Transform::from_xyz(
                        pos.x as f32 - b.width as f32 / 2.0 + 0.5,
                        pos.y as f32 - b.height as f32 / 2.0 + 0.5,
                        5.0,
                    )
                    .with_scale(Vec3::splat(1.0 / 512.0)),
                    ..default()
                })
                .id(),
        );
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
