//! HTTP static-file server + matchbox signaling server + lobby WS service.
//!
//! Three things bound at the same time:
//!   - HTTP on `HTTP_ADDR` (default 1234) serves `web/` and hosts the
//!     `/lobbies` WebSocket below.
//!   - Matchbox signaling on `MATCHBOX_ADDR` (default 3536) — untouched
//!     full-mesh handshake server.
//!   - Lobby WS on `/lobbies` — in-memory directory of open lobbies.
//!     Clients connect to discover and join lobbies; the host eventually
//!     calls Start, and the server broadcasts the agreed peer roster so all
//!     clients build the same GGRS session.
//!
//! The lobby service is intentionally tiny: no auth, in-memory state, GC by
//! heartbeat. A lobby's id doubles as its matchbox room name
//! (`lobby-{id}`), so once Start fires the clients already know where to
//! point matchbox.

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
        ClientMsg, Lobby, LobbyId, LobbyState, MAX_PLAYERS, ServerMsg,
    };
    use futures_util::{SinkExt, StreamExt};
    use log::{debug, info, warn};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::{RwLock, broadcast, mpsc};

    /// Per-connection identifier. Incremented atomically; doesn't survive
    /// restart, which is fine — state is in-memory anyway.
    pub type ConnId = u64;

    /// Heartbeat must arrive at least this often or the member is dropped.
    /// Client sends every ~3 s, so 10 s gives us three misses of slack.
    const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);
    /// A Finished lobby lingers for this long so the row briefly shows
    /// "Finished" in everyone's browser before disappearing.
    const FINISHED_GRACE: Duration = Duration::from_secs(5);

    struct MemberInfo {
        peer_id: Option<String>,
        last_heartbeat: Instant,
    }

    struct LobbyRecord {
        lobby: Lobby,
        host: ConnId,
        members: HashMap<ConnId, MemberInfo>,
    }

    pub struct AppState {
        lobbies: RwLock<HashMap<LobbyId, LobbyRecord>>,
        conns: RwLock<HashMap<ConnId, mpsc::UnboundedSender<ServerMsg>>>,
        /// Fires whenever the lobby map changes. Per-connection tasks
        /// subscribe and push fresh `LobbyList`s downstream on every tick.
        list_changed: broadcast::Sender<()>,
        next_conn: AtomicU64,
    }

    impl AppState {
        pub fn new() -> Arc<Self> {
            let (tx, _) = broadcast::channel(16);
            Arc::new(Self {
                lobbies: RwLock::new(HashMap::new()),
                conns: RwLock::new(HashMap::new()),
                list_changed: tx,
                next_conn: AtomicU64::new(1),
            })
        }
    }

    pub fn router(state: Arc<AppState>) -> Router {
        let gc_state = state.clone();
        tokio::spawn(async move { gc_loop(gc_state).await });
        Router::new()
            .route("/lobbies", get(ws_handler))
            .with_state(state)
    }

    async fn ws_handler(
        ws: WebSocketUpgrade,
        State(state): State<Arc<AppState>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| handle_socket(socket, state))
    }

    async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
        let conn_id = state.next_conn.fetch_add(1, Ordering::Relaxed);
        let (mut ws_tx, mut ws_rx) = socket.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMsg>();

        state.conns.write().await.insert(conn_id, out_tx.clone());

        // Outgoing pump: mpsc → websocket. Encoded as JSON text frames.
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

        // List-change watcher: re-broadcasts the snapshot to this conn.
        let list_state = state.clone();
        let list_tx = out_tx.clone();
        let mut list_sub = state.list_changed.subscribe();
        let listener = tokio::spawn(async move {
            // Initial snapshot.
            let _ = list_tx.send(snapshot(&list_state).await);
            while list_sub.recv().await.is_ok() {
                if list_tx.send(snapshot(&list_state).await).is_err() {
                    break;
                }
            }
        });

        // Incoming pump: parse ClientMsg, mutate state.
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

        // Connection gone — tear down our membership and tasks.
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
                // One lobby per connection. Reject if already in one.
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
                        peer_id: None,
                        last_heartbeat: Instant::now(),
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
                        },
                        host: conn_id,
                        members,
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
                if record.lobby.state != LobbyState::Waiting {
                    let _ = out_tx.send(ServerMsg::JoinDenied {
                        id,
                        reason: "lobby already started".into(),
                    });
                    return;
                }
                if record.lobby.players_present >= MAX_PLAYERS {
                    let _ = out_tx.send(ServerMsg::JoinDenied {
                        id,
                        reason: "lobby full".into(),
                    });
                    return;
                }
                record.members.insert(
                    conn_id,
                    MemberInfo {
                        peer_id: None,
                        last_heartbeat: Instant::now(),
                    },
                );
                record.lobby.players_present = record.members.len() as u8;
                info!("conn {} joined lobby {}", conn_id, id);
                drop(lobbies);
                let _ = state.list_changed.send(());
            }
            ClientMsg::StartLobby { id } => {
                let mut lobbies = state.lobbies.write().await;
                let Some(record) = lobbies.get_mut(&id) else {
                    return;
                };
                if record.host != conn_id || record.lobby.state != LobbyState::Waiting {
                    return;
                }
                // Roster is sorted by ConnId to give every client the same
                // deterministic ordering (important for GGRS player handles).
                let total_members = record.members.len();
                let mut roster_pairs: Vec<(ConnId, String)> = record
                    .members
                    .iter()
                    .filter_map(|(c, m)| m.peer_id.clone().map(|p| (*c, p)))
                    .collect();
                roster_pairs.sort_by_key(|(c, _)| *c);
                let roster: Vec<String> =
                    roster_pairs.into_iter().map(|(_, p)| p).collect();
                if roster.len() < 2 {
                    let _ = out_tx.send(ServerMsg::Error {
                        msg: "need at least 2 players".into(),
                    });
                    return;
                }
                // All present members must have heartbeated their matchbox
                // PeerId — otherwise starting would silently kick them out of
                // the roster. Better to make the host wait a moment.
                if roster.len() < total_members {
                    let _ = out_tx.send(ServerMsg::Error {
                        msg: "Some players aren't fully connected yet — try Start again in a moment".into(),
                    });
                    return;
                }
                record.lobby.state = LobbyState::Playing;
                let targets: Vec<ConnId> = record.members.keys().copied().collect();
                drop(lobbies);
                info!("starting lobby {} with {} players", id, roster.len());
                let conns = state.conns.read().await;
                for cid in targets {
                    if let Some(tx) = conns.get(&cid) {
                        let _ = tx.send(ServerMsg::LobbyStarting {
                            id: id.clone(),
                            roster: roster.clone(),
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
            ClientMsg::Heartbeat { id, peer_id } => {
                let mut lobbies = state.lobbies.write().await;
                let Some(record) = lobbies.get_mut(&id) else {
                    return;
                };
                let Some(member) = record.members.get_mut(&conn_id) else {
                    return;
                };
                member.last_heartbeat = Instant::now();
                let pid_changed = peer_id.is_some() && member.peer_id != peer_id;
                if peer_id.is_some() {
                    member.peer_id = peer_id;
                }
                drop(lobbies);
                if pid_changed {
                    // Roster-affecting metadata changed; refresh subscribers
                    // so any future Start uses the right PeerIds. (The
                    // LobbyList itself doesn't include PeerIds, so this is
                    // a no-op for the browser, but doesn't hurt.)
                    let _ = state.list_changed.send(());
                }
            }
            ClientMsg::LeaveLobby { id: _ } => {
                disconnect(state, conn_id).await;
            }
        }
    }

    /// Remove a connection from any lobby it is in. If the connection was
    /// the host, tear down the lobby and `Kicked` everyone else. Always
    /// fires a list-changed broadcast.
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

    /// Drops timed-out members and Finished lobbies on a 1 Hz cadence.
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
mod signaling_proxy {
    //! Reverse-proxies browser WebSocket upgrades on `/signaling/{room}`
    //! through to the loopback matchbox signaling server. Lets the
    //! deployment expose a single external port for static files + lobby
    //! WS + signaling, instead of needing a second open port (and a second
    //! TLS termination) for matchbox.
    use axum::Router;
    use axum::extract::Path;
    use axum::extract::ws::{Message as AxumMsg, WebSocket, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use futures_util::{SinkExt, StreamExt};
    use log::{debug, warn};
    use std::net::SocketAddr;
    use tokio_tungstenite::tungstenite::Message as TungMsg;

    pub fn router(upstream_addr: SocketAddr) -> Router {
        Router::new().route(
            "/signaling/{room}",
            get(move |ws, path| proxy_ws(ws, path, upstream_addr)),
        )
    }

    async fn proxy_ws(
        ws: WebSocketUpgrade,
        Path(room): Path<String>,
        upstream_addr: SocketAddr,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |client| async move {
            let url = format!("ws://{}/{}", upstream_addr, room);
            debug!("signaling proxy: opening upstream {}", url);
            let (upstream, _) = match tokio_tungstenite::connect_async(&url).await {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("signaling proxy upstream connect failed: {}", e);
                    return;
                }
            };
            pump(client, upstream).await;
        })
    }

    async fn pump(
        client: WebSocket,
        upstream: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) {
        let (mut client_tx, mut client_rx) = client.split();
        let (mut up_tx, mut up_rx) = upstream.split();

        let c2u = async {
            while let Some(Ok(msg)) = client_rx.next().await {
                let Some(out) = axum_to_tung(msg) else { break };
                if up_tx.send(out).await.is_err() {
                    break;
                }
            }
        };
        let u2c = async {
            while let Some(Ok(msg)) = up_rx.next().await {
                let Some(out) = tung_to_axum(msg) else { break };
                if client_tx.send(out).await.is_err() {
                    break;
                }
            }
        };

        tokio::select! {
            _ = c2u => {},
            _ = u2c => {},
        }
    }

    fn axum_to_tung(m: AxumMsg) -> Option<TungMsg> {
        Some(match m {
            AxumMsg::Text(t) => TungMsg::Text(t.as_str().into()),
            AxumMsg::Binary(b) => TungMsg::Binary(b.to_vec().into()),
            AxumMsg::Ping(p) => TungMsg::Ping(p.to_vec().into()),
            AxumMsg::Pong(p) => TungMsg::Pong(p.to_vec().into()),
            AxumMsg::Close(_) => return None,
        })
    }

    fn tung_to_axum(m: TungMsg) -> Option<AxumMsg> {
        Some(match m {
            TungMsg::Text(t) => AxumMsg::Text(t.as_str().to_owned().into()),
            TungMsg::Binary(b) => AxumMsg::Binary(b.to_vec().into()),
            TungMsg::Ping(p) => AxumMsg::Ping(p.to_vec().into()),
            TungMsg::Pong(p) => AxumMsg::Pong(p.to_vec().into()),
            TungMsg::Close(_) | TungMsg::Frame(_) => return None,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    use axum::Router;
    use log::{error, info};
    use matchbox_signaling::SignalingServer;
    use std::env;
    use std::net::SocketAddr;
    use tower_http::services::ServeDir;

    colog::init();

    let http_addr: SocketAddr = env::var("HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:1234".to_string())
        .parse()
        .expect("HTTP_ADDR must be a valid socket address");
    // Default to loopback — the HTTP server proxies `/signaling/{room}` to
    // this address, so there is no reason to expose it externally. Override
    // with MATCHBOX_ADDR=0.0.0.0:3536 to expose it directly (skipping the
    // proxy).
    let ws_addr: SocketAddr = env::var("MATCHBOX_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3536".to_string())
        .parse()
        .expect("MATCHBOX_ADDR must be a valid socket address");

    let signaling = tokio::spawn(async move {
        info!("matchbox signaling listening on ws://{} (proxied)", ws_addr);
        let server = SignalingServer::full_mesh_builder(ws_addr).build();
        if let Err(e) = server.serve().await {
            error!("matchbox signaling exited: {}", e);
        }
    });

    let lobby_state = lobby_service::AppState::new();
    let http = tokio::spawn(async move {
        let app: Router = Router::new()
            .merge(lobby_service::router(lobby_state))
            .merge(signaling_proxy::router(ws_addr))
            .fallback_service(ServeDir::new("web"));
        let listener = tokio::net::TcpListener::bind(http_addr)
            .await
            .expect("http bind");
        info!(
            "http static server + lobby ws listening on http://{}",
            http_addr
        );
        if let Err(e) = axum::serve(listener, app).await {
            error!("http server exited: {}", e);
        }
    });

    tokio::select! {
        _ = signaling => error!("signaling task exited"),
        _ = http => error!("http task exited"),
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
