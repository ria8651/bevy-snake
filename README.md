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

Install wasm-bindgen and run:

```bash
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --out-name bevy-snake \
  --out-dir web \
  --target web target/wasm32-unknown-unknown/release/hierarchical-wfc.wasm
```

You'll then need to add a `html` file to load the generated `wasm` and `js`. Something like [this](https://github.com/bevyengine/bevy/blob/main/examples/wasm/index.html).

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
