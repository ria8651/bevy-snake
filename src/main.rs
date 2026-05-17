use bevy::prelude::*;
use bevy_ggrs::Session;
use net::GameConfig;

mod net;
mod render;
mod ui;

#[derive(States, Default, Debug, Hash, PartialEq, Eq, Clone)]
pub enum ClientState {
    #[default]
    WaitingForOpponent,
    Playing,
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
            net::NetPlugin,
            render::BoardRenderPlugin,
            ui::UiPlugin,
        ))
        .init_state::<ClientState>()
        .insert_resource(ClearColor(Color::srgb(0.1, 0.1, 0.1)))
        .add_systems(Update, (drive_state, exit_on_cmd_w))
        .run();
}

fn drive_state(
    session: Option<Res<Session<GameConfig>>>,
    state: Res<State<ClientState>>,
    mut next: ResMut<NextState<ClientState>>,
) {
    let desired = if session.is_some() {
        ClientState::Playing
    } else {
        ClientState::WaitingForOpponent
    };
    if *state.get() != desired {
        next.set(desired);
    }
}

fn exit_on_cmd_w(mut exit: EventWriter<AppExit>, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyW)
        && keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight])
    {
        exit.send(AppExit::Success);
    }
}
