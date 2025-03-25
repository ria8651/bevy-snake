#[cfg(not(target_arch = "wasm32"))]
fn main() {
    colog::init();

    bevy_snake::server::start_server(("0.0.0.0", 1234));
}

#[cfg(target_arch = "wasm32")]
fn main() {}
