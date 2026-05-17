use crate::ClientState;
use bevy::feathers::{
    FeathersPlugins,
    controls::{ButtonProps, ButtonVariant, button, radio},
    dark_theme::create_dark_theme,
    theme::{ThemeBackgroundColor, ThemedText, UiTheme},
    tokens,
};
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::{Activate, RadioGroup, ValueChange, observe};
use bevy_snake::board::{AppleCount, Board, BoardSize, PlayerCount};
use bevy_snake::settings::{GameSettings, Speed};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()))
            .add_systems(OnEnter(ClientState::Lobby), spawn_lobby)
            .add_systems(OnExit(ClientState::Lobby), despawn::<LobbyUi>)
            .add_systems(OnEnter(ClientState::WaitingForOpponent), spawn_waiting)
            .add_systems(OnExit(ClientState::WaitingForOpponent), despawn::<WaitingUi>)
            .add_systems(OnEnter(ClientState::Playing), spawn_score_hud)
            .add_systems(OnExit(ClientState::Playing), despawn::<ScoreHudUi>)
            .add_systems(
                Update,
                (
                    pre_check_radios.run_if(in_state(ClientState::Lobby)),
                    update_scores.run_if(in_state(ClientState::Playing)),
                ),
            );
    }
}

#[derive(Component)]
struct LobbyUi;

#[derive(Component)]
struct WaitingUi;

#[derive(Component)]
struct ScoreHudUi;

#[derive(Component)]
struct ScoreText;

#[derive(Component, Clone, Copy)]
struct PlayerCountRadio(PlayerCount);

#[derive(Component, Clone, Copy)]
struct BoardSizeRadio(BoardSize);

#[derive(Component, Clone, Copy)]
struct AppleCountRadio(AppleCount);

#[derive(Component, Clone, Copy)]
struct SpeedRadio(Speed);

fn despawn<T: Component>(query: Query<Entity, With<T>>, mut commands: Commands) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

fn spawn_lobby(mut commands: Commands) {
    commands.spawn((
        LobbyUi,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        ThemeBackgroundColor(tokens::WINDOW_BG),
        TabGroup::default(),
        children![(
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(20.0)),
                min_width: Val::Px(280.0),
                ..default()
            },
            children![
                (
                    Text::new("Snake, WITH GUNS!"),
                    ThemedText,
                    TextFont {
                        font_size: 28.0,
                        ..default()
                    },
                ),
                section_label("Players"),
                player_count_group(),
                section_label("Board size"),
                board_size_group(),
                section_label("Apples"),
                apple_count_group(),
                section_label("Speed"),
                speed_group(),
                (
                    button(
                        ButtonProps {
                            variant: ButtonVariant::Primary,
                            ..default()
                        },
                        (),
                        Spawn((Text::new("Play"), ThemedText)),
                    ),
                    observe(
                        |_: On<Activate>, mut next: ResMut<NextState<ClientState>>| {
                            next.set(ClientState::WaitingForOpponent);
                        },
                    ),
                ),
            ],
        )],
    ));
}

fn section_label(text: &str) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        ThemedText,
        TextFont {
            font_size: 14.0,
            ..default()
        },
        Node {
            margin: UiRect::top(Val::Px(6.0)),
            ..default()
        },
    )
}

fn radio_row() -> Node {
    Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(4.0),
        ..default()
    }
}

fn player_count_group() -> impl Bundle {
    (
        radio_row(),
        RadioGroup,
        observe(
            |change: On<ValueChange<Entity>>,
             q_value: Query<(Entity, &PlayerCountRadio)>,
             mut settings: ResMut<GameSettings>,
             mut commands: Commands| {
                if let Ok((_, value)) = q_value.get(change.value) {
                    settings.board.players = value.0;
                }
                sync_checked(q_value.iter().map(|(e, _)| e), change.value, &mut commands);
            },
        ),
        children![
            radio(
                PlayerCountRadio(PlayerCount::One),
                Spawn((Text::new("1 (solo)"), ThemedText)),
            ),
            radio(
                PlayerCountRadio(PlayerCount::Two),
                Spawn((Text::new("2"), ThemedText)),
            ),
            radio(
                PlayerCountRadio(PlayerCount::Three),
                Spawn((Text::new("3"), ThemedText)),
            ),
            radio(
                PlayerCountRadio(PlayerCount::Four),
                Spawn((Text::new("4"), ThemedText)),
            ),
        ],
    )
}

