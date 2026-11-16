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
//! fit gained 23. `fit-policy` learns the move-ordering tables: it adjusts
//! the default scores, per line pattern and combination for the mover and
//! for the opponent, so that a softmax over the candidate moves predicts the
//! move played. Run it after `fit`, which resets them to their defaults.
//! `init` writes the untuned factored defaults.

use std::io::{BufRead, BufWriter, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tengen_arena::openings::{Rng, opening};
use tengen_arena::record::{Outcome, parse_game, write_game};
use tengen_engine::eval::{self, DEFAULT_FACTORED, Factored, Features, SCALE, THREAT_COUNT, Weights};
use tengen_engine::movegen;
use tengen_engine::pattern::{PCODE_COUNT, Pattern, Pattern4, PatternCode, Tables};
use tengen_engine::search::MATE_BOUND;
use tengen_engine::{Board, Color, Engine, Limits};

const USAGE: &str = "usage:
  tengen-tune selfplay --games N [--nodes 30000] [--threads N] [--seed 1] [--size 15] --out FILE [--weights FILE]
  tengen-tune fit --data FILE[,FILE...] [--epochs 2000] [--per-code] --out FILE
  tengen-tune fit-policy --data FILE[,FILE...] [--weights FILE] [--epochs 300] --out FILE
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
        Some("fit-policy") => {
            let data = flag("--data").unwrap_or_else(|| fail("--data is required"));
            let epochs = number("--epochs", 300) as usize;
            let tuned = fit_policy(&data.split(',').collect::<Vec<_>>(), &weights, epochs);
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

/// One self-play game, played until a five or a full board.
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
        let Some(pos) = result.best_move else {
            break Outcome::Draw;
        };
        // Play wins out rather than adjudicate them, so the winning moves
        // appear in the record for move-ordering training.
        let five = board.p4(pos, us) == Pattern4::AFive;
        board.play(pos);
        if five {
            break win_for(us);
        }
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

/// Node budget of the shallow search that filters out tactical positions.
const FILTER_NODES: u64 = 20_000;

/// Threads for loading and fitting.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Every line of the game files.
fn read_lines(paths: &[&str]) -> Vec<String> {
    let mut lines = Vec::new();
    for path in paths {
        let file = std::fs::File::open(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        lines.extend(std::io::BufReader::new(file).lines().map_while(Result::ok));
    }
    lines
}

/// Run `f` on contiguous chunks of `items` in parallel, passing each item's
/// index; results come back in order.
fn par_chunks<T: Sync, R: Send>(items: &[T], f: impl Fn(usize, &T) -> R + Sync) -> Vec<R> {
    let chunk = items.len().div_ceil(threads()).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .enumerate()
            .map(|(c, part)| {
                let f = &f;
                scope.spawn(move || {
                    part.iter()
                        .enumerate()
                        .map(|(i, item)| f(c * chunk + i, item))
                        .collect::<Vec<R>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread"))
            .collect()
    })
}

/// Sum per-sample gradients over `samples` in parallel.
fn par_gradient<T: Sync>(samples: &[T], n: usize, f: impl Fn(&T, &mut [f64]) + Sync) -> Vec<f64> {
    let chunk = samples.len().div_ceil(threads()).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = samples
            .chunks(chunk)
            .map(|part| {
                let f = &f;
                scope.spawn(move || {
                    let mut grad = vec![0.0; n];
                    for s in part {
                        f(s, &mut grad);
                    }
                    grad
                })
            })
            .collect();
        let mut total = vec![0.0; n];
        for h in handles {
            for (t, g) in total.iter_mut().zip(h.join().expect("worker thread")) {
                *t += g;
            }
        }
        total
    })
}

/// Training and validation positions (every tenth game is held out).
///
/// Positions where a shallow search already finds a forced win are left
/// out: the threat search settles those, so the static evaluation should not
/// be trained to anticipate them.
fn load(paths: &[&str]) -> (Vec<Sample>, Vec<Sample>) {
    let tables = Tables::get();
    let lines = read_lines(paths);
    let tactical = std::sync::atomic::AtomicUsize::new(0);
    thread_local! {
        static ENGINE: std::cell::RefCell<Engine> = std::cell::RefCell::new(Engine::new(8));
    }
    let per_game = par_chunks(&lines, |index, line| {
        let mut samples = Vec::new();
        let Some((size, outcome, moves)) = parse_game(line) else {
            return (index, samples);
        };
        // The filter search is capped: a few positions with many fours on
        // both sides make the threat search explode, and those are not
        // quiet anyway.
        let limits = Limits {
            depth: Some(1),
            nodes: Some(FILTER_NODES),
            ..Limits::default()
        };
        let mut board = Board::new(size);
        for (ply, &pos) in moves.iter().enumerate() {
            board.play(pos);
            if ply < 3 || ply + 4 >= moves.len() || !is_quiet(&board) {
                continue;
            }
            let result = ENGINE.with(|e| e.borrow_mut().search(&board, &limits, |_| {}));
            if result.value.abs() >= MATE_BOUND || result.nodes >= FILTER_NODES {
                tactical.fetch_add(1, Ordering::Relaxed);
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
            samples.push(Sample {
                features,
                pattern,
                pattern4,
                result,
            });
        }
        (index, samples)
    });
    let (mut train, mut valid) = (Vec::new(), Vec::new());
    for (index, samples) in per_game {
        if (index + 1) % 10 == 0 {
            valid.extend(samples)
        } else {
            train.extend(samples)
        }
    }
    eprintln!(
        "{} training and {} validation positions from {} games ({} with forced wins skipped)",
        train.len(),
        valid.len(),
        lines.len(),
        tactical.into_inner()
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

/// Per line pattern and per combination, early and late in the game, and
/// the threat term.
struct FactoredModel;

const FACTORS: usize = Pattern::COUNT + Pattern4::COUNT;

impl Model for FactoredModel {
    fn params(&self) -> usize {
        2 * FACTORS + THREAT_COUNT
    }
    fn eval(&self, s: &Sample, p: &[f64]) -> f64 {
        let t = s.features.phase();
        let dot = |w: &[f64]| -> f64 {
            let (lines, combos) = w.split_at(Pattern::COUNT);
            let a: f64 = s.pattern.iter().zip(lines).map(|(&n, w)| n as f64 * w).sum();
            let b: f64 = s.pattern4.iter().zip(combos).map(|(&n, w)| n as f64 * w).sum();
            a + b
        };
        (1.0 - t) * dot(&p[..FACTORS]) + t * dot(&p[FACTORS..2 * FACTORS]) + p[2 * FACTORS + s.features.threat]
    }
    fn grad(&self, s: &Sample, g: f64, grad: &mut [f64]) {
        let t = s.features.phase();
        for (offset, weight) in [(0, 1.0 - t), (FACTORS, t)] {
            for (i, &n) in s.pattern.iter().enumerate() {
                grad[offset + i] += g * weight * n as f64;
            }
            for (i, &n) in s.pattern4.iter().enumerate() {
                grad[offset + Pattern::COUNT + i] += g * weight * n as f64;
            }
        }
        grad[2 * FACTORS + s.features.threat] += g;
    }
}

/// A value per pattern code, early and late in the game, and the threat
/// term.
struct CodeModel;

impl Model for CodeModel {
    fn params(&self) -> usize {
        2 * PCODE_COUNT + THREAT_COUNT
    }
    fn eval(&self, s: &Sample, p: &[f64]) -> f64 {
        let t = s.features.phase();
        let cells: f64 = s
            .features
            .pcode
            .iter()
            .map(|&(code, n)| n as f64 * ((1.0 - t) * p[code as usize] + t * p[PCODE_COUNT + code as usize]))
            .sum();
        cells + p[2 * PCODE_COUNT + s.features.threat]
    }
    fn grad(&self, s: &Sample, g: f64, grad: &mut [f64]) {
        let t = s.features.phase();
        for &(code, n) in &s.features.pcode {
            grad[code as usize] += g * (1.0 - t) * n as f64;
            grad[PCODE_COUNT + code as usize] += g * t * n as f64;
        }
        grad[2 * PCODE_COUNT + s.features.threat] += g;
    }
}

fn sigmoid(v: f64) -> f64 {
    1.0 / (1.0 + (-v / SCALE).exp())
}

/// Mean squared error of the predicted results.
fn loss(model: &(dyn Model + Sync), samples: &[Sample], p: &[f64]) -> f64 {
    let sum = par_gradient(samples, 1, |s, acc| {
        acc[0] += (s.result - sigmoid(model.eval(s, p))).powi(2)
    });
    sum[0] / samples.len().max(1) as f64
}

/// Full-batch Adam on mean squared error plus `l2 * |p - prior|^2`.
fn optimize(
    model: &(dyn Model + Sync),
    samples: &[Sample],
    start: &[f64],
    prior: &[f64],
    l2: f64,
    epochs: usize,
) -> Vec<f64> {
    let n = model.params();
    let mut p = start.to_vec();
    let (mut m, mut v) = (vec![0.0; n], vec![0.0; n]);
    let (lr, beta1, beta2) = (0.5, 0.9, 0.999);
    let count = samples.len() as f64;
    for epoch in 1..=epochs {
        let grad = par_gradient(samples, n, |s, grad| {
            let q = sigmoid(model.eval(s, &p));
            // d/dv of (r - q)^2 with q = sigmoid(v / SCALE).
            model.grad(s, -2.0 * (s.result - q) * q * (1.0 - q) / SCALE / count, grad);
        });
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
    let d = &DEFAULT_FACTORED;
    let start: Vec<f64> = d
        .pattern
        .iter()
        .chain(&d.pattern4)
        .chain(&d.pattern)
        .chain(&d.pattern4)
        .chain(&d.threat)
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
            stage1[2 * FACTORS + t] = start[2 * FACTORS + t];
        }
    }
    let round = |p: &[f64]| p.iter().map(|v| v.round() as i32).collect::<Vec<_>>();
    let rounded = round(&stage1);
    let model = |offset: usize| Factored {
        pattern: rounded[offset..offset + Pattern::COUNT].try_into().unwrap(),
        pattern4: rounded[offset + Pattern::COUNT..offset + FACTORS].try_into().unwrap(),
        threat: rounded[2 * FACTORS..].try_into().unwrap(),
    };
    let (early, late) = (model(0), model(FACTORS));
    let mut best = early.weights();
    best.pcode_late = late.weights().pcode;
    let as_params = |w: &Weights| {
        w.pcode
            .iter()
            .chain(&w.pcode_late)
            .chain(&w.threat)
            .map(|&v| v as f64)
            .collect::<Vec<_>>()
    };
    let codes = CodeModel;
    let prior = as_params(&best);
    let mut best_loss = loss(&codes, &valid, &prior);
    eprintln!(
        "factored: train {:.6} -> {:.6}, validation {:.6} -> {best_loss:.6}",
        loss(&factored, &train, &start),
        loss(&factored, &train, &stage1),
        loss(&factored, &valid, &start),
    );
    eprintln!("early pattern {:?} pattern4 {:?}", early.pattern, early.pattern4);
    eprintln!("late pattern {:?} pattern4 {:?}", late.pattern, late.pattern4);

    let strengths: &[f64] = if per_code { &[1e-7, 1e-8, 1e-9] } else { &[] };
    for &l2 in strengths {
        let p = optimize(&codes, &train, &prior, &prior, l2, epochs);
        let rounded = round(&p);
        let mut threat = rounded[2 * PCODE_COUNT..].to_vec();
        for (t, &count) in seen.iter().enumerate() {
            if count < 100 {
                threat[t] = early.threat[t];
            }
        }
        let pcode = rounded[..PCODE_COUNT].to_vec();
        let (policy_own, policy_opp) = eval::default_policy(&pcode);
        let weights = Weights {
            pcode,
            pcode_late: rounded[PCODE_COUNT..2 * PCODE_COUNT].to_vec(),
            threat,
            policy_own,
            policy_opp,
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

/// A position for move-ordering training: every candidate's pattern codes
/// for the mover and the opponent, and which candidate was played.
struct PolicySample {
    candidates: Vec<(PatternCode, PatternCode)>,
    target: usize,
}

/// Positions where the mover chose freely among all candidates (no forced
/// reply), with every tenth game held out.
fn load_policy(paths: &[&str]) -> (Vec<PolicySample>, Vec<PolicySample>) {
    let lines = read_lines(paths);
    let per_game = par_chunks(&lines, |index, line| {
        let mut samples = Vec::new();
        let Some((size, _, moves)) = parse_game(line) else {
            return (index, samples);
        };
        let mut board = Board::new(size);
        let mut list = Vec::new();
        for &played in &moves {
            if board.ply() >= 2
                && movegen::generate(&mut board, &mut list) == (movegen::Gen::Moves { forced: false })
                && let Some(target) = list.iter().position(|m| m.pos == played)
            {
                let us = board.side_to_move();
                let candidates = list
                    .iter()
                    .map(|m| (board.pcode(m.pos, us), board.pcode(m.pos, us.opponent())))
                    .collect();
                samples.push(PolicySample { candidates, target });
            }
            board.play(played);
        }
        (index, samples)
    });
    let (mut train, mut valid) = (Vec::new(), Vec::new());
    for (index, samples) in per_game {
        if (index + 1) % 10 == 0 {
            valid.extend(samples)
        } else {
            train.extend(samples)
        }
    }
    eprintln!(
        "{} training and {} validation positions from {} games",
        train.len(),
        valid.len(),
        lines.len()
    );
    (train, valid)
}

/// Mean cross-entropy and top-1 accuracy of the move-ordering tables.
fn policy_loss(samples: &[PolicySample], own: &[f64], opp: &[f64], scale: f64) -> (f64, f64) {
    let totals = par_gradient(samples, 2, |s, acc| {
        let scores: Vec<f64> = s
            .candidates
            .iter()
            .map(|&(a, b)| (own[a as usize] + opp[b as usize]) / scale)
            .collect();
        let max = scores.iter().cloned().fold(f64::MIN, f64::max);
        let sum: f64 = scores.iter().map(|v| (v - max).exp()).sum();
        acc[0] += max + sum.ln() - scores[s.target];
        acc[1] += (scores[s.target] >= max) as u8 as f64;
    });
    let n = samples.len().max(1) as f64;
    (totals[0] / n, totals[1] / n)
}

/// Learn the move-ordering tables as corrections to the defaults: one per
/// line pattern and per combination, for the mover and for the opponent.
fn fit_policy(paths: &[&str], start: &Weights, epochs: usize) -> Weights {
    let (train, valid) = load_policy(paths);
    if train.is_empty() || valid.is_empty() {
        fail("not enough positions to fit");
    }
    let tables = Tables::get();
    let base_own: Vec<f64> = start.policy_own.iter().map(|&v| v as f64).collect();
    let base_opp: Vec<f64> = start.policy_opp.iter().map(|&v| v as f64).collect();

    // The softmax temperature that best fits the current scores, by a
    // coarse search; the corrections are then learned at that temperature
    // so their scale stays comparable with history and killer bonuses.
    let scale = [25.0, 50.0, 100.0, 200.0, 400.0, 800.0]
        .into_iter()
        .min_by(|&a, &b| {
            policy_loss(&train, &base_own, &base_opp, a)
                .0
                .total_cmp(&policy_loss(&train, &base_own, &base_opp, b).0)
        })
        .unwrap();
    let scale = [scale * 0.7, scale, scale * 1.4]
        .into_iter()
        .min_by(|&a, &b| {
            policy_loss(&train, &base_own, &base_opp, a)
                .0
                .total_cmp(&policy_loss(&train, &base_own, &base_opp, b).0)
        })
        .unwrap();
    let (before, before_acc) = policy_loss(&valid, &base_own, &base_opp, scale);
    eprintln!(
        "temperature {scale:.0}: validation loss {before:.4}, top-1 {:.1}%",
        before_acc * 100.0
    );

    // Parameters: line pattern and combination corrections, own then opponent.
    const N: usize = 2 * (Pattern::COUNT + Pattern4::COUNT);
    let mut params = [0.0f64; N];
    let expand = |params: &[f64; N]| {
        let mut own = base_own.clone();
        let mut opp = base_opp.clone();
        for code in 0..PCODE_COUNT {
            let (lines, combo) = (
                tables.patterns(code as PatternCode),
                tables.pattern4(code as PatternCode) as usize,
            );
            for (table, offset) in [(&mut own, 0), (&mut opp, Pattern::COUNT + Pattern4::COUNT)] {
                let delta: f64 = lines.iter().map(|&p| params[offset + p as usize]).sum::<f64>()
                    + params[offset + Pattern::COUNT + combo];
                table[code] += delta;
            }
        }
        (own, opp)
    };
    let (mut m, mut v) = ([0.0; N], [0.0; N]);
    let (lr, beta1, beta2) = (2.0, 0.9, 0.999);
    for epoch in 1..=epochs {
        let (own, opp) = expand(&params);
        // Gradient per pattern code, own codes first, then the opponent's.
        let count = train.len() as f64;
        let g = par_gradient(&train, 2 * PCODE_COUNT, |s, g| {
            let scores: Vec<f64> = s
                .candidates
                .iter()
                .map(|&(a, b)| (own[a as usize] + opp[b as usize]) / scale)
                .collect();
            let max = scores.iter().cloned().fold(f64::MIN, f64::max);
            let sum: f64 = scores.iter().map(|x| (x - max).exp()).sum();
            for (i, &(a, b)) in s.candidates.iter().enumerate() {
                let p = (scores[i] - max).exp() / sum;
                let d = (p - (i == s.target) as u8 as f64) / scale / count;
                g[a as usize] += d;
                g[PCODE_COUNT + b as usize] += d;
            }
        });
        let (g_own, g_opp) = g.split_at(PCODE_COUNT);
        let mut grad = [0.0; N];
        for code in 0..PCODE_COUNT {
            let (lines, combo) = (
                tables.patterns(code as PatternCode),
                tables.pattern4(code as PatternCode) as usize,
            );
            for (g, offset) in [(g_own[code], 0), (g_opp[code], Pattern::COUNT + Pattern4::COUNT)] {
                for &p in &lines {
                    grad[offset + p as usize] += g;
                }
                grad[offset + Pattern::COUNT + combo] += g;
            }
        }
        // Adjudicated self-play games stop once a win is found, so winning
        // moves are rarely seen played; leave the corrections of the
        // strongest combinations at zero rather than learn that bias.
        for (i, g) in grad.iter_mut().enumerate() {
            let combo = i % (Pattern::COUNT + Pattern4::COUNT);
            let own = i < Pattern::COUNT + Pattern4::COUNT;
            let frozen = if own {
                Pattern4::FFlex3x2
            } else {
                Pattern4::CBlock4Flex3
            };
            if combo >= Pattern::COUNT && combo - Pattern::COUNT >= frozen as usize {
                *g = 0.0;
            }
        }
        for i in 0..N {
            m[i] = beta1 * m[i] + (1.0 - beta1) * grad[i];
            v[i] = beta2 * v[i] + (1.0 - beta2) * grad[i] * grad[i];
            let m_hat = m[i] / (1.0 - beta1.powi(epoch as i32));
            let v_hat = v[i] / (1.0 - beta2.powi(epoch as i32));
            params[i] -= lr * m_hat / (v_hat.sqrt() + 1e-12);
        }
        if epoch % 50 == 0 {
            let (loss, acc) = policy_loss(&valid, &own, &opp, scale);
            eprintln!("epoch {epoch}: validation loss {loss:.4}, top-1 {:.1}%", acc * 100.0);
        }
    }
    let (own, opp) = expand(&params);
    let (after, after_acc) = policy_loss(&valid, &own, &opp, scale);
    eprintln!(
        "validation loss {before:.4} -> {after:.4}, top-1 {:.1}% -> {:.1}%",
        before_acc * 100.0,
        after_acc * 100.0
    );
    eprintln!(
        "own corrections {:?}",
        params[..Pattern::COUNT + Pattern4::COUNT]
            .iter()
            .map(|x| x.round() as i32)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "opponent corrections {:?}",
        params[Pattern::COUNT + Pattern4::COUNT..]
            .iter()
            .map(|x| x.round() as i32)
            .collect::<Vec<_>>()
    );
    Weights {
        policy_own: own.iter().map(|x| x.round() as i32).collect(),
        policy_opp: opp.iter().map(|x| x.round() as i32).collect(),
        ..start.clone()
    }
}
