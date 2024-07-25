use crate::board::{Board, Direction};
use actix_web::{
    middleware,
    web::{self, Data},
    App as ActixApp, HttpRequest, HttpResponse, HttpServer,
};
use actix_ws::Message;
use bevy::prelude::*;
use futures_util::StreamExt;
use lazy_static::lazy_static;
use serde::Deserialize;
use std::{
    sync::Mutex as StdMutex,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::{
        broadcast::{self, error::TryRecvError, Receiver, Sender},
        Mutex,
    },
    time::interval,
};

pub struct WebPlugin;

impl Plugin for WebPlugin {
    fn build(&self, app: &mut App) {
        let (web_commands_tx, web_commands_rx) = broadcast::channel(16);
        let (web_updates_tx, _) = broadcast::channel(1);
        start_web(web_commands_tx, web_updates_tx.clone());

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

#[derive(Clone, Debug)]
pub enum WebCommands {
    SendInput { direction: Direction },
}

#[derive(Clone, Debug)]
pub enum WebUpdates {
    UpdateBoard { board: Board },
}

lazy_static! {
    static ref RUNTIME: StdMutex<Runtime> = StdMutex::new(Runtime::new().unwrap());
}

fn start_web(web_commands: Sender<WebCommands>, web_updates: Sender<WebUpdates>) {
    let rt = RUNTIME.lock().unwrap();
    rt.spawn(async {
        // build our application with a route
        HttpServer::new(move || {
            ActixApp::new()
                .wrap(middleware::Logger::default())
                .service(web::resource("/").to(|| async { "Hello world!" }))
                .service(web::resource("/board").to(board))
                .service(web::resource("/input").to(send_input))
                .service(web::resource("/ws").to(snake_ws))
                .app_data(Data::new(web_commands.clone()))
                .app_data(Data::new(web_updates.clone()))
                .app_data(Data::new(Mutex::new(web_updates.subscribe())))
                .app_data(Data::new(Mutex::new(None::<Board>)))
        })
        .bind(("0.0.0.0", 1234))
        .unwrap()
        .run()
        .await
        .unwrap();
    });
}

async fn board(
    web_updates: Data<Mutex<Receiver<WebUpdates>>>,
    board: Data<Mutex<Option<Board>>>,
) -> HttpResponse {
    let mut web_updates = web_updates.lock().await;
    loop {
        match web_updates.try_recv() {
            Ok(WebUpdates::UpdateBoard { board: new_board }) => {
                *board.lock().await = Some(new_board);
            }
            Err(TryRecvError::Lagged(i)) => {
                warn!("{} updates missed", i);
            }
            Err(TryRecvError::Closed) => {
                error!("web_updates channel closed");
                break;
            }
            Err(TryRecvError::Empty) => {
                break;
            }
        }
    }

    HttpResponse::Ok().json(board.lock().await.clone())
}

#[derive(Deserialize, Debug)]
struct Input {
    direction: Direction,
}

async fn send_input(
    input: web::Json<Input>,
    web_commands: Data<Sender<WebCommands>>,
) -> HttpResponse {
    web_commands
        .send(WebCommands::SendInput {
            direction: input.direction,
        })
        .unwrap();

    HttpResponse::Ok().finish()
}

async fn snake_ws(
    req: HttpRequest,
    stream: web::Payload,
    web_updates: Data<Sender<WebUpdates>>,
    web_commands: Data<Sender<WebCommands>>,
) -> Result<HttpResponse, actix_web::Error> {
    let (res, session, msg_stream) = actix_ws::handle(&req, stream)?;

    // spawn websocket handler (and don't await it) so that the response is returned immediately
    actix_web::rt::spawn(snake_ws_handler(
        session,
        msg_stream,
        web_updates.subscribe(),
        web_commands.as_ref().clone(),
    ));

    Ok(res)
}

pub async fn snake_ws_handler(
    mut session: actix_ws::Session,
    mut msg_stream: actix_ws::MessageStream,
    mut web_updates: Receiver<WebUpdates>,
    web_commands: Sender<WebCommands>,
) {
    info!("web socket connected");

    let mut last_heartbeat = Instant::now();
    let mut interval = interval(Duration::from_secs(5));

    let reason = loop {
        // create "next client timeout check" future
        let tick = interval.tick();
        let board_update = web_updates.recv();

        tokio::select! {
            // received a board update from the game
            update = board_update => {
                match update {
                    Ok(WebUpdates::UpdateBoard { board }) => {
                        if let Err(e) = session.text(serde_json::to_string(&board).unwrap()).await {
                            error!("{}", e);
                            break None;
                        }
                    }

                    Err(err) => {
                        error!("{}", err);
                        break None;
                    }
                }
            }

            // received message from WebSocket client
            msg = msg_stream.next() => {
                match msg {
                    Some(Ok(msg)) => match msg {
                        Message::Text(text) => {
                            let input = match serde_json::from_str::<Input>(&text) {
                                Ok(input) => input,
                                Err(err) => {
                                    session.text(format!("invalid input: {}", err)).await.unwrap();
                                    error!("{}", err);
                                    break None;
                                }
                            };

                            if let Err(e) = web_commands.send(WebCommands::SendInput { direction: input.direction }) {
                                error!("{}", e);
                                break None;
                            }
                        }

                        Message::Binary(_) => {
                            session.text("i dont want your binary data").await.unwrap();
                        }

                        Message::Close(reason) => {
                            break reason;
                        }

                        Message::Ping(bytes) => {
                            last_heartbeat = Instant::now();
                            session.pong(&bytes).await.ok();
                        }

                        Message::Pong(_) => {
                            last_heartbeat = Instant::now();
                        }

                        Message::Continuation(_) => {
                            warn!("no support for continuation frames");
                        }

                        // no-op; ignore
                        Message::Nop => {}
                    }

                    Some(Err(err)) => {
                        error!("{}", err);
                        break None;
                    }

                    None => break None,
                }
            }

            // heartbeat interval ticked
            _ = tick => {
                // if no heartbeat ping/pong received recently, close the connection
                if Instant::now().duration_since(last_heartbeat) > Duration::from_secs(10) {
                    info!("client has not sent heartbeat in over 10s; disconnecting");

                    break None;
                }

                // send heartbeat ping
                let _ = session.ping(b"").await;
            }
        }
    };

    // attempt to close connection gracefully
    let _ = session.close(reason).await;

    info!("disconnected");
}
