//! Self-play arena: plays two Piskvork brains against each other and stops
//! early with a sequential probability ratio test (SPRT).
//!
//! Games are played in pairs from the same balanced opening with colors
//! swapped, and the test uses pentanomial statistics over the pairs, which
//! removes most of the opening's bias from the result.
//!
//! ```text
//! tengen-arena --new ./new/pbrain-tengen --base ./base/pbrain-tengen \
//!     --games 2000 --concurrency 4 --turn-ms 200 --sprt 0,5
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tengen_arena::brain::Brain;
use tengen_arena::openings::{Rng, opening};
use tengen_arena::record::{Outcome, write_game};
use tengen_arena::sprt::{Pentanomial, Sprt};
use tengen_engine::pattern::Pattern4;
use tengen_engine::{Board, Color, Engine, Pos};

/// Command-line settings.
#[derive(Clone, Debug)]
struct Options {
    new: String,
    base: String,
    /// Maximum number of games (rounded up to pairs).
    games: usize,
    concurrency: usize,
    size: usize,
    turn_ms: u64,
    match_ms: u64,
    /// SPRT bounds in logistic Elo; `None` plays a fixed number of games.
    sprt: Option<(f64, f64)>,
    seed: u64,
    /// Append finished games here, one per line, for tuning.
    record: Option<String>,
}

