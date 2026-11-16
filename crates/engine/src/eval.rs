//! Static evaluation.
//!
//! The evaluation is linear so it can be tuned by logistic regression on game
//! results (Texel tuning):
//!
//! * every empty cell contributes a value per color, looked up by its pattern
//!   code: the sum of its four line-pattern weights plus a bonus for their
//!   combination;
//! * a threat term, indexed by which kinds of forcing moves each side has,
//!   captures what the side to move can do with its tempo.
//!
//! The result is from the side to move's point of view, in units where
//! [`SCALE`] maps to a win probability of about 73%.

use std::sync::OnceLock;

use crate::board::{Board, Color, Pos};
use crate::pattern::{Pattern, Pattern4, PatternCode, Tables, PCODE_COUNT};

/// Evaluation score. Mate scores live above [`crate::search::MATE_BOUND`].
pub type Value = i32;

/// Eval units per logistic unit: `win_probability = 1 / (1 + exp(-v / SCALE))`.
pub const SCALE: f64 = 200.0;

/// Number of threat-mask entries.
pub const THREAT_COUNT: usize = 1 << 6;

/// Tunable evaluation weights.
#[derive(Clone, Debug, PartialEq)]
pub struct Weights {
    /// Value of each line pattern on an empty cell.
    pub pattern: [i32; Pattern::COUNT],
    /// Bonus for each pattern combination on an empty cell.
    pub pattern4: [i32; Pattern4::COUNT],
    /// Threat term, indexed by [`threat_mask`].
    pub threat: [i32; THREAT_COUNT],
}

impl Default for Weights {
    fn default() -> Weights {
        DEFAULT_WEIGHTS.clone()
    }
}

/// Hand-set starting point, refined by `tengen-tune`.
pub const DEFAULT_WEIGHTS: Weights = Weights {
    //        Dead B1 F1 B2 F2 F2A F2B B3 F3 F3S B4  F4   F5
    pattern: [0, 1, 2, 3, 8, 10, 12, 10, 30, 36, 24, 100, 250],
    //         None L  K  J   I  H   G   F  E   D    C    B    A
    pattern4: [0, 0, 0, 8, 10, 0, 20, 80, 0, 25, 150, 300, 600],
    threat: default_threats(),
};

/// Bits of the threat mask.
mod bit {
    pub const SELF_FLEX4: usize = 1 << 0;
    pub const SELF_FOUR: usize = 1 << 1;
    pub const SELF_FLEX3: usize = 1 << 2;
    pub const OPP_FLEX4: usize = 1 << 3;
    pub const OPP_FOUR: usize = 1 << 4;
    pub const OPP_FLEX3: usize = 1 << 5;
}

/// Default bonus for the side to move.
const TEMPO: i32 = 60;

const fn default_threats() -> [i32; THREAT_COUNT] {
    let mut out = [0; THREAT_COUNT];
    let mut mask = 0;
    while mask < THREAT_COUNT {
        let opp_forcing = (mask & bit::OPP_FOUR != 0) || (mask & bit::OPP_FLEX4 != 0);
        let self_forcing = (mask & bit::SELF_FOUR != 0) || (mask & bit::SELF_FLEX4 != 0);
        // Having the move is worth about half a stone's potential.
        let mut v = TEMPO;
        if mask & bit::SELF_FLEX4 != 0 {
            v += if opp_forcing { 300 } else { 1500 };
        }
        if mask & bit::SELF_FOUR != 0 {
            v += 60;
        }
        if mask & bit::SELF_FLEX3 != 0 {
            v += if opp_forcing { 40 } else { 150 };
        }
        if mask & bit::OPP_FLEX4 != 0 {
            v -= if self_forcing { 200 } else { 600 };
        }
        if mask & bit::OPP_FOUR != 0 {
            v -= 30;
        }
        if mask & bit::OPP_FLEX3 != 0 {
            v -= 50;
        }
        out[mask] = v;
        mask += 1;
    }
    out
}

/// Value of an empty cell for one color, per pattern code.
pub struct CellValues {
    values: Vec<i32>,
    threat: [i32; THREAT_COUNT],
}

