# Bevy Snake

A simple snake game, inspired by google snake. Adds multiplayer and GUNS!!

## Building

Just clone the repo and run:

```bash
cargo run --release
```

Hopefully it'll work `¯\_(ツ)_/¯`

### Web

Install wasm-bindgen and run:

```bash
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --out-name bevy-snake \
  --out-dir web \
  --target web target/wasm32-unknown-unknown/release/hierarchical-wfc.wasm
```
