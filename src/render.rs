use crate::board::{Board, Cell};
use bevy::{
    prelude::*,
    render::camera::ScalingMode,
    sprite::{MaterialMesh2dBundle, Mesh2dHandle},
    utils::HashMap,
};

pub struct BoardRenderPlugin;

impl Plugin for BoardRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, draw_board);
    }
}

#[derive(Component)]
struct MainCamera;

#[derive(Resource)]
struct RenderResources {
    apple_texture: Handle<Image>,
    capsule_mesh: Handle<Mesh>,
    snake_materials: Vec<Handle<ColorMaterial>>,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        Camera2dBundle {
            transform: Transform::from_xyz(0.0, 0.0, 500.0),
            ..default()
        },
        MainCamera,
    ));

    commands.insert_resource(RenderResources {
        apple_texture: asset_server.load("images/apple.png"),
        capsule_mesh: meshes.add(Capsule2d::new(0.35, 1.0)),
        snake_materials: vec![
            materials.add(Color::srgb(0.0, 0.7, 0.25)),
            materials.add(Color::srgb(0.3, 0.4, 0.7)),
            materials.add(Color::srgb(0.7, 0.4, 0.3)),
            materials.add(Color::srgb(0.7, 0.7, 0.7)),
        ],
    });
}

#[derive(Component)]
struct BoardTile;

#[derive(Component)]
struct SnakePart;

fn draw_board(
    mut commands: Commands,
    mut camera_query: Query<&mut OrthographicProjection, With<MainCamera>>,
    mut board_size: Local<(usize, usize)>,
    mut apples: Local<HashMap<IVec2, Entity>>,
    mut walls: Local<HashMap<IVec2, Entity>>,
    board: Res<Board>,
    board_tiles: Query<Entity, With<BoardTile>>,
    snake_parts: Query<Entity, With<SnakePart>>,
    render_resources: Res<RenderResources>,
) {
    let board_pos = |pos: Vec2, depth: f32| -> Transform {
        Transform::from_xyz(
            pos.x - board.width() as f32 / 2.0 + 0.5,
            pos.y - board.height() as f32 / 2.0 + 0.5,
            depth,
        )
    };

    // background
    if (board.width(), board.height()) != *board_size {
        for tile in board_tiles.iter() {
            commands.entity(tile).despawn();
        }

        let mut camera_projection = camera_query.single_mut();
        camera_projection.scaling_mode = ScalingMode::AutoMin {
            min_height: board.height() as f32,
            min_width: board.width() as f32,
        };

        for x in 0..board.width() {
            for y in 0..board.height() {
                let color = if (x + y) % 2 == 0 {
                    Color::srgb(0.3, 0.5, 0.3)
                } else {
                    Color::srgb(0.25, 0.45, 0.25)
                };

                commands.spawn((
                    SpriteBundle {
                        sprite: Sprite { color, ..default() },
                        transform: board_pos(Vec2::new(x as f32, y as f32), -10.0),
                        ..default()
                    },
                    BoardTile,
                ));
            }
        }

        *board_size = (board.width(), board.height());
    }

    // apples
    for (pos, cell) in board.cells() {
        match cell {
            Cell::Apple => {
                if apples.contains_key(&pos) {
                    continue;
                }

                let bundle = SpriteBundle {
                    texture: render_resources.apple_texture.clone(),
                    transform: board_pos(pos.as_vec2(), 10.0).with_scale(Vec3::splat(1.0 / 512.0)),
                    ..default()
                };
                apples.insert(pos, commands.spawn(bundle).id());
            }
            _ => {
                if let Some(entity) = apples.remove(&pos) {
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    // walls
    for (pos, cell) in board.cells() {
        match cell {
            Cell::Wall => {
                if walls.contains_key(&pos) {
                    continue;
                }

                let bundle = SpriteBundle {
                    sprite: Sprite {
                        color: Color::srgb(0.1, 0.1, 0.1),
                        ..default()
                    },
                    transform: board_pos(pos.as_vec2(), 5.0),
                    ..default()
                };
                walls.insert(pos, commands.spawn(bundle).id());
            }
            _ => {
                if let Some(entity) = walls.remove(&pos) {
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    // snakes
    for entity in snake_parts.iter() {
        commands.entity(entity).despawn();
    }

    for (snake_id, snake) in board.snakes().into_iter().enumerate() {
        if snake.len() < 2 {
            continue;
        }

        for i in 1..snake.len() {
            let (pos, _) = snake[i];
            let (prev_pos, _) = snake[i - 1];
            let mid_pos = (pos.as_vec2() + prev_pos.as_vec2()) / 2.0;

            let capsule_pos = board_pos(mid_pos, 0.0);
            commands.spawn((
                MaterialMesh2dBundle {
                    mesh: Mesh2dHandle(render_resources.capsule_mesh.clone()),
                    material: render_resources.snake_materials[snake_id].clone(),
                    transform: capsule_pos.looking_at(
                        capsule_pos.translation + Vec3::Z,
                        (pos.as_vec2() - mid_pos).extend(0.0),
                    ),
                    ..default()
                },
                SnakePart,
            ));
        }
    }
}
