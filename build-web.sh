#!/usr/bin/env bash
# Builds the browser version into dist/, ready to serve as static files.
set -euo pipefail

WASM_BINDGEN_VERSION="0.2.100" # must match the wasm-bindgen crate in Cargo.lock

command -v wasm-bindgen >/dev/null || {
  echo "wasm-bindgen not found. Install it with:" >&2
  echo "  cargo install wasm-bindgen-cli --version ${WASM_BINDGEN_VERSION}" >&2
  exit 1
}

rustup target add wasm32-unknown-unknown

cargo build --lib --release --target wasm32-unknown-unknown

rm -rf dist
mkdir -p dist
wasm-bindgen --target web --no-typescript \
  --out-dir dist \
  target/wasm32-unknown-unknown/release/word_legend_app.wasm

# Stamp the page with a hash of the bundle, so each build is fetched under new
# URLs rather than served from a browser's ten-minute cache of the last one.
BUILD="$(cat dist/word_legend_app.js dist/word_legend_app_bg.wasm | sha256sum | cut -c1-12)"
sed "s/__BUILD__/${BUILD}/g" index.html > dist/index.html
grep -q "__BUILD__" dist/index.html && { echo "index.html was not stamped" >&2; exit 1; }
# Without this, GitHub Pages runs the output through Jekyll.
touch dist/.nojekyll

echo
echo "dist/ built (build ${BUILD}):"
du -h dist/* | sort -h
echo
echo "Serve locally with:  python3 -m http.server -d dist 8080"
