use bevy::prelude::*;
use bevy_snake::{
    board::{Board, BoardSettings},
    server::start_server,
};

mod client;
mod game;
mod render;
mod ui;

#[derive(States, Default, Debug, Hash, PartialEq, Eq, Clone)]
pub enum GameState {
    InGame,
    #[default]
    GameOver,
}

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

#[derive(Resource, Default)]
pub struct GameTime(f32);
#[derive(Component, Deref, DerefMut)]
pub struct AnimationTimer(Timer);

fn main() {
    // start server
    std::thread::spawn(|| start_server(("0.0.0.0", 1234)));

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
        .insert_resource(GameTime::default())
        .init_state::<GameState>()
        .add_systems(Update, game_state.after(game::update_game))
        .add_systems(Update, settings_system.run_if(in_state(GameState::InGame)))
        .run();
}

fn game_state(
    mut next_game_state: ResMut<NextState<GameState>>,
    mut exit: EventWriter<AppExit>,
    game_state: Res<State<GameState>>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    board: Res<Board>,
) {
    match game_state.get() {
        GameState::InGame => {
            let snakes = board.count_snakes();
            if snakes <= (settings.board_settings.players as usize != 1) as usize {
                next_game_state.set(GameState::GameOver);
            }
        }
        GameState::GameOver => {
            if keys.just_pressed(KeyCode::Space) {
                next_game_state.set(GameState::InGame);
            }
        }
    }

    if keys.just_pressed(KeyCode::KeyW)
        && keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight])
    {
        exit.send(AppExit::Success);
    }
}

fn settings_system(
    mut settings: ResMut<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mut game_time: ResMut<GameTime>,
    time: Res<Time>,
) {
    if keys.just_pressed(KeyCode::KeyI) {
        settings.interpolation = !settings.interpolation;
    }

    game_time.0 += time.delta_secs();
    if settings.tps_ramp {
        settings.tps = (game_time.0 * 0.1 + 5.0).clamp(5.0, 7.0);
    }
}
