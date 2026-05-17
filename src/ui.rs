use crate::{
    client::ClientConnection,
    game::get_wt_url,
    game::{Points, RttEstimator, TickClock},
    ClientState, ConnectionError, GizmoSetting, Settings,
};
use bevy::prelude::*;
use bevy_inspector_egui::{
    bevy_egui::{EguiContexts, EguiPlugin},
    egui::{self, Color32},
};
use bevy_snake::{
    board::{AppleCount, Board, BoardSize, PlayerCount},
    GameCommands,
};
use web_time::Instant;

/// Pause sentinel — matches `server::PAUSE_SENTINEL_MS`. Sent in
/// `SetTickRate` to pause the server tick loop.
const PAUSE_SENTINEL_MS: u32 = u32::MAX;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin)
            .register_type::<Settings>()
            .add_systems(Update, (ui_system, connecting_system, error_system));
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut last_score: Local<usize>,
    points: Res<Points>,
    board: Res<Board>,
    state: Res<State<ClientState>>,
    tick_clock: Res<TickClock>,
    rtt: Res<RttEstimator>,
    mut client_connections: Query<&mut ClientConnection>,
) {
    if *state.get() != ClientState::Connected {
        return;
    }
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

        let prev_interval = settings.tick_interval_ms;
        ui.horizontal(|ui| {
            ui.label("Speed: ");
            ui.selectable_value(&mut settings.tick_interval_ms, None, "None");
            ui.selectable_value(&mut settings.tick_interval_ms, Some(1000), "Slow");
            ui.selectable_value(&mut settings.tick_interval_ms, Some(133), "Medium");
            ui.selectable_value(&mut settings.tick_interval_ms, Some(100), "Fast");
        });
        if settings.tick_interval_ms != prev_interval {
            if let Ok(conn) = client_connections.get_single_mut() {
                let wire_ms = settings.tick_interval_ms.unwrap_or(PAUSE_SENTINEL_MS);
                conn.send_command(GameCommands::SetTickRate {
                    tick_interval_ms: wire_ms,
                });
            }
        }

        ui.horizontal(|ui| {
            ui.label("Gizmos: ");
            ui.selectable_value(&mut settings.gizmos, GizmoSetting::None, "None");
            ui.selectable_value(&mut settings.gizmos, GizmoSetting::TreeSearch, "TreeSearch");
            ui.selectable_value(&mut settings.gizmos, GizmoSetting::CycleBasis, "CycleBasis");
        });

        ui.checkbox(&mut settings.ai, "AI");
        ui.checkbox(&mut settings.walls, "Walls");
        ui.checkbox(&mut settings.walls_debug, "Walls debug");

        ui.collapsing("Net / Sync", |ui| {
            let now = Instant::now();
            ui.label(format!(
                "tick_period: {:.1} ms",
                tick_clock.tick_period.as_secs_f64() * 1000.0
            ));
            let rtt_str = rtt
                .last_rtt_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "—".to_string());
            let owt_str = rtt
                .one_way_ms
                .map(|v| format!("{:.1}", v))
                .unwrap_or_else(|| "—".to_string());
            ui.label(format!("RTT: {} ms (OWT: {} ms)", rtt_str, owt_str));
            ui.label(format!(
                "phase_error: {:+.2} ms",
                tick_clock.last_phase_error_ns as f64 / 1e6
            ));
            let until = tick_clock.next_tick_at.saturating_duration_since(now);
            ui.label(format!(
                "next tick in: {:.1} ms",
                until.as_secs_f64() * 1000.0
            ));
            ui.label(format!(
                "last snapshot snapped: {}",
                tick_clock.last_snapped
            ));
        });

        ui.label("Controls");
        ui.label("Snake 1: WASD to move, LShift to shoot");
        ui.label("Snake 2: Arrows to move, RAlt to shoot");
        ui.label("Snake 3: PL;' to move, \\ to shoot");
        ui.label("Snake 4: YGHJ to move, B to shoot");
        ui.label("Space to restart");
    });
}

fn connecting_system(mut contexts: EguiContexts, state: Res<State<ClientState>>) {
    if *state.get() != ClientState::Connecting {
        return;
    }
    egui::Window::new("Connecting")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(contexts.ctx_mut(), |ui| {
            ui.label("Connecting to server…");
        });
}

fn error_system(
    mut contexts: EguiContexts,
    state: Res<State<ClientState>>,
    error: Res<ConnectionError>,
    mut next_state: ResMut<NextState<ClientState>>,
    mut commands: Commands,
    connections: Query<Entity, With<ClientConnection>>,
) {
    if *state.get() != ClientState::Error {
        return;
    }
    egui::Window::new("Connection error")
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(contexts.ctx_mut(), |ui| {
            ui.heading(error.headline());
            if let Some(hint) = error.hint() {
                ui.add_space(4.0);
                ui.label(hint);
            }
            if !error.detail.is_empty() {
                ui.add_space(4.0);
                ui.collapsing("Details", |ui| {
                    ui.monospace(&error.detail);
                });
            }
            ui.add_space(8.0);
            if ui.button("Retry").clicked() {
                for entity in &connections {
                    commands.entity(entity).despawn();
                }
                commands.spawn(ClientConnection::new(get_wt_url()));
                next_state.set(ClientState::Connecting);
            }
        });
}
