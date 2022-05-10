use bevy::{
    prelude::*,
    render::{camera::ScalingMode, mesh::PrimitiveTopology},
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};
use rand::Rng;
use std::collections::{HashMap, VecDeque};

#[derive(Component)]
struct Snake {
    body: Vec<IVec2>,
    head_dir: IVec2,
    tail_dir: IVec2,
}

struct Board {
    width: i32,
    height: i32,
    colour1: Color,
    colour2: Color,
}

struct MovmentTimer(Timer);

#[derive(PartialEq, Clone, Copy)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

const DIR: [[i32; 2]; 4] = [[0, 1], [0, -1], [-1, 0], [1, 0]];

struct InputQueue(VecDeque<Direction>);

struct Apples {
    list: HashMap<IVec2, Entity>,
    sprite: Option<Handle<Image>>,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(Board {
            width: 17,
            height: 15,
            colour1: Color::rgb(0.3, 0.3, 0.3),
            colour2: Color::rgb(0.2, 0.2, 0.2),
        })
        .insert_resource(MovmentTimer(Timer::from_seconds(1.0 / 8.0, true)))
        .insert_resource(InputQueue(VecDeque::new()))
        .insert_resource(Apples {
            list: HashMap::new(),
            sprite: None,
        })
        .add_system(bevy::input::system::exit_on_esc_system)
        .add_startup_system(scene_setup)
        .add_startup_system(snake_setup)
        .add_system(snake_system)
        .run();
}

