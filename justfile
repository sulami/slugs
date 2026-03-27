wasm:
    cargo build --profile wasm-release --target wasm32-unknown-unknown
    wasm-bindgen --no-typescript --target web \
      --out-dir ./out/ \
      --out-name "slugs" \
      ./target/wasm32-unknown-unknown/wasm-release/slugs.wasm
