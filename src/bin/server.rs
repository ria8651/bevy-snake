#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy_snake::server::{start_server, ServerConfig};
    use std::env;

    colog::init();

    let wt_addr = env::var("WT_ADDR")
        .unwrap_or_else(|_| "[::]:1234".to_string())
        .parse()
        .expect("WT_ADDR must be a valid socket address");
    let http_addr = env::var("HTTP_ADDR")
        .unwrap_or_else(|_| "[::]:1234".to_string())
        .parse()
        .expect("HTTP_ADDR must be a valid socket address");
    let wt_url = env::var("WT_URL").unwrap_or_else(|_| "https://localhost:1234".to_string());
    let cert_sans = env::var("CERT_SANS")
        .unwrap_or_else(|_| "localhost,127.0.0.1".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    start_server(ServerConfig {
        wt_addr,
        http_addr,
        wt_url,
        cert_sans,
    });
}

#[cfg(target_arch = "wasm32")]
fn main() {}
