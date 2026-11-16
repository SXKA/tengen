#!/bin/sh
# Build the engine for the browser into src/engine/tengen.wasm.
#
# Installs a minimal Rust toolchain with the wasm target when cargo is
# missing (as on Vercel's build machines).
set -eu
cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1; then
  if [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
  else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --target wasm32-unknown-unknown
    PATH="$HOME/.cargo/bin:$PATH"
  fi
fi
if command -v rustup >/dev/null 2>&1; then
  rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
fi

cargo build --manifest-path ../Cargo.toml -p tengen-wasm --release --target wasm32-unknown-unknown ${TENGEN_CARGO_FLAGS:-}
cp ../target/wasm32-unknown-unknown/release/tengen_wasm.wasm src/engine/tengen.wasm
echo "built src/engine/tengen.wasm ($(wc -c < src/engine/tengen.wasm) bytes)"
