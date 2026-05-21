use crate::net::{InputQueues, InterpolationPhase, RenderClock};
use bevy::{
    camera::{Camera, ClearColorConfig, RenderTarget, ScalingMode},
    image::ImageSampler,
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{Extent3d, TextureFormat, TextureUsages},
    ui::ComputedNode,
    window::{PrimaryWindow, WindowResized},
};
use bevy_snake::board::{Board, Cell};

pub struct BoardRenderPlugin;

impl Plugin for BoardRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            // resize first so draw_board's projection sees the new aspect ratio
            (resize_board_texture, draw_board).chain(),
        );
    }
}

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct UiCamera;

/// Marker on the `ImageNode` that displays the board texture. The resize
/// system uses this to find the panel's pixel size each frame.
#[derive(Component)]
pub struct BoardImageNode;

/// Handle to the off-screen render target the board camera draws into. The
/// UI displays this via `ImageNode { image: board_target.handle.clone(), .. }`.
#[derive(Resource)]
pub struct BoardRenderTarget {
    pub handle: Handle<Image>,
}

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
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    // Initial size is a placeholder — resize_board_texture will reallocate
    // to match the UI panel's physical pixel size on the first frame the
    // ImageNode has a non-zero layout.
    let mut board_image = Image::new_target_texture(512, 512, TextureFormat::Rgba8UnormSrgb, None);
    board_image.texture_descriptor.usage |= TextureUsages::COPY_DST;
    // Nearest sampling — the texture is resized to match the display node's
    // pixel size, so there's no resampling in steady state, but during a
    // resize tick a brief mismatch would otherwise read as blur.
    board_image.sampler = ImageSampler::nearest();
    let board_handle = images.add(board_image);

    // Board camera renders into the off-screen texture. Lower order so it
    // runs before the UI camera; clear color matches the old window clear.
    // RenderTarget is its own required component on the Camera in Bevy 0.18.
    commands.spawn((
        Camera2d,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.1, 0.1, 0.1)),
            ..default()
        },
        RenderTarget::Image(board_handle.clone().into()),
        Transform::from_xyz(0.0, 0.0, 500.0),
        MainCamera,
    ));

    // UI camera renders the side panel + the ImageNode showing the board
    // texture. Default render target (= window).
    commands.spawn((Camera2d, UiCamera));

    commands.insert_resource(BoardRenderTarget {
        handle: board_handle,
    });

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

/// Keep the board render target sized exactly to the `BoardImageNode`'s
/// physical pixel extent. `ComputedNode.size()` is already in physical
/// pixels, so a 1:1 ImageNode→texture mapping means no up/downsampling.
fn resize_board_texture(
    target: Res<BoardRenderTarget>,
    mut images: ResMut<Assets<Image>>,
    image_node: Query<&ComputedNode, With<BoardImageNode>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    mut resize_events: MessageReader<WindowResized>,
) {
    // Read every layout pass — cheap and the resize itself is no-op'd when
    // the size already matches. (Reading WindowResized purely to ensure we
    // re-run after a DPI change; the panel size *should* change too, but
    // this is belt and suspenders.)
    let _ = resize_events.read().count();
    let Ok(node) = image_node.single() else {
        return;
    };
    let size = node.size();
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    // ComputedNode.size is in physical pixels; matches the texture extent
    // directly. Fall back to scale_factor multiplication if a future Bevy
    // version flips this to logical pixels.
    let _scale = primary_window
        .single()
        .map(|w| w.scale_factor())
        .unwrap_or(1.0);
    let new_extent = Extent3d {
        width: size.x.round().max(1.0) as u32,
        height: size.y.round().max(1.0) as u32,
        ..default()
    };
    let Some(image) = images.get_mut(&target.handle) else {
        return;
    };
    if image.texture_descriptor.size == new_extent {
        return;
    }
    image.resize(new_extent);
}

#[derive(Component)]
struct BoardTile;

#[derive(Component)]
struct SnakePart;

#[derive(Component)]
struct Apple;

fn draw_board(
    mut commands: Commands,
    mut camera_query: Query<&mut Projection, With<MainCamera>>,
    mut apple_query: Query<&mut Transform, With<Apple>>,
    mut board_size: Local<(usize, usize)>,
    mut apples: Local<HashMap<IVec2, Entity>>,
    mut walls: Local<HashMap<IVec2, Entity>>,
    board: Res<Board>,
    render_clock: Res<RenderClock>,
    queues: Res<InputQueues>,
    phase: Res<InterpolationPhase>,
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

        if let Ok(mut projection) = camera_query.single_mut() {
            if let Projection::Orthographic(ortho) = projection.as_mut() {
                ortho.scaling_mode = ScalingMode::AutoMin {
                    min_height: board.height() as f32,
                    min_width: board.width() as f32,
                };
            }
        }

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

    let interpolation = render_clock.movement_progress(time.elapsed_secs_f64());

    for (snake_id, snake) in board.snakes().into_iter() {
        let mut parts: Vec<Vec2> = snake.parts.iter().map(|pos| pos.as_vec2()).collect();

        // Head animation runs in two phases split at `lean_start`:
        //
        // - Phase 1 (interp 0→lean_start): head was rendered half a cell
        //   behind its current grid position right after the movement frame,
        //   sliding forward in `snake.dir` to catch up by interp=lean_start.
        //   This is the visual "the snake just stepped into this cell."
        // - Phase 2 (interp lean_start→1): head extends forward into the
        //   queued direction (or keeps going straight if no queued turn)
        //   by up to half a cell.
        //
        // `lean_start` defaults to 0.3 so a queued direction is visible
        // quickly after the press, but it's exposed via `InterpolationPhase`
        // as an in-game slider. Clamped well away from 0 and 1 so the
        // per-phase rescale below doesn't divide by ~0.
        let lean_start = phase.0.clamp(0.05, 0.95);

        let next_dir = queues
            .front(snake_id as usize)
            .filter(|_| interpolation > lean_start)
            .filter(|d| *d != snake.dir.opposite())
            .unwrap_or(snake.dir);
        let next_input = next_dir.as_vec2().as_vec2();

        let h = parts.len() - 1; // head
        if interpolation > lean_start {
            parts.insert(h, parts[h]);
        }

        let h = parts.len() - 1;
        parts[0] = parts[0] + (parts[1] - parts[0]) * interpolation;
        let denom = if interpolation < lean_start {
            lean_start
        } else {
            1.0 - lean_start
        };
        let offset = (interpolation - lean_start) * 0.5 / denom;
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
