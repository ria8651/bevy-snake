use bevy::prelude::*;
use bevy_snake::board::BoardSettings;

mod client;
mod game;
mod render;
mod ui;

// #[derive(States, Default, Debug, Hash, PartialEq, Eq, Clone)]
// pub enum GameState {
//     #[default]
//     InGame,
//     GameOver,
// }

#[derive(PartialEq, Eq)]
pub enum Speed {
    Slow,
    Medium,
    Fast,
}

#[derive(PartialEq, Eq, Reflect)]
pub enum GizmoSetting {
    None,
    CycleBasis,
    TreeSearch,
}

#[derive(Resource, Reflect)]
pub struct Settings {
    pub interpolation: bool,
    pub do_game_tick: bool,
    pub tps: f32,
    pub tps_ramp: bool,
    pub board_settings: BoardSettings,
    pub ai: bool,
    pub gizmos: GizmoSetting,
    pub walls: bool,
    pub walls_debug: bool,
}

#[derive(Component, Deref, DerefMut)]
pub struct AnimationTimer(Timer);

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
            client::ClientPlugin,
            game::GamePlugin,
            game::AIPlugin,
            render::BoardRenderPlugin,
            ui::UiPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.1, 0.1, 0.1)))
        .insert_resource(Settings {
            interpolation: true,
            do_game_tick: true,
            tps: 7.5,
            tps_ramp: false,
            board_settings: BoardSettings::default(),
            ai: true,
            gizmos: GizmoSetting::None,
            walls: false,
            walls_debug: false,
        })
        .add_systems(Startup, start_server)
        .add_systems(Update, game_state.after(game::update_game))
        .run();
}

fn start_server() {
    // #[cfg(not(target_arch = "wasm32"))]
    // std::thread::spawn(|| bevy_snake::server::start_server("127.0.0.1:1234"));
}

fn game_state(mut exit: EventWriter<AppExit>, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyW)
        && keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight])
    {
        exit.send(AppExit::Success);
    }
}
