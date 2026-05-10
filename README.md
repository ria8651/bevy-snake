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
  --out-dir web \
  --target web target/wasm32-unknown-unknown/release/bevy-snake.wasm
```

Then run the server, which serves `web/` over HTTP and runs the WebTransport endpoint:

```bash
cargo run --bin server --release
# open http://localhost:8080
```

The server generates a fresh self-signed cert in memory at startup and injects
its SHA-256 hash into `web/index.html` at request time, so the wasm client picks
up the current hash automatically. No cert files on disk, no manual
copy-pasting between restarts.

Environment overrides:

| Var | Default | Notes |
|---|---|---|
| `WT_ADDR` | `0.0.0.0:1234` | UDP/QUIC bind for WebTransport |
| `HTTP_ADDR` | `0.0.0.0:8080` | TCP bind for static-file + HTML server |
| `WT_URL` | `https://localhost:1234` | URL injected into `window.WT_URL` |
| `CERT_SANS` | `localhost,127.0.0.1` | Comma-separated SANs for the generated cert |

For the production-style setup (e.g. behind a public hostname), set
`CERT_SANS=bink.eu.org` and `WT_URL=https://bink.eu.org:1234`.

The reference WebTransport JS bootstrap (used internally by the wasm client) is:

```js
const transport = new WebTransport(window.WT_URL, {
  serverCertificateHashes: [
    {
      algorithm: "sha-256",
      value: new Uint8Array(window.WT_CERT_HASH.match(/../g).map(h => parseInt(h, 16))).buffer,
    },
  ],
});
```

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
