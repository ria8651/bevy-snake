use crate::ClientState;
use bevy::prelude::*;
use bevy_snake::board::Board;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    update_scores,
                    toggle_waiting_overlay,
                ),
            );
    }
}

#[derive(Component)]
struct ScoreText;

#[derive(Component)]
struct WaitingOverlay;

fn setup(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(8.0),
                ..default()
            },
            Text::new(""),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::WHITE),
            ScoreText,
        ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            WaitingOverlay,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("Waiting for opponent…"),
                TextFont {
                    font_size: 36.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

fn update_scores(
    board: Res<Board>,
    state: Res<State<ClientState>>,
    mut texts: Query<&mut Text, With<ScoreText>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    if *state.get() != ClientState::Playing {
        text.0.clear();
        return;
    }
    // One line per living snake: "P0: 5"
    let mut lines = Vec::new();
    let mut snake_ids: Vec<u8> = board.snakes().keys().copied().collect();
    snake_ids.sort();
    for id in snake_ids {
        let snake = &board.snakes()[&id];
        let score = snake.parts.len().saturating_sub(4);
        lines.push(format!("P{}: {}", id, score));
    }
    if lines.is_empty() {
        lines.push("Press Space to restart".to_string());
    }
    text.0 = lines.join("\n");
}

fn toggle_waiting_overlay(
    state: Res<State<ClientState>>,
    mut overlay: Query<&mut Node, With<WaitingOverlay>>,
) {
    let Ok(mut node) = overlay.single_mut() else {
        return;
    };
    node.display = match *state.get() {
        ClientState::WaitingForOpponent => Display::Flex,
        ClientState::Playing => Display::None,
    };
}
