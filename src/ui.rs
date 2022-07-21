use super::*;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_startup_system(ui_setup).add_system(ui_system);
    }
}

#[derive(Component)]
struct PointId(u32);

fn ui_setup(mut commands: Commands, asset_server: Res<AssetServer>, colours: Res<Colours>) {
    // ui camera
    commands.spawn_bundle(UiCameraBundle::default());

    // point counters
    commands
        .spawn_bundle(NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                align_items: AlignItems::FlexEnd,
                align_content: AlignContent::FlexEnd,
                justify_content: JustifyContent::Center,
                size: Size::new(Val::Percent(100.0), Val::Percent(100.0)),
                ..default()
            },
            color: Color::NONE.into(),
            ..default()
        })
        .with_children(|parent| {
            for i in 0..4 {
                parent
                    .spawn_bundle(TextBundle {
                        text: Text {
                            sections: vec![TextSection {
                                value: i.to_string(),
                                style: TextStyle {
                                    font: asset_server.load("fonts/FiraSans-Bold.ttf"),
                                    font_size: 40.0,
                                    color: colours.colours[i],
                                },
                            }],
                            ..Default::default()
                        },
                        style: Style {
                            size: Size::new(Val::Px(50.0), Val::Px(50.0)),
                            ..Default::default()
                        },

                        ..Default::default()
                    })
                    .insert(PointId(i as u32));
            }
        });
}

fn ui_system(mut point_query: Query<(&PointId, &mut Text, &mut Style)>, points: Res<snake::Points>) {
    for (point_id, mut text, mut style) in point_query.iter_mut() {
        let id = point_id.0;
        if points.points[id as usize] == 0 {
            style.display = Display::None;
        } else {
            style.display = Display::Flex;
        }

        text.sections[0].value = points.points[id as usize].to_string();
    }
}