fn board_size_group() -> impl Bundle {
    (
        radio_row(),
        RadioGroup,
        observe(
            |change: On<ValueChange<Entity>>,
             q_value: Query<(Entity, &BoardSizeRadio)>,
             mut settings: ResMut<GameSettings>,
             mut commands: Commands| {
                if let Ok((_, value)) = q_value.get(change.value) {
                    settings.board.board_size = value.0;
                }
                sync_checked(q_value.iter().map(|(e, _)| e), change.value, &mut commands);
            },
        ),
        children![
            radio(
                BoardSizeRadio(BoardSize::Small),
                Spawn((Text::new("Small"), ThemedText)),
            ),
            radio(
                BoardSizeRadio(BoardSize::Medium),
                Spawn((Text::new("Medium"), ThemedText)),
            ),
            radio(
                BoardSizeRadio(BoardSize::Large),
                Spawn((Text::new("Large"), ThemedText)),
            ),
        ],
    )
}

fn apple_count_group() -> impl Bundle {
    (
        radio_row(),
        RadioGroup,
        observe(
            |change: On<ValueChange<Entity>>,
             q_value: Query<(Entity, &AppleCountRadio)>,
             mut settings: ResMut<GameSettings>,
             mut commands: Commands| {
                if let Ok((_, value)) = q_value.get(change.value) {
                    settings.board.apples = value.0;
                }
                sync_checked(q_value.iter().map(|(e, _)| e), change.value, &mut commands);
            },
        ),
        children![
            radio(
                AppleCountRadio(AppleCount::One),
                Spawn((Text::new("1"), ThemedText)),
            ),
            radio(
                AppleCountRadio(AppleCount::Three),
                Spawn((Text::new("3"), ThemedText)),
            ),
            radio(
                AppleCountRadio(AppleCount::Five),
                Spawn((Text::new("5"), ThemedText)),
            ),
        ],
    )
}

fn speed_group() -> impl Bundle {
    (
        radio_row(),
        RadioGroup,
        observe(
            |change: On<ValueChange<Entity>>,
             q_value: Query<(Entity, &SpeedRadio)>,
             mut settings: ResMut<GameSettings>,
             mut commands: Commands| {
                if let Ok((_, value)) = q_value.get(change.value) {
                    settings.speed = value.0;
                }
                sync_checked(q_value.iter().map(|(e, _)| e), change.value, &mut commands);
            },
        ),
        children![
            radio(
                SpeedRadio(Speed::Slow),
                Spawn((Text::new("Slow"), ThemedText)),
            ),
            radio(
                SpeedRadio(Speed::Normal),
                Spawn((Text::new("Normal"), ThemedText)),
            ),
            radio(
                SpeedRadio(Speed::Fast),
                Spawn((Text::new("Fast"), ThemedText)),
            ),
        ],
    )
}

fn sync_checked(
    radios: impl Iterator<Item = Entity>,
    selected: Entity,
    commands: &mut Commands,
) {
    for radio in radios {
        if radio == selected {
            commands.entity(radio).insert(Checked);
        } else {
            commands.entity(radio).remove::<Checked>();
        }
    }
}

/// Fires once per group after the lobby is spawned: marks the radio that
/// matches the current `GameSettings` as `Checked`. Uses `Added<...>` so we
/// only do the work for newly-spawned radios.
fn pre_check_radios(
    settings: Res<GameSettings>,
    q_player: Query<(Entity, &PlayerCountRadio), Added<PlayerCountRadio>>,
    q_board: Query<(Entity, &BoardSizeRadio), Added<BoardSizeRadio>>,
    q_apple: Query<(Entity, &AppleCountRadio), Added<AppleCountRadio>>,
    q_speed: Query<(Entity, &SpeedRadio), Added<SpeedRadio>>,
    mut commands: Commands,
) {
    for (e, m) in &q_player {
        if m.0 == settings.board.players {
            commands.entity(e).insert(Checked);
        }
    }
    for (e, m) in &q_board {
        if m.0 == settings.board.board_size {
            commands.entity(e).insert(Checked);
        }
    }
    for (e, m) in &q_apple {
        if m.0 == settings.board.apples {
            commands.entity(e).insert(Checked);
        }
    }
    for (e, m) in &q_speed {
        if m.0 == settings.speed {
            commands.entity(e).insert(Checked);
        }
    }
}

fn spawn_waiting(mut commands: Commands) {
    commands
        .spawn((
            WaitingUi,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
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

fn spawn_score_hud(mut commands: Commands) {
    commands.spawn((
        ScoreHudUi,
        ScoreText,
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
    ));
}

fn update_scores(board: Res<Board>, mut texts: Query<&mut Text, With<ScoreText>>) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let mut snake_ids: Vec<u8> = board.snakes().keys().copied().collect();
    snake_ids.sort();
    let mut lines = Vec::new();
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
