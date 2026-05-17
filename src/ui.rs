use crate::ClientState;
use crate::lobby::{CurrentLobby, LobbyClient, LobbyList, Role};
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
use bevy_snake::board::{AppleCount, Board, BoardSize};
use bevy_snake::lobby_proto::{LobbyState, MAX_PLAYERS};
use bevy_snake::settings::{GameSettings, Speed};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()))
            .insert_resource(BrowserListRev(u64::MAX))
            .add_systems(OnEnter(ClientState::Browsing), spawn_browser)
            .add_systems(OnExit(ClientState::Browsing), despawn::<BrowserUi>)
            .add_systems(OnEnter(ClientState::Creating), spawn_creating)
            .add_systems(OnExit(ClientState::Creating), despawn::<CreatingUi>)
            .add_systems(OnEnter(ClientState::WaitingForOpponent), spawn_waiting)
            .add_systems(OnExit(ClientState::WaitingForOpponent), despawn::<WaitingUi>)
            .add_systems(OnEnter(ClientState::Playing), spawn_score_hud)
            .add_systems(OnExit(ClientState::Playing), despawn::<ScoreHudUi>)
            .add_systems(OnEnter(ClientState::Finished), spawn_finished)
            .add_systems(OnExit(ClientState::Finished), despawn::<FinishedUi>)
            .add_systems(
                Update,
                (
                    pre_check_radios.run_if(in_state(ClientState::Creating)),
                    update_browser.run_if(in_state(ClientState::Browsing)),
                    update_waiting.run_if(in_state(ClientState::WaitingForOpponent)),
                    update_scores.run_if(in_state(ClientState::Playing)),
                ),
            );
    }
}

#[derive(Component)]
struct BrowserUi;

#[derive(Component)]
struct BrowserList;

#[derive(Component)]
struct CreatingUi;

#[derive(Component)]
struct WaitingUi;

#[derive(Component)]
struct WaitingText;

#[derive(Component)]
struct PlayerCountText;

#[derive(Component)]
struct StartButton;

#[derive(Component)]
struct StartButtonLabel;

#[derive(Component)]
struct ScoreHudUi;

#[derive(Component)]
struct ScoreText;

#[derive(Component)]
struct FinishedUi;

#[derive(Component, Clone, Copy)]
struct BoardSizeRadio(BoardSize);

#[derive(Component, Clone, Copy)]
struct AppleCountRadio(AppleCount);

#[derive(Component, Clone, Copy)]
struct SpeedRadio(Speed);

/// Tracks the `LobbyList.rev` we last rendered so the browser only rebuilds
/// when the server-pushed list actually changes.
#[derive(Resource)]
struct BrowserListRev(u64);

fn despawn<T: Component>(query: Query<Entity, With<T>>, mut commands: Commands) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

// ── Browser ─────────────────────────────────────────────────────────────

fn spawn_browser(mut commands: Commands, mut rev: ResMut<BrowserListRev>) {
    rev.0 = u64::MAX; // force the next update_browser pass to populate the list
    commands.spawn((
        BrowserUi,
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
                min_width: Val::Px(420.0),
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
                browser_buttons_row(),
                (
                    BrowserList,
                    Node {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        margin: UiRect::top(Val::Px(8.0)),
                        ..default()
                    },
                ),
            ],
        )],
    ));
}

fn browser_buttons_row() -> impl Bundle {
    (
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
        children![
            (
                button(
                    ButtonProps {
                        variant: ButtonVariant::Primary,
                        ..default()
                    },
                    (),
                    Spawn((Text::new("Create lobby"), ThemedText)),
                ),
                observe(
                    |_: On<Activate>, mut next: ResMut<NextState<ClientState>>| {
                        next.set(ClientState::Creating);
                    },
                ),
            ),
            (
                button(
                    ButtonProps::default(),
                    (),
                    Spawn((Text::new("Solo play"), ThemedText)),
                ),
                observe(
                    |_: On<Activate>,
                     mut current: ResMut<CurrentLobby>,
                     mut next: ResMut<NextState<ClientState>>| {
                        current.clear();
                        current.role = Role::Solo;
                        next.set(ClientState::WaitingForOpponent);
                    },
                ),
            ),
        ],
    )
}

