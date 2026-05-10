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
        let mut watchdog = interval(Duration::from_secs(2));
        loop {
            select! {
                // register a new client
                client = self.register_client.recv() => {
                    let Some(client) = client else {
                        error!("register_client channel closed");
                        break;
                    };
                    let was_empty = self.clients.clients.is_empty();
                    self.register_client(client).await;
                    if was_empty {
                        info!("first client connected; ticker.reset_immediately()");
                        ticker.reset_immediately();
                    }
                }
                // process client commands
                (client, command) = self.clients.next_command() => {
                    if self.process_command(client, command).await {
                        ticker.reset_immediately();
                    }
                }
                // tick the game board (only when there's at least one client to receive it)
                _ = ticker.tick(), if !self.clients.clients.is_empty() => {
                    if self.tick().await {
                        // game over: park the ticker until a RestartGame fires reset_immediately()
                        ticker.reset_after(Duration::from_secs(1_000_000));
                    }
                }
                // diagnostic watchdog: silence here means the loop is wedged
                // (most likely inside a broadcast send().await on a slow client)
                _ = watchdog.tick() => {
                    info!(
                        "watchdog: tick={}, clients={}, queued_inputs={}",
                        self.tick,
                        self.clients.clients.len(),
                        self.queued_inputs.len(),
                    );
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
                self.tick = 0;
                self.queued_inputs.clear();
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
            let start = std::time::Instant::now();
            let result = client.game_updates.send(game_update.clone()).await;
            let elapsed = start.elapsed();
            if let Err(e) = result {
                error!("{}", e);
                delete.push(index);
                continue;
            }
            if elapsed > Duration::from_millis(50) {
                warn!(
                    "broadcast to client {} took {}ms — slow consumer (channel cap is 1, so the per-session task is not draining fast enough)",
                    index,
                    elapsed.as_millis(),
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{AppleCount, BoardEvent, BoardSize, Cell, Direction, PlayerCount, Snake};
    use bevy::math::IVec2;
    use std::time::Duration;
    use tokio::sync::mpsc::{channel, Receiver, Sender};
    use tokio::time::timeout;

    // ---- helpers ----

    fn fresh_loop_with(settings: BoardSettings) -> GameLoop {
        let (_register_tx, register_rx) = channel::<Client>(1);
        GameLoop {
            clients: Clients::new(),
            register_client: register_rx,
            queued_inputs: HashMap::new(),
            rng: StdRng::seed_from_u64(0xC0FFEE_u64),
            board: Board::new(settings),
            tick: 0,
        }
    }

    fn fresh_loop() -> GameLoop {
        fresh_loop_with(BoardSettings::default())
    }

    async fn register(
        g: &mut GameLoop,
    ) -> (Sender<GameCommands>, Receiver<GameUpdates>, GameUpdates) {
        let (client, cmd_tx, mut upd_rx) = Client::new();
        g.register_client(client).await;
        let initial = upd_rx.recv().await.expect("initial update missing");
        (cmd_tx, upd_rx, initial)
    }

    async fn next_update(rx: &mut Receiver<GameUpdates>) -> GameUpdates {
        timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("timed out waiting for update")
            .expect("update channel closed")
    }

    fn snake(board: &Board, id: u8) -> Option<Snake> {
        board.snakes().get(&id).cloned()
    }

    fn head_of(board: &Board, id: u8) -> IVec2 {
        snake(board, id).expect("snake alive").head
    }

    fn input(tick: u64, direction: Direction) -> GameCommands {
        GameCommands::Input {
            tick,
            direction,
            timestamp: 1,
        }
    }

    fn restart(settings: BoardSettings) -> GameCommands {
        GameCommands::RestartGame {
            board_settings: settings,
        }
    }

    // ---- protocol round-trips ----
    //
    // These pin down the JSON wire format. If anyone changes the message types
    // in src/lib.rs, src/board.rs, or adjusts a serde rename, these flag it.

    #[test]
    fn protocol_input_command_round_trips() {
        for &dir in &[
            Direction::Up,
            Direction::Down,
            Direction::Left,
            Direction::Right,
        ] {
            let cmd = GameCommands::Input {
                tick: 42,
                direction: dir,
                timestamp: 1_234_567_890,
            };
            let json = serde_json::to_string(&cmd).unwrap();
            let parsed: GameCommands = serde_json::from_str(&json).unwrap();
            match parsed {
                GameCommands::Input {
                    tick,
                    direction,
                    timestamp,
                } => {
                    assert_eq!(tick, 42);
                    assert_eq!(direction, dir);
                    assert_eq!(timestamp, 1_234_567_890);
                }
                _ => panic!("expected Input variant after round-trip"),
            }
        }
    }

    #[test]
    fn protocol_restart_command_round_trips() {
        for &size in &[BoardSize::Small, BoardSize::Medium, BoardSize::Large] {
            for &apples in &[AppleCount::One, AppleCount::Three, AppleCount::Five] {
                for &players in &[
                    PlayerCount::One,
                    PlayerCount::Two,
                    PlayerCount::Three,
                    PlayerCount::Four,
                ] {
                    let settings = BoardSettings {
                        board_size: size,
                        apples,
                        players,
                    };
                    let json = serde_json::to_string(&restart(settings)).unwrap();
                    let parsed: GameCommands = serde_json::from_str(&json).unwrap();
                    match parsed {
                        GameCommands::RestartGame { board_settings } => {
                            assert_eq!(board_settings.board_size, size);
                            assert_eq!(board_settings.apples, apples);
                            assert_eq!(board_settings.players, players);
                        }
                        _ => panic!("expected RestartGame variant"),
                    }
                }
            }
        }
    }

    #[test]
    fn protocol_ticked_update_round_trips() {
        let board = Board::new(BoardSettings::default());
        let events = vec![
            BoardEvent::GameOver,
            BoardEvent::AppleEaten { snake: 0 },
            BoardEvent::SnakeDamaged { snake: 1 },
        ];
        let upd = GameUpdates::Ticked {
            tick: 99,
            board: board.clone(),
            events: events.clone(),
            timestamp: 555,
        };
        let json = serde_json::to_string(&upd).unwrap();
        let parsed: GameUpdates = serde_json::from_str(&json).unwrap();
        let GameUpdates::Ticked {
            tick,
            board: parsed_board,
            events: parsed_events,
            timestamp,
        } = parsed;
        assert_eq!(tick, 99);
        assert_eq!(timestamp, 555);
        assert_eq!(parsed_events, events);
        assert_eq!(parsed_board.width(), board.width());
        assert_eq!(parsed_board.height(), board.height());
        for (pos, original) in board.cells() {
            let copy = parsed_board.get(pos).unwrap();
            let same = match (original, copy) {
                (Cell::Empty, Cell::Empty) => true,
                (Cell::Wall, Cell::Wall) => true,
                (
                    Cell::Snake { id: a, part: pa },
                    Cell::Snake { id: b, part: pb },
                ) => a == b && pa == pb,
                (Cell::Apple { natural: a }, Cell::Apple { natural: b }) => a == b,
                _ => false,
            };
            assert!(same, "cell at {:?} did not round-trip", pos);
        }
    }

    // ---- client registration (server.rs:267) ----

    #[tokio::test]
    async fn register_sends_initial_ticked_to_new_client() {
        let mut g = fresh_loop();
        let (_tx, _rx, initial) = register(&mut g).await;
        let GameUpdates::Ticked {
            tick,
            events,
            board,
            timestamp,
        } = initial;
        assert_eq!(tick, 0, "initial tick must be 0");
        assert!(events.is_empty(), "initial update has no events");
        assert_eq!(board.width(), 10, "default Small board width");
        assert_eq!(board.height(), 9, "default Small board height");
        assert!(timestamp > 0, "timestamp should be set");
    }

    #[tokio::test]
    async fn register_does_not_broadcast_to_existing_clients() {
        let mut g = fresh_loop();
        let (_tx_a, mut rx_a, _initial_a) = register(&mut g).await;
        let (_tx_b, _rx_b, _initial_b) = register(&mut g).await;
        // A's queue must remain empty: registration is a unicast, not a broadcast.
        let result = timeout(Duration::from_millis(50), rx_a.recv()).await;
        assert!(
            result.is_err(),
            "A should not receive any update from B's registration"
        );
    }

    #[tokio::test]
    async fn register_multiple_clients_each_get_initial_state() {
        let mut g = fresh_loop();
        for i in 0..3 {
            let (client, _cmd_tx, mut upd_rx) = Client::new();
            g.register_client(client).await;
            let initial = upd_rx.recv().await.expect("client receives initial");
            let GameUpdates::Ticked { tick, .. } = initial;
            assert_eq!(tick, 0, "client {} initial tick", i);
        }
        assert_eq!(g.clients.clients.len(), 3);
    }

    // ---- tick processing (server.rs:341) ----

    #[tokio::test]
    async fn tick_increments_tick_counter() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        assert_eq!(g.tick, 0);
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(g.tick, 1);
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(g.tick, 2);
    }

    #[tokio::test]
    async fn tick_broadcasts_ticked_to_all_clients() {
        let mut g = fresh_loop();
        let (_tx_a, mut rx_a, _) = register(&mut g).await;
        let (_tx_b, mut rx_b, _) = register(&mut g).await;
        g.tick().await;
        let GameUpdates::Ticked { tick: ta, .. } = next_update(&mut rx_a).await;
        let GameUpdates::Ticked { tick: tb, .. } = next_update(&mut rx_b).await;
        assert_eq!(ta, 1);
        assert_eq!(tb, 1);
    }

    #[tokio::test]
    async fn tick_advances_snake_one_cell_in_current_direction() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        let head_before = head_of(&g.board, 0);
        // Default snake faces Right (head right of neck).
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(1, 0),
            "default snake should move right one cell per tick"
        );
    }

    #[tokio::test]
    async fn tick_with_no_clients_still_advances_state() {
        let mut g = fresh_loop();
        assert_eq!(g.tick, 0);
        let exit = g.tick().await;
        assert!(!exit, "tick should not signal exit when no game over");
        assert_eq!(g.tick, 1);
    }

    #[tokio::test]
    async fn tick_returns_true_on_game_over() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        // Drive the snake straight off the bottom edge.
        g.process_command(0, input(0, Direction::Down)).await;
        let mut got_game_over = false;
        let mut last_exit = false;
        for _ in 0..6 {
            last_exit = g.tick().await;
            let GameUpdates::Ticked { events, .. } = next_update(&mut rx).await;
            if events.contains(&BoardEvent::GameOver) {
                got_game_over = true;
                break;
            }
        }
        assert!(got_game_over, "snake should have died from going OOB");
        assert!(last_exit, "tick must return true on GameOver");
    }

    // ---- input handling (server.rs:288) ----

    #[tokio::test]
    async fn input_is_applied_on_next_tick() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        let head_before = head_of(&g.board, 0);
        g.process_command(0, input(0, Direction::Up)).await;
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(0, 1),
            "Up input should move snake up one cell"
        );
    }

    #[tokio::test]
    async fn input_overwrites_previous_input_for_same_client() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        let head_before = head_of(&g.board, 0);
        // Snake moves Right. Up and Down are both legal turns.
        g.process_command(0, input(0, Direction::Up)).await;
        g.process_command(0, input(0, Direction::Down)).await;
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(0, -1),
            "second input wins; HashMap::insert overwrites the queued direction"
        );
    }

    #[tokio::test]
    async fn input_with_stale_tick_is_still_accepted() {
        // Pins down current behaviour at server.rs:295 — stale-tick inputs warn
        // but are still applied. If we ever change to drop them, this test
        // should be updated deliberately rather than silently regressing.
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(g.tick, 1);
        let head_before = head_of(&g.board, 0);
        // Send an input claiming tick=0, but server is at tick=1.
        g.process_command(0, input(0, Direction::Up)).await;
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(0, 1),
            "stale-tick input is still applied (current behaviour)"
        );
    }

    #[tokio::test]
    async fn input_reverse_direction_is_ignored() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        let head_before = head_of(&g.board, 0);
        // Snake moves Right; Left is the opposite and must be ignored by board.rs:275.
        g.process_command(0, input(0, Direction::Left)).await;
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(1, 0),
            "Left input must be ignored; snake keeps moving Right"
        );
    }

    #[tokio::test]
    async fn input_routes_by_client_index_to_snake_id() {
        // Two players: snake 0 starts moving Right, snake 1 starts moving Left.
        // The contract here is implicit but load-bearing: the index of the client
        // in GameLoop.clients == the snake id it controls (server.rs:344-347).
        let settings = BoardSettings {
            board_size: BoardSize::Small,
            apples: AppleCount::One,
            players: PlayerCount::Two,
        };
        let mut g = fresh_loop_with(settings);
        let (_tx_a, mut rx_a, _) = register(&mut g).await;
        let (_tx_b, mut rx_b, _) = register(&mut g).await;

        let head0_before = head_of(&g.board, 0);
        let head1_before = head_of(&g.board, 1);

        g.process_command(0, input(0, Direction::Down)).await;
        g.process_command(1, input(0, Direction::Up)).await;
        g.tick().await;
        let _ = next_update(&mut rx_a).await;
        let _ = next_update(&mut rx_b).await;

        assert_eq!(
            head_of(&g.board, 0),
            head0_before + IVec2::new(0, -1),
            "client 0 input must steer snake 0"
        );
        assert_eq!(
            head_of(&g.board, 1),
            head1_before + IVec2::new(0, 1),
            "client 1 input must steer snake 1"
        );
    }

    #[tokio::test]
    async fn input_for_dead_snake_does_not_panic() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        // Kill snake 0 by walking off the bottom edge.
        g.process_command(0, input(0, Direction::Down)).await;
        for _ in 0..6 {
            g.tick().await;
            let _ = next_update(&mut rx).await;
            if g.board.count_snakes() == 0 {
                break;
            }
        }
        assert_eq!(g.board.count_snakes(), 0, "snake should be dead");

        // Send another input — server must not panic and tick must complete.
        g.process_command(0, input(0, Direction::Up)).await;
        g.tick().await;
        let _ = next_update(&mut rx).await;
    }

    // ---- game reset (server.rs:319) ----

    #[tokio::test]
    async fn restart_resets_tick_counter_to_zero() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        for _ in 0..3 {
            g.tick().await;
            let _ = next_update(&mut rx).await;
        }
        assert_eq!(g.tick, 3);
        g.process_command(0, restart(BoardSettings::default())).await;
        let _ = next_update(&mut rx).await;
        assert_eq!(g.tick, 0, "RestartGame must reset tick to 0");
    }

    #[tokio::test]
    async fn restart_replaces_board_with_new_settings() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        assert_eq!(g.board.width(), 10);
        assert_eq!(g.board.height(), 9);
        let new_settings = BoardSettings {
            board_size: BoardSize::Medium,
            apples: AppleCount::Three,
            players: PlayerCount::Two,
        };
        g.process_command(0, restart(new_settings)).await;
        let _ = next_update(&mut rx).await;
        assert_eq!(g.board.width(), 17, "Medium board width");
        assert_eq!(g.board.height(), 15, "Medium board height");
        assert_eq!(g.board.count_snakes(), 2, "two players => two snakes");
    }

    #[tokio::test]
    async fn restart_broadcasts_new_state_to_all_clients() {
        let mut g = fresh_loop();
        let (_tx_a, mut rx_a, _) = register(&mut g).await;
        let (_tx_b, mut rx_b, _) = register(&mut g).await;
        g.process_command(0, restart(BoardSettings::default())).await;
        let GameUpdates::Ticked {
            tick: ta,
            events: ea,
            ..
        } = next_update(&mut rx_a).await;
        let GameUpdates::Ticked {
            tick: tb,
            events: eb,
            ..
        } = next_update(&mut rx_b).await;
        assert_eq!(ta, 0);
        assert_eq!(tb, 0);
        assert!(ea.is_empty());
        assert!(eb.is_empty());
    }

    #[tokio::test]
    async fn restart_after_game_over_revives_snakes() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        g.process_command(0, input(0, Direction::Down)).await;
        for _ in 0..6 {
            g.tick().await;
            let _ = next_update(&mut rx).await;
            if g.board.count_snakes() == 0 {
                break;
            }
        }
        assert_eq!(g.board.count_snakes(), 0);

        g.process_command(0, restart(BoardSettings::default())).await;
        let _ = next_update(&mut rx).await;
        assert_eq!(g.board.count_snakes(), 1, "restart must respawn the snake");
        assert_eq!(g.tick, 0);

        let head_before = head_of(&g.board, 0);
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(1, 0),
            "respawned snake must be controllable"
        );
    }

    #[tokio::test]
    async fn restart_clears_queued_inputs() {
        // Bug pin: today, RestartGame doesn't clear queued_inputs (server.rs:319-337).
        // A direction queued before restart will leak into the post-restart snake's
        // first tick. This test will fail until that's fixed.
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        g.process_command(0, input(0, Direction::Up)).await;
        g.process_command(0, restart(BoardSettings::default())).await;
        let _ = next_update(&mut rx).await;

        let head_before = head_of(&g.board, 0);
        g.tick().await;
        let _ = next_update(&mut rx).await;
        assert_eq!(
            head_of(&g.board, 0),
            head_before + IVec2::new(1, 0),
            "queued input from before restart must not leak; snake should keep its starting direction (Right)"
        );
    }

    #[tokio::test]
    async fn restart_returns_true_to_reset_ticker() {
        let mut g = fresh_loop();
        let (_tx, mut rx, _) = register(&mut g).await;
        let result = g
            .process_command(0, restart(BoardSettings::default()))
            .await;
        let _ = next_update(&mut rx).await;
        assert!(
            result,
            "RestartGame must return true so the outer loop calls ticker.reset_immediately() (server.rs:253-255)"
        );
    }

    #[tokio::test]
    async fn input_returns_false_does_not_reset_ticker() {
        let mut g = fresh_loop();
        let (_tx, _rx, _) = register(&mut g).await;
        let result = g.process_command(0, input(0, Direction::Up)).await;
        assert!(!result, "Input must not request a ticker reset");
    }

    // ---- multi-client broadcast (server.rs:414) ----

    #[tokio::test]
    async fn broadcast_reaches_all_clients_on_tick() {
        let mut g = fresh_loop();
        let (_tx_a, mut rx_a, _) = register(&mut g).await;
        let (_tx_b, mut rx_b, _) = register(&mut g).await;
        let (_tx_c, mut rx_c, _) = register(&mut g).await;
        g.tick().await;
        let _ = next_update(&mut rx_a).await;
        let _ = next_update(&mut rx_b).await;
        let _ = next_update(&mut rx_c).await;
    }

    #[tokio::test]
    async fn disconnected_client_is_dropped_from_broadcast() {
        // A client whose update receiver is dropped (mirrors a closed WebTransport
        // session) must be removed by the broadcast cleanup at server.rs:421-424,
        // not crash the game loop or block other clients.
        let mut g = fresh_loop();
        let (_tx_a, rx_a, _) = register(&mut g).await;
        let (_tx_b, mut rx_b, _) = register(&mut g).await;
        drop(rx_a);

        g.tick().await;
        let _ = next_update(&mut rx_b).await;
        assert_eq!(
            g.clients.clients.len(),
            1,
            "dropped client must be removed from the client list"
        );

        g.tick().await;
        let _ = next_update(&mut rx_b).await;
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
