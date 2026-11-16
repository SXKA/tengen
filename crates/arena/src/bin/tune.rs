//! Evaluation tuning by self-play and logistic regression (Texel tuning).
//!
//! ```text
//! tengen-tune selfplay --games 4000 --nodes 30000 --out games.txt [--weights w.txt]
//! tengen-tune fit --data games.txt --out weights.txt
//! tengen-tune init --out weights.txt
//! ```
//!
//! `selfplay` plays fast in-process games from balanced random openings.
//! `fit` replays them, keeps positions without immediate tactics, and fits
//! the evaluation so that `sigmoid(eval / SCALE)` predicts each game's result
//! for the side to move, using the factored model (per line pattern and
//! combination). With `--per-code` it then refines every pattern code on its
//! own, pulled toward the factored values, and keeps the refinement if it
//! predicts the held-out games (one in ten) better. That refinement has
//! overfitted so far: on 3,800 games it lost about 24 Elo where the factored
//! fit gained 23. `init` writes the untuned factored defaults.

use std::io::{BufRead, BufWriter, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tengen_arena::openings::{Rng, opening};
use tengen_arena::record::{Outcome, parse_game, write_game};
use tengen_engine::eval::{self, DEFAULT_FACTORED, Factored, Features, SCALE, THREAT_COUNT, Weights};
use tengen_engine::pattern::{PCODE_COUNT, Pattern, Pattern4, Tables};
use tengen_engine::search::MATE_BOUND;
use tengen_engine::{Board, Color, Engine, Limits};

const USAGE: &str = "usage:
  tengen-tune selfplay --games N [--nodes 30000] [--threads N] [--seed 1] [--size 15] --out FILE [--weights FILE]
  tengen-tune fit --data FILE[,FILE...] [--epochs 2000] [--per-code] --out FILE
  tengen-tune init --out FILE";

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
            let epochs = number("--epochs", 2000) as usize;
            let per_code = args.iter().any(|a| a == "--per-code");
            let tuned = fit(&data.split(',').collect::<Vec<_>>(), epochs, per_code);
            std::fs::write(&out, tuned.to_string()).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
        }
        Some("init") => {
            let weights = DEFAULT_FACTORED.weights();
            std::fs::write(&out, weights.to_string()).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
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
    /// The same position in the factored model's terms: own minus opponent
    /// count of each line pattern and of each combination.
    pattern: [i32; Pattern::COUNT],
    pattern4: [i32; Pattern4::COUNT],
    result: f64,
}

/// Whether the static evaluation is meant to judge this position: the side
/// to move has no four-three or better, and the opponent cannot make an
/// open four.
fn is_quiet(board: &Board) -> bool {
    let us = board.side_to_move();
    let them = us.opponent();
    board.p4_count_at_least(us, Pattern4::CBlock4Flex3) == 0 && board.p4_count_at_least(them, Pattern4::BFlex4) == 0
}

