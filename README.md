# Bevy Snake

A simple snake game, inspired by google snake. Adds multiplayer and GUNS!!

Web demo: <https://bink.eu.org/snake/>

## Building

Just clone the repo and run:

```bash
cargo run --release
```

Hopefully it'll work `¯\_(ツ)_/¯`

### Web

Add the `wasm32-unknown-unknown` target and install `wasm-bindgen`:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
```

Build the wasm bundle into `web/`:

```bash
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --no-typescript --out-name bevy-snake \
  --out-dir web --target web \
  target/wasm32-unknown-unknown/release/bevy-snake.wasm
```

Run the server, which serves `web/` over HTTP **and** runs the matchbox
signaling server on a separate port:

```bash
cargo run --bin server --release
# open http://localhost:1234
```

Solo (1-player) play skips matchbox entirely. For 2+ player matches all
clients must reach the matchbox URL compiled into the wasm — by default
`ws://localhost:3536/snake`. Override at build time via `MATCHBOX_ROOM_URL`
(the `?next={N}` query string is appended automatically based on the lobby
player count).

Environment overrides for the server:

| Var | Default | Notes |
|---|---|---|
| `HTTP_ADDR` | `0.0.0.0:1234` | TCP bind for the static-file server (`web/`) |
| `MATCHBOX_ADDR` | `0.0.0.0:3536` | WebSocket bind for matchbox signaling |

For a deploy behind a public hostname, override `MATCHBOX_ROOM_URL` at the
client's compile step, e.g.

```bash
MATCHBOX_ROOM_URL=wss://bink.eu.org/snake \
  cargo build --release --target wasm32-unknown-unknown
```

Use `wss://` so the WebSocket works from an HTTPS page.

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
