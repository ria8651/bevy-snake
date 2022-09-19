use super::*;

pub struct WallPlugin;

impl Plugin for WallPlugin {
    fn build(&self, app: &mut App) {
        app.add_system_set(
            SystemSet::on_update(GameState::Playing).with_system(
                wall_system
                    .after(snake::damage_snake_system)
                    .after(snake::snake_system)
                    .after(reset_game),
            ),
        );
    }
}

pub struct Walls {
    pub list: HashMap<IVec2, Entity>,
}

pub enum WallEv {
    Spawn,
    Destroy(IVec2),
}

#[derive(Component)]
struct DebugGizmo;

fn wall_system(
    mut walls: ResMut<Walls>,
    apples: Res<Apples>,
    snake_query: Query<&Snake>,
    mut commands: Commands,
    b: Res<Board>,
    mut wall_ev: EventReader<WallEv>,
    settings: Res<Settings>,
    debug_gizmo_query: Query<Entity, With<DebugGizmo>>,
) {
    let mut rng = rand::thread_rng();

    let unspawnable_positions = vec![
        IVec2::new(0, 1),
        IVec2::new(1, 0),
        IVec2::new(b.width - 1, 1),
        IVec2::new(b.width - 2, 0),
        IVec2::new(0, b.height - 2),
        IVec2::new(1, b.height - 1),
        IVec2::new(b.width - 1, b.height - 2),
        IVec2::new(b.width - 2, b.height - 1),
    ];
    let corner_cases = vec![
        (IVec2::new(0, 2), IVec2::new(2, 0)),
        (IVec2::new(0, b.height - 3), IVec2::new(2, b.height - 1)),
        (IVec2::new(b.width - 3, 0), IVec2::new(b.width - 1, 2)),
        (IVec2::new(b.width - 3, b.height - 1), IVec2::new(b.width - 1, b.height - 3)),
    ];
    let is_valid = |pos, walls: &Walls| {
        // stop snake getting stuck in corner
        if unspawnable_positions.contains(&pos) {
            return false;
        }

        // stop walls spawning on other walls or on an apple
        if walls.list.contains_key(&pos) || apples.list.contains_key(&pos) {
            return false;
        }

        for snake in snake_query.iter() {
            // to stop crash when snake is killed
            if snake.body.len() > 0 {
                // stop walls spawning on the snake
                if snake.body.contains(&pos) {
                    return false;
                }

                // stop walls spawning near snake head
                if (snake.body[0] - pos).abs().max_element() <= 2 {
                    return false;
                }
            }
        }

        for (wall, _) in walls.list.iter() {
            // stop walls spawning near other walls
            if (*wall - pos).abs().max_element() <= 1 {
                return false;
            }

            // stop walls spawning at the edge of the board from blocking the snake
            if wall.x == 0 || wall.x == b.width - 1 {
                if pos == *wall + IVec2::new(0, 2) || pos == *wall + IVec2::new(0, -2) {
                    return false;
                }
            }
            if wall.y == 0 || wall.y == b.height - 1 {
                if pos == *wall + IVec2::new(2, 0) || pos == *wall + IVec2::new(-2, 0) {
                    return false;
                }
            }

            // stop a special case in the corner
            if corner_cases.contains(&(*wall, pos)) || corner_cases.contains(&(pos, *wall)) {
                return false;
            }
        }

        return true;
    };

    for wall_ev in wall_ev.iter() {
        match wall_ev {
            WallEv::Spawn => {
                let mut count = 0;
                let mut pos;
                'wall: loop {
                    pos = IVec2::new(rng.gen_range(0..b.width), rng.gen_range(0..b.height));

                    count += 1;
                    if count > 1000 {
                        return;
                    }

                    if is_valid(pos, &walls) {
                        break 'wall;
                    }
                }

                walls.list.insert(
                    pos,
                    commands
                        .spawn_bundle(SpriteBundle {
                            sprite: Sprite {
                                color: Color::rgb(0.1, 0.1, 0.1),
                                ..default()
                            },
                            transform: Transform::from_xyz(
                                pos.x as f32 - b.width as f32 / 2.0 + 0.5,
                                pos.y as f32 - b.height as f32 / 2.0 + 0.5,
                                5.0,
                            ),
                            ..default()
                        })
                        .id(),
                );
            }
            WallEv::Destroy(pos) => {
                if let Some(entity) = walls.list.remove(&pos) {
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    for entity in debug_gizmo_query.iter() {
        commands.entity(entity).despawn();
    }

    if settings.walls_debug {
        for x in 0..b.width {
            for y in 0..b.height {
                let pos = IVec2::new(x, y);
                if !is_valid(pos, &walls) {
                    commands
                        .spawn_bundle(SpriteBundle {
                            sprite: Sprite {
                                color: Color::rgba(1.0, 0.1, 0.1, 0.2),
                                ..default()
                            },
                            transform: Transform::from_xyz(
                                pos.x as f32 - b.width as f32 / 2.0 + 0.5,
                                pos.y as f32 - b.height as f32 / 2.0 + 0.5,
                                4.0,
                            ),
                            ..default()
                        })
                        .insert(DebugGizmo);
                }
            }
        }
    }
}
