use crate::{
    board::{Board, BoardSettings, Direction},
    GameCommands, GameUpdates,
};
use actix_web::{
    middleware::Logger,
    web::{self, Data, Payload},
    App, HttpRequest, HttpResponse, HttpServer,
};
use actix_ws::Message;
use futures::future::{pending, select_all};
use log::{debug, error, info, warn};
use rand::{rngs::StdRng, SeedableRng};
use std::{collections::HashMap, net::ToSocketAddrs, time::Duration};
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    time::{interval, Instant},
};

#[tokio::main]
pub async fn start_server<A: ToSocketAddrs + Send + 'static>(ip: A) {
    let (client_tx, client_rx) = channel(1);

    // start the web server
    let web = tokio::spawn(async {
        // build our application with a route
        HttpServer::new(move || {
            App::new()
                .wrap(Logger::default())
                .service(web::resource("/").to(|| async { "Hello world!" }))
                .service(web::resource("/board").to(board))
                .service(web::resource("/ws").to(snake_ws))
                .app_data(Data::new(client_tx.clone()))
        })
        .bind(ip)
        .unwrap()
        .run()
        .await
        .unwrap();
    });

    // start the game
    let game = tokio::spawn(game_loop(client_rx));

    // exit if either the web server or game loop exits
    tokio::select! {
        _ = web => { error!("web server exited"); }
        _ = game => { error!("game loop exited"); }
    }
}

async fn game_loop(register_client: Receiver<Client>) {
    let mut game_loop = GameLoop::new(register_client).await;
    game_loop.game_loop().await;
}

struct GameLoop {
    clients: Clients,
    register_client: Receiver<Client>,
    queued_inputs: HashMap<usize, Direction>,
    rng: StdRng,
    board: Board,
    tick: u64,
}

impl GameLoop {
    async fn new(register_client: Receiver<Client>) -> Self {
        Self {
            clients: Clients::new(),
            register_client,
            queued_inputs: HashMap::new(),
            rng: StdRng::from_os_rng(),
            board: Board::new(BoardSettings::default()),
            tick: 0,
        }
    }

