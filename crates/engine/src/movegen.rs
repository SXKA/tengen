//! Move generation with forced-response detection.
//!
//! Before listing moves, the generator checks the threats on the board:
//!
//! * the side to move can make five: that move wins;
//! * the opponent can make five in two places: the game is lost;
//! * the opponent can make five in one place: it must be blocked;
//! * the opponent can make an open four next move: only moves that stop it,
//!   or fours of our own that seize the initiative, are worth searching.
//!
//! Defences are found by trying every cell that could matter (along the
//! threatening lines) and keeping those after which the opponent has no open
//! four left. This is exact and cheap because the situation is rare.

use crate::board::{Board, Color, Pos};
use crate::eval::CellValues;
use crate::pattern::{Pattern, Pattern4};

/// A move with an ordering score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoredMove {
    pub pos: Pos,
    pub score: i32,
}

/// What the position demands of the side to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gen {
    /// Playing here makes five.
    Win(Pos),
    /// The opponent wins in this many plies whatever we do.
    Lost(u8),
    /// Search the listed moves; `forced` when they only answer a threat.
    Moves { forced: bool },
}

/// Ordering score of `pos` for the side `us`: what it builds plus what it
/// prevents, from the move-ordering tables of the evaluation weights.
#[inline]
pub fn move_score(board: &Board, pos: Pos, us: Color, values: &CellValues) -> i32 {
    values.policy(board.pcode(pos, us), board.pcode(pos, us.opponent()))
}

/// Fill `moves` for the side to move and report what the position demands.
///
/// `board` is only borrowed mutably to try defences; it is unchanged on
/// return.
pub fn generate(board: &mut Board, moves: &mut Vec<ScoredMove>) -> Gen {
    moves.clear();
    let us = board.side_to_move();
    let them = us.opponent();
    let values = CellValues::standard();

    if let Some(pos) = board.five_point(us) {
        return Gen::Win(pos);
    }
    match board.p4_count(them, Pattern4::AFive) {
        0 => {}
        1 => {
            let pos = board.five_point(them).expect("counted five point");
            moves.push(ScoredMove { pos, score: 0 });
            return Gen::Moves { forced: true };
        }
        _ => return Gen::Lost(2),
    }

    if board.p4_count(them, Pattern4::BFlex4) > 0 {
        generate_defences(board, moves, values);
        if moves.is_empty() {
            // Nothing stops the open four: it is played next move and one of
            // its two fives completes the move after.
            return Gen::Lost(4);
        }
        return Gen::Moves { forced: true };
    }

    for pos in board.candidates() {
        moves.push(ScoredMove {
            pos,
            score: move_score(board, pos, us, values),
        });
    }
    if moves.is_empty() && !board.is_full() {
        // No stone yet: start in the center.
        moves.push(ScoredMove {
            pos: board.center(),
            score: 0,
        });
    }
    Gen::Moves { forced: false }
}

/// Moves against an opponent who can make an open four: our own fours, and
/// cells after which no open-four point remains.
fn generate_defences(board: &mut Board, moves: &mut Vec<ScoredMove>, values: &CellValues) {
    let us = board.side_to_move();
    let them = us.opponent();

    // Cells to try: every empty cell within reach of a threat point along
    // the lines that make it a threat.
    let mut tried = [0u64; crate::board::CELLS.div_ceil(64)];
    let mut try_cells = Vec::with_capacity(32);
    let mut mark = |pos: Pos, out: &mut Vec<Pos>| {
        let (i, b) = (pos.index() / 64, pos.index() % 64);
        if tried[i] & (1 << b) == 0 {
            tried[i] |= 1 << b;
            out.push(pos);
        }
    };
    for point in board.area_cells() {
        if !board.is_empty(point) || board.p4(point, them) != Pattern4::BFlex4 {
            continue;
        }
        mark(point, &mut try_cells);
        for dir in 0..4 {
            if board.pattern(point, them, dir) < Pattern::B4 {
                continue;
            }
            for k in (-4..=4).filter(|&k| k != 0) {
                if let Some(q) = board.step(point, dir, k)
                    && board.is_empty(q)
                {
                    mark(q, &mut try_cells);
                }
            }
        }
    }

    for pos in board.candidates() {
        if board.p4(pos, us) >= Pattern4::EBlock4 {
            mark(pos, &mut try_cells);
        }
    }

    for &pos in &try_cells {
        let four = board.p4(pos, us) >= Pattern4::EBlock4;
        let score = move_score(board, pos, us, values);
        if four {
            moves.push(ScoredMove { pos, score });
            continue;
        }
        board.play(pos);
        let stops = board.p4_count(them, Pattern4::BFlex4) == 0;
        board.undo();
        if stops {
            moves.push(ScoredMove { pos, score });
        }
    }
}

