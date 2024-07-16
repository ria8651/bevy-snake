use super::*;

pub struct GunPlugin;

impl Plugin for GunPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                bullet_spawner.after(snake::snake_system),
                bullet_system.run_if(in_state(GameState::InGame)),
            ),
        );
    }
}

#[derive(Event)]
pub struct SpawnBulletEv(pub Bullet);

#[derive(Component, Clone, Copy)]
pub struct Bullet {
    pub id: u32,
    pub pos: IVec2,
    pub dir: IVec2,
    pub speed: u32,
}

pub fn bullet_spawner(
    mut commands: Commands,
    mut bullet_spawn_ev: EventReader<SpawnBulletEv>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut snake_query: Query<&mut Snake>,
    b: Res<Board>,
) {
    for ev in bullet_spawn_ev.read() {
        let bullet = ev.0;

        for mut snake in snake_query.iter_mut() {
            if snake.id == bullet.id {
                let len = snake.body.len();
                if len > 0 {
                    snake.body.remove(len - 1);
                }
            }
        }

        commands.spawn((
            MaterialMesh2dBundle {
                mesh: meshes.add(Rectangle::new(0.2, 0.2)).into(),
                material: materials.add(ColorMaterial::from(Color::srgb(1.0, 1.0, 0.26))),
                transform: Transform::from_xyz(
                    -b.width as f32 / 2.0 + bullet.pos.x as f32 + 0.5,
                    -b.height as f32 / 2.0 + bullet.pos.y as f32 + 0.5,
                    11.0,
                ),
                ..default()
            },
            bullet,
        ));
    }
}

pub fn bullet_system(
    mut commands: Commands,
    mut snake_query: Query<&Snake>,
    mut bullet_query: Query<(&mut Bullet, &mut Transform, Entity)>,
    time: Res<Time>,
    mut timer: ResMut<BulletTimer>,
    b: Res<Board>,
    settings: Res<Settings>,
    mut explosion_ev: EventWriter<ExplosionEv>,
    mut damage_ev: EventWriter<DamageSnakeEv>,
    walls: Res<Walls>,
    mut wall_ev: EventWriter<WallEv>,
) {
    use std::time::Duration;
    timer
        .0
        .set_duration(Duration::from_secs_f32(1.0 / settings.tps));
    timer.0.tick(time.delta());

    'outer: for (mut bullet, mut transform, bullet_entity) in bullet_query.iter_mut() {
        if timer.0.just_finished() {
            // handle collision
            for i in 0..=bullet.speed {
                let pos = bullet.pos + bullet.dir * i as i32;

                if !in_bounds(pos, &b) {
                    explosion_ev.send(ExplosionEv { pos });
                    commands.entity(bullet_entity).despawn();
                    continue 'outer;
                }

                for snake in snake_query.iter_mut() {
                    for j in 0..snake.body.len() {
                        if snake.body[j] == pos {
                            if j < 2 {
                                if snake.id == bullet.id {
                                    continue;
                                }
                            }

                            commands.entity(bullet_entity).despawn();
                            explosion_ev.send(ExplosionEv { pos });
                            damage_ev.send(DamageSnakeEv {
                                snake_id: snake.id,
                                snake_pos: j,
                            });

                            continue 'outer;
                        }
                    }
                }

                for (wall_pos, _) in walls.list.iter() {
                    if *wall_pos == pos {
                        explosion_ev.send(ExplosionEv { pos });
                        wall_ev.send(WallEv::Destroy(pos));
                        commands.entity(bullet_entity).despawn();
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
        transform.translation = Vec3::new(
            -b.width as f32 / 2.0
                + bullet.pos.x as f32
                + 0.5
                + interpolation * bullet.dir.x as f32 * 2.0,
            -b.height as f32 / 2.0
                + bullet.pos.y as f32
                + 0.5
                + interpolation * bullet.dir.y as f32 * 2.0,
            11.0,
        );
    }
}
