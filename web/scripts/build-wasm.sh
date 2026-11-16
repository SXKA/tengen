#!/bin/sh
# Build the engine for the browser: src/engine/tengen.wasm (one thread) and
# src/engine/tengen-mt.wasm (shared memory, several threads).
#
# Installs a minimal Rust toolchain when cargo is missing (as on Vercel's
# build machines). The threaded build compiles the standard library with
# atomics, which needs the rust-src component.
set -eu
cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1; then
  if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  fi
  PATH="$HOME/.cargo/bin:$PATH"
fi
if command -v rustup >/dev/null 2>&1; then
  rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
  rustup component add rust-src >/dev/null 2>&1 || true
fi

target=../target
cargo build --manifest-path ../Cargo.toml -p tengen-wasm --release --target wasm32-unknown-unknown ${TENGEN_CARGO_FLAGS:-}
cp "$target/wasm32-unknown-unknown/release/tengen_wasm.wasm" src/engine/tengen.wasm
../crates/wasm/build-threads.sh "$target"
cp "$target/threads/wasm32-unknown-unknown/release/tengen_wasm.wasm" src/engine/tengen-mt.wasm
echo "built tengen.wasm ($(wc -c < src/engine/tengen.wasm) bytes) and tengen-mt.wasm ($(wc -c < src/engine/tengen-mt.wasm) bytes)"
