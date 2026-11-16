# Tengen

[![CI](https://github.com/SXKA/tengen/actions/workflows/ci.yml/badge.svg)](https://github.com/SXKA/tengen/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
[![中文](https://img.shields.io/badge/語言-繁體中文-green.svg)](README.zh-TW.md)

Tengen is a freestyle gomoku engine written in Rust. It plays in the browser, compiled to WebAssembly, and in
[Gomocup](https://gomocup.org/) managers such as Piskvork as `pbrain-tengen`.

*Tengen* (天元) is the center point of the board, where most games begin.

## Play

- **In the browser**: open the website linked in the repository's About box. The engine runs locally in a
  Web Worker; nothing is sent to a server.
- **In Piskvork or another Gomocup manager**: download `pbrain-tengen` for your system from
  [Releases](https://github.com/SXKA/tengen/releases/latest) and add it as a brain. It supports 15×15 and 20×20
  boards with the freestyle rule (five or more in a row wins).

## How it works

The engine follows the design of strong classical gomoku engines, in particular
[Rapfi](https://github.com/dhbloo/rapfi), without a neural network:

- **Bitboard lines.** Stones are kept as one bitmask per color, direction and line. The 9-cell window around any
  cell is a shift away.
- **Pattern tables.** A 2¹⁸-entry table classifies every window into one of 13 line patterns (open three, four,
  and so on). Each empty cell caches its patterns for both colors, an order-independent code of the four, and their
  combination class (four-three, double three, ...). Placing a stone updates only the cells along its four lines.
- **Linear evaluation.** Every empty cell contributes a value by pattern code, plus a term for which forcing moves
  each side has. The weights are fitted to self-play results by logistic regression (Texel tuning), so they are
  measured rather than guessed.
- **Search.** Principal variation search with iterative deepening, aspiration windows and a lock-free
  transposition table; a threat-space quiescence search that finds victories by continuous fours (VCF); exact
  defences against open threes, verified by playing them; extensions for forced replies; reverse futility pruning,
  razoring, late move pruning and reductions. In the native brain, Lazy SMP spreads the search over all cores.

The [wiki](https://github.com/SXKA/tengen/wiki) (Traditional Chinese) explains each part in detail.

## Build

You need Rust 1.97 or newer.

```sh
cargo build --release -p tengen-piskvork   # target/release/pbrain-tengen
cargo test --workspace --release
target/release/pbrain-tengen bench         # node count and speed at depth 7
```

The web version needs Node.js 22 and the `wasm32-unknown-unknown` target:

```sh
rustup target add wasm32-unknown-unknown
cd web
npm ci
npm run wasm   # builds src/engine/tengen.wasm
npm run dev
```

On Vercel, set the project's root directory to `web`; `vercel.json` installs Rust during the build.

## Testing changes

A change to the search or evaluation is only kept if it wins games. `tengen-arena` plays two brains against each
other from balanced random openings, each opening twice with colors swapped, and stops when a sequential
probability ratio test (SPRT) decides:

```sh
cargo build --release -p tengen-arena
target/release/tengen-arena --new path/to/new/pbrain-tengen --base path/to/old/pbrain-tengen \
    --games 4000 --turn-ms 100 --sprt 0,10
```

The **SPRT** workflow in GitHub Actions runs the same test between a branch and `master`. The **Tune** workflow
plays self-play games with `tengen-tune`, fits new evaluation weights and checks them against the current ones.

## Layout

| Path | Contents |
| --- | --- |
| `crates/engine` | The engine library: board, patterns, evaluation, move generation, search |
| `crates/piskvork` | `pbrain-tengen`, the Gomocup protocol front end |
| `crates/arena` | `tengen-arena` (matches and SPRT) and `tengen-tune` (self-play and tuning) |
| `crates/wasm` | The WebAssembly build with a plain C ABI |
| `web` | The browser app: React, with the engine in a Web Worker |

## Acknowledgements

Tengen's pattern vocabulary, evaluation structure and search techniques are modelled on
[Rapfi](https://github.com/dhbloo/rapfi) by dhbloo and contributors. The code is an independent implementation.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
