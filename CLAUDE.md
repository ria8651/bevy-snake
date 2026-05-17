# CLAUDE.md

Multiplayer snake game (Bevy 0.18 + GGRS rollback over matchbox WebRTC).
Native + wasm clients. See [README.md](README.md) for full build details.

## Build & run

```bash
# native dev
cargo run --release        # use --release on macOS — debug build hits an
                           # objc2 NSScreen panic in winit/picking on arm64

# wasm + signaling server
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --no-typescript --out-name bevy-snake \
  --out-dir web --target web \
  target/wasm32-unknown-unknown/release/bevy-snake.wasm
cargo run --bin server --release    # http://localhost:1234

# tests
cargo test --lib                    # board logic tests only
```

`[src/bin/server.rs](src/bin/server.rs)` runs the matchbox signaling
endpoint, hosts the lobby directory at `/lobbies`, **and** serves `web/`.
It does not host gameplay — all sim runs in clients.

## Architecture

Peer-to-peer rollback netcode with a thin lobby-discovery layer on top.
Every client runs the same deterministic simulation; GGRS handles input
delivery, rollback, and desync detection.

- `[src/main.rs](src/main.rs)` — `App` setup. `ClientState` state machine:
  `Browsing → Creating → WaitingForOpponent → Playing → Finished`.
  `drive_state` flips `WaitingForOpponent ↔ Playing` based on
  `Session<GameConfig>` presence, and routes `Playing → Finished/Browsing`
  on session loss / all-snakes-dead.
- `[src/board.rs](src/board.rs)` — pure game logic. `Board::tick`, `Snake`,
  `BoardSettings`, `Direction`, `BoardEvent`. No I/O, no RNG side effects —
  takes `&mut StdRng` so the caller controls determinism.
- `[src/settings.rs](src/settings.rs)` — `GameSettings { board, speed }`
  resource. Edited from the Creating UI, serialized to/from the host's
  lobby record so joiners inherit it.
- `[src/lobby_proto.rs](src/lobby_proto.rs)` — wire types shared between
  client and server (`Lobby`, `LobbyState`, `ClientMsg`, `ServerMsg`).
- `[src/lobby.rs](src/lobby.rs)` — `LobbyPlugin`. Owns one WebSocket to
  the lobby service via `ewebsock`. Resources: `LobbyClient` (NonSend —
  ws handles are `!Send` on wasm), `LobbyList`, `CurrentLobby`. Maintains
  the lobby list + drives `ClientState` transitions on `LobbyCreated` /
  `LobbyStarting` / `Kicked`.
- `[src/net.rs](src/net.rs)` — `NetPlugin`. Owns GGRS plugin registration,
  rollback resources, matchbox socket lifecycle, input buffering.
  `start_session` runs `OnEnter(WaitingForOpponent)` and either takes the
  solo synctest path or opens a `MatchboxSocket` to the lobby's room.
  `wait_for_players` gates session build on the Start roster broadcast
  from the lobby server (not on a peer count).
- `[src/render.rs](src/render.rs)` — sprite-based board rendering with
  interpolated snake positions (`MovementFrame::movement_progress`).
- `[src/ui.rs](src/ui.rs)` — Bevy Feathers widgets. Per-state UI:
  `OnEnter(Browsing)` spawns the lobby list, `OnEnter(Creating)` the
  settings panel + Host button, `OnEnter(WaitingForOpponent)` the dim
  overlay with the Start button (host only), `OnEnter(Playing)` the score
  HUD, `OnEnter(Finished)` the game-over screen.
- `[src/bin/server.rs](src/bin/server.rs)` — axum static files + matchbox
  signaling + lobby `/lobbies` WS service. In-memory lobby registry,
  heartbeat GC, no persistence. Native-only.

## Determinism rules (rollback)

Everything in a rollback resource must be deterministic across peers. If a
new resource needs rollback, register it with
`app.rollback_resource_with_clone::<T>()` in `NetPlugin::build`.

- Use the `RngState` seed + `MovementFrame.frame/generation` to derive any
  in-tick RNG. Never call `rand::random`/`thread_rng` from inside
  `GgrsSchedule`. Solo-mode seeding in `start_session` uses
  `rand::random::<u64>()` only because that happens **outside** the
  rollback schedule.
- `option_env!("MATCHBOX_ROOM_URL")` and `option_env!("LOBBY_WS_URL")` are
  compile-time. The matchbox base is composed with the lobby's room name
  at runtime (`{base}/lobby-{id}`); player count is no longer in the URL.
- The Start roster broadcast from the lobby server is the readiness
  signal — every peer arrives at the same `Vec<PeerId>` independently and
  builds GGRS players in that order, which deterministically assigns
  handles 0..n across the network.
- Adding a non-deterministic call (system time, OS RNG, file I/O) inside
  `GgrsSchedule` will desync peers. Desync detection is on
  (`DesyncDetection::On { interval: 10 }`), so it'll be loud in logs.

## Gotchas

- **macOS dev build panic**: `objc2-foundation` NSEnumerator panics on
  Apple Silicon when Feathers / `bevy_picking` initializes. Workaround is
  `cargo run --release`. Upstream issue, not in our code.
- **57 MB wasm**: `[profile.release] debug = true` in Cargo.toml bakes
  symbols into the wasm. Run `wasm-opt -Os -g0` (or flip `debug = false`)
  before deploying.
- **Settings lock at session start**: `start_session` clones
  `GameSettings` into the rollback resources. Changing settings during
  `Playing` does nothing; the player has to leave back to `Browsing` first.
- **Player count is set at Start, not in settings**: `GameSettings.board.players`
  is overwritten by `start_session` / `wait_for_players` from the Start
  roster size. Don't put it in the lobby UI; the host decides by waiting
  for players to arrive and then clicking Start.
- **Solo skips matchbox + lobby entirely** — clicking Solo Play goes
  straight to `WaitingForOpponent` with `CurrentLobby.role == Role::Solo`,
  no WS, no socket. If you're testing networking, use Create/Join.
- **`LobbyClient` is NonSend**: `ewebsock`'s wasm backend stores
  `Rc<WebSocket>`, which is `!Send`. The plugin uses `NonSendMut` so the
  same code works on native and wasm. Observers touching `LobbyClient`
  must take `NonSendMut`, not `ResMut`.

## Style

- Prefer `Res<GameSettings>` over re-introducing constants like
  `NUM_PLAYERS` or `FRAMES_PER_MOVEMENT` — those were removed deliberately.
- Game logic stays in `board.rs` (testable, no Bevy types in its public
  API beyond what `serde` needs). Bevy systems live in `net.rs` / `ui.rs`
  / `render.rs`.
- Tests use the in-source `#[cfg(test)] mod tests` pattern (see bottom of
  `board.rs`).
