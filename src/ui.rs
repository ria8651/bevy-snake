use super::*;
use bevy_egui::{egui, EguiContext, EguiPlugin};

// const BUTTON_COLOUR: Color = Color::rgb(0.4, 0.4, 0.4);
// const BUTTON_HOVER: Color = Color::rgb(0.5, 0.5, 0.5);
// const BUTTON_PRESS: Color = Color::rgb(0.3, 0.8, 0.1);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EguiPlugin)
            .add_startup_system(ui_setup)
            .add_system(ui_system);
    }
}

#[derive(Component)]
struct PointId(u32);

fn ui_setup(mut commands: Commands, asset_server: Res<AssetServer>, colours: Res<Colours>) {
    // ui camera
    commands.spawn_bundle(UiCameraBundle::default());

    // // button bundle
    // let button = ButtonBundle {
    //     style: Style {
    //         justify_content: JustifyContent::Center,
    //         align_items: AlignItems::Center,
    //         margin: Rect {
    //             left: Val::Px(0.0),
    //             right: Val::Px(0.0),
    //             top: Val::Px(0.0),
    //             bottom: Val::Px(25.0),
    //         },
    //         size: Size::new(Val::Px(150.0), Val::Px(40.0)),
    //         ..default()
    //     },
    //     color: BUTTON_COLOUR.into(),
    //     ..default()
    // };

    // // pause menu
    // commands
    //     .spawn_bundle(NodeBundle {
    //         style: Style {
    //             position_type: PositionType::Absolute,
    //             align_items: AlignItems::Center,
    //             justify_content: JustifyContent::Center,
    //             size: Size::new(Val::Percent(100.0), Val::Percent(100.0)),
    //             ..default()
    //         },
    //         color: Color::rgba(0.0, 0.0, 0.0, 0.5).into(),
    //         ..default()
    //     })
    //     .with_children(|parent| {
    //         parent
    //             .spawn_bundle(NodeBundle {
    //                 style: Style {
    //                     align_items: AlignItems::Center,
    //                     flex_direction: FlexDirection::Column,
    //                     padding: Rect::all(Val::Px(25.0)),
    //                     ..default()
    //                 },
    //                 color: Color::rgb(0.3, 0.3, 0.3).into(),
    //                 ..default()
    //             })
    //             .with_children(|parent| {
    //                 parent.spawn_bundle(button.clone());
    //                 parent.spawn_bundle(button.clone());
    //             });
    //     });

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
                            ..default()
                        },
                        style: Style {
                            size: Size::new(Val::Px(50.0), Val::Px(50.0)),
                            ..default()
                        },

                        ..default()
                    })
                    .insert(PointId(i as u32));
            }
        });
}

fn ui_system(
    mut point_query: Query<(&PointId, &mut Text, &mut Style)>,
    points: Res<snake::Points>,
    mut egui_context: ResMut<EguiContext>,
    mut game_state: ResMut<State<GameState>>,
    mut settings: ResMut<Settings>,
) {
    // egui ui
    egui::Window::new("Settings")
        .anchor(egui::Align2::RIGHT_TOP, [-5.0, 5.0])
        .show(egui_context.ctx_mut(), |ui| {
            ui.label(format!("tps: {:.1}", settings.tps));

            if ui.button("End game").clicked() {
                game_state.set(GameState::GameOver).unwrap();
            }

            ui.add(egui::Slider::new(&mut settings.snake_count, 1..=4).text("Players: "));

            ui.label("Controls");
            ui.label("Snake 1: WASD to move, LShift to shoot");
            ui.label("Snake 2: Arrows to move, RAlt to shoot");
            ui.label("Snake 3: PL;' to move, \\ to shoot");
            ui.label("Snake 4: YGHJ to move, B to shoot");
            ui.label("Space to restart");
        });

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
