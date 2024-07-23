use crate::board::{Board, BoardSettings, Direction};
use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use bevy::prelude::*;
use crossbeam::channel::{self, Receiver, Sender};
use lazy_static::lazy_static;
use serde::Deserialize;
use std::sync::Mutex as StdMutex;
use tokio::{runtime::Runtime, sync::Mutex};

pub struct WebPlugin;

impl Plugin for WebPlugin {
    fn build(&self, app: &mut App) {
        let (web_commands_tx, web_commands_rx) = channel::unbounded();
        let (web_updates_tx, web_updates_rx) = channel::unbounded();
        start_web(web_commands_tx, web_updates_rx);

        app.insert_resource(WebResources {
            web_commands: web_commands_rx,
            web_updates: web_updates_tx,
        });
    }
}

#[derive(Resource)]
pub struct WebResources {
    pub web_commands: Receiver<WebCommands>,
    pub web_updates: Sender<WebUpdates>,
}

pub enum WebCommands {
    SendInput { direction: Direction },
}

pub enum WebUpdates {
    UpdateBoard { board: Board },
}

lazy_static! {
    static ref RUNTIME: StdMutex<Runtime> = StdMutex::new(Runtime::new().unwrap());
    static ref APP_STATE: Mutex<Option<AppState>> = Mutex::new(None);
}

struct AppState {
    board: Board,
    web_commands: Sender<WebCommands>,
    web_updates: Receiver<WebUpdates>,
}

fn start_web(web_commands: Sender<WebCommands>, web_updates: Receiver<WebUpdates>) {
    let rt = RUNTIME.lock().unwrap();
    rt.spawn(async {
        // store our application state in a mutex
        {
            let mut app_state = APP_STATE.lock().await;
            *app_state = Some(AppState {
                board: Board::new(BoardSettings::default()),
                web_commands,
                web_updates,
            });
        }

        // build our application with a route
        let app = Router::new()
            .route("/", get(home))
            .route("/board", get(board))
            .route("/input", post(send_input));

        // run our app with hyper, listening globally on port 1234
        let listener = tokio::net::TcpListener::bind("0.0.0.0:1234").await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });
}

async fn home() -> &'static str {
    "Hello, World!"
}

async fn board() -> (StatusCode, Json<Board>) {
    let mut app_state = APP_STATE.lock().await;
    let app_state = app_state.as_mut().unwrap();

    while let Some(WebUpdates::UpdateBoard { board }) = app_state.web_updates.try_recv().ok() {
        app_state.board = board;
    }

    (StatusCode::OK, Json(app_state.board.clone()))
}

async fn send_input(Json(input): Json<Input>) -> StatusCode {
    info!("Received input: {:?}", input);

    let direction = match input.direction.try_into() {
        Ok(direction) => direction,
        Err(e) => {
            error!("Failed to convert input to direction: {:?}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    let mut app_state = APP_STATE.lock().await;
    let app_state = app_state.as_mut().unwrap();

    app_state
        .web_commands
        .send(WebCommands::SendInput { direction })
        .unwrap();

    StatusCode::OK
}

// the input to our `create_user` handler
#[derive(Deserialize, Debug)]
struct Input {
    direction: Direction,
}