fn update_browser(
    list: Res<LobbyList>,
    mut rev: ResMut<BrowserListRev>,
    container: Query<Entity, With<BrowserList>>,
    mut commands: Commands,
) {
    if list.rev == rev.0 {
        return;
    }
    rev.0 = list.rev;
    let Ok(container) = container.single() else {
        return;
    };
    // Wipe the children and rebuild from scratch. The list is tiny (a
    // handful at most) so the simplicity wins over diffing.
    commands.entity(container).despawn_related::<Children>();
    if list.lobbies.is_empty() {
        commands.entity(container).with_children(|p| {
            p.spawn((
                Text::new("No open lobbies. Create one!"),
                ThemedText,
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
            ));
        });
        return;
    }
    for lobby in &list.lobbies {
        let id = lobby.id.clone();
        let label = format!(
            "{}/{}  ·  {}  ·  {} apples  ·  {}  ·  {}",
            lobby.players_present,
            MAX_PLAYERS,
            board_size_label(lobby.settings.board.board_size),
            lobby.settings.board.apples as u8,
            speed_label(lobby.settings.speed),
            state_label(lobby.state),
        );
        let joinable =
            lobby.state == LobbyState::Waiting && lobby.players_present < MAX_PLAYERS;
        let settings_copy = lobby.settings;
        commands.entity(container).with_children(|p| {
            let btn = (
                button(
                    ButtonProps {
                        variant: if joinable {
                            ButtonVariant::Normal
                        } else {
                            ButtonVariant::Normal
                        },
                        ..default()
                    },
                    (),
                    Spawn((Text::new(label), ThemedText)),
                ),
                observe(
                    move |_: On<Activate>,
                          mut client: NonSendMut<LobbyClient>,
                          mut current: ResMut<CurrentLobby>,
                          mut settings: ResMut<GameSettings>,
                          mut next: ResMut<NextState<ClientState>>| {
                        if !joinable {
                            return;
                        }
                        *settings = settings_copy;
                        client.join_lobby(id.clone());
                        current.clear();
                        current.id = Some(id.clone());
                        current.room_name = Some(format!("lobby-{}", id));
                        current.role = Role::Joiner;
                        next.set(ClientState::WaitingForOpponent);
                    },
                ),
            );
            p.spawn(btn);
        });
    }
}

fn board_size_label(b: BoardSize) -> &'static str {
    match b {
        BoardSize::Small => "Small",
        BoardSize::Medium => "Medium",
        BoardSize::Large => "Large",
    }
}

fn speed_label(s: Speed) -> &'static str {
    match s {
        Speed::Slow => "Slow",
        Speed::Normal => "Normal",
        Speed::Fast => "Fast",
    }
}

fn state_label(s: LobbyState) -> &'static str {
    match s {
        LobbyState::Waiting => "Waiting",
        LobbyState::Playing => "Playing",
        LobbyState::Finished => "Finished",
    }
}

// ── Creating (settings panel + Host button) ─────────────────────────────

