# Tengen

[![CI](https://github.com/SXKA/tengen/actions/workflows/ci.yml/badge.svg)](https://github.com/SXKA/tengen/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
[![中文](https://img.shields.io/badge/語言-繁體中文-green.svg)](README.zh-TW.md)

Tengen is a gomoku and renju engine written in Rust. It plays in the browser, compiled to WebAssembly, and in
[Gomocup](https://gomocup.org/) managers such as Piskvork as `pbrain-tengen`.

*Tengen* (天元) is the center point of the board, where most games begin.

## Play

- **In the browser**: open [tengen-bice.vercel.app](https://tengen-bice.vercel.app). The engine runs locally in a
  Web Worker; nothing is sent to a server.
- **Watch engines play**: the site's *Watch engines* mode sets two engines against each other, each in its own
  worker in your browser: Tengen, [Rapfi](https://github.com/dhbloo/rapfi) (with its NNUE, downloaded when chosen,
  or its classical evaluation), [SlowRenju](https://github.com/wind23/SlowRenju) by Tianyi Hao and Pela by Petr
  Laštovička from [Piskvork](https://github.com/plastovicka/Piskvork). Stones can show their move numbers in both
  modes.
- **In Piskvork or another Gomocup manager**: download `pbrain-tengen` for your system from
  [Releases](https://github.com/SXKA/tengen/releases/latest) and add it as a brain. It supports boards from 5×5 to
  20×20 and every rule of the Piskvork protocol.

## Rules

| `INFO rule` | Rule | Wins |
| --- | --- | --- |
| 0 | Freestyle | Five or more in a row |
| 1 | Standard | Exactly five; six or more does not win |
| 4 | Renju | Black: exactly five, and Black may not make an overline, two fours or two open threes at once. White: five or more |
| 8 | Freestyle caro | Six or more, or five not blocked by stones at both ends |
| 9 | Caro (Gomocup) | Exactly five, not blocked by stones at both ends; the board edge does not block |

Adding 2 to the rule number (a continuous game) is accepted too: stones the manager marks as belonging to nobody
(`BOARD` field 3) block lines for both sides. The brain also answers `SWAP2BOARD` for swap2 openings.

The win conditions and renju's forbidden points are checked in CI against the
[Gomocup judge](https://github.com/Gomocup/GomocupJudge) on thousands of random positions.

## How it works

The engine follows the design of strong classical gomoku engines, in particular
[Rapfi](https://github.com/dhbloo/rapfi), without a neural network:

- **Bitboard lines.** Stones are kept as one bitmask per color, direction and line. The window around any cell is a
  shift away.
- **Pattern tables.** A 2¹⁸-entry table classifies every 9-cell window into one of 13 line patterns (open three,
  four, and so on). The other rules look one cell further, to tell a five from an overline and, in caro, a stone
  from the board edge, with 2²⁰-entry tables per rule and color. Each empty cell caches its patterns for both colors,
  an order-independent code of the four, and their combination class (four-three, double three, ...). Placing a
  stone updates only the cells along its four lines.
- **Renju.** Overlines and double fours are read straight from the tables. A double three is forbidden only if both
  threes can become straight fours by moves that are not forbidden themselves; that is checked by trying the moves,
  recursively. Black never plays a forbidden point, and White wins when a four can only be blocked on one.
- **Linear evaluation.** Every empty cell contributes a value by pattern code, plus a term for which forcing moves
  each side has. There are two tables, one for the opening and one for later in the game, blended by the number of
  stones (a tapered evaluation). The weights are fitted to self-play results by logistic regression (Texel tuning),
  so they are measured rather than guessed.
- **Learned move ordering.** Moves are ordered by per-code scores for what they build and what they block, fitted to
  the moves chosen in self-play.
- **Search.** Principal variation search with iterative deepening, aspiration windows and a lock-free
  transposition table; a threat-space quiescence search that finds victories by continuous fours (VCF); exact
  defences against open threes, verified by playing them; extensions for forced replies; reverse futility pruning,
  razoring, late move pruning and reductions, and fail-high reductions. Lazy SMP spreads the search over all cores, in the browser too: extra
  Web Workers join the search through shared WebAssembly memory.

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

The multi-threaded build (`tengen-mt.wasm`) also needs the `rust-src` component. On Vercel, set the project's root
directory to `web`; the build script installs Rust, and `vercel.json` sends the cross-origin isolation headers that
shared memory requires.

## Testing changes

A change to the search or evaluation is only kept if it wins games. `tengen-arena` plays two brains against each
other from balanced random openings, each opening twice with colors swapped, and stops when a sequential
probability ratio test (SPRT) decides:

```sh
cargo build --release -p tengen-arena
target/release/tengen-arena --new path/to/new/pbrain-tengen --base path/to/old/pbrain-tengen \
    --games 4000 --turn-ms 100 --sprt 0,10
```

The **SPRT** workflow in GitHub Actions runs the same test between a branch and `master`, under any rule. The
**Tune** workflow plays self-play games with `tengen-tune` under a chosen rule, fits new evaluation weights and
checks them against the current ones; each rule can have its own weights (`crates/engine/src/weights-RULE.txt`,
falling back to the freestyle `weights.txt`). The **Versus Rapfi** workflow builds
[Rapfi](https://github.com/dhbloo/rapfi) and plays a fixed match against it, with its NNUE and with its classical
evaluation, to measure the distance to the strongest open-source engine.

The other engines on the website are built by two manual workflows, which push the files to a branch for a pull
request: **Rapfi WASM** compiles Rapfi with Emscripten, and **Classic engines WASM** compiles Pela and SlowRenju,
which were written for Windows, with the compatibility layer in `engines/`. That layer is tested natively too:
`sh engines/build.sh pela PATH_TO_PISKVORK out native` builds a `pbrain-pela` that speaks the Piskvork protocol.

## Layout

| Path | Contents |
| --- | --- |
| `crates/engine` | The engine library: board, patterns, evaluation, move generation, search |
| `crates/piskvork` | `pbrain-tengen`, the Gomocup protocol front end |
| `crates/arena` | `tengen-arena` (matches and SPRT) and `tengen-tune` (self-play and tuning) |
| `crates/wasm` | The WebAssembly builds (single- and multi-threaded) with a plain C ABI |
| `web` | The browser app: React, with the engine in a Web Worker |
| `web/public/engines` | The other engines for engine matches, built by workflows, with their notices |
| `engines` | What Pela and SlowRenju need to build outside Windows: a protocol front end, shims, a patch |

## Acknowledgements

Tengen's pattern vocabulary, evaluation structure and search techniques are modelled on
[Rapfi](https://github.com/dhbloo/rapfi) by dhbloo and contributors. The code is an independent implementation.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

The website also serves builds of other engines under their own licenses: Rapfi (GPL-3.0) with networks from
[rapfi-networks](https://github.com/dhbloo/rapfi-networks) (CC0), SlowRenju (GPL-3.0) and Pela (GPL). Each
folder in `web/public/engines` has a `NOTICE.md` naming the exact source it was built from.
