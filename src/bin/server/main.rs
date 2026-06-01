//! Authoritative game server + lobby + static-file host.
//!
//! Architecture:
//!   - **Bevy app** (main thread) hosts the Lightyear `ServerPlugins` and
//!     runs the authoritative `Board::tick_movement` per movement tick.
//!   - **Tokio task** runs the axum HTTP server on `HTTP_ADDR` (default
//!     1234): serves `web/` static files + `/lobbies` WebSocket lobby
//!     discovery.
//!   - **Lightyear WebSocket server** listens on `GAME_ADDR` (default
//!     1235). When the lobby's host hits Start, the lobby task issues each
//!     member a `GameSessionReady` message with a unique netcode `client_id`
//!     + shared `private_key` + `endpoint`. Members open a Lightyear
//!     connection to that endpoint.
//!
//! For now there is a single global game session; supporting multiple
//! concurrent lobbies is a follow-up (would key on `protocol_id` per lobby).

#[cfg(not(target_arch = "wasm32"))]
mod lobby_service {
    use axum::Router;
    use axum::extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    };
    use axum::response::IntoResponse;
    use axum::routing::get;
    use bevy_snake::lobby_proto::{
        ClientMsg, GameSessionCreds, Lobby, LobbyId, LobbyState, MAX_PLAYERS, ServerMsg,
    };
    use bevy_snake::net_proto::PROTOCOL_ID;
    use log::{debug, info, warn};
    use std::collections::HashMap;
    // v1 runs a single global game session, so every lobby's credentials must
    // sign auth tokens with the same private key the game server is configured
    // with. Multi-session isolation is a follow-up; when it lands, each lobby
    // will allocate its own key and pass it into a per-lobby NetcodeServer.
    use bevy_snake::game_server::SESSION_KEY;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::{RwLock, broadcast, mpsc};

    pub type ConnId = u64;

    const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);
    const FINISHED_GRACE: Duration = Duration::from_secs(5);

    /// Where to send game-server credentials.
    ///
    /// `connect_endpoint` is what the client uses to open the WebSocket —
    /// either an absolute `ws[s]://` URL or a path like `/game` (resolved
    /// against the page origin). `netcode_server_addr` is what goes into the
    /// netcode auth token; it must match the game server's `LocalAddr`.
    /// These differ when the WebSocket is reverse-proxied: the browser hits
    /// the front-door path on the HTTP port, but netcode validates against
    /// the backend's bind addr behind the proxy.
    #[derive(Clone)]
    pub struct GameEndpoint {
        pub connect_endpoint: String,
        pub netcode_server_addr: std::net::SocketAddr,
    }

    struct MemberInfo {
        last_heartbeat: Instant,
        /// Allocated when StartLobby fires.
        client_id: Option<u64>,
    }

    struct LobbyRecord {
        lobby: Lobby,
        host: ConnId,
        members: HashMap<ConnId, MemberInfo>,
        next_client_id: u64,
    }

    pub struct AppState {
        lobbies: RwLock<HashMap<LobbyId, LobbyRecord>>,
        conns: RwLock<HashMap<ConnId, mpsc::UnboundedSender<ServerMsg>>>,
        list_changed: broadcast::Sender<()>,
        next_conn: AtomicU64,
        pub endpoint: GameEndpoint,
    }

    impl AppState {
        pub fn new(endpoint: GameEndpoint) -> Arc<Self> {
            let (tx, _) = broadcast::channel(16);
            Arc::new(Self {
                lobbies: RwLock::new(HashMap::new()),
                conns: RwLock::new(HashMap::new()),
                list_changed: tx,
                next_conn: AtomicU64::new(1),
                endpoint,
            })
        }
    }

    pub fn router(state: Arc<AppState>) -> Router {
        let gc_state = state.clone();
        tokio::spawn(async move { gc_loop(gc_state).await });
        Router::new()
            .route("/lobbies", get(ws_handler))
            .route("/game", get(game_proxy))
            .with_state(state)
    }

    /// Same-origin reverse proxy: upgrades the browser's WebSocket, opens a
    /// matching WebSocket to the Lightyear backend at
    /// `state.endpoint.netcode_server_addr`, and shuttles frames in both
    /// directions until either end closes. Lets the deployment expose a
    /// single public port even though Lightyear runs its own listener.
    async fn game_proxy(
        ws: WebSocketUpgrade,
        State(state): State<Arc<AppState>>,
    ) -> impl IntoResponse {
        let backend = state.endpoint.netcode_server_addr;
        ws.on_upgrade(move |frontend| async move {
            proxy_session(frontend, backend).await;
        })
    }

    async fn proxy_session(frontend: WebSocket, backend_addr: std::net::SocketAddr) {
        use axum::extract::ws::Message as FrontMsg;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message as BackMsg;

        let url = format!("ws://{}/", backend_addr);
        let backend = match tokio_tungstenite::connect_async(&url).await {
            Ok((ws, _)) => ws,
            Err(e) => {
                warn!("game proxy: backend connect to {} failed: {}", url, e);
                return;
            }
        };
        let (mut b_tx, mut b_rx) = backend.split();
        let (mut f_tx, mut f_rx) = frontend.split();

        let client_to_backend = async move {
            while let Some(Ok(msg)) = f_rx.next().await {
                let out = match msg {
                    FrontMsg::Text(t) => BackMsg::Text(t.as_str().into()),
                    FrontMsg::Binary(b) => BackMsg::Binary(b.to_vec().into()),
                    FrontMsg::Ping(b) => BackMsg::Ping(b.to_vec().into()),
                    FrontMsg::Pong(b) => BackMsg::Pong(b.to_vec().into()),
                    FrontMsg::Close(_) => {
                        let _ = b_tx.send(BackMsg::Close(None)).await;
                        break;
                    }
                };
                if b_tx.send(out).await.is_err() {
                    break;
                }
            }
        };
        let backend_to_client = async move {
            while let Some(Ok(msg)) = b_rx.next().await {
                let out = match msg {
                    BackMsg::Text(t) => FrontMsg::Text(t.as_str().to_string().into()),
                    BackMsg::Binary(b) => FrontMsg::Binary(b.to_vec().into()),
                    BackMsg::Ping(b) => FrontMsg::Ping(b.to_vec().into()),
                    BackMsg::Pong(b) => FrontMsg::Pong(b.to_vec().into()),
                    BackMsg::Close(_) => {
                        let _ = f_tx.send(FrontMsg::Close(None)).await;
                        break;
                    }
                    BackMsg::Frame(_) => continue,
                };
                if f_tx.send(out).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            _ = client_to_backend => {}
            _ = backend_to_client => {}
        }
    }

    async fn ws_handler(
        ws: WebSocketUpgrade,
        State(state): State<Arc<AppState>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| handle_socket(socket, state))
    }

    async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
        use futures_util::{SinkExt, StreamExt};
        let conn_id = state.next_conn.fetch_add(1, Ordering::Relaxed);
        let (mut ws_tx, mut ws_rx) = socket.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMsg>();

        state.conns.write().await.insert(conn_id, out_tx.clone());

        let outgoing = tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                let json = match serde_json::to_string(&msg) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("encode error: {}", e);
                        continue;
                    }
                };
                if ws_tx.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        });

        let list_state = state.clone();
        let list_tx = out_tx.clone();
        let mut list_sub = state.list_changed.subscribe();
        let listener = tokio::spawn(async move {
            let _ = list_tx.send(snapshot(&list_state).await);
            while list_sub.recv().await.is_ok() {
                if list_tx.send(snapshot(&list_state).await).is_err() {
                    break;
                }
            }
        });

        while let Some(Ok(msg)) = ws_rx.next().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                _ => continue,
            };
            let parsed: ClientMsg = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    let _ = out_tx.send(ServerMsg::Error {
                        msg: format!("bad message: {e}"),
                    });
                    continue;
                }
            };
            handle_client_msg(&state, conn_id, &out_tx, parsed).await;
        }

        disconnect(&state, conn_id).await;
        state.conns.write().await.remove(&conn_id);
        listener.abort();
        outgoing.abort();
    }

    async fn handle_client_msg(
        state: &Arc<AppState>,
        conn_id: ConnId,
        out_tx: &mpsc::UnboundedSender<ServerMsg>,
        msg: ClientMsg,
    ) {
        match msg {
            ClientMsg::CreateLobby { settings } => {
                let mut lobbies = state.lobbies.write().await;
                if lobbies.values().any(|r| r.members.contains_key(&conn_id)) {
                    let _ = out_tx.send(ServerMsg::Error {
                        msg: "already in a lobby".into(),
                    });
                    return;
                }
                let id = nanoid::nanoid!(8);
                let mut members = HashMap::new();
                members.insert(
                    conn_id,
                    MemberInfo {
                        last_heartbeat: Instant::now(),
                        client_id: None,
                    },
                );
                lobbies.insert(
                    id.clone(),
                    LobbyRecord {
                        lobby: Lobby {
                            id: id.clone(),
                            settings,
                            state: LobbyState::Waiting,
                            players_present: 1,
                            spectators_present: 0,
                        },
                        host: conn_id,
                        members,
                        next_client_id: 1,
                    },
                );
                drop(lobbies);
                info!("conn {} created lobby {}", conn_id, id);
                let _ = out_tx.send(ServerMsg::LobbyCreated { id });
                let _ = state.list_changed.send(());
            }
            ClientMsg::JoinLobby { id } => {
                let mut lobbies = state.lobbies.write().await;
                if lobbies.values().any(|r| r.members.contains_key(&conn_id)) {
                    let _ = out_tx.send(ServerMsg::JoinDenied {
                        id,
                        reason: "already in a lobby".into(),
                    });
                    return;
                }
                let Some(record) = lobbies.get_mut(&id) else {
                    let _ = out_tx.send(ServerMsg::JoinDenied {
                        id,
                        reason: "lobby not found".into(),
                    });
                    return;
                };
                // In-progress lobbies still accept connections as spectators.
                if record.lobby.players_present >= MAX_PLAYERS
                    && record.lobby.state == LobbyState::Waiting
                {
                    let _ = out_tx.send(ServerMsg::JoinDenied {
                        id,
                        reason: "lobby full".into(),
                    });
                    return;
                }
                record.members.insert(
                    conn_id,
                    MemberInfo {
                        last_heartbeat: Instant::now(),
                        client_id: None,
                    },
                );
                if record.lobby.state == LobbyState::Waiting {
                    record.lobby.players_present = record.members.len() as u8;
                } else {
                    record.lobby.spectators_present =
                        record.lobby.spectators_present.saturating_add(1);
                }
                let in_progress = record.lobby.state != LobbyState::Waiting;
                let id_for_send = id.clone();
                info!("conn {} joined lobby {}", conn_id, id);
                if in_progress {
                    // Allocate a client_id for the spectator and hand them
                    // the existing session's credentials so they can connect.
                    record.next_client_id += 1;
                    let client_id = record.next_client_id;
                    if let Some(member) = record.members.get_mut(&conn_id) {
                        member.client_id = Some(client_id);
                    }
                    let creds = GameSessionCreds {
                        endpoint: state.endpoint.connect_endpoint.clone(),
                        netcode_server_addr: state.endpoint.netcode_server_addr,
                        client_id,
                        private_key: SESSION_KEY,
                        protocol_id: PROTOCOL_ID,
                    };
                    drop(lobbies);
                    let _ = out_tx.send(ServerMsg::GameSessionReady {
                        id: id_for_send,
                        creds,
                    });
                } else {
                    drop(lobbies);
                }
                let _ = state.list_changed.send(());
            }
            ClientMsg::StartLobby { id } => {
                let mut lobbies = state.lobbies.write().await;
                let Some(record) = lobbies.get_mut(&id) else {
                    return;
                };
                if record.host != conn_id {
                    return;
                }
                if record.lobby.players_present < 2 {
                    let _ = out_tx.send(ServerMsg::Error {
                        msg: "need at least 2 players".into(),
                    });
                    return;
                }
                record.lobby.state = LobbyState::InProgress;
                // Allocate client_ids for each member who doesn't have one.
                let mut handouts: Vec<(ConnId, GameSessionCreds)> = Vec::new();
                let mut next_id = record.next_client_id;
                for (cid, member) in record.members.iter_mut() {
                    if member.client_id.is_none() {
                        next_id += 1;
                        member.client_id = Some(next_id);
                    }
                    let creds = GameSessionCreds {
                        endpoint: state.endpoint.connect_endpoint.clone(),
                        netcode_server_addr: state.endpoint.netcode_server_addr,
                        client_id: member.client_id.unwrap(),
                        private_key: SESSION_KEY,
                        protocol_id: PROTOCOL_ID,
                    };
                    handouts.push((*cid, creds));
                }
                record.next_client_id = next_id;
                drop(lobbies);

                info!("starting lobby {} ({} members)", id, handouts.len());
                let conns = state.conns.read().await;
                for (cid, creds) in handouts {
                    if let Some(tx) = conns.get(&cid) {
                        let _ = tx.send(ServerMsg::GameSessionReady {
                            id: id.clone(),
                            creds,
                        });
                    }
                }
                drop(conns);
                let _ = state.list_changed.send(());
            }
            ClientMsg::UpdateState { id, state: new_state } => {
                let mut lobbies = state.lobbies.write().await;
                let Some(record) = lobbies.get_mut(&id) else {
                    return;
                };
                if record.host != conn_id {
                    return;
                }
                record.lobby.state = new_state;
                debug!("lobby {} → {:?}", id, new_state);
                drop(lobbies);
                let _ = state.list_changed.send(());
            }
            ClientMsg::Heartbeat { id } => {
                let mut lobbies = state.lobbies.write().await;
                let Some(record) = lobbies.get_mut(&id) else {
                    return;
                };
                let Some(member) = record.members.get_mut(&conn_id) else {
                    return;
                };
                member.last_heartbeat = Instant::now();
            }
            ClientMsg::LeaveLobby { id: _ } => {
                disconnect(state, conn_id).await;
            }
        }
    }

    async fn disconnect(state: &Arc<AppState>, conn_id: ConnId) {
        let mut to_kick: Vec<(ConnId, LobbyId, &'static str)> = Vec::new();
        let mut changed = false;
        {
            let mut lobbies = state.lobbies.write().await;
            let mut hosts_to_remove: Vec<LobbyId> = Vec::new();
            for (id, record) in lobbies.iter_mut() {
                if record.host == conn_id {
                    for &m in record.members.keys() {
                        if m != conn_id {
                            to_kick.push((m, id.clone(), "host left"));
                        }
                    }
                    hosts_to_remove.push(id.clone());
                } else if record.members.remove(&conn_id).is_some() {
                    record.lobby.players_present = record.members.len() as u8;
                    changed = true;
                }
            }
            for id in hosts_to_remove {
                lobbies.remove(&id);
                changed = true;
            }
        }
        if !to_kick.is_empty() {
            let conns = state.conns.read().await;
            for (cid, id, reason) in to_kick {
                if let Some(tx) = conns.get(&cid) {
                    let _ = tx.send(ServerMsg::Kicked {
                        id,
                        reason: reason.into(),
                    });
                }
            }
        }
        if changed {
            let _ = state.list_changed.send(());
        }
    }

    async fn snapshot(state: &Arc<AppState>) -> ServerMsg {
        let lobbies = state.lobbies.read().await;
        let list: Vec<Lobby> = lobbies.values().map(|r| r.lobby.clone()).collect();
        ServerMsg::LobbyList { lobbies: list }
    }

    async fn gc_loop(state: Arc<AppState>) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let now = Instant::now();
            let mut to_disconnect: Vec<ConnId> = Vec::new();
            let mut to_remove: Vec<LobbyId> = Vec::new();
            {
                let lobbies = state.lobbies.read().await;
                for (id, record) in lobbies.iter() {
                    for (cid, m) in record.members.iter() {
                        if now.duration_since(m.last_heartbeat) > HEARTBEAT_TIMEOUT {
                            to_disconnect.push(*cid);
                        }
                    }
                    if record.lobby.state == LobbyState::Finished
                        && record
                            .members
                            .values()
                            .all(|m| now.duration_since(m.last_heartbeat) > FINISHED_GRACE)
                    {
                        to_remove.push(id.clone());
                    }
                }
            }
            for cid in to_disconnect {
                warn!("conn {} heartbeat timeout", cid);
                disconnect(&state, cid).await;
            }
            if !to_remove.is_empty() {
                let mut lobbies = state.lobbies.write().await;
                for id in &to_remove {
                    lobbies.remove(id);
                }
                drop(lobbies);
                let _ = state.list_changed.send(());
            }
        }
    }

}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy::prelude::*;
    use log::{error, info};
    use std::env;
    use std::net::SocketAddr;
    use std::sync::Arc;

    colog::init();

    let http_addr: SocketAddr = env::var("HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:1234".to_string())
        .parse()
        .expect("HTTP_ADDR must be a valid socket address");
    // Lightyear's WebSocket listener. Default to localhost-only so it isn't
    // exposed directly — the HTTP server on `http_addr` reverse-proxies
    // `/game` to it and that's the only public-facing entry point.
    let game_addr: SocketAddr = env::var("GAME_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:1235".to_string())
        .parse()
        .expect("GAME_ADDR must be a valid socket address");
    // What we advertise to clients. Default is a same-origin path; the
    // browser ends up calling `ws://<page-host>/game`, axum upgrades and
    // proxies to the Lightyear backend on `game_addr`. Operators can override
    // (e.g. `wss://other.host/game`) when the game server is on a different
    // hostname than the HTTP server.
    let connect_endpoint =
        env::var("GAME_PUBLIC_URL").unwrap_or_else(|_| "/game".to_string());

    // Run axum + lobby in a tokio runtime on a dedicated thread.
    let lobby_endpoint = lobby_service::GameEndpoint {
        connect_endpoint: connect_endpoint.clone(),
        netcode_server_addr: game_addr,
    };
    let _tokio_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            use axum::Router;
            use axum::http::{HeaderValue, header};
            use tower_http::services::ServeDir;
            use tower_http::set_header::SetResponseHeaderLayer;
            let lobby_state = lobby_service::AppState::new(lobby_endpoint);
            // Dev: tell browsers not to cache assets. The 67 MB wasm bundle
            // gets aggressively cached by Firefox, so rebuilds appear stale
            // until you manually disable cache. Override at deploy time.
            let no_cache = SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, no-store, must-revalidate"),
            );
            let app: Router = Router::new()
                .merge(lobby_service::router(lobby_state))
                .fallback_service(ServeDir::new("web"))
                .layer(no_cache);
            let listener = match tokio::net::TcpListener::bind(http_addr).await {
                Ok(l) => l,
                Err(e) => {
                    error!("http bind {}: {}", http_addr, e);
                    return;
                }
            };
            info!("http static + lobby ws on http://{}", http_addr);
            if let Err(e) = axum::serve(listener, app).await {
                error!("http server exited: {}", e);
            }
        });
    });

    let _ = Arc::new(()); // suppress unused-import warning if any
    info!("game server (Lightyear) on ws://{}", game_addr);

    // Run the Bevy + Lightyear game server.
    //
    // Pace the outer schedule so it doesn't busy-spin at 100% CPU. Without
    // this, `MinimalPlugins`' `ScheduleRunnerPlugin` defaults to
    // `RunMode::Loop { wait: None }`, the global change-tick races past
    // `MAX_CHANGE_AGE` within seconds, and `check_change_ticks` spams
    // warnings about Startup / host-server systems that haven't re-run.
    // Lightyear's authoritative sim runs in `FixedUpdate` at
    // `SERVER_TICK_HZ`, so the outer loop just needs to drain inputs
    // faster than that.
    use bevy::app::ScheduleRunnerPlugin;
    bevy::app::App::new()
        .add_plugins(
            bevy::MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / 240.0),
            )),
        )
        .add_plugins(bevy_snake::game_server::GameServerPlugin {
            bind: game_addr,
        })
        .run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
