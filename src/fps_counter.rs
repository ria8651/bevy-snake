use bevy::{
    app::Plugin,
    diagnostic::{Diagnostics, FrameTimeDiagnosticsPlugin},
    prelude::*,
};

#[derive(Component)]
pub struct FpsCounter;

impl Plugin for FpsCounter {
    fn build(&self, app: &mut App) {
        app.add_startup_system(setup)
            .add_system(fps_system)
            .add_plugin(FrameTimeDiagnosticsPlugin::default());
    }
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    // ui
    commands.spawn_bundle(UiCameraBundle::default());

    // fps
    commands
        .spawn_bundle(TextBundle {
            text: Text {
                sections: vec![TextSection {
                    value: "0.00".to_string(),
                    style: TextStyle {
                        font: asset_server.load("fonts/FiraSans-Bold.ttf"),
                        font_size: 40.0,
                        color: Color::rgb(1.0, 1.0, 1.0),
                    },
                }],
                ..default()
            },
            style: Style {
                position_type: PositionType::Absolute,
                position: Rect {
                    top: Val::Px(10.0),
                    left: Val::Px(10.0),
                    ..default()
                },
                ..default()
            },
            ..default()
        })
        .insert(FpsCounter);
}

fn fps_system(diagnostics: Res<Diagnostics>, mut query: Query<&mut Text, With<FpsCounter>>) {
    if let Some(fps) = diagnostics.get(FrameTimeDiagnosticsPlugin::FPS) {
        if let Some(average) = fps.average() {
            for mut text in query.iter_mut() {
                text.sections[0].value = format!("{:.1}", average);
            }
        }
    }
}
