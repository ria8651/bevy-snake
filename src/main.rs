use bevy::{
    prelude::*,
    render::{camera::ScalingMode, mesh::PrimitiveTopology},
    sprite::MaterialMesh2dBundle,
};

#[derive(Component)]
struct Snake {
    body: Vec<(i32, i32)>,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_system(bevy::input::system::exit_on_esc_system)
        .add_startup_system(scene_setup)
        .add_startup_system(snake_setup)
        .run();
}

fn scene_setup(mut commands: Commands) {
    let width = 17;
    let height = 15;

    let colour1 = Color::rgb(0.3, 0.3, 0.3);
    let colour2 = Color::rgb(0.2, 0.2, 0.2);

    commands.spawn_bundle(OrthographicCameraBundle {
        orthographic_projection: OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical,
            scale: height as f32 / 2.0,
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

    for x in 0..width {
        for y in 0..height {
            let color = if (x + y) % 2 == 0 { colour1 } else { colour2 };

            commands.spawn_bundle(SpriteBundle {
                sprite: Sprite { color, ..default() },
                transform: Transform {
                    translation: Vec3::new(
                        x as f32 - width as f32 / 2.0 + 0.5,
                        y as f32 - height as f32 / 2.0 + 0.5,
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
) {
    let snake = Snake {
        body: vec![(5, 5), (5, 6), (5, 7)],
    };

    let mesh = mesh_snake(&snake);

    commands.spawn_bundle(MaterialMesh2dBundle {
        mesh: meshes.add(mesh).into(),
        material: materials.add(ColorMaterial::from(Color::rgb(0.0, 0.7, 0.25))),
        transform: Transform::from_xyz(0.0, 0.0, 10.0),
        ..default()
    });
}

fn mesh_snake(snake: &Snake) -> Mesh {
    let mut snake_mesh = Mesh::new(PrimitiveTopology::TriangleList);

    let verticies = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals = vec![[0.0, 0.0, 1.0]; 3];
    let uvs = vec![[0.0, 0.0]; 3];

    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verticies);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);

    snake_mesh
}
