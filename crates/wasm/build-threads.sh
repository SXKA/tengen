#!/bin/sh
# Build the multi-threaded WebAssembly module. It needs a standard library
# compiled with atomics, so it uses -Zbuild-std (the rust-src component) and,
# on a stable toolchain, RUSTC_BOOTSTRAP=1.
set -eu
out_dir=${1:-target}
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals \
  -C link-arg=--shared-memory -C link-arg=--max-memory=1073741824 -C link-arg=--import-memory \
  -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align \
  -C link-arg=--export=__tls_base -C link-arg=--export=__stack_pointer" \
  cargo build --manifest-path "$(dirname "$0")/Cargo.toml" --release --target wasm32-unknown-unknown \
  -Zbuild-std=std,panic_abort --target-dir "$out_dir/threads"