    async fn game_loop(&mut self) {
        let mut ticker = interval(Duration::from_secs_f32(1.0 / 7.5));
        loop {
            select! {
                // register a new client
                client = self.register_client.recv() => {
                    let Some(client) = client else {
                        error!("register_client channel closed");
                        break;
                    };
                    self.register_client(client).await;
                }
                // process client commands
                (client, command) = self.clients.next_command() => {
                    self.process_command(client, command).await;
                }
                // tick the game board
                _ = ticker.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    async fn register_client(&mut self, client: Client) {
        info!(
            "new client registered at index {}",
            self.clients.clients.len()
        );
        client
            .game_updates
            .send(GameUpdates::Ticked {
                board: self.board.clone(),
                events: Vec::new(),
                tick: self.tick,
            })
            .await
            .unwrap();
        self.clients.push(client);
    }

    async fn process_command(&mut self, client: usize, command: GameCommands) {
        match command {
            GameCommands::Input { direction, tick } => {
                if tick != self.tick {
                    warn!(
                        "client missed game tick; expected {}, got {}",
                        self.tick, tick
                    );
                    return;
                }

                self.queued_inputs.insert(client, direction);
            }
            GameCommands::RestartGame => {
                self.board = Board::new(BoardSettings::default());
                self.clients
                    .broadcast(GameUpdates::Ticked {
                        tick: self.tick,
                        board: self.board.clone(),
                        events: Vec::new(),
                    })
                    .await;
            }
        }
    }

    async fn tick(&mut self) {
        self.tick += 1;

        let mut inputs = [None; 4];
        for (client, direction) in self.queued_inputs.drain() {
            inputs[client] = Some(direction);
        }

        let events = match self.board.tick_board(&inputs, &mut self.rng) {
            Ok(events) => events,
            Err(e) => {
                error!("Board error: {}", e);
                return;
            }
        };

        self.clients
            .broadcast(GameUpdates::Ticked {
                board: self.board.clone(),
                events,
                tick: self.tick,
            })
            .await;

        debug!("ticked board ({}):\n{:?}", self.tick, self.board);
    }
}

struct Clients {
    clients: Vec<Client>,
}

impl Clients {
    fn new() -> Self {
        Self {
            clients: Vec::new(),
        }
    }

    fn push(&mut self, client: Client) {
        self.clients.push(client);
    }

    async fn next_command(&mut self) -> (usize, GameCommands) {
        loop {
            if self.clients.is_empty() {
                // return pending future if there are no clients
                return pending().await;
            }

            let (game_command, index, _) = select_all(
                self.clients
                    .iter_mut()
                    .map(|client| Box::pin(client.game_commands.recv())),
            )
            .await;

            if let Some(game_command) = game_command {
                return (index, game_command);
            }

            self.clients.remove(index);
        }
    }

    async fn broadcast(&mut self, game_update: GameUpdates) {
        let mut delete = Vec::new();
        for (index, client) in self.clients.iter_mut().enumerate() {
            if let Err(e) = client.game_updates.send(game_update.clone()).await {
                error!("{}", e);
                delete.push(index);
            }
        }
        for index in delete.into_iter().rev() {
            self.clients.remove(index);
        }
    }
}

struct Client {
    game_commands: Receiver<GameCommands>,
    game_updates: Sender<GameUpdates>,
}

impl Client {
    fn new() -> (Self, Sender<GameCommands>, Receiver<GameUpdates>) {
        let (game_commands_tx, game_commands_rx) = channel(1);
        let (game_updates_tx, game_updates_rx) = channel(1);

        (
            Self {
                game_commands: game_commands_rx,
                game_updates: game_updates_tx,
            },
            game_commands_tx,
            game_updates_rx,
        )
    }
}

async fn board(board: Data<Mutex<Option<Board>>>) -> HttpResponse {
    HttpResponse::Ok().json(board.lock().await.clone())
}

async fn snake_ws(
    req: HttpRequest,
    stream: Payload,
    client_tx: Data<Sender<Client>>,
) -> Result<HttpResponse, actix_web::Error> {
    let (res, session, msg_stream) = actix_ws::handle(&req, stream)?;

    // spawn websocket handler (and don't await it) so that the response is returned immediately
    actix_web::rt::spawn(snake_ws_handler(session, msg_stream, (**client_tx).clone()));

    Ok(res)
}

async fn snake_ws_handler(
    mut session: actix_ws::Session,
    mut msg_stream: actix_ws::MessageStream,
    client_tx: Sender<Client>,
) {
    info!("web socket connected");

    let mut last_heartbeat = Instant::now();
    let mut interval = interval(Duration::from_secs(5));

    let (client, game_commands, mut game_updates) = Client::new();
    client_tx.send(client).await.unwrap();

    let reason = loop {
        // create "next client timeout check" future
        let tick = interval.tick();

        tokio::select! {
            // received a board update from the game
            update = game_updates.recv() => {
                match update {
                    Some(game_update) => {
                        if let Err(e) = session.text(serde_json::to_string(&game_update).unwrap()).await {
                            error!("{}", e);
                            break None;
                        }
                    }

                    None => {
                        break None;
                    }
                }
            }

            // received message from WebSocket client
            msg = msg_stream.recv() => {
                match msg {
                    Some(Ok(msg)) => match msg {
                        Message::Text(text) => {
                            let command = match serde_json::from_str::<GameCommands>(&text) {
                                Ok(input) => input,
                                Err(err) => {
                                    session.text(format!("invalid input: {}", err)).await.unwrap();
                                    error!("{}", err);
                                    break None;
                                }
                            };

                            if let Err(e) = game_commands.send(command).await {
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
