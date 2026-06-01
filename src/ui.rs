use crate::ClientState;
use crate::lobby::{CurrentLobby, LobbyClient, LobbyList, Role};
use crate::net::{ConnectStage, InterpolationPhase, NetStatus, PendingInput};
use crate::notice::{Notice, NoticeLevel};
use crate::render::{BoardImageNode, BoardRenderTarget};
use bevy::feathers::{
    FeathersPlugins,
    controls::{ButtonProps, ButtonVariant, SliderProps, button, radio, slider},
    dark_theme::create_dark_theme,
    theme::{ThemeBackgroundColor, ThemedText, UiTheme},
    tokens,
};
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::{Activate, RadioGroup, SliderPrecision, SliderValue, ValueChange, observe};
use bevy_snake::board::{AppleCount, Board, BoardSize};
use bevy_snake::lobby_proto::{LobbyState, MAX_PLAYERS};
use bevy_snake::settings::{GameSettings, Speed};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FeathersPlugins)
            .insert_resource(UiTheme(create_dark_theme()))
            .insert_resource(BrowserListRev(u64::MAX))
            .add_systems(Startup, spawn_notice_banner)
            .add_systems(OnEnter(ClientState::Browsing), spawn_browser)
            .add_systems(OnExit(ClientState::Browsing), despawn::<BrowserUi>)
            .add_systems(OnEnter(ClientState::WaitingForOpponent), spawn_waiting)
            .add_systems(OnExit(ClientState::WaitingForOpponent), despawn::<WaitingUi>)
            .add_systems(OnEnter(ClientState::Playing), spawn_score_hud)
            .add_systems(OnExit(ClientState::Playing), despawn::<ScoreHudUi>)
            .add_systems(OnEnter(ClientState::Finished), spawn_finished)
            .add_systems(OnExit(ClientState::Finished), despawn::<FinishedUi>)
            .add_systems(
                Update,
                (
                    update_notice_banner,
                    (pre_check_radios, update_browser, update_connectivity_pill)
                        .run_if(in_state(ClientState::Browsing)),
                    update_waiting.run_if(in_state(ClientState::WaitingForOpponent)),
                    (update_scores, update_game_over_banner)
                        .run_if(in_state(ClientState::Playing)),
                ),
            );
    }
}

#[derive(Component)]
struct BrowserUi;

#[derive(Component)]
struct BrowserList;

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

/// "Game over" overlay that appears within Playing state when the board has
/// no snakes left. Visibility-toggled by `update_game_over_banner` instead
/// of spawned/despawned so we don't churn entities every frame.
#[derive(Component)]
struct GameOverBanner;

#[derive(Component)]
struct FinishedUi;

/// Subtitle text on the Finished overlay. Driven by the most-recent Notice
/// at the moment Finished is entered, then static.
#[derive(Component)]
struct FinishedSubtitle;

/// Always-present non-modal banner at the top of the screen. Visibility +
/// content driven by [`Notice`]. Spawned once at startup, never despawned.
#[derive(Component)]
struct NoticeBanner;

#[derive(Component)]
struct NoticeBannerText;

#[derive(Component)]
struct NoticeBannerBg;

#[derive(Component)]
struct ConnectivityPill;

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

// ── Notice banner (top-of-screen, all states) ───────────────────────────