fn spawn_creating(mut commands: Commands) {
    commands.spawn((
        CreatingUi,
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
                    Text::new("Create lobby"),
                    ThemedText,
                    TextFont {
                        font_size: 24.0,
                        ..default()
                    },
                ),
                section_label("Board size"),
                board_size_group(),
                section_label("Apples"),
                apple_count_group(),
                section_label("Speed"),
                speed_group(),
                (
                    Node {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        margin: UiRect::top(Val::Px(8.0)),
                        ..default()
                    },
                    children![
                        (
                            button(
                                ButtonProps {
                                    variant: ButtonVariant::Primary,
                                    ..default()
                                },
                                (),
                                Spawn((Text::new("Host"), ThemedText)),
                            ),
                            observe(
                                |_: On<Activate>,
                                 mut client: NonSendMut<LobbyClient>,
                                 settings: Res<GameSettings>| {
                                    client.create_lobby(*settings);
                                    // We don't transition yet — the
                                    // LobbyCreated ack does that, so the
                                    // user knows the host succeeded before
                                    // we move on.
                                },
                            ),
                        ),
                        (
                            button(
                                ButtonProps::default(),
                                (),
                                Spawn((Text::new("Cancel"), ThemedText)),
                            ),
                            observe(
                                |_: On<Activate>,
                                 mut next: ResMut<NextState<ClientState>>| {
                                    next.set(ClientState::Browsing);
                                },
                            ),
                        ),
                    ],
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

/// Fires once per group after the Creating panel is spawned: marks the
/// radio matching the current `GameSettings` as `Checked`. Uses
/// `Added<...>` so we only do the work for newly-spawned radios.
fn pre_check_radios(
    settings: Res<GameSettings>,
    q_board: Query<(Entity, &BoardSizeRadio), Added<BoardSizeRadio>>,
    q_apple: Query<(Entity, &AppleCountRadio), Added<AppleCountRadio>>,
    q_speed: Query<(Entity, &SpeedRadio), Added<SpeedRadio>>,
    mut commands: Commands,
) {
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

// ── Waiting ─────────────────────────────────────────────────────────────

fn spawn_waiting(mut commands: Commands, current: Res<CurrentLobby>) {
    let is_host = current.role == Role::Host;
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
            // Inner auto-sized container; without it, Feathers' button
            // flex-grows to fill the outer overlay.
            p.spawn((Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                ..default()
            },))
                .with_children(|p| {
            // Big "n/4 players" line so the host can see at a glance
            // whether anyone has joined.
            p.spawn((
                PlayerCountText,
                Text::new("…"),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            // Sub-line: role-specific status ("Hosting", "Waiting for
            // host…", etc).
            p.spawn((
                WaitingText,
                Text::new("Connecting…"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
            ));
            p.spawn((
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    margin: UiRect::top(Val::Px(16.0)),
                    ..default()
                },
                children![
                    // One button regardless of role — its label and click
                    // behavior pivot on `is_host`. Joiner taps are silently
                    // ignored. Both branches share an observer signature so
                    // the children![] macro accepts them in either case.
                    (
                        StartButton,
                        button(
                            ButtonProps {
                                variant: if is_host {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Normal
                                },
                                ..default()
                            },
                            (),
                            Spawn((
                                Text::new(if is_host {
                                    "Start (need 2+ players)"
                                } else {
                                    "Waiting for host…"
                                }),
                                ThemedText,
                                StartButtonLabel,
                            )),
                        ),
                        observe(
                            move |_: On<Activate>,
                                  mut client: NonSendMut<LobbyClient>,
                                  list: Res<LobbyList>,
                                  current: Res<CurrentLobby>| {
                                if !is_host {
                                    return;
                                }
                                let Some(id) = current.id.clone() else {
                                    return;
                                };
                                let count = list
                                    .lobbies
                                    .iter()
                                    .find(|l| l.id == id)
                                    .map(|l| l.players_present)
                                    .unwrap_or(0);
                                if count < 2 {
                                    return;
                                }
                                client.start_lobby(id);
                            },
                        ),
                    ),
                    (
                        button(
                            ButtonProps::default(),
                            (),
                            Spawn((Text::new("Leave"), ThemedText)),
                        ),
                        observe(
                            |_: On<Activate>,
                             mut client: NonSendMut<LobbyClient>,
                             mut current: ResMut<CurrentLobby>,
                             mut next: ResMut<NextState<ClientState>>| {
                                if let Some(id) = current.id.clone() {
                                    client.leave_lobby(id);
                                }
                                current.clear();
                                next.set(ClientState::Browsing);
                            },
                        ),
                    ),
                ],
            ));
                });
        });
}

fn update_waiting(
    current: Res<CurrentLobby>,
    list: Res<LobbyList>,
    mut count_text: Query<
        &mut Text,
        (With<PlayerCountText>, Without<WaitingText>, Without<StartButtonLabel>),
    >,
    mut status_text: Query<
        &mut Text,
        (With<WaitingText>, Without<PlayerCountText>, Without<StartButtonLabel>),
    >,
    mut start_label: Query<
        &mut Text,
        (With<StartButtonLabel>, Without<PlayerCountText>, Without<WaitingText>),
    >,
) {
    if current.role == Role::Solo {
        if let Ok(mut t) = count_text.single_mut() {
            t.0 = "Solo".into();
        }
        if let Ok(mut t) = status_text.single_mut() {
            t.0 = "Starting solo game…".into();
        }
        return;
    }
    let count = current
        .id
        .as_ref()
        .and_then(|id| list.lobbies.iter().find(|l| &l.id == id))
        .map(|l| l.players_present)
        .unwrap_or(1);
    if let Ok(mut t) = count_text.single_mut() {
        t.0 = format!("{}/{} players", count, MAX_PLAYERS);
    }
    if let Ok(mut t) = status_text.single_mut() {
        t.0 = match current.role {
            Role::Host => {
                if count < 2 {
                    "Hosting — waiting for someone to join".into()
                } else {
                    "Hosting — click Start when ready".into()
                }
            }
            Role::Joiner => "Waiting for host to start…".into(),
            _ => "Connecting…".into(),
        };
    }
    if let Ok(mut t) = start_label.single_mut() {
        if current.role == Role::Host {
            t.0 = if count < 2 {
                "Start (need 2+ players)".into()
            } else {
                format!("Start ({} players)", count)
            };
        }
    }
}

// ── Score HUD ───────────────────────────────────────────────────────────

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

// ── Finished ────────────────────────────────────────────────────────────

fn spawn_finished(mut commands: Commands) {
    // Outer = fullscreen dim overlay; inner = auto-sized column that holds
    // the actual content. Without the inner wrapper, Feathers' button
    // flex-grows to fill the outer's height — that's how we ended up with
    // a screen-tall "Back to lobbies" button.
    commands
        .spawn((
            FinishedUi,
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
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(16.0),
                    ..default()
                },
                children![
                    (
                        Text::new("Game over"),
                        TextFont {
                            font_size: 36.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ),
                    (
                        button(
                            ButtonProps {
                                variant: ButtonVariant::Primary,
                                ..default()
                            },
                            (),
                            Spawn((Text::new("Back to lobbies"), ThemedText)),
                        ),
                        observe(
                            |_: On<Activate>,
                             mut client: NonSendMut<LobbyClient>,
                             mut current: ResMut<CurrentLobby>,
                             mut next: ResMut<NextState<ClientState>>| {
                                if current.role == Role::Host {
                                    if let Some(id) = current.id.clone() {
                                        client.mark_finished(id);
                                    }
                                }
                                if let Some(id) = current.id.clone() {
                                    client.leave_lobby(id);
                                }
                                current.clear();
                                next.set(ClientState::Browsing);
                            },
                        ),
                    ),
                ],
            ));
        });
}

