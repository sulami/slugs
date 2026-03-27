wasm:
    cargo build --profile wasm-release --target wasm32-unknown-unknown
    wasm-bindgen --no-typescript --target web \
      --out-dir ./ \
      --out-name "slugs" \
      ./target/wasm32-unknown-unknown/wasm-release/slugs.wasm
    wasm-opt -O slugs_bg.wasm -o slugs_bg.wasm
