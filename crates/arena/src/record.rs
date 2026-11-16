//! Game records: one game per line, `SIZE RESULT x,y x,y ...`, with the
//! result from Black's (the first mover's) point of view. Games under rules
//! other than freestyle write the size as `SIZE:RULE`, e.g. `15:renju`.

use std::io::{self, Write};

use tengen_engine::{Board, Pos, Rule};

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
pub fn write_game(out: &mut impl Write, size: usize, rule: Rule, outcome: Outcome, moves: &[Pos]) -> io::Result<()> {
    let result = match outcome {
        Outcome::BlackWins => "1-0",
        Outcome::WhiteWins => "0-1",
        Outcome::Draw => "1/2",
    };
    let cells: Vec<String> = moves.iter().map(|p| format!("{},{}", p.x(), p.y())).collect();
    let board = match rule {
        Rule::Freestyle => size.to_string(),
        _ => format!("{size}:{rule}"),
    };
    writeln!(out, "{board} {result} {}", cells.join(" "))
}

/// A game read back by [`parse_game`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub size: usize,
    pub rule: Rule,
    pub outcome: Outcome,
    pub moves: Vec<Pos>,
}

impl Game {
    /// An empty board for this game.
    pub fn board(&self) -> Board {
        Board::with_rule(self.size, self.rule)
    }
}

/// Parse one line written by [`write_game`].
pub fn parse_game(line: &str) -> Option<Game> {
    let mut words = line.split_whitespace();
    let first = words.next()?;
    let (size, rule) = match first.split_once(':') {
        Some((size, rule)) => (size.parse().ok()?, Rule::parse(rule)?),
        None => (first.parse().ok()?, Rule::Freestyle),
    };
    if !(5..=tengen_engine::board::MAX_SIZE).contains(&size) {
        return None;
    }
    let outcome = match words.next()? {
        "1-0" => Outcome::BlackWins,
        "0-1" => Outcome::WhiteWins,
        "1/2" => Outcome::Draw,
        _ => return None,
    };
    let mut seen = vec![false; size * size];
    let mut moves = Vec::new();
    for word in words {
        let (x, y) = word.split_once(',')?;
        let (x, y): (usize, usize) = (x.parse().ok()?, y.parse().ok()?);
        if x >= size || y >= size || std::mem::replace(&mut seen[y * size + x], true) {
            return None;
        }
        moves.push(Pos::new(x, y));
    }
    Some(Game {
        size,
        rule,
        outcome,
        moves,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_round_trip() {
        let moves = vec![Pos::new(7, 7), Pos::new(8, 8), Pos::new(0, 14)];
        for rule in Rule::ALL {
            let mut out = Vec::new();
            write_game(&mut out, 15, rule, Outcome::WhiteWins, &moves).unwrap();
            let line = String::from_utf8(out).unwrap();
            let game = parse_game(&line).unwrap();
            assert_eq!((game.size, game.rule, game.outcome), (15, rule, Outcome::WhiteWins));
            assert_eq!(game.moves, moves);
        }
        assert!(line_is_freestyle("15 1-0 7,7"));
        assert_eq!(parse_game("15 1-0 7,7 7,7"), None);
        assert_eq!(parse_game("15:go 1-0 7,7"), None);
        assert_eq!(parse_game("15 1-0 15,0"), None);
    }

    fn line_is_freestyle(line: &str) -> bool {
        parse_game(line).is_some_and(|g| g.rule == Rule::Freestyle)
    }
}
