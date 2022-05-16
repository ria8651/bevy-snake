use bevy::{
    diagnostic::{Diagnostics, FrameTimeDiagnosticsPlugin},
    prelude::*,
    render::{camera::ScalingMode, mesh::PrimitiveTopology},
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};
use bevy_kira_audio::{Audio, AudioPlugin, AudioSource};
use rand::Rng;
use std::collections::{HashMap, VecDeque};

mod meshing;
use meshing::*;

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
enum GameState {
    Menu,
    Playing,
    Paused,
    GameOver,
}

#[derive(Component)]
pub struct Snake {
    id: u32,
    body: Vec<IVec2>,
    input_map: InputMap,
    input_queue: VecDeque<Direction>,
    head_dir: IVec2,
    tail_dir: IVec2,
}

#[derive(Component)]
pub struct Bullet {
    id: u32,
    pos: IVec2,
    dir: IVec2,
    speed: u32,
}

struct Settings {
    interpolation: bool,
    boom_texture_atlas_handle: Option<Handle<TextureAtlas>>,
    boom_sound_handle: Option<Handle<AudioSource>>,
}

struct InputMap {
    up: KeyCode,
    down: KeyCode,
    left: KeyCode,
    right: KeyCode,
    shoot: KeyCode,
}

struct Board {
    width: i32,
    height: i32,
    colour1: Color,
    colour2: Color,
}

struct MovmentTimer(Timer);
struct BulletTimer(Timer);
#[derive(Component, Deref, DerefMut)]
struct AnimationTimer(Timer);

#[derive(PartialEq, Clone, Copy)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

const DIR: [[i32; 2]; 4] = [[0, 1], [0, -1], [-1, 0], [1, 0]];

struct Apples {
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
    let movment_timer = Timer::from_seconds(1.0 / 8.0, true);

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
            boom_texture_atlas_handle: None,
            boom_sound_handle: None,
        })
        .insert_resource(MovmentTimer(movment_timer.clone()))
        .insert_resource(BulletTimer(movment_timer))
        .insert_resource(Apples {
            list: HashMap::new(),
            apples_to_spawn: Vec::new(),
            sprite: None,
        })
        .add_system(bevy::input::system::exit_on_esc_system)
        .add_plugin(FrameTimeDiagnosticsPlugin::default())
        .add_startup_system(scene_setup)
        .add_startup_system(snake_setup)
        .add_startup_system(reset_game)
        .add_system(state_controller)
        .add_system(settings_system)
        .add_state(GameState::Menu)
        .add_system_set(
            SystemSet::on_update(GameState::Playing)
                .with_system(snake_system)
                .with_system(bullet_system)
                .with_system(spawn_apples)
                .with_system(animate_explostions),
        )
        .add_system_set(SystemSet::on_enter(GameState::Playing).with_system(reset_game))
        .add_system(fps_system)
        .run();
}

