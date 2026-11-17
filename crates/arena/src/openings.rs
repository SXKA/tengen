//! Balanced openings: random ones a short search judges even, or a book
//! of openings a stronger engine judged even (`tengen-tune openings`).

use tengen_engine::pattern::Pattern4;
use tengen_engine::{Board, Engine, Limits, Pos, Rule};

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
pub fn opening(size: usize, rule: Rule, rng: &mut Rng, engine: &mut Engine) -> Vec<Pos> {
    let center = size / 2;
    loop {
        let stones = 2 + rng.below(4);
        let mut board = Board::with_rule(size, rule);
        let mut moves = Vec::new();
        while moves.len() < stones {
            let (x, y) = (center - 3 + rng.below(7), center - 3 + rng.below(7));
            let pos = Pos::new(x, y);
            let near = moves.is_empty() || board.is_candidate(pos);
            if near && board.is_legal(pos) {
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
        if value.abs() <= 150 {
            return moves;
        }
    }
}

/// Openings from a book file: one per line, moves as `X,Y` separated by
/// spaces; anything after a `;`, blank lines and `#` comments are ignored.
pub fn load_book(path: &str) -> Result<Vec<Vec<Pos>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut book = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.split(';').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let moves = line
            .split_whitespace()
            .map(|word| {
                let (x, y) = word.split_once(',')?;
                Some(Pos::new(x.parse().ok()?, y.parse().ok()?))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(format!("{path}:{}: bad move list", number + 1))?;
        book.push(moves);
    }
    if book.is_empty() {
        return Err(format!("{path}: no openings"));
    }
    Ok(book)
}

/// The order a run plays a book's openings in: shuffled by `seed`, then
/// repeated if there are more pairs than openings.
pub fn book_order(len: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    let mut rng = Rng(seed ^ 0x6f70_656e_696e_6773);
    for i in (1..len).rev() {
        order.swap(i, rng.below(i + 1));
    }
    order
}
