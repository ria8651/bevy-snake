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

`[src/bin/server.rs](src/bin/server.rs)` runs the matchbox signaling endpoint
**and** serves `web/`. It does not host gameplay — all sim runs in clients.

## Architecture

Peer-to-peer rollback netcode. Every client runs the same deterministic
simulation; GGRS handles input delivery, rollback, and desync detection.

- `[src/main.rs](src/main.rs)` — `App` setup. `ClientState` state machine:
  `Lobby → WaitingForOpponent → Playing`. `drive_state` flips
  `WaitingForOpponent ↔ Playing` based on `Session<GameConfig>` presence.
- `[src/board.rs](src/board.rs)` — pure game logic. `Board::tick`, `Snake`,
  `BoardSettings`, `Direction`, `BoardEvent`. No I/O, no RNG side effects —
  takes `&mut StdRng` so the caller controls determinism.
- `[src/settings.rs](src/settings.rs)` — `GameSettings { board, speed }`
  resource, edited from the lobby UI, read by the net + UI plugins.
- `[src/net.rs](src/net.rs)` — `NetPlugin`. Owns GGRS plugin registration,
  rollback resources (`Board`, `MovementFrame`, `RngState`, `InputQueues`),
  matchbox socket lifecycle, input buffering. `start_session` runs
  `OnEnter(WaitingForOpponent)` — 1-player goes through `Session::SyncTest`,
  2+ opens a `MatchboxSocket` and `wait_for_players` builds the P2P session
  once peers connect.
- `[src/render.rs](src/render.rs)` — sprite-based board rendering with
  interpolated snake positions (`MovementFrame::movement_progress`).
- `[src/ui.rs](src/ui.rs)` — Bevy Feathers widgets. Per-state UI:
  `OnEnter(Lobby)` spawns the settings panel, `OnEnter(WaitingForOpponent)`
  spawns the dim overlay, `OnEnter(Playing)` spawns the score HUD.
- `[src/bin/server.rs](src/bin/server.rs)` — axum static files + matchbox
  signaling. Native-only (`#[cfg(not(target_arch = "wasm32"))]`).

## Determinism rules (rollback)

Everything in a rollback resource must be deterministic across peers. If a
new resource needs rollback, register it with
`app.rollback_resource_with_clone::<T>()` in `NetPlugin::build`.

- Use the `RngState` seed + `MovementFrame.frame/generation` to derive any
  in-tick RNG. Never call `rand::random`/`thread_rng` from inside
  `GgrsSchedule`. Solo-mode seeding in `start_session` uses
  `rand::random::<u64>()` only because that happens **outside** the
  rollback schedule.
- `option_env!("MATCHBOX_ROOM_URL")` is compile-time; `?next={N}` is
  appended at runtime from `GameSettings.board.players`.
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
  `Playing` does nothing; the player has to leave back to `Lobby` first.
- **1-player skips matchbox entirely** — no socket, no session URL. If
  you're testing networking, pick 2+ in the lobby.

## Style

- Prefer `Res<GameSettings>` over re-introducing constants like
  `NUM_PLAYERS` or `FRAMES_PER_MOVEMENT` — those were removed deliberately.
- Game logic stays in `board.rs` (testable, no Bevy types in its public
  API beyond what `serde` needs). Bevy systems live in `net.rs` / `ui.rs`
  / `render.rs`.
- Tests use the in-source `#[cfg(test)] mod tests` pattern (see bottom of
  `board.rs`).
