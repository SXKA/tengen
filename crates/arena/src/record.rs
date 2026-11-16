//! Game records: one game per line, `SIZE RESULT x,y x,y ...`, with the
//! result from Black's (the first mover's) point of view.

use std::io::{self, Write};

use tengen_engine::{Board, Pos};

/// Game result from Black's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    BlackWins,
    WhiteWins,
    Draw,
}

impl Outcome {
    /// Black's score: 1, 0.5 or 0.
    pub fn black_score(self) -> f64 {
        match self {
            Outcome::BlackWins => 1.0,
            Outcome::Draw => 0.5,
            Outcome::WhiteWins => 0.0,
        }
    }
}

/// Append one game.
pub fn write_game(out: &mut impl Write, size: usize, outcome: Outcome, moves: &[Pos]) -> io::Result<()> {
    let result = match outcome {
        Outcome::BlackWins => "1-0",
        Outcome::WhiteWins => "0-1",
        Outcome::Draw => "1/2",
    };
    let cells: Vec<String> = moves.iter().map(|p| format!("{},{}", p.x(), p.y())).collect();
    writeln!(out, "{size} {result} {}", cells.join(" "))
}

/// Parse one line written by [`write_game`].
pub fn parse_game(line: &str) -> Option<(usize, Outcome, Vec<Pos>)> {
    let mut words = line.split_whitespace();
    let size: usize = words.next()?.parse().ok()?;
    let outcome = match words.next()? {
        "1-0" => Outcome::BlackWins,
        "0-1" => Outcome::WhiteWins,
        "1/2" => Outcome::Draw,
        _ => return None,
    };
    let mut board = Board::new(size);
    let mut moves = Vec::new();
    for word in words {
        let (x, y) = word.split_once(',')?;
        let pos = Pos::new(x.parse().ok()?, y.parse().ok()?);
        if pos.x() >= size || pos.y() >= size || !board.is_empty(pos) {
            return None;
        }
        board.play(pos);
        moves.push(pos);
    }
    Some((size, outcome, moves))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_round_trip() {
        let moves = vec![Pos::new(7, 7), Pos::new(8, 8), Pos::new(0, 14)];
        let mut out = Vec::new();
        write_game(&mut out, 15, Outcome::WhiteWins, &moves).unwrap();
        let line = String::from_utf8(out).unwrap();
        assert_eq!(parse_game(&line), Some((15, Outcome::WhiteWins, moves)));
        assert_eq!(parse_game("15 1-0 7,7 7,7"), None);
    }
}
