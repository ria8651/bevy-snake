# Agent notes

For build/run commands see [README.md](README.md). This file captures things an
agent working on the codebase needs to know that aren't obvious from the source.

## Architecture at a glance

Native server + wasm browser client over WebTransport (QUIC).

- [src/board.rs](src/board.rs) — pure game logic. `Board`, `tick_board`
  (server, with RNG for apple/wall spawn), `tick_board_no_spawn` (client,
  deterministic, no RNG), collision/apple handling. No I/O.
- [src/lib.rs](src/lib.rs) — wire types: `GameCommands` (Input, RestartGame),
  `GameUpdates::Ticked` (includes `applied_inputs` so the client can tell
  "server didn't see my input" from "I predicted wrong").
- [src/transport.rs](src/transport.rs) — `encode_framed` / `decode_payload`
  (u32-BE length + bincode), `read_frame` helper, and a test-only
  `MockTransport` pair with configurable drop/reorder/delay.
- [src/server.rs](src/server.rs) — `start_server`, the `GameLoop` state
  machine, the per-session WebTransport task (`per_session`, `send_loop`,
  `recv_loop`, `datagram_recv_loop`), HTTP static-file server.
- [src/client.rs](src/client.rs) — `ClientConnection` Bevy component + the
  wasm WebTransport task. Reliable framed stream for snapshots /
  RestartGame; datagrams for `GameCommands::Input` via `send_datagram`.
- [src/game.rs](src/game.rs) — Bevy systems. `update_game` runs client-side
  prediction: receives a snapshot → trusts authoritative state → drops
  ack'd inputs from `LocalInputLog` → replays remaining inputs via
  `tick_board_no_spawn`. Between snapshots it advances the predicted
  `Board` on the local `TickTimer` so the player sees immediate feedback.

Tick cadence is 7.5/sec (≈133 ms/tick).

## Netcode model

Server is authoritative. Client predicts forward from each snapshot using
`Board::tick_board_no_spawn` — same deterministic logic minus the
RNG-driven apple / wall spawning. New apples appear with the next server
snapshot; the client never has RNG (anti-cheat: knowing future apple
positions would be a huge advantage).

Each `GameUpdates::Ticked` carries `applied_inputs`: one slot per snake id
with the direction the server actually applied this tick. Clients use it
to predict opponents' next moves with a "constant velocity" assumption
(reuse last applied direction) and to reconcile their own snake.

Wire transport split:

- **Reliable** (one persistent uni-stream per direction, `u32` length +
  bincode): snapshots (server→client), RestartGame (client→server).
- **Unreliable** (WebTransport datagrams, single bincode payload each):
  `GameCommands::Input` (client→server). Loss is tolerated by the
  server's per-tick HashMap dedup; reorder is harmless. At 7.5 Hz the
  next snapshot will overwrite any divergence within 133 ms.

## Tests

Unit tests are split across three modules:

- `server::tests` (26 tests) — drives `GameLoop` directly through its
  private methods (`register_client`, `process_command`, `tick`). No
  WebTransport, no real networking. Covers: protocol round-trips, client
  registration, tick processing, input handling, game reset, multi-client
  broadcast.
- `transport::tests` and `transport::mock` (8 tests) — framed encoding
  round-trips, frame-too-large rejection, datagram payload round-trip,
  and `MockTransport` with simulated loss/reorder/delay.
- `board::tests` (3 tests) — that `tick_board_no_spawn` matches
  `tick_board` when no apples are eaten, that the spawn count is
  reported correctly, and that client prediction (no RNG) matches server
  output when nothing has been eaten in the predicted window.

```sh
cargo test --lib                            # whole suite
cargo test --lib server::tests::restart_    # just the reset group
cargo test --lib transport                  # transport + mock
```

## Known issues / quirks

### Firefox WebTransport (was: broken after a few streams)

The previous "Firefox stops yielding incoming uni-streams after ~2" bug is
mitigated by using one persistent uni-stream per direction with
length-prefixed bincode frames. Worth re-testing Firefox after any change
to the framing path; if you see snapshots stop flowing in Firefox while
Chrome continues, suspect that the persistent stream got recreated.

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
  cancellations — preserves it from a real lock-leak bug. The framed-read
  path (`frame_stream` in [src/client.rs](src/client.rs)) is wrapped the
  same way: the read state (RecvStream + scratch buffer) lives inside the
  unfold so a cancelled future doesn't drop a half-read frame.
- The reliable framed stream is opened once per session. Don't open a new
  uni-stream per message; that's what triggered the Firefox bug.
- Datagrams are best-effort by design. The server's `datagram_recv_loop`
  logs+drops a malformed datagram rather than killing the session.
- Inputs are routed by variant on the client (`send_command_routed` in
  [src/client.rs](src/client.rs)): `GameCommands::Input` → datagram,
  everything else → reliable framed stream. If you add a new variant,
  pick the right wire path explicitly.

## Prediction & reconcile (client)

`update_game` in [src/game.rs](src/game.rs) maintains:

- `Board` (rendered) — predicted state.
- `AuthoritativeBoard`, `AuthoritativeTick`, `LastAppliedInputs` — last
  snapshot from server.
- `LocalInputLog` — local inputs sent but not yet ack'd, keyed by the tick
  they were submitted for.
- `PredictedTick` — how far the predicted board has advanced.

Each Bevy frame:

1. If a snapshot arrived: replace authoritative state, drop ack'd inputs,
   start from authoritative `Board` and replay remaining input_log entries
   via `tick_board_no_spawn`. Reset the `TickTimer` so the next predicted
   tick aligns with the server's cadence.
2. If the local `TickTimer` just finished and we haven't received a
   snapshot this frame: advance the predicted `Board` by one tick using
   the head of the input log for our snake and `LastAppliedInputs` for
   others.
3. Collect key presses, push into the visual input queue and the
   `LocalInputLog`, send the first one immediately as a datagram.

Anti-cheat: the client never has RNG. Apple/wall spawns happen only
server-side, then propagate via the next snapshot's full Board.

## Style

- Tests in `mod tests` use `#[tokio::test]`. Channels are bounded — drain
  every broadcast before the next tick or the test will hang.
- `info!`/`warn!`/`error!` from the `log` crate. `colog` is initialized in
  [src/bin/server.rs](src/bin/server.rs).