fn spawn_notice_banner(mut commands: Commands) {
    // ZIndex pushes the banner above all other state-specific UI; otherwise
    // the Browsing root (which is z=0, position Absolute) can cover it.
    commands
        .spawn((
            NoticeBanner,
            NoticeBannerBg,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                // Toggled to Flex by update_notice_banner when a Notice is present.
                display: Display::None,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: Val::Px(12.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.5, 0.0, 0.0, 0.92)),
            ZIndex(1000),
        ))
        .with_children(|p| {
            p.spawn((
                NoticeBannerText,
                Text::new(""),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            p.spawn((
                button(
                    ButtonProps::default(),
                    (),
                    Spawn((Text::new("Dismiss"), ThemedText)),
                ),
                observe(|_: On<Activate>, mut notice: ResMut<Notice>| {
                    notice.clear();
                }),
            ));
        });
}

fn update_notice_banner(
    notice: Res<Notice>,
    mut banner_q: Query<(&mut Node, &mut BackgroundColor), With<NoticeBanner>>,
    mut text_q: Query<&mut Text, With<NoticeBannerText>>,
) {
    let Ok((mut node, mut bg)) = banner_q.single_mut() else {
        return;
    };
    let Ok(mut text) = text_q.single_mut() else {
        return;
    };
    match notice.0.as_ref() {
        Some(entry) => {
            if node.display != Display::Flex {
                node.display = Display::Flex;
            }
            let color = match entry.level {
                NoticeLevel::Error => Color::srgba(0.55, 0.10, 0.10, 0.95),
                NoticeLevel::Warn => Color::srgba(0.55, 0.40, 0.05, 0.95),
            };
            if bg.0 != color {
                bg.0 = color;
            }
            if text.0 != entry.message {
                text.0 = entry.message.clone();
            }
        }
        None => {
            if node.display != Display::None {
                node.display = Display::None;
            }
        }
    }
}

// ── Browser (main menu: settings + buttons + lobby list) ────────────────

fn spawn_browser(mut commands: Commands, mut rev: ResMut<BrowserListRev>) {
    rev.0 = u64::MAX; // force the next update_browser pass to populate the list
    commands.spawn((
        BrowserUi,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            // Column so `justify_content` works on the vertical axis and
            // `align_items` on the horizontal axis. Top-aligned (FlexStart)
            // because with the settings inlined the column can overflow
            // short viewports; center-aligned would push the heading off
            // the top of the screen.
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexStart,
            padding: UiRect::top(Val::Px(40.0)),
            overflow: Overflow::scroll_y(),
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
                min_width: Val::Px(440.0),
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
                // Lobby-WS connectivity indicator. Status text + color
                // driven by `update_connectivity_pill`.
                (
                    ConnectivityPill,
                    Text::new("○ Connecting to lobby server…"),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)),
                ),
                // The same settings groups the Creating page used to host;
                // they mutate the live GameSettings resource, which both
                // Solo Play and Create Lobby read at launch time.
                section_label("Board size"),
                board_size_group(),
                section_label("Apples"),
                apple_count_group(),
                section_label("Speed"),
                speed_group(),
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
            (
                button(
                    ButtonProps::default(),
                    (),
                    Spawn((Text::new("Create lobby"), ThemedText)),
                ),
                // Skips straight to creating a lobby with the live settings;
                // the LobbyCreated server ack flips us into WaitingForOpponent.
                observe(
                    |_: On<Activate>,
                     mut client: NonSendMut<LobbyClient>,
                     settings: Res<GameSettings>| {
                        client.create_lobby(*settings);
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
                        current.role = Role::Joiner;
                        next.set(ClientState::WaitingForOpponent);
                    },
                ),
            );
            p.spawn(btn);
        });
    }
}

fn update_connectivity_pill(
    status: Res<NetStatus>,
    mut q: Query<(&mut Text, &mut TextColor), With<ConnectivityPill>>,
) {
    let Ok((mut text, mut color)) = q.single_mut() else {
        return;
    };
    let (msg, c) = match status.stage {
        ConnectStage::LobbyConnected => (
            "● Connected to lobby server",
            Color::srgba(0.4, 0.8, 0.4, 0.85),
        ),
        ConnectStage::LobbyConnecting => (
            "○ Connecting to lobby server…",
            Color::srgba(1.0, 1.0, 1.0, 0.6),
        ),
        // While in Browsing the netcode stages shouldn't appear, but if
        // they do (e.g. mid-bounce) treat them as "not ready to host".
        _ => ("× Lobby server unreachable", Color::srgba(0.9, 0.4, 0.4, 0.9)),
    };
    if text.0 != msg {
        text.0 = msg.into();
    }
    if color.0 != c {
        color.0 = c;
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
        LobbyState::InProgress => "In progress",
        LobbyState::Finished => "Finished",
    }
}