/// Fill `moves` with every move that makes a four (or five) for the side to
/// move, strongest first. Used by the threat-space quiescence search.
pub fn generate_fours(board: &Board, moves: &mut Vec<ScoredMove>) {
    moves.clear();
    let us = board.side_to_move();
    if board.p4_count_at_least(us, Pattern4::EBlock4) == 0 {
        return;
    }
    let values = CellValues::standard();
    for pos in board.candidates() {
        let p4 = board.p4(pos, us);
        if p4 >= Pattern4::EBlock4 {
            let score = (p4 as i32) * 10_000 + move_score(board, pos, us, values);
            moves.push(ScoredMove { pos, score });
        }
    }
    moves.sort_unstable_by_key(|m| std::cmp::Reverse(m.score));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_with(moves: &[(usize, usize)]) -> Board {
        let mut board = Board::new(15);
        for &(x, y) in moves {
            board.play(Pos::new(x, y));
        }
        board
    }

    fn generated(board: &mut Board) -> (Gen, Vec<Pos>) {
        let mut moves = Vec::new();
        let kind = generate(board, &mut moves);
        let mut cells: Vec<Pos> = moves.iter().map(|m| m.pos).collect();
        cells.sort();
        (kind, cells)
    }

    #[test]
    fn empty_board_plays_center() {
        let mut board = Board::new(15);
        assert_eq!(
            generated(&mut board),
            (Gen::Moves { forced: false }, vec![Pos::new(7, 7)])
        );
    }

    #[test]
    fn open_four_against_us_is_lost() {
        // Black's (3..=6, 7) is an open four; White to move.
        let mut board = board_with(&[(3, 7), (3, 0), (4, 7), (5, 0), (5, 7), (7, 0), (6, 7)]);
        assert_eq!(generated(&mut board).0, Gen::Lost(2));
        // With an extra White stone far away, Black is to move and wins.
        board.play(Pos::new(13, 13));
        let (kind, _) = generated(&mut board);
        assert!(
            matches!(kind, Gen::Win(p) if p == Pos::new(2, 7) || p == Pos::new(7, 7)),
            "{kind:?}"
        );
    }

    #[test]
    fn blocks_a_single_five_point() {
        // White's (3..=6, 0) is blocked on the left by Black's (2, 0).
        let mut board = board_with(&[(2, 0), (3, 0), (7, 7), (4, 0), (9, 9), (5, 0), (11, 11), (6, 0)]);
        assert_eq!(
            generated(&mut board),
            (Gen::Moves { forced: true }, vec![Pos::new(7, 0)])
        );
    }

    #[test]
    fn defends_open_three() {
        // White's open three (6..=8, 5) with plenty of room; Black to move.
        let mut board = board_with(&[(7, 7), (6, 5), (12, 12), (7, 5), (12, 2), (8, 5)]);
        let (kind, cells) = generated(&mut board);
        assert_eq!(kind, Gen::Moves { forced: true });
        // With room on both sides only the adjacent ends stop the open four.
        assert_eq!(cells, vec![Pos::new(5, 5), Pos::new(9, 5)]);
    }

    #[test]
    fn split_three_has_three_defences() {
        // White's split three (5, 5), (6, 5), (8, 5): the gap and both ends.
        let mut board = board_with(&[(7, 7), (5, 5), (12, 12), (6, 5), (12, 2), (8, 5)]);
        let (kind, cells) = generated(&mut board);
        assert_eq!(kind, Gen::Moves { forced: true });
        assert_eq!(cells, vec![Pos::new(4, 5), Pos::new(7, 5), Pos::new(9, 5)]);
    }

    #[test]
    fn two_open_threes_are_lost() {
        let mut board = board_with(&[
            (0, 14),
            (6, 5),
            (14, 14),
            (7, 5),
            (14, 0),
            (8, 5),
            (0, 0),
            (6, 9),
            (7, 14),
            (7, 9),
            (7, 0),
            (8, 9),
        ]);
        assert_eq!(generated(&mut board).0, Gen::Lost(4));
    }

    #[test]
    fn fours_are_listed_strongest_first() {
        let mut board = board_with(&[(5, 7), (0, 0), (6, 7), (0, 2), (7, 7), (0, 4)]);
        let mut moves = Vec::new();
        generate_fours(&board, &mut moves);
        // The open three makes open fours at (4, 7) and (8, 7) and split
        // fours at (3, 7) and (9, 7).
        let first: Vec<Pos> = moves.iter().take(2).map(|m| m.pos).collect();
        assert!(first.contains(&Pos::new(4, 7)) && first.contains(&Pos::new(8, 7)));
        assert_eq!(moves.len(), 4);
        board.play(Pos::new(4, 7));
        assert_eq!(board.p4_count(Color::Black, Pattern4::AFive), 2);
    }
}
