use crate::{
    board::{AppleCount, Board, BoardSize, PlayerCount},
    game::Points,
    GameState, Settings,
};
use bevy::prelude::*;
use bevy_inspector_egui::{
    bevy_egui::{EguiContexts, EguiPlugin},
    egui::{self, Color32},
};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin)
            .register_type::<Settings>()
            .add_systems(Update, ui_system);
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut next_game_state: ResMut<NextState<GameState>>,
    mut last_score: Local<usize>,
    points: Res<Points>,
    board: Res<Board>,
) {
    egui::Window::new("Settings").show(contexts.ctx_mut(), |ui| {
        // scores
        ui.horizontal(|ui| {
            let point_colours = vec![
                Color::srgb(0.0, 0.7, 0.25),
                Color::srgb(0.3, 0.4, 0.7),
                Color::srgb(0.7, 0.4, 0.3),
                Color::srgb(0.7, 0.7, 0.7),
            ];

            if let PlayerCount::One = settings.board_settings.players {
                let score = if let Some(snake) = board.snakes().values().next() {
                    let score = snake.parts.len() - 4;
                    *last_score = score;
                    score
                } else {
                    *last_score
                };

                ui.label(format!("Score: {}", score));
            } else {
                for (i, score) in points.iter().enumerate() {
                    let col = point_colours[i].to_srgba();
                    let col = Color32::from_rgb(
                        (col.red * 255.0) as u8,
                        (col.green * 255.0) as u8,
                        (col.blue * 255.0) as u8,
                    );
                    ui.colored_label(col, format!("{}", score));
                }
            }
        });

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
            ui.selectable_value(&mut settings.tps, -1.0, "None");
            ui.selectable_value(&mut settings.tps, 1.0, "Slow");
            ui.selectable_value(&mut settings.tps, 7.5, "Medium");
            ui.selectable_value(&mut settings.tps, 10.0, "Fast");
            // ui.selectable_value(&mut settings.tps, 0.0, "Ramp");
        });
        settings.tps_ramp = settings.tps == 0.0;
        settings.do_game_tick = settings.tps != -1.0;

        ui.checkbox(&mut settings.ai, "AI");
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
}
