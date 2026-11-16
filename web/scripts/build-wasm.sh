#!/bin/sh
# Build the engine for the browser: src/engine/tengen.wasm (one thread) and
# src/engine/tengen-mt.wasm (shared memory, several threads).
#
# Builds with the same Rust release as CI, installed through rustup when
# needed. Vercel's build machines come with an older cargo that cannot
# compile the engine, so rustup is installed next to it and takes
# precedence. The threaded build compiles the standard library with atomics,
# which needs the rust-src component.
set -eu
cd "$(dirname "$0")/.."

RUST_VERSION=1.97.0
if ! command -v rustup >/dev/null 2>&1; then
  if [ ! -x "$HOME/.cargo/bin/rustup" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
  fi
  PATH="$HOME/.cargo/bin:$PATH"
fi
rustup toolchain install "$RUST_VERSION" --profile minimal \
  --target wasm32-unknown-unknown --component rust-src >/dev/null
export RUSTUP_TOOLCHAIN="$RUST_VERSION"

target=../target
cargo build --manifest-path ../Cargo.toml -p tengen-wasm --release --target wasm32-unknown-unknown ${TENGEN_CARGO_FLAGS:-}
cp "$target/wasm32-unknown-unknown/release/tengen_wasm.wasm" src/engine/tengen.wasm
../crates/wasm/build-threads.sh "$target"
cp "$target/threads/wasm32-unknown-unknown/release/tengen_wasm.wasm" src/engine/tengen-mt.wasm
echo "built tengen.wasm ($(wc -c < src/engine/tengen.wasm) bytes) and tengen-mt.wasm ($(wc -c < src/engine/tengen-mt.wasm) bytes)"
