use bevy::{
    prelude::*,
    render::{camera::ScalingMode, mesh::PrimitiveTopology},
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
};

#[derive(Component)]
struct Snake {
    body: Vec<IVec2>,
}

struct Board {
    width: i32,
    height: i32,
    colour1: Color,
    colour2: Color,
}

struct MovmentTimer(Timer);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(Board {
            width: 17,
            height: 15,
            colour1: Color::rgb(0.3, 0.3, 0.3),
            colour2: Color::rgb(0.2, 0.2, 0.2),
        })
        .insert_resource(MovmentTimer(Timer::from_seconds(0.2, true)))
        .add_system(bevy::input::system::exit_on_esc_system)
        .add_startup_system(scene_setup)
        .add_startup_system(snake_setup)
        .add_system(snake_system)
        .run();
}

fn scene_setup(mut commands: Commands, b: Res<Board>) {
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
    };

    let mesh = mesh_snake(&snake);

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
    mut snake: Query<(&mut Snake, &mut Mesh2dHandle)>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    mut timer: ResMut<MovmentTimer>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        let (mut snake, mut mesh_handle) = snake.single_mut();
        let head = snake.body[0];
        let neck = snake.body[1];
        let len = snake.body.len();
        snake.body.remove(len - 1);
        snake.body.insert(0, head + head - neck);

        let mesh = mesh_snake(&snake);
        *mesh_handle = meshes.add(mesh).into();
    }
}

fn mesh_snake(snake: &Snake) -> Mesh {
    let mut snake_mesh = Mesh::new(PrimitiveTopology::TriangleList);

    let mut verticies = Vec::new();

    for pos in snake.body.iter() {
        verticies.push([pos.x as f32 + 0.1, pos.y as f32 + 0.1, 0.0]);
        verticies.push([pos.x as f32 + 0.9, pos.y as f32 + 0.1, 0.0]);
        verticies.push([pos.x as f32 + 0.1, pos.y as f32 + 0.9, 0.0]);
        verticies.push([pos.x as f32 + 0.1, pos.y as f32 + 0.9, 0.0]);
        verticies.push([pos.x as f32 + 0.9, pos.y as f32 + 0.1, 0.0]);
        verticies.push([pos.x as f32 + 0.9, pos.y as f32 + 0.9, 0.0]);
    }

    let mut positions = Vec::<[f32; 3]>::new();
    let mut normals = Vec::<[f32; 3]>::new();
    let mut uvs = Vec::<[f32; 2]>::new();
    for position in &verticies {
        positions.push(*position);
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([0.0, 0.0]);
    }

    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);

    snake_mesh
}