impl Options {
    fn parse() -> Result<Options, String> {
        let mut o = Options {
            new: String::new(),
            base: String::new(),
            games: 1000,
            concurrency: std::thread::available_parallelism().map_or(1, |n| n.get()),
            size: 15,
            turn_ms: 200,
            match_ms: 0,
            sprt: None,
            seed: 1,
            record: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or(format!("{flag} needs a value"));
            let number = |v: String| v.parse::<u64>().map_err(|e| format!("{flag}: {e}"));
            match flag.as_str() {
                "--new" => o.new = value()?,
                "--base" => o.base = value()?,
                "--games" => o.games = number(value()?)? as usize,
                "--concurrency" => o.concurrency = number(value()?)?.max(1) as usize,
                "--size" => o.size = number(value()?)? as usize,
                "--turn-ms" => o.turn_ms = number(value()?)?,
                "--match-ms" => o.match_ms = number(value()?)?,
                "--seed" => o.seed = number(value()?)?,
                "--record" => o.record = Some(value()?),
                "--sprt" => {
                    let v = value()?;
                    let (a, b) = v.split_once(',').ok_or("--sprt takes elo0,elo1")?;
                    let parse = |s: &str| s.trim().parse::<f64>().map_err(|e| format!("--sprt: {e}"));
                    o.sprt = Some((parse(a)?, parse(b)?));
                }
                "--help" | "-h" => return Err(String::new()),
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        if o.new.is_empty() || o.base.is_empty() {
            return Err("--new and --base are required".into());
        }
        Ok(o)
    }
}

const USAGE: &str = "usage: tengen-arena --new BRAIN --base BRAIN [--games N] [--concurrency N]
       [--size 15] [--turn-ms 200] [--match-ms 0] [--sprt ELO0,ELO1] [--seed 1] [--record FILE]";

/// Play one game. `black` moves first after the opening stones.
fn play_game(
    options: &Options,
    black_path: &str,
    white_path: &str,
    opening: &[Pos],
) -> (Outcome, Vec<Pos>, Option<String>) {
    let mut board = Board::new(options.size);
    for &pos in opening {
        board.play(pos);
    }
    // Brains in color order; who moves next is decided by the board.
    let spawn = |path: &str| Brain::spawn(path, options.size, options.turn_ms, options.match_ms);
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
        let turn_limit = options.turn_ms as f64 * 2.0 + 500.0;
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

/// Shared tallies.
struct Tally {
    pairs: Pentanomial,
    wins: u64,
    draws: u64,
    losses: u64,
    errors: Vec<String>,
    done: bool,
}

fn main() {
    let options = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let sprt = options.sprt.map(|(e0, e1)| Sprt::new(e0, e1, 0.05, 0.05));
    let pairs_total = options.games.div_ceil(2);

    // Openings are generated up front so every run with a seed is the same.
    eprintln!("generating {pairs_total} openings...");
    let mut rng = Rng(options.seed);
    let mut engine = Engine::new(16);
    let openings: Vec<Vec<Pos>> = (0..pairs_total)
        .map(|_| opening(options.size, &mut rng, &mut engine))
        .collect();

    let tally = Arc::new(Mutex::new(Tally {
        pairs: Pentanomial::default(),
        wins: 0,
        draws: 0,
        losses: 0,
        errors: Vec::new(),
        done: false,
    }));
    let record = options.record.as_ref().map(|path| {
        Mutex::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("open record file"),
        )
    });
    let next = AtomicUsize::new(0);
    let started = Instant::now();

    std::thread::scope(|scope| {
        for _ in 0..options.concurrency.min(pairs_total) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= pairs_total || tally.lock().unwrap().done {
                        break;
                    }
                    let opening = &openings[index];
                    // The new engine plays Black in game 1 and White in game 2.
                    let (b1, w1) = (&options.new, &options.base);
                    let g1 = play_game(&options, b1, w1, opening);
                    let g2 = play_game(&options, w1, b1, opening);
                    // Score for the new engine.
                    let score = |outcome: Outcome, new_is_black: bool| match (outcome, new_is_black) {
                        (Outcome::Draw, _) => 1,
                        (Outcome::BlackWins, true) | (Outcome::WhiteWins, false) => 2,
                        _ => 0,
                    };
                    let s1 = score(g1.0, b1 == &options.new);
                    let s2 = score(g2.0, w1 == &options.new);

                    if let Some(file) = &record {
                        let mut file = file.lock().unwrap();
                        for (outcome, moves, _) in [&g1, &g2] {
                            let _ = write_game(&mut *file, options.size, *outcome, moves);
                        }
                    }

                    let mut t = tally.lock().unwrap();
                    for (s, error) in [(s1, &g1.2), (s2, &g2.2)] {
                        match s {
                            2 => t.wins += 1,
                            1 => t.draws += 1,
                            _ => t.losses += 1,
                        }
                        if let Some(e) = error {
                            t.errors.push(e.clone());
                        }
                    }
                    t.pairs.add(s1 + s2);
                    let games = t.wins + t.draws + t.losses;
                    let (elo, error) = t.pairs.elo();
                    let llr = sprt.as_ref().map(|s| s.llr(&t.pairs));
                    if games.is_multiple_of(20) || games == 2 * pairs_total as u64 {
                        let llr_text = match (llr, &sprt) {
                            (Some(l), Some(s)) => format!("  LLR {l:.2} [{:.2}, {:.2}]", s.lower(), s.upper()),
                            _ => String::new(),
                        };
                        eprintln!(
                            "games {games}  +{} ={} -{}  elo {elo:+.1} ± {error:.1}{llr_text}  ({:.0}s)",
                            t.wins,
                            t.draws,
                            t.losses,
                            started.elapsed().as_secs_f64()
                        );
                    }
                    if let (Some(l), Some(s)) = (llr, &sprt)
                        && (l >= s.upper() || l <= s.lower())
                    {
                        t.done = true;
                    }
                }
            });
        }
    });

    let t = tally.lock().unwrap();
    let (elo, error) = t.pairs.elo();
    let games = t.wins + t.draws + t.losses;
    println!("## Tengen arena\n");
    println!("| games | wins | draws | losses | Elo (95%) | pentanomial |");
    println!("|---|---|---|---|---|---|");
    println!(
        "| {games} | {} | {} | {} | {elo:+.1} ± {error:.1} | {:?} |",
        t.wins, t.draws, t.losses, t.pairs.counts
    );
    if let Some(s) = &sprt {
        let llr = s.llr(&t.pairs);
        let verdict = if llr >= s.upper() {
            "H1 accepted: the new version is stronger"
        } else if llr <= s.lower() {
            "H0 accepted: no improvement"
        } else {
            "inconclusive"
        };
        let (e0, e1) = options.sprt.unwrap();
        println!(
            "\nSPRT [{e0}, {e1}]: LLR {llr:.2} (bounds {:.2}, {:.2}), {verdict}",
            s.lower(),
            s.upper()
        );
    }
    println!(
        "\nturn {} ms, match {} ms, board {}x{}",
        options.turn_ms, options.match_ms, options.size, options.size
    );
    if !t.errors.is_empty() {
        println!("\n{} game(s) ended by errors, first: {}", t.errors.len(), t.errors[0]);
    }
}
