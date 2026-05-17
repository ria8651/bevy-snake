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
    let ws_addr: SocketAddr = env::var("MATCHBOX_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3536".to_string())
        .parse()
        .expect("MATCHBOX_ADDR must be a valid socket address");

    let signaling = tokio::spawn(async move {
        info!("matchbox signaling listening on ws://{}", ws_addr);
        let server = SignalingServer::full_mesh_builder(ws_addr).build();
        if let Err(e) = server.serve().await {
            error!("matchbox signaling exited: {}", e);
        }
    });

    let http = tokio::spawn(async move {
        let app: Router = Router::new().fallback_service(ServeDir::new("web"));
        let listener = tokio::net::TcpListener::bind(http_addr)
            .await
            .expect("http bind");
        info!("http static server listening on http://{}", http_addr);
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