impl CellValues {
    /// Tables for the default weights.
    pub fn standard() -> &'static CellValues {
        static VALUES: OnceLock<CellValues> = OnceLock::new();
        VALUES.get_or_init(|| CellValues::new(&DEFAULT_WEIGHTS))
    }

    /// Tables for arbitrary weights.
    pub fn new(weights: &Weights) -> CellValues {
        let tables = Tables::get();
        let values = (0..PCODE_COUNT as PatternCode)
            .map(|code| {
                let lines: i32 = tables.patterns(code).iter().map(|&p| weights.pattern[p as usize]).sum();
                lines + weights.pattern4[tables.pattern4(code) as usize]
            })
            .collect();
        CellValues { values, threat: weights.threat }
    }

    /// Value of a cell with pattern code `code`.
    #[inline]
    pub fn value(&self, code: PatternCode) -> i32 {
        self.values[code as usize]
    }
}

/// Which kinds of forcing moves each side has, for the threat term.
pub fn threat_mask(board: &Board, us: Color) -> usize {
    let them = us.opponent();
    let flex3 = |c| {
        board.p4_count(c, Pattern4::HFlex3)
            + board.p4_count(c, Pattern4::GFlex3Plus)
            + board.p4_count(c, Pattern4::FFlex3x2)
            > 0
    };
    let four = |c| {
        board.p4_count(c, Pattern4::EBlock4)
            + board.p4_count(c, Pattern4::DBlock4Plus)
            + board.p4_count(c, Pattern4::CBlock4Flex3)
            > 0
    };
    let flex4 = |c| board.p4_count(c, Pattern4::BFlex4) > 0;
    let mut mask = 0;
    for (cond, b) in [
        (flex4(us), bit::SELF_FLEX4),
        (four(us), bit::SELF_FOUR),
        (flex3(us), bit::SELF_FLEX3),
        (flex4(them), bit::OPP_FLEX4),
        (four(them), bit::OPP_FOUR),
        (flex3(them), bit::OPP_FLEX3),
    ] {
        if cond {
            mask |= b;
        }
    }
    mask
}

/// Static evaluation from the side to move's point of view.
#[inline]
pub fn evaluate(board: &Board) -> Value {
    let us = board.side_to_move();
    let values = CellValues::standard();
    board.pattern_value(us) - board.pattern_value(us.opponent()) + values.threat[threat_mask(board, us)]
}

/// Win probability of the side to move for an evaluation.
pub fn win_probability(value: Value) -> f64 {
    1.0 / (1.0 + (-(value as f64) / SCALE).exp())
}

/// Linear features of a position, for tuning: `evaluate` equals the dot
/// product of these counts with the weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Features {
    /// Own minus opponent count of each line pattern over empty cells.
    pub pattern: [i32; Pattern::COUNT],
    /// Own minus opponent count of each combination over empty cells.
    pub pattern4: [i32; Pattern4::COUNT],
    /// Active threat-mask entry.
    pub threat: usize,
}

impl Features {
    /// Extract the features of `board` for the side to move.
    pub fn of(board: &Board) -> Features {
        let us = board.side_to_move();
        let mut f = Features { threat: threat_mask(board, us), ..Features::default() };
        let n = board.size();
        for pos in (0..n * n).map(|i| Pos::new(i % n, i / n)).filter(|&p| board.is_empty(p)) {
            for (color, sign) in [(us, 1), (us.opponent(), -1)] {
                for dir in 0..4 {
                    f.pattern[board.pattern(pos, color, dir) as usize] += sign;
                }
                f.pattern4[board.p4(pos, color) as usize] += sign;
            }
        }
        f
    }

    /// Dot product with `weights`, i.e. the evaluation those weights give.
    pub fn dot(&self, weights: &Weights) -> i32 {
        let lines: i32 = self.pattern.iter().zip(&weights.pattern).map(|(n, w)| n * w).sum();
        let combos: i32 = self.pattern4.iter().zip(&weights.pattern4).map(|(n, w)| n * w).sum();
        lines + combos + weights.threat[self.threat]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_reproduce_the_evaluation() {
        let mut board = Board::new(15);
        for (x, y) in [(7, 7), (8, 8), (7, 8), (6, 6), (8, 6), (9, 9), (6, 8), (0, 1)] {
            board.play(Pos::new(x, y));
            assert_eq!(Features::of(&board).dot(&DEFAULT_WEIGHTS), evaluate(&board));
        }
    }

    #[test]
    fn open_three_favors_its_owner() {
        let mut board = Board::new(15);
        for (x, y) in [(7, 7), (0, 14), (8, 7), (14, 14), (9, 7)] {
            board.play(Pos::new(x, y));
        }
        // White to move against Black's open three.
        assert!(evaluate(&board) < -150, "{}", evaluate(&board));
        assert_eq!(win_probability(0), 0.5);
    }
}
