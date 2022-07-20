use super::*;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system_set(SystemSet::on_update(GameState::Playing).with_system(explostion_system));
    }
}

pub struct ExplosionEv {
    pub pos: IVec2,
}

fn explostion_system(
    mut commands: Commands,
    settings: Res<Settings>,
    audio: Res<Audio>,
    b: Res<Board>,
    mut explosion_ev: EventReader<ExplosionEv>,
    time: Res<Time>,
    texture_atlases: Res<Assets<TextureAtlas>>,
    mut query: Query<(
        &mut AnimationTimer,
        &mut TextureAtlasSprite,
        &Handle<TextureAtlas>,
        Entity,
    )>,
) {
    for explosion in explosion_ev.iter() {
        commands
            .spawn_bundle(SpriteSheetBundle {
                texture_atlas: settings.boom_texture_atlas_handle.as_ref().unwrap().clone(),
                transform: Transform::from_xyz(
                    explosion.pos.x as f32 - b.width as f32 / 2.0 + 0.5,
                    explosion.pos.y as f32 - b.height as f32 / 2.0 + 0.5,
                    12.0,
                )
                .with_scale(Vec3::new(0.01, 0.01, 1.0)),
                ..default()
            })
            .insert(AnimationTimer(Timer::from_seconds(0.04, true)));

        audio.play(settings.boom_sound_handle.as_ref().unwrap().clone());
    }

    for (mut timer, mut sprite, texture_atlas_handle, entity) in query.iter_mut() {
        timer.tick(time.delta());
        if timer.just_finished() {
            let texture_atlas = texture_atlases.get(texture_atlas_handle).unwrap();
            sprite.index = sprite.index + 1;
            if sprite.index >= texture_atlas.textures.len() {
                commands.entity(entity).despawn();
            }
        }
    }
}