fn spawn_apple(pos: IVec2, apples: &mut Apples, commands: &mut Commands, b: &Board) {
    apples.list.insert(
        pos,
        commands
            .spawn_bundle(SpriteBundle {
                texture: apples.sprite.as_ref().unwrap().clone(),
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

fn scene_setup(
    mut commands: Commands,
    b: Res<Board>,
    mut apples: ResMut<Apples>,
    assets: Res<AssetServer>,
) {
    apples.sprite = Some(assets.load("images/apple.png"));

    spawn_apple(IVec2::new(8, 7), &mut apples, &mut commands, &b);
    spawn_apple(IVec2::new(10, 7), &mut apples, &mut commands, &b);
    spawn_apple(IVec2::new(8, 9), &mut apples, &mut commands, &b);

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
}

fn snake_setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    b: Res<Board>,
) {
    let snake = Snake {
        body: vec![
            IVec2::new(5, 7),
            IVec2::new(4, 7),
            IVec2::new(3, 7),
            IVec2::new(2, 7),
        ],
        head_dir: IVec2::new(1, 0),
        tail_dir: IVec2::new(-1, 0),
    };

    let mesh = mesh_snake(&snake, 0.0);

    commands
        .spawn_bundle(MaterialMesh2dBundle {
            mesh: meshes.add(mesh).into(),
            material: materials.add(ColorMaterial::from(Color::rgb(0.0, 0.7, 0.25))),
            transform: Transform::from_xyz(-b.width as f32 / 2.0, -b.height as f32 / 2.0, 10.0),
            ..default()
        })
        .insert(snake);
}

fn snake_system(
    mut commands: Commands,
    mut snake: Query<(&mut Snake, &mut Mesh2dHandle)>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    mut timer: ResMut<MovmentTimer>,
    keys: Res<Input<KeyCode>>,
    mut input_queue: ResMut<InputQueue>,
    mut apples: ResMut<Apples>,
    b: Res<Board>,
) {
    let (mut snake, mut mesh_handle) = snake.single_mut();

    let head = snake.body[0];
    let neck = snake.body[1];
    let forward = head - neck;

    let last_in_queue = *input_queue.0.back().unwrap_or(&get_direction(forward));
    if input_queue.0.len() < 3 {
        if keys.just_pressed(KeyCode::Up) || keys.just_pressed(KeyCode::W) {
            if last_in_queue != Direction::Down && last_in_queue != Direction::Up {
                input_queue.0.push_back(Direction::Up);
            }
        } else if keys.just_pressed(KeyCode::Down) || keys.just_pressed(KeyCode::S) {
            if last_in_queue != Direction::Up && last_in_queue != Direction::Down {
                input_queue.0.push_back(Direction::Down);
            }
        } else if keys.just_pressed(KeyCode::Left) || keys.just_pressed(KeyCode::A) {
            if last_in_queue != Direction::Right && last_in_queue != Direction::Left {
                input_queue.0.push_back(Direction::Left);
            }
        } else if keys.just_pressed(KeyCode::Right) || keys.just_pressed(KeyCode::D) {
            if last_in_queue != Direction::Left && last_in_queue != Direction::Right {
                input_queue.0.push_back(Direction::Right);
            }
        }
    }

    if timer.0.tick(time.delta()).just_finished() {
        let current_dir = head - neck;

        if let Some(direction) = input_queue.0.pop_front() {
            let dir = DIR[direction as usize];
            let dir = IVec2::new(dir[0], dir[1]);
            if current_dir + dir != IVec2::ZERO {
                snake.body.insert(0, head + dir);
            } else {
                snake.body.insert(0, head + head - neck);
            }
        } else {
            snake.body.insert(0, head + head - neck);
        }

        let new_head = snake.body[0];
        if new_head.x < 0 || new_head.x >= b.width {
            end_game();
        }
        if new_head.y < 0 || new_head.y >= b.height {
            end_game();
        }
        for snake_body in snake.body.iter().skip(1) {
            if *snake_body == new_head {
                end_game();
            }
        }

        let head = snake.body[0];
        if let Some(apple_entity) = apples.list.get(&head) {
            commands.entity(*apple_entity).despawn();
            apples.list.remove(&head);
            let mut rng = rand::thread_rng();
            let pos = IVec2::new(rng.gen_range(0..b.width), rng.gen_range(0..b.height));
            spawn_apple(pos, &mut apples, &mut commands, &b);
        } else {
            let len = snake.body.len();
            snake.tail_dir = snake.body[len - 2] - snake.body[len - 1];
            snake.body.remove(len - 1);
        }
    }

    let interpolation = timer.0.elapsed_secs() / timer.0.duration().as_secs_f32() - 0.5;
    snake.head_dir = if let Some(dir) = input_queue.0.get(0) {
        DIR[*dir as usize].into()
    } else {
        head - neck
    };

    let mesh = mesh_snake(&snake, interpolation);
    *mesh_handle = meshes.add(mesh).into();
}

fn end_game() {
    println!("DEEEAAAAATTHHHHH!!!!");
}

fn mesh_snake(snake: &Snake, interpolation: f32) -> Mesh {
    let mut verticies = Vec::new();

    fn push_quad(
        verticies: &mut Vec<[f32; 3]>,
        pos: IVec2,
        offset: Vec2,
        half_size: Vec2,
        flip: IVec2,
    ) {
        let offset = if flip.y == 1 {
            Vec2::new(offset.y, offset.x)
        } else {
            offset
        };

        let half_size = if flip.y == 1 {
            Vec2::new(half_size.y, half_size.x)
        } else {
            half_size
        };
        let pos = Vec2::new(pos.x as f32, pos.y as f32) + 0.5 + offset * flip.x as f32;

        verticies.push([pos.x - half_size.x, pos.y - half_size.y, 0.0]);
        verticies.push([pos.x + half_size.x, pos.y - half_size.y, 0.0]);
        verticies.push([pos.x - half_size.x, pos.y + half_size.y, 0.0]);

        verticies.push([pos.x - half_size.x, pos.y + half_size.y, 0.0]);
        verticies.push([pos.x + half_size.x, pos.y - half_size.y, 0.0]);
        verticies.push([pos.x + half_size.x, pos.y + half_size.y, 0.0]);
    }

    fn push_circle(verticies: &mut Vec<[f32; 3]>, pos: IVec2, offset: Vec2, radius: f32) {
        let pos = Vec2::new(pos.x as f32, pos.y as f32) + 0.5 + offset;

        let segments = 64;

        let step = std::f32::consts::TAU / segments as f32;
        let mut angle = step;
        let mut last = Vec2::new(0.0, radius);
        for _ in 0..segments {
            let x = radius * angle.sin();
            let y = radius * angle.cos();

            verticies.push([pos.x, pos.y, 0.0]);
            verticies.push([pos.x + x, pos.y + y, 0.0]);
            verticies.push([pos.x + last.x, pos.y + last.y, 0.0]);

            angle += step;
            last = Vec2::new(x, y);
        }
    }

    let width = 0.6;
    let head_size = 0.7;

    let head = snake.body[0];
    let neck = snake.body[1];
    let len = snake.body.len();
    let tail = snake.body[len - 1];

    let mut start = 1;
    let mut end = len - 1;

    if interpolation >= 0.0 {
        start = 0;

        // Interpolate head
        push_quad(
            &mut verticies,
            head,
            Vec2::new(0.0, interpolation / 2.0),
            Vec2::new(width / 2.0, interpolation / 2.0),
            calculate_flip(snake.head_dir),
        );
        push_circle(
            &mut verticies,
            head,
            snake.head_dir.as_vec2() * interpolation,
            head_size / 2.0,
        );

        // Interpolate tail
        let tail_dir = snake.body[len - 2] - snake.body[len - 1];
        push_quad(
            &mut verticies,
            tail,
            Vec2::new(0.0, interpolation / 2.0 + 0.25),
            Vec2::new(width / 2.0, -interpolation / 2.0 + 0.25),
            calculate_flip(tail_dir),
        );
        push_circle(
            &mut verticies,
            tail,
            tail_dir.as_vec2() * interpolation,
            width / 2.0,
        );
    } else {
        end = len;

        // Interpolate head
        push_quad(
            &mut verticies,
            head,
            Vec2::new(0.0, interpolation / 2.0 - 0.25),
            Vec2::new(width / 2.0, interpolation / 2.0 + 0.25),
            calculate_flip(head - neck),
        );
        push_circle(
            &mut verticies,
            head,
            (head - neck).as_vec2() * interpolation,
            head_size / 2.0,
        );

        // Interpolate tail
        push_quad(
            &mut verticies,
            tail,
            Vec2::new(0.0, interpolation / 2.0),
            Vec2::new(width / 2.0, -interpolation / 2.0),
            calculate_flip(snake.tail_dir),
        );
        push_circle(
            &mut verticies,
            tail,
            snake.tail_dir.as_vec2() * interpolation,
            width / 2.0,
        );
    }

    let mut last = head;
    for i in start..end {
        let pos = snake.body[i];

        push_circle(&mut verticies, pos, Vec2::new(0.0, 0.0), width / 2.0);

        if i > 0 {
            let flip1 = calculate_flip(last - pos);
            push_quad(
                &mut verticies,
                pos,
                Vec2::new(0.0, 0.25),
                Vec2::new(width / 2.0, 0.25),
                flip1,
            );
        }

        if i < len - 1 {
            let next = snake.body[i + 1];
            let flip2 = calculate_flip(next - pos);
            push_quad(
                &mut verticies,
                pos,
                Vec2::new(0.0, 0.25),
                Vec2::new(width / 2.0, 0.25),
                flip2,
            );
        }

        last = pos;
    }

    let mut positions = Vec::<[f32; 3]>::new();
    let mut normals = Vec::<[f32; 3]>::new();
    let mut uvs = Vec::<[f32; 2]>::new();
    for position in &verticies {
        positions.push(*position);
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([0.0, 0.0]);
    }

    let mut snake_mesh = Mesh::new(PrimitiveTopology::TriangleList);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);

    snake_mesh
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
