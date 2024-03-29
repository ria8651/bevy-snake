use super::*;
use bevy_inspector_egui::{
    bevy_egui::{EguiContexts, EguiPlugin},
    egui,
};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin)
            .add_systems(Startup, ui_setup)
            .add_systems(Update, ui_system);
    }
}

#[derive(Component)]
struct PointId(u32);

fn ui_setup(mut commands: Commands, asset_server: Res<AssetServer>, colours: Res<Colours>) {
    // point counters
    commands
        .spawn(NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                align_items: AlignItems::FlexStart,
                align_content: AlignContent::FlexStart,
                justify_content: JustifyContent::Center,
                height: Val::Percent(100.0),
                width: Val::Percent(100.0),
                ..default()
            },
            background_color: Color::NONE.into(),
            ..default()
        })
        .with_children(|parent| {
            for i in 0..4 {
                parent
                    .spawn(TextBundle {
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
                            width: Val::Px(50.0),
                            height: Val::Px(50.0),
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
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut wall_ev: EventWriter<WallEv>,
    snake_query: Query<&Snake>,
) {
    egui::Window::new("Settings").show(contexts.ctx_mut(), |ui| {
        ui.label(format!("tps: {:.1}", settings.tps));

        ui.add(egui::Slider::new(&mut settings.snake_count, 1..=4).text("Players"));

        ui.checkbox(&mut settings.tps_ramp, "Speed ramp");
        if !settings.tps_ramp {
            ui.horizontal(|ui| {
                ui.label("Speed: ");
                ui.selectable_value(&mut settings.tps, 5.0, "Slow");
                ui.selectable_value(&mut settings.tps, 7.5, "Medium");
                ui.selectable_value(&mut settings.tps, 10.0, "Fast");
            });
        }

        ui.horizontal(|ui| {
            ui.label("Board size: ");
            ui.selectable_value(&mut settings.board_size, BoardSize::Small, "Small");
            ui.selectable_value(&mut settings.board_size, BoardSize::Medium, "Medium");
            ui.selectable_value(&mut settings.board_size, BoardSize::Large, "Large");
        });

        ui.horizontal(|ui| {
            ui.label("Apples: ");
            ui.selectable_value(&mut settings.apple_count, 1, "One");
            ui.selectable_value(&mut settings.apple_count, 3, "Three");
            ui.selectable_value(&mut settings.apple_count, 5, "Five");
        });

        ui.checkbox(&mut settings.walls, "Walls");
        ui.checkbox(&mut settings.walls_debug, "Walls debug");

        if ui.button("Spawn wall").clicked() {
            wall_ev.send(WallEv::Spawn);
        }

        ui.label("Controls");
        ui.label("Snake 1: WASD to move, LShift to shoot");
        ui.label("Snake 2: Arrows to move, RAlt to shoot");
        ui.label("Snake 3: PL;' to move, \\ to shoot");
        ui.label("Snake 4: YGHJ to move, B to shoot");
        ui.label("Space to restart");
    });

    for (point_id, mut text, mut style) in point_query.iter_mut() {
        let id = point_id.0;
        if settings.snake_count == 1 {
            if id == 0 {
                style.display = Display::Flex;
                for snake in snake_query.iter() {
                    if snake.id == 0 {
                        text.sections[0].value = snake.body.len().to_string();
                    }
                }
            } else {
                style.display = Display::None;
            }
        } else {
            if points.points[id as usize] == 0 {
                style.display = Display::None;
            } else {
                style.display = Display::Flex;
            }

            text.sections[0].value = points.points[id as usize].to_string();
        }
    }
}
