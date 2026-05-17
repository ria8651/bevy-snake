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
    /// Lobby browser: list of open lobbies + Create / Solo buttons.
    #[default]
    Browsing,
    /// Settings panel + Host button. We sit here until the server acks
    /// `CreateLobby` with a `LobbyCreated` message.
    Creating,
    /// Either waiting in a lobby (host or joiner) for the Start broadcast,
    /// or, post-Start, waiting on `wait_for_players` to finish the
    /// matchbox/GGRS dance.
    WaitingForOpponent,
    Playing,
    /// Game over screen with "Back to lobbies".
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
///   - GGRS session presence ↔ `Playing`
///   - `Playing` → `Finished` when all snakes are dead and no restart
fn drive_state(
    session: Option<Res<Session<GameConfig>>>,
    state: Res<State<ClientState>>,
    mut next: ResMut<NextState<ClientState>>,
    current: Res<CurrentLobby>,
    board: Res<bevy_snake::board::Board>,
    time: Res<Time<Real>>,
    mut end_acc: Local<f32>,
) {
    match (state.get(), session.is_some()) {
        (ClientState::WaitingForOpponent, true) => {
            next.set(ClientState::Playing);
            *end_acc = 0.0;
        }
        (ClientState::Playing, false) => {
            // Session was torn down (peer disconnect, restart, etc).
            // Solo never loses the session here, so this is always a net
            // game ending.
            let dest = if current.id.is_some() {
                ClientState::Finished
            } else {
                ClientState::Browsing
            };
            next.set(dest);
            *end_acc = 0.0;
        }
        (ClientState::Playing, true) => {
            // If everyone is dead and no one has restarted within a couple
            // seconds, the game is effectively over.
            if board.snakes().is_empty() {
                *end_acc += time.delta_secs();
                if *end_acc > 3.0 {
                    next.set(ClientState::Finished);
                    *end_acc = 0.0;
                }
            } else {
                *end_acc = 0.0;
            }
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