fn state_controller(mut game_state: ResMut<State<GameState>>, keys: Res<Input<KeyCode>>) {
    match game_state.current() {
        GameState::Menu => game_state.set(GameState::Playing).unwrap(),
        GameState::Playing => {
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
}

fn scene_setup(
    mut commands: Commands,
    b: Res<Board>,
    mut apples: ResMut<Apples>,
    asset_server: Res<AssetServer>,
    mut texture_atlases: ResMut<Assets<TextureAtlas>>,
    mut settings: ResMut<Settings>,
    audio: Res<Audio>,
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

    // fps
    commands.spawn_bundle(TextBundle {
        text: Text {
            sections: vec![TextSection {
                value: "0.00".to_string(),
                style: TextStyle {
                    font: asset_server.load("fonts/FiraMono-Medium.ttf"),
                    font_size: 40.0,
                    color: Color::rgb(1.0, 1.0, 1.0),
                    ..Default::default()
                },
            }],
            ..Default::default()
        },
        style: Style {
            position_type: PositionType::Absolute,
            position: Rect {
                top: Val::Px(10.0),
                left: Val::Px(10.0),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    });

    let texture_handle = asset_server.load("images/spritesheet.png");
    let texture_atlas = TextureAtlas::from_grid(texture_handle, Vec2::new(512.0, 512.0), 31, 1);
    let texture_atlas_handle = texture_atlases.add(texture_atlas);
    settings.boom_texture_atlas_handle = Some(texture_atlas_handle);

    settings.boom_sound_handle = Some(asset_server.load("sounds/boom.ogg"));

    // song
    let music = asset_server.load("sounds/song.ogg");
    audio.play_looped(music);
}

fn snake_setup(
    mut commands: Commands,
    mut materials: ResMut<Assets<ColorMaterial>>,
    b: Res<Board>,
) {
    let snake1 = Snake {
        id: 0,
        body: Vec::new(),
        input_map: InputMap {
            up: KeyCode::W,
            down: KeyCode::S,
            left: KeyCode::A,
            right: KeyCode::D,
            shoot: KeyCode::R,
        },
        input_queue: VecDeque::new(),
        head_dir: IVec2::new(0, 0),
        tail_dir: IVec2::new(0, 0),
    };
    commands
        .spawn_bundle(MaterialMesh2dBundle {
            material: materials.add(ColorMaterial::from(Color::rgb(0.0, 0.7, 0.25))),
            transform: Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 10.0),
            ..default()
        })
        .insert(snake1);

    // let snake2 = Snake {
    //     id: 1,
    //     body: Vec::new(),
    //     input_map: InputMap {
    //         up: KeyCode::Up,
    //         down: KeyCode::Down,
    //         left: KeyCode::Left,
    //         right: KeyCode::Right,
    //         shoot: KeyCode::M,
    //     },
    //     input_queue: VecDeque::new(),
    //     head_dir: IVec2::new(-1, 0),
    //     tail_dir: IVec2::new(1, 0),
    // };
    // commands
    //     .spawn_bundle(MaterialMesh2dBundle {
    //         material: materials.add(ColorMaterial::from(Color::rgb(0.3, 0.4, 0.7))),
    //         transform: Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 10.0),
    //         ..default()
    //     })
    //     .insert(snake2);
}

fn settings_system(mut settings: ResMut<Settings>, keys: Res<Input<KeyCode>>) {
    if keys.just_pressed(KeyCode::I) {
        settings.interpolation = !settings.interpolation;
    }
}

fn reset_game(
    mut snake_query: Query<&mut Snake>,
    bullet_query: Query<(Entity, With<Bullet>)>,
    mut commands: Commands,
    mut apples: ResMut<Apples>,
) {
    for mut snake in snake_query.iter_mut() {
        if snake.id == 0 {
            snake.body = vec![
                IVec2::new(12, 13),
                IVec2::new(13, 13),
                IVec2::new(14, 13),
                IVec2::new(15, 13),
            ];
            snake.head_dir = IVec2::new(1, 0);
            snake.tail_dir = IVec2::new(-1, 0);
        } else if snake.id == 1 {
            snake.body = vec![
                IVec2::new(4, 1),
                IVec2::new(3, 1),
                IVec2::new(2, 1),
                IVec2::new(1, 1),
            ];
            snake.head_dir = IVec2::new(-1, 0);
            snake.tail_dir = IVec2::new(1, 0);
        }
    }

    for bullet in bullet_query.iter() {
        commands.entity(bullet.0).despawn();
    }
    for apple in apples.list.iter().clone() {
        commands.entity(*apple.1).despawn();
    }

    apples.list = HashMap::new();
    apples.apples_to_spawn = vec![AppleSpawn::Random; 3];
}

fn snake_system(
    mut commands: Commands,
    mut snake_query: Query<(&mut Snake, &mut Mesh2dHandle)>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    mut timer: ResMut<MovmentTimer>,
    keys: Res<Input<KeyCode>>,
    mut apples: ResMut<Apples>,
    b: Res<Board>,
    mut app_state: ResMut<State<GameState>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    settings: Res<Settings>,
) {
    timer.0.tick(time.delta());

    for (mut snake, mut mesh_handle) in snake_query.iter_mut() {
        let head = snake.body[0];
        let neck = snake.body[1];
        let current_dir = head - neck;
        let forward = head - neck;

        let last_in_queue = *snake.input_queue.back().unwrap_or(&get_direction(forward));
        if snake.input_queue.len() < 3 {
            if keys.just_pressed(snake.input_map.up) {
                if last_in_queue != Direction::Down && last_in_queue != Direction::Up {
                    snake.input_queue.push_back(Direction::Up);
                }
            }
            if keys.just_pressed(snake.input_map.down) {
                if last_in_queue != Direction::Up && last_in_queue != Direction::Down {
                    snake.input_queue.push_back(Direction::Down);
                }
            }
            if keys.just_pressed(snake.input_map.left) {
                if last_in_queue != Direction::Right && last_in_queue != Direction::Left {
                    snake.input_queue.push_back(Direction::Left);
                }
            }
            if keys.just_pressed(snake.input_map.right) {
                if last_in_queue != Direction::Left && last_in_queue != Direction::Right {
                    snake.input_queue.push_back(Direction::Right);
                }
            }
        }

        let len = snake.body.len();
        if keys.just_pressed(snake.input_map.shoot) && len > 2 {
            spawn_bullet(
                snake.id,
                head,
                current_dir,
                &mut commands,
                &mut materials,
                &b,
            );
            snake.body.remove(len - 1);
        }

        if timer.0.just_finished() {
            if let Some(direction) = snake.input_queue.pop_front() {
                let dir: IVec2 = DIR[direction as usize].into();
                snake.body.insert(0, head + dir);
            } else {
                snake.body.insert(0, head + current_dir);
            }

            let head = snake.body[0];
            if let Some(apple_entity) = apples.list.get(&head) {
                commands.entity(*apple_entity).despawn();
                apples.list.remove(&head);

                apples.apples_to_spawn.push(AppleSpawn::Random);
            } else {
                let len = snake.body.len();
                snake.tail_dir = snake.body[len - 2] - snake.body[len - 1];

                // Shrink Snake
                snake.body.remove(len - 1);
            }
        }
        snake.head_dir = if let Some(dir) = snake.input_queue.get(0) {
            DIR[*dir as usize].into()
        } else {
            head - neck
        };

        let interpolation = if settings.interpolation {
            timer.0.elapsed_secs() / timer.0.duration().as_secs_f32() - 0.5
        } else {
            0.0
        };
        let mesh = mesh_snake(&snake, interpolation);
        *mesh_handle = meshes.add(mesh).into();
    }

    // Handle end game
    if timer.0.just_finished() {
        for (snake, _) in snake_query.iter() {
            let new_head = snake.body[0];
            if !in_bounds(new_head, &b) {
                app_state.set(GameState::GameOver).unwrap();
                return;
            }

            for (other_snake, _) in snake_query.iter() {
                for i in 0..other_snake.body.len() {
                    if snake.id == other_snake.id && i == 0 {
                        continue;
                    }

                    if other_snake.body[i] == new_head {
                        app_state.set(GameState::GameOver).unwrap();
                        return;
                    }
                }
            }
        }
    }
}

fn bullet_system(
    mut commands: Commands,
    mut snake_query: Query<&mut Snake>,
    mut bullet_query: Query<(&mut Bullet, &mut Mesh2dHandle, Entity)>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    mut timer: ResMut<BulletTimer>,
    b: Res<Board>,
    settings: Res<Settings>,
    mut apples: ResMut<Apples>,
    mut app_state: ResMut<State<GameState>>,
    audio: Res<Audio>,
) {
    timer.0.tick(time.delta());
    'outer: for (mut bullet, mut mesh_handle, bullet_entity) in bullet_query.iter_mut() {
        if timer.0.just_finished() {
            for i in 0..=bullet.speed {
                let pos = bullet.pos + bullet.dir * i as i32;

                if !in_bounds(pos, &b) {
                    boom(&mut commands, &settings, &audio, pos, &b);
                    commands.entity(bullet_entity).despawn();
                    continue 'outer;
                }

                for mut snake in snake_query.iter_mut() {
                    for j in 0..snake.body.len() {
                        if snake.body[j] == pos {
                            if j < 2 {
                                if snake.id == bullet.id {
                                    continue;
                                }

                                // Headshot
                                app_state.set(GameState::GameOver).unwrap();
                                return;
                            }

                            boom(&mut commands, &settings, &audio, pos, &b);
                            commands.entity(bullet_entity).despawn();

                            for _ in j..snake.body.len() {
                                let pos = snake.body[j];
                                snake.body.remove(j);
                                apples.apples_to_spawn.push(AppleSpawn::Pos(pos));
                            }

                            continue 'outer;
                        }
                    }
                }
            }

            let pos = bullet.pos + bullet.dir * bullet.speed as i32;
            bullet.pos = pos;
        }

        let interpolation = if settings.interpolation {
            timer.0.elapsed_secs() / timer.0.duration().as_secs_f32() - 0.5
        } else {
            0.0
        };
        let mesh = mesh_bullet(&bullet, interpolation);
        *mesh_handle = meshes.add(mesh).into();
    }
}

fn spawn_apples(
    mut apples: ResMut<Apples>,
    snake_query: Query<&Snake>,
    mut commands: Commands,
    b: Res<Board>,
) {
    let mut rng = rand::thread_rng();
    let mut count = 0;

    'outer: while let Some(apple) = apples.apples_to_spawn.pop() {
        let mut pos;
        loop {
            pos = if let AppleSpawn::Pos(pos) = apple {
                pos
            } else {
                IVec2::new(rng.gen_range(0..b.width), rng.gen_range(0..b.height))
            };

            count += 1;
            if count > 1000 {
                break 'outer;
            }

            if apples.list.contains_key(&pos) {
                continue;
            }

            for snake in snake_query.iter() {
                if snake.body.contains(&pos) {
                    continue;
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

fn spawn_bullet(
    id: u32,
    pos: IVec2,
    dir: IVec2,
    commands: &mut Commands,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    b: &Board,
) {
    let bullet = Bullet {
        id,
        pos,
        dir,
        speed: 2,
    };
    commands
        .spawn_bundle(MaterialMesh2dBundle {
            material: materials.add(ColorMaterial::from(Color::rgb(1.0, 1.0, 0.26))),
            transform: Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 10.0),
            ..default()
        })
        .insert(bullet);
}

fn boom(commands: &mut Commands, settings: &Settings, audio: &Audio, pos: IVec2, b: &Board) {
    commands
        .spawn_bundle(SpriteSheetBundle {
            texture_atlas: settings.boom_texture_atlas_handle.as_ref().unwrap().clone(),
            transform: Transform::from_xyz(
                pos.x as f32 - b.width as f32 / 2.0 + 0.5,
                pos.y as f32 - b.height as f32 / 2.0 + 0.5,
                12.0,
            )
            .with_scale(Vec3::new(0.01, 0.01, 1.0)),
            ..default()
        })
        .insert(AnimationTimer(Timer::from_seconds(0.04, true)));

    audio.play(settings.boom_sound_handle.as_ref().unwrap().clone());
}

fn animate_explostions(
    mut commands: Commands,
    time: Res<Time>,
    texture_atlases: Res<Assets<TextureAtlas>>,
    mut query: Query<(
        &mut AnimationTimer,
        &mut TextureAtlasSprite,
        &Handle<TextureAtlas>,
        Entity,
    )>,
) {
    for (mut timer, mut sprite, texture_atlas_handle, entity) in query.iter_mut() {
        timer.tick(time.delta());
        if timer.just_finished() {
            let texture_atlas = texture_atlases.get(texture_atlas_handle).unwrap();
            sprite.index = sprite.index + 1;
            if sprite.index >= texture_atlas.textures.len() {
                commands.entity(entity).despawn();
            }
        }
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

fn get_direction(dir: IVec2) -> Direction {
    match dir.to_array() {
        [0, 1] => Direction::Up,
        [0, -1] => Direction::Down,
        [1, 0] => Direction::Right,
        [-1, 0] => Direction::Left,
        _ => panic!("Invalid direction"),
    }
}

fn fps_system(diagnostics: Res<Diagnostics>, mut query: Query<&mut Text>) {
    if let Some(fps) = diagnostics.get(FrameTimeDiagnosticsPlugin::FPS) {
        if let Some(average) = fps.average() {
            for mut text in query.iter_mut() {
                text.sections[0].value = format!("{:.1}", average);
            }
        }
    }
}
