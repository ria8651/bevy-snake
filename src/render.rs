use crate::net::{InputQueues, MovementFrame};
use bevy::{prelude::*, render::camera::ScalingMode, utils::HashMap};
use bevy_snake::board::{Board, Cell};

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
    circle_mesh: Handle<Mesh>,
    square_mesh: Handle<Mesh>,
    snake_materials: Vec<Handle<ColorMaterial>>,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((Camera2d, Transform::from_xyz(0.0, 0.0, 500.0), MainCamera));

    commands.insert_resource(RenderResources {
        apple_texture: asset_server.load("images/apple.png"),
        circle_mesh: meshes.add(Circle::new(0.35)),
        square_mesh: meshes.add(Rectangle::from_size(Vec2::new(0.7, 1.0))),
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

#[derive(Component)]
struct Apple;

fn draw_board(
    mut commands: Commands,
    mut camera_query: Query<&mut OrthographicProjection, With<MainCamera>>,
    mut apple_query: Query<&mut Transform, With<Apple>>,
    mut board_size: Local<(usize, usize)>,
    mut apples: Local<HashMap<IVec2, Entity>>,
    mut walls: Local<HashMap<IVec2, Entity>>,
    board: Res<Board>,
    movement_frame: Res<MovementFrame>,
    queues: Res<InputQueues>,
    board_tiles: Query<Entity, With<BoardTile>>,
    snake_parts: Query<Entity, With<SnakePart>>,
    render_resources: Res<RenderResources>,
    time: Res<Time>,
) {
    let board_pos = |pos: Vec2, depth: f32| -> Transform {
        Transform::from_xyz(
            pos.x - board.width() as f32 / 2.0 + 0.5,
            pos.y - board.height() as f32 / 2.0 + 0.5,
            depth,
        )
    };

    // background — rebuild iff the board dimensions changed
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
                    Sprite { color, ..default() },
                    board_pos(Vec2::new(x as f32, y as f32), -10.0),
                    BoardTile,
                ));
            }
        }

        for (_, &entity) in apples.iter() {
            commands.entity(entity).despawn();
        }
        apples.clear();

        for (_, &entity) in walls.iter() {
            commands.entity(entity).despawn();
        }
        walls.clear();

        *board_size = (board.width(), board.height());
    }

    // apples
    for (pos, cell) in board.cells() {
        match cell {
            Cell::Apple { .. } => {
                if apples.contains_key(&pos) {
                    continue;
                }

                apples.insert(
                    pos,
                    commands
                        .spawn((
                            Sprite {
                                image: render_resources.apple_texture.clone(),
                                ..default()
                            },
                            board_pos(pos.as_vec2(), 10.0).with_scale(Vec3::splat(1.0 / 512.0)),
                            Apple,
                        ))
                        .id(),
                );
            }
            _ => {
                if let Some(entity) = apples.remove(&pos) {
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    for mut apple in apple_query.iter_mut() {
        let scale = (10.0 * time.elapsed_secs()).sin() * 0.1 + 1.0;
        apple.scale = Vec3::splat(1.0 / 512.0) * scale;
    }

    // walls
    for (pos, cell) in board.cells() {
        match cell {
            Cell::Wall => {
                if walls.contains_key(&pos) {
                    continue;
                }

                walls.insert(
                    pos,
                    commands
                        .spawn((
                            Sprite {
                                color: Color::srgb(0.1, 0.1, 0.1),
                                ..default()
                            },
                            board_pos(pos.as_vec2(), 5.0),
                        ))
                        .id(),
                );
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

    let interpolation = movement_frame.movement_progress();

    for (snake_id, snake) in board.snakes().into_iter() {
        let mut parts: Vec<Vec2> = snake.parts.iter().map(|pos| pos.as_vec2()).collect();

        // Head animation runs in two phases split at LEAN_START:
        //
        // - Phase 1 (interp 0→LEAN_START): head was rendered half a cell
        //   behind its current grid position right after the movement frame,
        //   sliding forward in `snake.dir` to catch up by interp=LEAN_START.
        //   This is the visual "the snake just stepped into this cell."
        // - Phase 2 (interp LEAN_START→1): head extends forward into the
        //   queued direction (or keeps going straight if no queued turn)
        //   by up to half a cell.
        //
        // LEAN_START sits well before the midpoint so a queued direction is
        // visible quickly after the press. The offset is rescaled per-phase
        // so the start/end positions still match -0.5/+0.5 cells regardless
        // of where the crossover sits.
        const LEAN_START: f32 = 0.3;

        let next_dir = queues
            .front(snake_id as usize)
            .filter(|_| interpolation > LEAN_START)
            .filter(|d| *d != snake.dir.opposite())
            .unwrap_or(snake.dir);
        let next_input = next_dir.as_vec2().as_vec2();

        let h = parts.len() - 1; // head
        if interpolation > LEAN_START {
            parts.insert(h, parts[h]);
        }

        let h = parts.len() - 1;
        parts[0] = parts[0] + (parts[1] - parts[0]) * interpolation;
        let denom = if interpolation < LEAN_START {
            LEAN_START
        } else {
            1.0 - LEAN_START
        };
        let offset = (interpolation - LEAN_START) * 0.5 / denom;
        parts[h] = parts[h] + next_input * offset;

        for i in 0..parts.len() {
            commands.spawn((
                Mesh2d(render_resources.circle_mesh.clone()),
                MeshMaterial2d(render_resources.snake_materials[snake_id as usize].clone()),
                board_pos(parts[i], 0.0),
                SnakePart,
            ));
        }

        for i in 1..parts.len() {
            let pos = parts[i];
            let prev = parts[i - 1];
            let mid_pos = (pos + prev) / 2.0;
            let scale = (pos - prev).length();

            let capsule_pos = board_pos(mid_pos, 0.0);
            commands.spawn((
                Mesh2d(render_resources.square_mesh.clone()),
                MeshMaterial2d(render_resources.snake_materials[snake_id as usize].clone()),
                capsule_pos
                    .looking_at(
                        capsule_pos.translation + Vec3::Z,
                        (pos - mid_pos).extend(0.0),
                    )
                    .with_scale(Vec3::new(1.0, scale, 1.0)),
                SnakePart,
            ));
        }
    }
}
