use crate::board::Board;
use bevy::{prelude::*, render::camera::ScalingMode};

pub struct BoardRenderPlugin;

impl Plugin for BoardRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, draw_board);
    }
}

#[derive(Component)]
struct MainCamera;

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera2dBundle {
            transform: Transform::from_xyz(0.0, 0.0, 500.0),
            ..default()
        },
        MainCamera,
    ));
}

#[derive(Component)]
struct BoardTile;

fn draw_board(
    mut commands: Commands,
    mut camera_query: Query<&mut OrthographicProjection, With<MainCamera>>,
    mut board_size: Local<(usize, usize)>,
    board_query: Query<&Board>,
    board_tiles: Query<Entity, With<BoardTile>>,
) {
    for tile in board_tiles.iter() {
        commands.entity(tile).despawn();
    }

    let board = board_query.single();

    if (board.width(), board.height()) != *board_size {
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
                        transform: Transform::from_xyz(
                            x as f32 - board.width() as f32 / 2.0 + 0.5,
                            y as f32 - board.height() as f32 / 2.0 + 0.5,
                            -1.0,
                        ),
                        ..default()
                    },
                    BoardTile,
                ));
            }
        }

        *board_size = (board.width(), board.height());
    }
}
