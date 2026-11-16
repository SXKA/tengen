//! Evaluation tuning by self-play and logistic regression (Texel tuning).
//!
//! ```text
//! tengen-tune selfplay --games 4000 --nodes 30000 --out games.txt [--weights w.txt]
//! tengen-tune fit --data games.txt [--weights w.txt] --out tuned.txt
//! ```
//!
//! `selfplay` plays fast in-process games from balanced random openings.
//! `fit` replays them, keeps positions without immediate tactics, and fits
//! the evaluation weights so that `sigmoid(eval / SCALE)` predicts each game's
//! result for the side to move.

use std::io::{BufRead, BufWriter, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tengen_arena::openings::{Rng, opening};
use tengen_arena::record::{Outcome, parse_game, write_game};
use tengen_engine::eval::{self, Features, SCALE, THREAT_COUNT, Weights};
use tengen_engine::pattern::{Pattern, Pattern4};
use tengen_engine::search::MATE_BOUND;
use tengen_engine::{Board, Color, Engine, Limits};

const USAGE: &str = "usage:
  tengen-tune selfplay --games N [--nodes 30000] [--threads N] [--seed 1] [--size 15] --out FILE [--weights FILE]
  tengen-tune fit --data FILE[,FILE...] [--weights FILE] [--epochs 3000] --out FILE";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let number = |name: &str, default: u64| {
        flag(name).map_or(default, |v| v.parse().unwrap_or_else(|_| fail(&format!("bad {name}"))))
    };
    let weights = match flag("--weights") {
        Some(path) => {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
            Weights::parse(&text).unwrap_or_else(|e| fail(&format!("{path}: {e}")))
        }
        None => Weights::default(),
    };
    let out = flag("--out").unwrap_or_else(|| fail("--out is required"));
    match args.first().map(String::as_str) {
        Some("selfplay") => {
            eval::install(&weights).unwrap_or_else(|e| fail(&e));
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) as u64;
            selfplay(SelfPlay {
                games: number("--games", 1000) as usize,
                nodes: number("--nodes", 30_000),
                threads: number("--threads", threads) as usize,
                seed: number("--seed", 1),
                size: number("--size", 15) as usize,
                out,
            });
        }
        Some("fit") => {
            let data = flag("--data").unwrap_or_else(|| fail("--data is required"));
            let epochs = number("--epochs", 3000) as usize;
            let tuned = fit(&data.split(',').collect::<Vec<_>>(), &weights, epochs);
            std::fs::write(&out, tuned.to_string()).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
            println!("{}", rust_source(&tuned));
        }
        _ => fail(""),
    }
}

fn fail(message: &str) -> ! {
    if !message.is_empty() {
        eprintln!("error: {message}");
    }
    eprintln!("{USAGE}");
    std::process::exit(2)
}

struct SelfPlay {
    games: usize,
    nodes: u64,
    threads: usize,
    seed: u64,
    size: usize,
    out: String,
}

fn selfplay(cfg: SelfPlay) {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&cfg.out)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", cfg.out)));
    let out = Mutex::new(BufWriter::new(file));
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..cfg.threads {
            scope.spawn(|| {
                let mut engine = Engine::new(32);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= cfg.games {
                        break;
                    }
                    let mut rng = Rng(cfg.seed.wrapping_mul(0x1000_0000_01b3) ^ index as u64);
                    let (outcome, moves) = play(&mut engine, &mut rng, &cfg);
                    let mut out = out.lock().unwrap();
                    let _ = write_game(&mut *out, cfg.size, outcome, &moves);
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if finished.is_multiple_of(100) || finished == cfg.games {
                        let _ = out.flush();
                        eprintln!(
                            "{finished}/{} games ({:.0}s)",
                            cfg.games,
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
            });
        }
    });
}

/// One self-play game, adjudicated as soon as either side sees a forced win.
fn play(engine: &mut Engine, rng: &mut Rng, cfg: &SelfPlay) -> (Outcome, Vec<tengen_engine::Pos>) {
    let mut board = Board::new(cfg.size);
    for pos in opening(cfg.size, rng, engine) {
        board.play(pos);
    }
    engine.clear();
    let limits = Limits {
        nodes: Some(cfg.nodes),
        ..Limits::default()
    };
    let win_for = |color: Color| match color {
        Color::Black => Outcome::BlackWins,
        Color::White => Outcome::WhiteWins,
    };
    let outcome = loop {
        if board.is_full() {
            break Outcome::Draw;
        }
        let us = board.side_to_move();
        let result = engine.search(&board, &limits, |_| {});
        if result.value >= MATE_BOUND {
            break win_for(us);
        }
        if result.value <= -MATE_BOUND {
            break win_for(us.opponent());
        }
        let Some(pos) = result.best_move else {
            break Outcome::Draw;
        };
        board.play(pos);
    };
    (outcome, board.moves().collect())
}

/// A training position: its features and the result for the side to move.
struct Sample {
    features: Features,
    result: f64,
}

/// Whether the static evaluation is meant to judge this position: nobody can
/// win at once or make an open four, and the side to move has no four-three.
fn is_quiet(board: &Board) -> bool {
    let us = board.side_to_move();
    let them = us.opponent();
    board.p4_count_at_least(us, Pattern4::CBlock4Flex3) == 0 && board.p4_count_at_least(them, Pattern4::BFlex4) == 0
}

