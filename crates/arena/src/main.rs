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

use tengen_arena::game::{Settings, play_game};
use tengen_arena::openings::{Rng, book_order, load_book, opening};
use tengen_arena::record::{Outcome, write_game};
use tengen_arena::sprt::{Pentanomial, Sprt};
use tengen_engine::{Engine, Rule};

/// Command-line settings.
#[derive(Clone, Debug)]
struct Options {
    new: String,
    base: String,
    /// Maximum number of games (rounded up to pairs).
    games: usize,
    concurrency: usize,
    size: usize,
    rule: Rule,
    turn_ms: u64,
    match_ms: u64,
    /// Memory each brain may use, in MiB (Piskvork's max_memory).
    memory_mb: u64,
    /// Milliseconds per move for the new brain only (time odds).
    new_turn_ms: Option<u64>,
    /// SPRT bounds in logistic Elo; `None` plays a fixed number of games.
    sprt: Option<(f64, f64)>,
    seed: u64,
    /// Append finished games here, one per line, for tuning.
    record: Option<String>,
    /// Play the openings of this book instead of random ones.
    openings: Option<String>,
    /// Deepest iteration for both brains.
    max_depth: Option<u32>,
}

impl Options {
    fn parse() -> Result<Options, String> {
        let mut o = Options {
            new: String::new(),
            base: String::new(),
            games: 1000,
            concurrency: std::thread::available_parallelism().map_or(1, |n| n.get()),
            size: 15,
            rule: Rule::Freestyle,
            turn_ms: 200,
            match_ms: 0,
            memory_mb: 160,
            new_turn_ms: None,
            sprt: None,
            seed: 1,
            record: None,
            openings: None,
            max_depth: None,
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
                "--rule" => {
                    let v = value()?;
                    o.rule = Rule::parse(&v).ok_or(format!("--rule: unknown rule {v}"))?;
                }
                "--turn-ms" => o.turn_ms = number(value()?)?,
                "--match-ms" => o.match_ms = number(value()?)?,
                "--memory-mb" => o.memory_mb = number(value()?)?,
                "--new-turn-ms" => o.new_turn_ms = Some(number(value()?)?),
                "--seed" => o.seed = number(value()?)?,
                "--record" => o.record = Some(value()?),
                "--openings" => o.openings = Some(value()?),
                "--max-depth" => o.max_depth = Some(number(value()?)? as u32),
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
       [--size 15] [--rule freestyle|standard|renju|caro|freestyle-caro] [--turn-ms 200] [--match-ms 0]
       [--memory-mb 160] [--new-turn-ms MS] [--sprt ELO0,ELO1] [--seed 1] [--record FILE] [--openings FILE]
       [--max-depth N]";

/// Pairs to play before the SPRT may stop the match.
const MIN_PAIRS: u64 = 50;

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
    let settings = Settings {
        size: options.size,
        rule: options.rule,
        turn_ms: options.turn_ms,
        match_ms: options.match_ms,
        memory_mb: options.memory_mb,
        turn_override: options.new_turn_ms.map(|ms| (options.new.clone(), ms)),
        max_depth: options.max_depth,
    };

    let book = options.openings.as_deref().map(|path| {
        let book = load_book(path).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        });
        let order = book_order(book.len(), options.seed);
        (book, order)
    });

    std::thread::scope(|scope| {
        for _ in 0..options.concurrency.min(pairs_total) {
            scope.spawn(|| {
                let mut engine = Engine::new(16);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= pairs_total || tally.lock().unwrap().done {
                        break;
                    }
                    // Each pair's opening depends only on the seed and its
                    // index, so a run is reproducible at any concurrency.
                    let mut rng = Rng(options.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index as u64);
                    let opening = &match &book {
                        Some((book, order)) => book[order[index % order.len()]].clone(),
                        None => opening(options.size, options.rule, &mut rng, &mut engine),
                    };
                    // The new engine plays Black in game 1 and White in game 2.
                    let (b1, w1) = (&options.new, &options.base);
                    let g1 = play_game(&settings, b1, w1, opening);
                    let g2 = play_game(&settings, w1, b1, opening);
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
                            let _ = write_game(&mut *file, options.size, options.rule, *outcome, moves);
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
                    if games.is_multiple_of(10) || games == 2 * pairs_total as u64 {
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
                    // The normal approximation behind the LLR is poor for
                    // tiny samples, so a decision needs a minimum of pairs.
                    if let (Some(l), Some(s)) = (llr, &sprt)
                        && t.pairs.pairs() >= MIN_PAIRS
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
    println!(
        "Rule {}, {}x{} board, {} ms a move.\n",
        options.rule, options.size, options.size, options.turn_ms
    );
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
        "\nturn {} ms, match {} ms, board {}x{}{}",
        options.turn_ms,
        options.match_ms,
        options.size,
        options.size,
        options.max_depth.map_or(String::new(), |d| format!(", depth {d}"))
    );
    if !t.errors.is_empty() {
        println!("\n{} game(s) ended by errors, first: {}", t.errors.len(), t.errors[0]);
    }
}
