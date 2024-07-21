use crate::{
    board::{AppleCount, BoardSize, PlayerCount},
    game::Points,
    GameState, Settings,
};
use bevy::prelude::*;
use bevy_inspector_egui::{
    bevy_egui::{EguiContexts, EguiPlugin},
    egui, DefaultInspectorConfigPlugin,
};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EguiPlugin, DefaultInspectorConfigPlugin))
            .register_type::<Settings>()
            .add_systems(Startup, ui_setup)
            .add_systems(Update, ui_system);
    }
}

#[derive(Component)]
struct PointId(u32);

fn ui_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let point_colours = vec![
        Color::srgb(0.0, 0.7, 0.25),
        Color::srgb(0.3, 0.4, 0.7),
        Color::srgb(0.7, 0.4, 0.3),
        Color::srgb(0.7, 0.7, 0.7),
    ];

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
                                    color: point_colours[i],
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
    mut _point_query: Query<(&PointId, &mut Text, &mut Style)>,
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut next_game_state: ResMut<NextState<GameState>>,
    _type_registry: Res<AppTypeRegistry>,
    _points: Res<Points>,
) {
    egui::Window::new("Settings").show(contexts.ctx_mut(), |ui| {
        ui.label(format!("tps: {:.1}", settings.tps));

        // ui_for_value(settings.as_mut(), ui, &type_registry.read());

        let bs = &mut settings.board_settings;
        ui.horizontal(|ui| {
            ui.label("Players: ");
            ui.selectable_value(&mut bs.players, PlayerCount::One, "One");
            ui.selectable_value(&mut bs.players, PlayerCount::Two, "Two");
            ui.selectable_value(&mut bs.players, PlayerCount::Three, "Three");
            ui.selectable_value(&mut bs.players, PlayerCount::Four, "Four");
        });
        ui.horizontal(|ui| {
            ui.label("Board Size: ");
            ui.selectable_value(&mut bs.board_size, BoardSize::Small, "Small");
            ui.selectable_value(&mut bs.board_size, BoardSize::Medium, "Medium");
            ui.selectable_value(&mut bs.board_size, BoardSize::Large, "Large");
        });

        ui.horizontal(|ui| {
            ui.label("Apples: ");
            ui.selectable_value(&mut bs.apples, AppleCount::One, "One");
            ui.selectable_value(&mut bs.apples, AppleCount::Three, "Three");
            ui.selectable_value(&mut bs.apples, AppleCount::Five, "Five");
        });

        ui.horizontal(|ui| {
            ui.label("Speed: ");
            ui.selectable_value(&mut settings.tps, 5.0, "Slow");
            ui.selectable_value(&mut settings.tps, 7.5, "Medium");
            ui.selectable_value(&mut settings.tps, 10.0, "Fast");
            ui.selectable_value(&mut settings.tps, 0.0, "Ramp");
        });
        settings.tps_ramp = settings.tps == 0.0;

        ui.checkbox(&mut settings.walls, "Walls");
        ui.checkbox(&mut settings.walls_debug, "Walls debug");

        if ui.button("New Game").clicked() {
            next_game_state.set(GameState::Start);
        }

        ui.label("Controls");
        ui.label("Snake 1: WASD to move, LShift to shoot");
        ui.label("Snake 2: Arrows to move, RAlt to shoot");
        ui.label("Snake 3: PL;' to move, \\ to shoot");
        ui.label("Snake 4: YGHJ to move, B to shoot");
        ui.label("Space to restart");
    });

    // for (point_id, mut text, mut style) in point_query.iter_mut() {
    //     let id = point_id.0;
    //     if settings.snake_count == 1 {
    //         if id == 0 {
    //             style.display = Display::Flex;
    //             for snake in snake_query.iter() {
    //                 if snake.id == 0 {
    //                     text.sections[0].value = snake.body.len().to_string();
    //                 }
    //             }
    //         } else {
    //             style.display = Display::None;
    //         }
    //     } else {
    //         if points[id as usize] == 0 {
    //             style.display = Display::None;
    //         } else {
    //             style.display = Display::Flex;
    //         }

    //         text.sections[0].value = points[id as usize].to_string();
    //     }
    // }
}
