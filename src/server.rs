use crate::{
    board::{Board, BoardEvent, BoardSettings, Direction},
    GameCommands, GameUpdates,
};
use axum::{response::Html, routing::get, Router};
use futures::future::{pending, select_all};
use log::*;
use rand::{rngs::StdRng, SeedableRng};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::Digest;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    select,
    sync::mpsc::{channel, Receiver, Sender},
    time::interval,
};
use tower_http::services::ServeDir;

pub struct ServerConfig {
    pub wt_addr: SocketAddr,
    pub http_addr: SocketAddr,
    pub wt_url: String,
    pub cert_sans: Vec<String>,
}

#[tokio::main]
pub async fn start_server(config: ServerConfig) {
    let (cert_chain, key, cert_hash) = generate_ephemeral_cert(config.cert_sans.clone());
    info!("generated ephemeral cert (sha256: {})", hex::encode(cert_hash));

    let (client_tx, client_rx) = channel(1);

    // start the game
    let game = tokio::spawn(game_loop(client_rx));

    // start the web transport server
    let wt = tokio::spawn(web_transport(
        config.wt_addr,
        cert_chain,
        key,
        client_tx,
    ));

    // start the HTTP server (static files + cert hash injection)
    let http = tokio::spawn(http_server(
        config.http_addr,
        cert_hash,
        config.wt_url,
    ));

    // exit if any task exits
    tokio::select! {
        _ = wt => { error!("web transport server exited"); }
        _ = http => { error!("http server exited"); }
        _ = game => { error!("game loop exited"); }
    }
}

fn generate_ephemeral_cert(
    sans: Vec<String>,
) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>, [u8; 32]) {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .expect("failed to generate key pair");

    let mut params = rcgen::CertificateParams::new(sans).expect("invalid SANs");
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::hours(24);

    let cert = params
        .self_signed(&key_pair)
        .expect("failed to self-sign cert");

    let der: Vec<u8> = cert.der().to_vec();
    let hash: [u8; 32] = sha2::Sha256::digest(&der).into();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    (vec![CertificateDer::from(der)], key, hash)
}

async fn http_server(addr: SocketAddr, cert_hash: [u8; 32], wt_url: String) {
    let state = Arc::new(HttpState {
        cert_hash_hex: hex::encode(cert_hash),
        wt_url,
    });

    let app = Router::new()
        .route("/", get(serve_index))
        .fallback_service(ServeDir::new("web"))
        .with_state(state);

    let listener = TcpListener::bind(addr).await.expect("bind http listener");
    info!("http server listening on {}", addr);
    if let Err(e) = axum::serve(listener, app).await {
        error!("http server error: {}", e);
    }
}

struct HttpState {
    cert_hash_hex: String,
    wt_url: String,
}

async fn serve_index(
    axum::extract::State(state): axum::extract::State<Arc<HttpState>>,
) -> Html<String> {
    let html = match tokio::fs::read_to_string("web/index.html").await {
        Ok(s) => s,
        Err(e) => {
            error!("failed to read web/index.html: {}", e);
            return Html(format!("<h1>web/index.html not found: {}</h1>", e));
        }
    };
    Html(
        html.replace("{{CERT_HASH}}", &state.cert_hash_hex)
            .replace("{{WT_URL}}", &state.wt_url),
    )
}

