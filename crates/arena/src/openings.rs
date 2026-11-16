//! Balanced random openings.

use tengen_engine::pattern::Pattern4;
use tengen_engine::{Board, Engine, Limits, Pos};

/// Tiny deterministic generator (splitmix64).
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// A random opening near the center that a short search judges balanced.
pub fn opening(size: usize, rng: &mut Rng, engine: &mut Engine) -> Vec<Pos> {
    let center = size / 2;
    loop {
        let stones = 2 + rng.below(4);
        let mut board = Board::new(size);
        let mut moves = Vec::new();
        while moves.len() < stones {
            let (x, y) = (center - 3 + rng.below(7), center - 3 + rng.below(7));
            let pos = Pos::new(x, y);
            let near = moves.is_empty() || board.is_candidate(pos);
            if board.is_empty(pos) && near {
                board.play(pos);
                moves.push(pos);
            }
        }
        let us = board.side_to_move();
        let quiet = board.p4_count_at_least(us, Pattern4::HFlex3) == 0
            && board.p4_count_at_least(us.opponent(), Pattern4::HFlex3) == 0;
        if !quiet {
            continue;
        }
        let value = engine.search(&board, &Limits::depth(6), |_| {}).value;
        if value.abs() <= 120 {
            return moves;
        }
    }
}
