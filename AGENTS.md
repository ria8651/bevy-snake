# Agent notes

For build/run commands see [README.md](README.md). This file captures things an
agent working on the codebase needs to know that aren't obvious from the source.

## Architecture at a glance

Native server + wasm browser client over WebTransport (QUIC).

- [src/board.rs](src/board.rs) — pure game logic. `Board`, `tick_board`,
  collision/apple/wall handling. No I/O.
- [src/lib.rs](src/lib.rs) — wire types: `GameCommands` (Input, RestartGame),
  `GameUpdates` (Ticked).
- [src/server.rs](src/server.rs) — `start_server`, the `GameLoop` state machine,
  the per-session WebTransport task, HTTP static-file server. The game loop is
  a `tokio::select!` over: client registration, client commands, the game
  ticker, and a watchdog (see Diagnostics).
- [src/client.rs](src/client.rs) — `ClientConnection` Bevy component + the
  WebTransport task that bridges the wire to a `tokio::mpsc` channel.
- [src/game.rs](src/game.rs) — Bevy systems. `update_game` consumes
  `NetworkUpdate`s from the connection and drives client-side state.

Tick cadence is 7.5/sec (≈133 ms/tick).

## Tests

26 unit tests live in a `#[cfg(test)] mod tests` block at the bottom of
[src/server.rs](src/server.rs). They drive `GameLoop` directly through its
private methods (`register_client`, `process_command`, `tick`) — no
WebTransport, no JSON, no real networking, fast and deterministic.

```sh
cargo test --lib server::tests              # whole suite
cargo test --lib server::tests::restart_    # just the reset group
```

Coverage groups: protocol JSON round-trips, client registration, tick
processing, input handling, game reset, multi-client broadcast.

## Known issues / quirks

### Firefox breaks WebTransport after a few streams

**Use Chrome / Chromium-based browsers.** Firefox's WebTransport
implementation stops yielding incoming unidirectional streams after the first
two (verified empirically — server keeps broadcasting, client's
`session.accept_uni()` never resolves again). It's not in our code: same wasm
client works in Chrome.

The robust fix would be one persistent uni stream per direction with
length-prefixed messages instead of one stream per message. Not yet
implemented.

### The server pauses ticking when no clients are connected

[server.rs game_loop](src/server.rs) guards the ticker branch with
`if !self.clients.clients.is_empty()`, and `reset_immediately()`s on first
connect. Reason: the default board snake walks Right and dies in ~1 second; if
the server ticks idle, anyone connecting just sees a dead board. Don't
"simplify" this away.

### `RestartGame` clears `queued_inputs`

Without this, a direction queued before restart leaks into the new snake's
first tick. There's a regression test (`restart_clears_queued_inputs`).

### Game-over pause is via `ticker.reset_after(1_000_000s)`

When all snakes die, `tick()` returns `true` and the game loop calls
`ticker.reset_after(Duration::from_secs(1_000_000))` to effectively park the
ticker. `RestartGame` calls `ticker.reset_immediately()` to wake it. Looks
weird; works.

## Diagnostics in the running server

Two info-level instrumentation hooks are kept in [server.rs](src/server.rs):

- **Watchdog**: every 2s logs `watchdog: tick=N, clients=M, queued_inputs=K`.
  Silence in this log = the game loop is wedged (almost certainly inside a
  broadcast `send().await` on a slow client).
- **Broadcast timing**: `Clients::broadcast` warns if any per-client
  `send().await` exceeds 50 ms. The per-client `Sender` is bounded at capacity
  1, so a slow per-session task quickly blocks the whole game loop.

When the wire goes weird, the first thing to do is read these logs.

## When you're touching the WebTransport plumbing

Things that have already bitten us; check before "improving":

- The per-client `mpsc::channel(1)` in [server.rs `Client::new`](src/server.rs)
  is intentional — small buffer means a misbehaving client surfaces fast as a
  blocked broadcast. Tests exercise this implicitly (`disconnected_client_is_dropped_from_broadcast`).
- `web_transport::SendStream` does not expose `finish()`. Drop calls quinn's
  Drop, which does FIN cleanly. Don't try to add a manual `finish()` — there
  isn't one.
- `web_transport_wasm::Session::accept_uni` creates a fresh
  `getReader()`-locked Reader on every call. The client wraps it in
  `stream::unfold` + `tokio::pin!` so the in-flight read survives `select!`
  cancellations — preserves it from a real lock-leak bug. Keep that pattern if
  you rewrite the loop.

## Style

- Tests in `mod tests` use `#[tokio::test]`. Channels are bounded — drain
  every broadcast before the next tick or the test will hang.
- `info!`/`warn!`/`error!` from the `log` crate. `colog` is initialized in
  [src/bin/server.rs](src/bin/server.rs).
