use bevy::prelude::*;
use bevy_ggrs::Session;
use lobby::CurrentLobby;
use net::GameConfig;

mod lobby;
mod net;
mod render;
mod ui;

#[derive(States, Default, Debug, Hash, PartialEq, Eq, Clone)]
pub enum ClientState {
    /// Main menu: game settings + Solo Play / Create Lobby buttons + list
    /// of open lobbies. The settings the user sees are the live
    /// `GameSettings` resource, applied directly to whichever path they
    /// choose (solo, hosting a lobby, or — overwritten — joining one).
    #[default]
    Browsing,
    /// Either waiting in a lobby (host or joiner) for the Start broadcast,
    /// or, post-Start, waiting on `wait_for_players` to finish the
    /// matchbox/GGRS dance.
    WaitingForOpponent,
    Playing,
    /// Game over screen, reached on multiplayer session loss (peer
    /// disconnect). Solo never gets here — death stays in Playing and the
    /// in-game side panel handles restart / back-to-menu.
    Finished,
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Snake, WITH GUNS!".to_string(),
                    canvas: Some("#bevy".to_string()),
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            }),
            lobby::LobbyPlugin,
            net::NetPlugin,
            render::BoardRenderPlugin,
            ui::UiPlugin,
        ))
        .init_state::<ClientState>()
        .insert_resource(ClearColor(Color::srgb(0.1, 0.1, 0.1)))
        .add_systems(Update, (drive_state, exit_on_cmd_w))
        .run();
}

/// Owns the cross-state transitions that depend on resources outside any
/// single plugin's purview. The lobby plugin and `wait_for_players` mutate
/// state directly via `NextState`; this system handles the leftover edges:
/// the GGRS session presence ↔ `Playing` correspondence.
///
/// All-snakes-dead does NOT transition to Finished. We stay in Playing so
/// the session stays alive — the in-session restart path
/// (`PendingInput.restart`, same as the Space key) is rolled forward by
/// GGRS deterministically. The UI shows a "Game over" banner while snakes
/// are empty.
fn drive_state(
    session: Option<Res<Session<GameConfig>>>,
    state: Res<State<ClientState>>,
    mut next: ResMut<NextState<ClientState>>,
    current: Res<CurrentLobby>,
) {
    match (state.get(), session.is_some()) {
        (ClientState::WaitingForOpponent, true) => {
            next.set(ClientState::Playing);
        }
        (ClientState::Playing, false) => {
            // Session was torn down by something external (peer disconnect,
            // explicit teardown). Solo never reaches this branch.
            let dest = if current.id.is_some() {
                ClientState::Finished
            } else {
                ClientState::Browsing
            };
            next.set(dest);
        }
        _ => {}
    }
}

fn exit_on_cmd_w(mut exit: MessageWriter<AppExit>, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyW)
        && keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight])
    {
        exit.write(AppExit::Success);
    }
}