/// Training and validation positions (every tenth game is held out).
///
/// Positions where a shallow search already finds a forced win are left
/// out: the threat search settles those, so the static evaluation should not
/// be trained to anticipate them.
fn load(paths: &[&str]) -> (Vec<Sample>, Vec<Sample>) {
    let tables = Tables::get();
    let (mut train, mut valid) = (Vec::new(), Vec::new());
    let mut games = 0;
    let mut tactical = 0;
    let mut engine = Engine::new(8);
    let limits = Limits::depth(1);
    for path in paths {
        let file = std::fs::File::open(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        for line in std::io::BufReader::new(file).lines() {
            let Some((size, outcome, moves)) = line.ok().as_deref().and_then(parse_game) else {
                continue;
            };
            games += 1;
            let target = if games % 10 == 0 { &mut valid } else { &mut train };
            let mut board = Board::new(size);
            for (ply, &pos) in moves.iter().enumerate() {
                board.play(pos);
                if ply < 3 || ply + 4 >= moves.len() || !is_quiet(&board) {
                    continue;
                }
                if engine.search(&board, &limits, |_| {}).value.abs() >= MATE_BOUND {
                    tactical += 1;
                    continue;
                }
                let black = outcome.black_score();
                let result = if board.side_to_move() == Color::Black {
                    black
                } else {
                    1.0 - black
                };
                let features = Features::of(&board);
                let (mut pattern, mut pattern4) = ([0; Pattern::COUNT], [0; Pattern4::COUNT]);
                for &(code, n) in &features.pcode {
                    for p in tables.patterns(code) {
                        pattern[p as usize] += n;
                    }
                    pattern4[tables.pattern4(code) as usize] += n;
                }
                target.push(Sample {
                    features,
                    pattern,
                    pattern4,
                    result,
                });
            }
        }
    }
    eprintln!(
        "{} training and {} validation positions from {games} games ({tactical} with forced wins skipped)",
        train.len(),
        valid.len()
    );
    (train, valid)
}

/// A linear model over some view of the samples.
trait Model {
    fn params(&self) -> usize;
    /// Evaluation of a sample under parameters `p`.
    fn eval(&self, s: &Sample, p: &[f64]) -> f64;
    /// Add `g` times the gradient of the evaluation to `grad`.
    fn grad(&self, s: &Sample, g: f64, grad: &mut [f64]);
}

/// Per line pattern, per combination, and the threat term.
struct FactoredModel;

impl Model for FactoredModel {
    fn params(&self) -> usize {
        Pattern::COUNT + Pattern4::COUNT + THREAT_COUNT
    }
    fn eval(&self, s: &Sample, p: &[f64]) -> f64 {
        let (lines, rest) = p.split_at(Pattern::COUNT);
        let (combos, threat) = rest.split_at(Pattern4::COUNT);
        let a: f64 = s.pattern.iter().zip(lines).map(|(&n, w)| n as f64 * w).sum();
        let b: f64 = s.pattern4.iter().zip(combos).map(|(&n, w)| n as f64 * w).sum();
        a + b + threat[s.features.threat]
    }
    fn grad(&self, s: &Sample, g: f64, grad: &mut [f64]) {
        for (i, &n) in s.pattern.iter().enumerate() {
            grad[i] += g * n as f64;
        }
        for (i, &n) in s.pattern4.iter().enumerate() {
            grad[Pattern::COUNT + i] += g * n as f64;
        }
        grad[Pattern::COUNT + Pattern4::COUNT + s.features.threat] += g;
    }
}

/// A value per pattern code, and the threat term.
struct CodeModel;

impl Model for CodeModel {
    fn params(&self) -> usize {
        PCODE_COUNT + THREAT_COUNT
    }
    fn eval(&self, s: &Sample, p: &[f64]) -> f64 {
        let cells: f64 = s
            .features
            .pcode
            .iter()
            .map(|&(code, n)| n as f64 * p[code as usize])
            .sum();
        cells + p[PCODE_COUNT + s.features.threat]
    }
    fn grad(&self, s: &Sample, g: f64, grad: &mut [f64]) {
        for &(code, n) in &s.features.pcode {
            grad[code as usize] += g * n as f64;
        }
        grad[PCODE_COUNT + s.features.threat] += g;
    }
}

fn sigmoid(v: f64) -> f64 {
    1.0 / (1.0 + (-v / SCALE).exp())
}

/// Mean squared error of the predicted results.
fn loss(model: &dyn Model, samples: &[Sample], p: &[f64]) -> f64 {
    let sum: f64 = samples
        .iter()
        .map(|s| (s.result - sigmoid(model.eval(s, p))).powi(2))
        .sum();
    sum / samples.len().max(1) as f64
}

/// Full-batch Adam on mean squared error plus `l2 * |p - prior|^2`.
fn optimize(model: &dyn Model, samples: &[Sample], start: &[f64], prior: &[f64], l2: f64, epochs: usize) -> Vec<f64> {
    let n = model.params();
    let mut p = start.to_vec();
    let (mut m, mut v) = (vec![0.0; n], vec![0.0; n]);
    let (lr, beta1, beta2) = (0.5, 0.9, 0.999);
    let count = samples.len() as f64;
    for epoch in 1..=epochs {
        let mut grad = vec![0.0; n];
        for s in samples {
            let q = sigmoid(model.eval(s, &p));
            // d/dv of (r - q)^2 with q = sigmoid(v / SCALE).
            model.grad(s, -2.0 * (s.result - q) * q * (1.0 - q) / SCALE / count, &mut grad);
        }
        for i in 0..n {
            let g = grad[i] + 2.0 * l2 * (p[i] - prior[i]);
            m[i] = beta1 * m[i] + (1.0 - beta1) * g;
            v[i] = beta2 * v[i] + (1.0 - beta2) * g * g;
            let m_hat = m[i] / (1.0 - beta1.powi(epoch as i32));
            let v_hat = v[i] / (1.0 - beta2.powi(epoch as i32));
            p[i] -= lr * m_hat / (v_hat.sqrt() + 1e-12);
        }
    }
    p
}

/// Fit the factored model and, with `per_code`, the per-code tables, keeping
/// whichever predicts the held-out games better.
fn fit(paths: &[&str], epochs: usize, per_code: bool) -> Weights {
    let (train, valid) = load(paths);
    if train.is_empty() || valid.is_empty() {
        fail("not enough positions to fit");
    }

    let factored = FactoredModel;
    let start: Vec<f64> = DEFAULT_FACTORED
        .pattern
        .iter()
        .chain(&DEFAULT_FACTORED.pattern4)
        .chain(&DEFAULT_FACTORED.threat)
        .map(|&v| v as f64)
        .collect();
    // Threat entries seen too rarely keep their hand-set values.
    let mut seen = vec![0usize; THREAT_COUNT];
    for s in &train {
        seen[s.features.threat] += 1;
    }
    let mut stage1 = optimize(&factored, &train, &start, &start, 1e-7, epochs);
    for (t, &count) in seen.iter().enumerate() {
        if count < 100 {
            stage1[Pattern::COUNT + Pattern4::COUNT + t] = start[Pattern::COUNT + Pattern4::COUNT + t];
        }
    }
    let round = |p: &[f64]| p.iter().map(|v| v.round() as i32).collect::<Vec<_>>();
    let rounded = round(&stage1);
    let tuned_factored = Factored {
        pattern: rounded[..Pattern::COUNT].try_into().unwrap(),
        pattern4: rounded[Pattern::COUNT..Pattern::COUNT + Pattern4::COUNT]
            .try_into()
            .unwrap(),
        threat: rounded[Pattern::COUNT + Pattern4::COUNT..].try_into().unwrap(),
    };
    let mut best = tuned_factored.weights();
    let as_params = |w: &Weights| w.pcode.iter().chain(&w.threat).map(|&v| v as f64).collect::<Vec<_>>();
    let codes = CodeModel;
    let prior = as_params(&best);
    let mut best_loss = loss(&codes, &valid, &prior);
    eprintln!(
        "factored: train {:.6} -> {:.6}, validation {:.6} -> {best_loss:.6}",
        loss(&factored, &train, &start),
        loss(&factored, &train, &stage1),
        loss(&factored, &valid, &start),
    );
    eprintln!(
        "factored pattern {:?}\nfactored pattern4 {:?}",
        tuned_factored.pattern, tuned_factored.pattern4
    );

    let strengths: &[f64] = if per_code { &[1e-7, 1e-8, 1e-9] } else { &[] };
    for &l2 in strengths {
        let p = optimize(&codes, &train, &prior, &prior, l2, epochs);
        let rounded = round(&p);
        let mut threat = rounded[PCODE_COUNT..].to_vec();
        for (t, &count) in seen.iter().enumerate() {
            if count < 100 {
                threat[t] = tuned_factored.threat[t];
            }
        }
        let weights = Weights {
            pcode: rounded[..PCODE_COUNT].to_vec(),
            threat,
        };
        let valid_loss = loss(&codes, &valid, &as_params(&weights));
        eprintln!(
            "per code, l2 {l2:e}: train {:.6}, validation {valid_loss:.6}",
            loss(&codes, &train, &p)
        );
        if valid_loss < best_loss {
            best_loss = valid_loss;
            best = weights;
        }
    }
    eprintln!("kept validation loss {best_loss:.6}");
    best
}
