use super::*;

pub struct SnakePlugin;

impl Plugin for SnakePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Points { points: [0; 4] }).add_system(
            damage_snake_system
                .after(snake_system)
                .after(bullet_system)
                .before(game_state),
        );
    }
}

#[derive(Component)]
pub struct Snake {
    pub id: u32,
    pub body: Vec<IVec2>,
    pub input_map: InputMap,
    pub input_queue: VecDeque<Direction>,
    pub head_dir: IVec2,
    pub tail_dir: IVec2,
}

impl Default for Snake {
    fn default() -> Self {
        Snake {
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
        }
    }
}

pub fn snake_system(
    mut commands: Commands,
    mut snake_query: Query<(&mut Snake, &mut Mesh2dHandle)>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    mut timer: ResMut<MovmentTimer>,
    keys: Res<Input<KeyCode>>,
    mut apples: ResMut<Apples>,
    b: Res<Board>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    settings: Res<Settings>,
    mut damage_ev: EventWriter<DamageSnakeEv>,
) {
    timer
        .0
        .set_duration(std::time::Duration::from_secs_f32(1.0 / settings.tps));
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
            snake.body.remove(len - 1);
            commands
                .spawn_bundle(MaterialMesh2dBundle {
                    material: materials.add(ColorMaterial::from(Color::rgb(1.0, 1.0, 0.26))),
                    transform: Transform::from_xyz(
                        -b.width as f32 / 2.0,
                        -b.height as f32 / 2.0,
                        11.0,
                    ),
                    ..default()
                })
                .insert(Bullet {
                    id: snake.id,
                    pos: head,
                    dir: current_dir,
                    speed: 2,
                });
        }

        if timer.0.just_finished() {
            let new_head = if let Some(direction) = snake.input_queue.pop_front() {
                let dir: IVec2 = DIR[direction as usize].into();
                head + dir
            } else {
                head + current_dir
            };
            
            snake.body.insert(0, new_head);

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
        'outer: for (snake, _) in snake_query.iter() {
            let new_head = snake.body[0];
            if !in_bounds(new_head, &b) {
                damage_ev.send(DamageSnakeEv {
                    snake_id: snake.id,
                    snake_pos: 0,
                });
                continue 'outer;
            }

            for (other_snake, _) in snake_query.iter() {
                for i in 0..other_snake.body.len() {
                    if snake.id == other_snake.id && i == 0 {
                        continue;
                    }

                    if other_snake.body[i] == new_head {
                        damage_ev.send(DamageSnakeEv {
                            snake_id: snake.id,
                            snake_pos: 0,
                        });
                        continue 'outer;
                    }
                }
            }
        }
    }
}

pub struct Points {
    pub points: [u32; 4],
}

pub struct DamageSnakeEv {
    pub snake_id: u32,
    pub snake_pos: usize,
}

pub fn damage_snake_system(
    mut commands: Commands,
    mut damage_snake_ev: EventReader<DamageSnakeEv>,
    mut snake_query: Query<(&mut Snake, Entity)>,
    mut apples: ResMut<Apples>,
    mut points: ResMut<Points>,
) {
    let mut dead_snakes = Vec::new();

    for ev in damage_snake_ev.iter() {
        for (mut snake, snake_entity) in snake_query.iter_mut() {
            if snake.id == ev.snake_id {
                if ev.snake_pos < 2 {
                    snake.body.remove(0);
                    commands.entity(snake_entity).despawn();
                    dead_snakes.push(snake.id);
                }

                for _ in ev.snake_pos..snake.body.len() {
                    let pos = snake.body[ev.snake_pos];
                    snake.body.remove(ev.snake_pos);
                    apples.apples_to_spawn.push(AppleSpawn::Pos(pos));
                }
            }
        }
    }

    for dead_snake_id in &dead_snakes {
        for (snake, _) in snake_query.iter() {
            if snake.id != *dead_snake_id && !dead_snakes.contains(&snake.id) {
                points.points[snake.id as usize] += 1;
            }
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

pub const DIR: [[i32; 2]; 4] = [[0, 1], [0, -1], [-1, 0], [1, 0]];

pub fn get_direction(dir: IVec2) -> Direction {
    match dir.to_array() {
        [0, 1] => Direction::Up,
        [0, -1] => Direction::Down,
        [1, 0] => Direction::Right,
        [-1, 0] => Direction::Left,
        _ => panic!("Invalid direction"),
    }
}
