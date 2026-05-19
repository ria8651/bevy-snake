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

Run the server, which serves `web/` over HTTP, hosts the lobby directory
at `/lobbies`, **and** reverse-proxies matchbox signaling at
`/signaling/{room}` (the matchbox server itself binds to loopback only):

```bash
cargo run --bin server --release
# open http://localhost:1234
```

## Lobbies

The main menu is a live lobby browser. To play multiplayer:

1. One player clicks **Create lobby**, picks board/apples/speed, and clicks
   **Host**. They land on a waiting screen.
2. Other players see the lobby in their browser and click it to join. They
   inherit the host's settings.
3. When the host is ready (anywhere from 2 to 4 players), they click
   **Start**. Everyone in the lobby at that moment is locked in.
4. Player count is dynamic — it's whoever happens to be in the lobby when
   Start is pressed.

**Solo Play** on the main menu skips lobbies and matchbox entirely.

## Server config

| Var | Default | Notes |
|---|---|---|
| `HTTP_ADDR` | `0.0.0.0:1234` | TCP bind for the static-file server, lobby WS, and signaling proxy |
| `MATCHBOX_ADDR` | `127.0.0.1:3536` | Bind for the matchbox signaling server. Loopback by default — the HTTP server proxies `/signaling/{room}` to it. Set to `0.0.0.0:3536` to expose it directly. |

## Client compile-time URLs

For a deploy behind a public hostname, override the URLs at the client's
compile step:

```bash
MATCHBOX_ROOM_URL=wss://bink.eu.org/signaling \
LOBBY_WS_URL=wss://bink.eu.org/lobbies \
  cargo build --release --target wasm32-unknown-unknown
```

| Var | Default | Notes |
|---|---|---|
| `MATCHBOX_ROOM_URL` | `ws://localhost:1234/signaling` | Base of the matchbox signaling server. Defaults to the same-origin proxied path; set to e.g. `wss://example.org/signaling` for cross-origin deploys. Each lobby appends its own room name (`/lobby-{id}`). |
| `LOBBY_WS_URL` | `ws://localhost:1234/lobbies` | Lobby directory WebSocket endpoint. |

Use `wss://` so the WebSockets work from an HTTPS page.

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
