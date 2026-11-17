//! Playing one game between two brains.

use std::time::Instant;

use tengen_engine::pattern::Pattern4;
use tengen_engine::{Board, Color, Pos, Rule};

use crate::brain::Brain;
use crate::record::Outcome;

/// How games are played.
#[derive(Clone, Debug)]
pub struct Settings {
    pub size: usize,
    pub rule: Rule,
    /// Milliseconds per move.
    pub turn_ms: u64,
    /// Milliseconds per game and player; 0 for no game clock.
    pub match_ms: u64,
    /// Memory each brain may use, in MiB.
    pub memory_mb: u64,
    /// Milliseconds per move for the brain with this command, instead of
    /// `turn_ms`: time odds.
    pub turn_override: Option<(String, u64)>,
}

impl Settings {
    /// Milliseconds per move for the brain run by `path`.
    pub fn turn_ms_for(&self, path: &str) -> u64 {
        match &self.turn_override {
            Some((command, ms)) if command == path => *ms,
            _ => self.turn_ms,
        }
    }
}

/// Play one game. `black` moves first after the opening stones. Returns
/// the result, the moves (opening included) and the error that ended the
/// game, if one did (a crash, a timeout, an illegal move).
pub fn play_game(
    options: &Settings,
    black_path: &str,
    white_path: &str,
    opening: &[Pos],
) -> (Outcome, Vec<Pos>, Option<String>) {
    let mut board = Board::with_rule(options.size, options.rule);
    for &pos in opening {
        board.play(pos);
    }
    // Brains in color order; who moves next is decided by the board.
    let spawn = |path: &str| {
        Brain::spawn(
            path,
            options.size,
            options.rule,
            options.turn_ms_for(path),
            options.match_ms,
            options.memory_mb,
        )
    };
    let mut brains = match (spawn(black_path), spawn(white_path)) {
        (Ok(b), Ok(w)) => [b, w],
        (Err(e), _) | (_, Err(e)) => return (Outcome::Draw, board.moves().collect(), Some(format!("spawn: {e}"))),
    };
    let mut seen = [false; 2];
    let mut time_left = [options.match_ms as f64; 2];
    let loses = |color: Color| match color {
        Color::Black => Outcome::WhiteWins,
        Color::White => Outcome::BlackWins,
    };

    let outcome = loop {
        if board.is_full() {
            break Outcome::Draw;
        }
        let color = board.side_to_move();
        let side = color as usize;
        let brain = &mut brains[side];
        let started = Instant::now();
        if options.match_ms > 0 {
            brain.send(&format!("INFO time_left {}", time_left[side].max(0.0) as u64));
        }
        let reply = if seen[side] {
            brain.turn(board.last_move().expect("opponent moved"))
        } else {
            seen[side] = true;
            brain.board(&board, color)
        };
        let spent = started.elapsed().as_secs_f64() * 1000.0;
        time_left[side] -= spent;
        // Generous tolerance: CI machines are noisy.
        let turn_ms = options.turn_ms_for(if side == 0 { black_path } else { white_path });
        let turn_limit = turn_ms as f64 * 2.0 + 500.0;
        let pos = match reply {
            Ok(_) if spent > turn_limit => {
                let error = format!("{color:?} took {spent:.0} ms");
                return (loses(color), board.moves().collect(), Some(error));
            }
            Ok(_) if options.match_ms > 0 && time_left[side] < -500.0 => {
                let error = format!("{color:?} lost on time");
                return (loses(color), board.moves().collect(), Some(error));
            }
            Ok(pos) if board.is_empty(pos) => pos,
            Ok(pos) => {
                let error = format!("{color:?} played occupied {},{}", pos.x(), pos.y());
                return (loses(color), board.moves().collect(), Some(error));
            }
            Err(e) => {
                let error = format!("{color:?}: {e}");
                return (loses(color), board.moves().collect(), Some(error));
            }
        };
        if color == Color::Black && board.is_forbidden(pos) {
            let error = format!("Black played the forbidden {},{}", pos.x(), pos.y());
            return (loses(color), board.moves().collect(), Some(error));
        }
        let five = board.p4(pos, color) == Pattern4::AFive;
        board.play(pos);
        if five {
            break match color {
                Color::Black => Outcome::BlackWins,
                Color::White => Outcome::WhiteWins,
            };
        }
    };
    for brain in brains.iter_mut() {
        brain.quit();
    }
    (outcome, board.moves().collect(), None)
}
