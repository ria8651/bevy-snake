use super::*;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            explosion_system.run_if(in_state(GameState::Playing)),
        );
    }
}

#[derive(Resource)]
struct EffectsResources {
    boom_atlas_layout_handle: Handle<TextureAtlasLayout>,
    boom_texture_handle: Handle<Image>,
    boom_sound_handle: Handle<AudioSource>,
}

fn setup(
    mut commands: Commands,
    mut texture_atlas_layouts: ResMut<Assets<TextureAtlasLayout>>,
    asset_server: Res<AssetServer>,
) {
    let texture_handle = asset_server.load("images/spritesheet.png");
    let layout = TextureAtlasLayout::from_grid(Vec2::splat(512.0), 31, 1, None, None);
    let boom_atlas_layout = texture_atlas_layouts.add(layout);

    // song
    // let music = asset_server.load("sounds/song.ogg");
    // audio.play_looped(music);

    commands.insert_resource(EffectsResources {
        boom_atlas_layout_handle: boom_atlas_layout,
        boom_texture_handle: texture_handle,
        boom_sound_handle: asset_server.load("sounds/boom.ogg"),
    });
}

#[derive(Event)]
pub struct ExplosionEv {
    pub pos: IVec2,
}

fn explosion_system(
    mut commands: Commands,
    mut explosion_ev: EventReader<ExplosionEv>,
    mut query: Query<(&mut AnimationTimer, &mut TextureAtlas, Entity)>,
    texture_atlas_layouts: Res<Assets<TextureAtlasLayout>>,
    effect_resources: Res<EffectsResources>,
    b: Res<Board>,
    time: Res<Time>,
) {
    for explosion in explosion_ev.read() {
        commands.spawn((
            SpriteBundle {
                texture: effect_resources.boom_texture_handle.clone(),
                transform: Transform::from_xyz(
                    explosion.pos.x as f32 - b.width as f32 / 2.0 + 0.5,
                    explosion.pos.y as f32 - b.height as f32 / 2.0 + 0.5,
                    12.0,
                )
                .with_scale(Vec3::new(0.01, 0.01, 1.0)),
                ..default()
            },
            TextureAtlas {
                layout: effect_resources.boom_atlas_layout_handle.clone(),
                index: 0,
            },
            AnimationTimer(Timer::from_seconds(0.04, TimerMode::Repeating)),
        ));

        commands.spawn(AudioBundle {
            source: effect_resources.boom_sound_handle.clone(),
            ..default()
        });
    }

    for (mut timer, mut texture_atlas, entity) in query.iter_mut() {
        timer.tick(time.delta());
        if timer.just_finished() {
            texture_atlas.index += 1;
            let texture_atlas_layout = texture_atlas_layouts.get(&texture_atlas.layout).unwrap();
            if texture_atlas.index >= texture_atlas_layout.textures.len() {
                commands.entity(entity).despawn();
            }
        }
    }
}
