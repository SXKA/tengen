//! Fixed-depth benchmark over a few positions.
//!
//! The node count is a signature of the search: any change that alters it
//! changes how the engine plays, which helps tell refactors from functional
//! changes.

use crate::board::{Board, Pos};
use crate::search::{Engine, Limits};

/// Opening positions as moves from the center outward, 15x15.
const POSITIONS: &[&[(usize, usize)]] = &[
    &[(7, 7)],
    &[(7, 7), (8, 8)],
    &[(7, 7), (8, 6), (8, 8)],
    &[(7, 7), (8, 8), (7, 8), (6, 6)],
    &[(7, 7), (7, 8), (8, 7), (6, 7), (9, 7)],
    &[(7, 7), (6, 8), (8, 8), (8, 6), (9, 9), (6, 6), (10, 10)],
    &[(7, 7), (8, 7), (6, 8), (5, 9), (7, 9), (7, 8), (6, 10), (8, 8)],
];

/// Result of [`run`].
#[derive(Clone, Copy, Debug)]
pub struct Bench {
    pub nodes: u64,
    pub elapsed_ms: f64,
}

impl Bench {
    /// Nodes per second.
    pub fn nps(&self) -> u64 {
        (self.nodes as f64 / (self.elapsed_ms / 1000.0).max(1e-3)) as u64
    }
}

/// Search every position to `depth` with one thread and a fresh table.
pub fn run(depth: i32, clock: fn() -> f64) -> Bench {
    let mut nodes = 0;
    let start = clock();
    for moves in POSITIONS {
        let mut board = Board::new(15);
        for &(x, y) in moves.iter() {
            board.play(Pos::new(x, y));
        }
        let mut engine = Engine::new(16);
        engine.set_clock(clock);
        nodes += engine.search(&board, &Limits::depth(depth), |_| {}).nodes;
    }
    Bench {
        nodes,
        elapsed_ms: clock() - start,
    }
}