async fn web_transport(
    addr: SocketAddr,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    client_tx: Sender<Client>,
) {
    // create the web transport server
    let mut server = web_transport_quinn::ServerBuilder::new()
        .with_addr(addr)
        .with_certificate(chain, key)
        .unwrap();

    info!("web transport server listening on {}", addr);

    // accept incoming connections
    while let Some(conn) = server.accept().await {
        info!("accepted connection to {}", conn.url());

        let session = match conn.ok().await {
            Ok(session) => session,
            Err(e) => {
                error!("failed to accept connection: {}", e);
                continue;
            }
        };

        info!("started session");

        let client_tx = client_tx.clone();
        tokio::spawn(async move {
            // create a new client
            let (client, game_commands, mut game_updates) = Client::new();
            client_tx.send(client).await.unwrap();

            loop {
                tokio::select! {
                    // send game updates to the client
                    Some(msg) = game_updates.recv() => {
                        match session.open_uni().await {
                            Ok(mut send) => {
                                let msg = serde_json::to_string(&msg).unwrap();
                                send.write_all(msg.as_bytes()).await.unwrap();
                                trace!("sent: {}", msg);
                            }
                            Err(e) => {
                                error!("failed to open uni stream: {}", e);
                                break;
                            }
                        }
                    }
                    // receive commands from the client
                    Ok(mut recv) = session.accept_uni() => {
                        let mut buf = String::new();
                        recv.read_to_string(&mut buf).await.unwrap();
                        trace!("received: {}", buf);
                        let command = serde_json::from_str(&buf).unwrap();
                        game_commands.send(command).await.unwrap();
                    }
                    // if the session is closed, exit the loop
                    else => {
                        info!("session closed");

                        break;
                    }
                }
            }
        });

        // // test sending a bidirectional stream
        // let (mut send, mut recv) = session.accept_bi().await.unwrap();
        // info!("accepted bidirectional stream");
        // let buf = recv.read_to_end(1024).await.unwrap();
        // info!("received: {}", String::from_utf8_lossy(&buf));
        // send.write_all(b"bi hello").await.unwrap();
        // info!("send: bi hello");

        // // test receiving a bidirectional stream
        // let (mut send, mut recv) = session.open_bi().await.unwrap();
        // info!("opened bidirectional stream");
        // send.write_all(b"other hello").await.unwrap();
        // send.finish().unwrap();
        // info!("sent: other hello");
        // let buf = recv.read_to_end(1024).await.unwrap();
        // info!("received: {}", String::from_utf8_lossy(&buf));
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
                    if self.process_command(client, command).await {
                        ticker.reset_immediately();
                    }
                }
                // tick the game board
                _ = ticker.tick() => {
                    if self.tick().await {
                        // ticker.reset_after(Duration::from_secs(1000000));
                    }
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
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            })
            .await
            .unwrap();
        self.clients.push(client);
    }

    async fn process_command(&mut self, client: usize, command: GameCommands) -> bool {
        match command {
            GameCommands::Input {
                direction,
                tick,
                timestamp,
            } => {
                if tick != self.tick {
                    warn!(
                        "client missed game tick; expected {}, got {}",
                        self.tick, tick
                    );
                    // return;
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                info!(
                    "client {} input: {:?} ({}) ({}ms ping)",
                    client,
                    direction,
                    self.tick,
                    now - timestamp
                );

                self.queued_inputs.insert(client, direction);

                false
            }
            GameCommands::RestartGame { board_settings } => {
                info!("restarting game");

                self.board = Board::new(board_settings);
                self.clients
                    .broadcast(GameUpdates::Ticked {
                        tick: self.tick,
                        board: self.board.clone(),
                        events: Vec::new(),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    })
                    .await;

                true
            }
        }
    }

    async fn tick(&mut self) -> bool {
        self.tick += 1;

        let mut inputs = [None; 16];
        for (client, direction) in self.queued_inputs.drain() {
            inputs[client] = Some(direction);
        }

        let events = match self.board.tick_board(&inputs, &mut self.rng) {
            Ok(events) => events,
            Err(e) => {
                error!("Board error: {}", e);
                return true;
            }
        };

        let exit = events.contains(&BoardEvent::GameOver);

        self.clients
            .broadcast(GameUpdates::Ticked {
                board: self.board.clone(),
                events,
                tick: self.tick,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            })
            .await;

        debug!("ticked board ({}):\n{:?}", self.tick, self.board);

        exit
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

// // start the web server
// let web = tokio::spawn(async {
//     // build our application with a route
//     HttpServer::new(move || {
//         App::new()
//             .wrap(Logger::default())
//             .service(web::resource("/").to(|| async { "Hello world!" }))
//             .service(web::resource("/board").to(board))
//             .service(web::resource("/ws").to(snake_ws))
//             .app_data(Data::new(client_tx.clone()))
//     })
//     .bind(ip)
//     .unwrap()
//     .run()
//     .await
//     .unwrap();
// });

// async fn board(board: Data<Mutex<Option<Board>>>) -> HttpResponse {
//     HttpResponse::Ok().json(board.lock().await.clone())
// }

// async fn snake_ws(
//     req: HttpRequest,
//     stream: Payload,
//     client_tx: Data<Sender<Client>>,
// ) -> Result<HttpResponse, actix_web::Error> {
//     let (res, session, msg_stream) = actix_ws::handle(&req, stream)?;

//     // spawn websocket handler (and don't await it) so that the response is returned immediately
//     actix_web::rt::spawn(snake_ws_handler(session, msg_stream, (**client_tx).clone()));

//     Ok(res)
// }

// async fn snake_ws_handler(
//     mut session: actix_ws::Session,
//     mut msg_stream: actix_ws::MessageStream,
//     client_tx: Sender<Client>,
// ) {
//     info!("web socket connected");

//     let mut last_heartbeat = Instant::now();
//     let mut interval = interval(Duration::from_secs(5));

//     let (client, game_commands, mut game_updates) = Client::new();
//     client_tx.send(client).await.unwrap();

//     let reason = loop {
//         // create "next client timeout check" future
//         let tick = interval.tick();

//         tokio::select! {
//             // received a board update from the game
//             update = game_updates.recv() => {
//                 match update {
//                     Some(game_update) => {
//                         if let Err(e) = session.text(serde_json::to_string(&game_update).unwrap()).await {
//                             error!("{}", e);
//                             break None;
//                         }
//                     }

//                     None => {
//                         break None;
//                     }
//                 }
//             }

//             // received message from WebSocket client
//             msg = msg_stream.recv() => {
//                 match msg {
//                     Some(Ok(msg)) => match msg {
//                         Message::Text(text) => {
//                             let command = match serde_json::from_str::<GameCommands>(&text) {
//                                 Ok(input) => input,
//                                 Err(err) => {
//                                     session.text(format!("invalid input: {}", err)).await.unwrap();
//                                     error!("{}", err);
//                                     break None;
//                                 }
//                             };

//                             if let Err(e) = game_commands.send(command).await {
//                                 error!("{}", e);
//                                 break None;
//                             }
//                         }

//                         Message::Binary(_) => {
//                             session.text("i dont want your binary data").await.unwrap();
//                         }

//                         Message::Close(reason) => {
//                             break reason;
//                         }

//                         Message::Ping(bytes) => {
//                             last_heartbeat = Instant::now();
//                             session.pong(&bytes).await.ok();
//                         }

//                         Message::Pong(_) => {
//                             last_heartbeat = Instant::now();
//                         }

//                         Message::Continuation(_) => {
//                             warn!("no support for continuation frames");
//                         }

//                         Message::Nop => {}
//                     }

//                     Some(Err(err)) => {
//                         error!("{}", err);
//                         break None;
//                     }

//                     None => break None,
//                 }
//             }

//             // heartbeat interval ticked
//             _ = tick => {
//                 // if no heartbeat ping/pong received recently, close the connection
//                 if Instant::now().duration_since(last_heartbeat) > Duration::from_secs(10) {
//                     info!("client has not sent heartbeat in over 10s; disconnecting");

//                     break None;
//                 }

//                 // send heartbeat ping
//                 let _ = session.ping(b"").await;
//             }
//         }
//     };

//     // attempt to close connection gracefully
//     let _ = session.close(reason).await;

//     info!("disconnected");
// }