// ── Settings groups (radio rows for board / apples / speed) ─────────────

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
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(12.0),
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

/// Fires once per group after the Browsing settings panel is spawned:
/// marks the radio matching the current `GameSettings` as `Checked`. Uses
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
    let solo = current.role == Role::Solo;
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
            // Solo flashes through this state for a single frame on its
            // way from WaitingForOpponent → Playing, so the Start/Leave
            // row would just be a misleading flicker. Skip it entirely.
            if !solo {
                p.spawn((
                    Node {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        margin: UiRect::top(Val::Px(16.0)),
                        ..default()
                    },
                    children![
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
            }
                });
        });
}

fn update_waiting(
    current: Res<CurrentLobby>,
    list: Res<LobbyList>,
    status: Res<NetStatus>,
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
        // Two layered facts: the netcode-stage status (precise — opening
        // matchbox / syncing / ...) and the role-based message (host vs
        // joiner). When the stage is AwaitingRoster the role-based
        // message is more useful; otherwise the stage detail wins.
        t.0 = match (&status.stage, current.role) {
            (ConnectStage::ConnectingToGameServer, _) => "Connecting to game server…".into(),
            (_, Role::Host) => {
                if count < 2 {
                    "Hosting — waiting for someone to join".into()
                } else {
                    "Hosting — click Start when ready".into()
                }
            }
            (_, Role::Joiner) => "Waiting for host to start…".into(),
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

// ── Playing: board + side panel ─────────────────────────────────────────

fn spawn_score_hud(
    mut commands: Commands,
    board_target: Res<BoardRenderTarget>,
    phase: Res<InterpolationPhase>,
) {
    let initial_phase = phase.0;
    commands
        .spawn((
            ScoreHudUi,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                ..default()
            },
            TabGroup::default(),
        ))
        .with_children(|root| {
            // Left: the board, displayed from the off-screen texture. Fills
            // remaining horizontal space; `BoardImageNode` is the marker
            // `resize_board_texture` looks for. Wrap in a positioning
            // container so the Game Over banner can overlay it without
            // displacing layout.
            root.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    position_type: PositionType::Relative,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
            ))
            .with_children(|p| {
                p.spawn((
                    BoardImageNode,
                    ImageNode::new(board_target.handle.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        right: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        ..default()
                    },
                ));
                p.spawn((
                    GameOverBanner,
                    Node {
                        // Toggled to Display::Flex by update_game_over_banner
                        // when snakes empty. Use Display rather than
                        // Visibility because Visibility::Hidden seems to
                        // leave the inner Text in an unmeasured state, so
                        // it ends up rendering as an empty bordered box.
                        // Display::None fully removes from layout; flipping
                        // to Flex triggers a fresh layout pass that
                        // measures the text.
                        display: Display::None,
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(24.0), Val::Px(16.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
                    BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.3)),
                ))
                .with_children(|p| {
                    p.spawn((
                        Text::new("Game over"),
                        ThemedText,
                        TextFont {
                            font_size: 42.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
            });

            // Right: control panel.
            root.spawn((
                Node {
                    width: Val::Px(260.0),
                    height: Val::Percent(100.0),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(12.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    ..default()
                },
                ThemeBackgroundColor(tokens::WINDOW_BG),
            ))
            .with_children(|p| {
                p.spawn((
                    ScoreText,
                    Text::new(""),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
                p.spawn(section_label("Feel"));
                p.spawn(section_label("Lean crossover"));
                // Feathers' `slider` and `button` both set `flex_grow: 1.0`
                // on their root Node, which means in this Column container
                // they'd each consume an equal slice of the panel's height.
                // Wrap each in a default Node so the *wrapper* sits at
                // flex_grow: 0 and the widget can still grow within it
                // (filling the wrapper's width, no extra vertical space).
                p.spawn(Node::default()).with_children(|p| {
                    p.spawn((
                        slider(
                            SliderProps {
                                value: initial_phase,
                                min: 0.05,
                                max: 0.95,
                            },
                            // Feathers' `update_slider_pos` query requires
                            // `&SliderPrecision` (not Optional), so without
                            // this the slider's text + gradient never update
                            // and the slider looks broken even though
                            // dragging is actually changing the value.
                            SliderPrecision(2),
                        ),
                        observe(
                            |change: On<ValueChange<f32>>,
                             mut phase: ResMut<InterpolationPhase>,
                             mut commands: Commands| {
                                // bevy_ui_widgets sliders fire ValueChange
                                // but DON'T write back SliderValue — that's
                                // delegated to whoever handles the event.
                                // Without this insert the slider visual is
                                // frozen at the initial value forever.
                                phase.0 = change.value;
                                commands
                                    .entity(change.source)
                                    .insert(SliderValue(change.value));
                            },
                        ),
                    ));
                });
                p.spawn(Node::default()).with_children(|p| {
                    p.spawn((
                        button(
                            ButtonProps {
                                variant: ButtonVariant::Primary,
                                ..default()
                            },
                            (),
                            Spawn((Text::new("Restart"), ThemedText)),
                        ),
                        observe(|_: On<Activate>, mut pending: ResMut<PendingInput>| {
                            // Same path as the Space key. Works whether or
                            // not snakes are currently alive — GGRS rolls
                            // forward the restart bit on the next frame.
                            pending.restart = true;
                        }),
                    ));
                });
                p.spawn(Node::default()).with_children(|p| {
                    p.spawn((
                        button(
                            ButtonProps::default(),
                            (),
                            Spawn((Text::new("Back to lobby"), ThemedText)),
                        ),
                        observe(
                            |_: On<Activate>,
                             mut client: NonSendMut<LobbyClient>,
                             mut current: ResMut<CurrentLobby>,
                             mut next: ResMut<NextState<ClientState>>| {
                                leave_to_browser(&mut client, &mut current, &mut next);
                            },
                        ),
                    ));
                });
            });
        });
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
    text.0 = lines.join("\n");
}

/// Show the Game Over banner iff no snakes are left on the board. The
/// session stays alive throughout — clicking Restart (or pressing Space)
/// hides the banner again once `apply_restart` re-seeds the board.
fn update_game_over_banner(
    board: Res<Board>,
    mut banner: Query<&mut Node, With<GameOverBanner>>,
) {
    let Ok(mut node) = banner.single_mut() else {
        return;
    };
    let want = if board.snakes().is_empty() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != want {
        node.display = want;
    }
}

/// Shared "exit the active game and return to the lobby browser" flow. Used
/// by the in-game Back-to-Lobby button and by the Finished modal.
fn leave_to_browser(
    client: &mut LobbyClient,
    current: &mut CurrentLobby,
    next: &mut NextState<ClientState>,
) {
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
}

// ── Finished ────────────────────────────────────────────────────────────

/// Finished is only reached when the GGRS session is torn down externally —
/// in practice, a multiplayer peer disconnect. Solo never enters Finished
/// (the session stays alive across death and the in-Playing Game Over
/// banner handles the visual). So the only useful action here is going
/// back to the lobby browser; restart isn't possible because the session
/// is gone.
fn spawn_finished(mut commands: Commands, notice: Res<Notice>) {
    // Capture the disconnect reason from Notice at entry — it's the
    // most-recent error and almost always describes *why* we ended up
    // here (peer disconnect, channel closed, etc.). Snapshot rather than
    // poll, because the user may dismiss the banner.
    let subtitle = notice
        .0
        .as_ref()
        .filter(|n| matches!(n.level, NoticeLevel::Error))
        .map(|n| n.message.clone());
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
            p.spawn((Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(16.0),
                ..default()
            },))
            .with_children(|p| {
                p.spawn((
                    Text::new("Game over"),
                    TextFont {
                        font_size: 36.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
                if let Some(sub) = subtitle {
                    p.spawn((
                        FinishedSubtitle,
                        Text::new(sub),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::srgba(1.0, 0.7, 0.7, 0.85)),
                    ));
                }
                p.spawn((
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
                            leave_to_browser(&mut client, &mut current, &mut next);
                        },
                    ),
                ));
            });
        });
}