fn load(paths: &[&str]) -> Vec<Sample> {
    let mut samples = Vec::new();
    let mut games = 0;
    for path in paths {
        let file = std::fs::File::open(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        for line in std::io::BufReader::new(file).lines() {
            let Some((size, outcome, moves)) = line.ok().as_deref().and_then(parse_game) else {
                continue;
            };
            games += 1;
            let mut board = Board::new(size);
            for (ply, &pos) in moves.iter().enumerate() {
                board.play(pos);
                if ply >= 3 && ply + 4 < moves.len() && is_quiet(&board) {
                    let black = outcome.black_score();
                    let result = if board.side_to_move() == Color::Black {
                        black
                    } else {
                        1.0 - black
                    };
                    samples.push(Sample {
                        features: Features::of(&board),
                        result,
                    });
                }
            }
        }
    }
    eprintln!("{} positions from {games} games", samples.len());
    samples
}

const PATTERNS: usize = Pattern::COUNT;
const COMBOS: usize = Pattern4::COUNT;
const PARAMS: usize = PATTERNS + COMBOS + THREAT_COUNT;

fn to_params(w: &Weights) -> Vec<f64> {
    w.pattern
        .iter()
        .chain(&w.pattern4)
        .chain(&w.threat)
        .map(|&v| v as f64)
        .collect()
}

fn from_params(p: &[f64]) -> Weights {
    let mut w = Weights::default();
    for (dst, src) in w
        .pattern
        .iter_mut()
        .chain(w.pattern4.iter_mut())
        .chain(w.threat.iter_mut())
        .zip(p)
    {
        *dst = src.round() as i32;
    }
    w
}

fn predict(sample: &Sample, p: &[f64]) -> f64 {
    let f = &sample.features;
    let mut v = p[PATTERNS + COMBOS + f.threat];
    for (i, &n) in f.pattern.iter().enumerate() {
        v += n as f64 * p[i];
    }
    for (i, &n) in f.pattern4.iter().enumerate() {
        v += n as f64 * p[PATTERNS + i];
    }
    v
}

fn loss(samples: &[Sample], p: &[f64]) -> f64 {
    let sum: f64 = samples
        .iter()
        .map(|s| {
            let q = 1.0 / (1.0 + (-predict(s, p) / SCALE).exp());
            (s.result - q).powi(2)
        })
        .sum();
    sum / samples.len().max(1) as f64
}

/// Fit with full-batch Adam on mean squared error, keeping the threat
/// entries that never occur at their starting values.
fn fit(paths: &[&str], start: &Weights, epochs: usize) -> Weights {
    let samples = load(paths);
    if samples.is_empty() {
        fail("no positions to fit");
    }
    let mut p = to_params(start);
    let initial = p.clone();
    let mut seen = [0usize; THREAT_COUNT];
    for s in &samples {
        seen[s.features.threat] += 1;
    }
    let before = loss(&samples, &p);
    let (mut m, mut v) = (vec![0.0; PARAMS], vec![0.0; PARAMS]);
    let (lr, beta1, beta2) = (0.5, 0.9, 0.999);
    for epoch in 1..=epochs {
        let mut grad = vec![0.0; PARAMS];
        for s in &samples {
            let q = 1.0 / (1.0 + (-predict(s, &p) / SCALE).exp());
            // d/dv of (r - q)^2 with q = sigmoid(v / SCALE).
            let g = -2.0 * (s.result - q) * q * (1.0 - q) / SCALE;
            for (i, &n) in s.features.pattern.iter().enumerate() {
                grad[i] += g * n as f64;
            }
            for (i, &n) in s.features.pattern4.iter().enumerate() {
                grad[PATTERNS + i] += g * n as f64;
            }
            grad[PATTERNS + COMBOS + s.features.threat] += g;
        }
        let n = samples.len() as f64;
        for i in 0..PARAMS {
            // Light pull toward the start keeps rare features sensible.
            let g = grad[i] / n + 1e-9 * (p[i] - initial[i]);
            m[i] = beta1 * m[i] + (1.0 - beta1) * g;
            v[i] = beta2 * v[i] + (1.0 - beta2) * g * g;
            let m_hat = m[i] / (1.0 - beta1.powi(epoch as i32));
            let v_hat = v[i] / (1.0 - beta2.powi(epoch as i32));
            p[i] -= lr * m_hat / (v_hat.sqrt() + 1e-12);
        }
        for (t, &count) in seen.iter().enumerate() {
            if count < 50 {
                p[PATTERNS + COMBOS + t] = initial[PATTERNS + COMBOS + t];
            }
        }
        if epoch % 500 == 0 {
            eprintln!("epoch {epoch}: loss {:.6}", loss(&samples, &p));
        }
    }
    let tuned = from_params(&p);
    let after = loss(&samples, &to_params(&tuned));
    eprintln!("loss {before:.6} -> {after:.6}");
    tuned
}

/// The weights as a Rust constant for `eval.rs`.
fn rust_source(w: &Weights) -> String {
    let list = |t: &[i32]| t.iter().map(i32::to_string).collect::<Vec<_>>().join(", ");
    format!(
        "pub const DEFAULT_WEIGHTS: Weights = Weights {{\n    pattern: [{}],\n    pattern4: [{}],\n    threat: [{}],\n}};",
        list(&w.pattern),
        list(&w.pattern4),
        list(&w.threat)
    )
}
