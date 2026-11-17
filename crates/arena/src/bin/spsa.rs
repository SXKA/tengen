//! Tune search parameters by self-play with SPSA (simultaneous perturbation
//! stochastic approximation), the way Stockfish's Fishtest does.
//!
//! ```text
//! tengen-spsa --brain target/release/pbrain-tengen --spec tuning/spsa.txt \
//!     [--pairs 10000] [--minutes 300] [--turn-ms 50] [--rule freestyle] \
//!     [--concurrency N] [--seed 1]
//! ```
//!
//! Each step perturbs every parameter by ±c at random, plays a pair of games
//! (same opening, colors swapped) between the two perturbed versions, and
//! moves the parameters toward the side that scored better. Steps run in
//! parallel, each from the parameters current when it starts. The spec file
//! has one line per parameter: `name start min max c_end r_end`, where
//! `c_end` is the perturbation and `r_end` the learning rate at the last
//! step (Fishtest's conventions: c shrinks as k^-0.101, a as
//! (A + k)^-0.602 with A one tenth of the steps).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tengen_arena::game::{Settings, play_game};
use tengen_arena::openings::{Rng, opening};
use tengen_arena::record::Outcome;
use tengen_engine::{Engine, Rule};

const USAGE: &str = "usage: tengen-spsa --brain PATH --spec FILE [--pairs 10000] [--minutes 300] [--turn-ms 50]
       [--rule freestyle] [--size 15] [--concurrency N] [--seed 1]";

/// One tuned parameter.
#[derive(Clone, Debug)]
struct Param {
    name: String,
    value: f64,
    min: f64,
    max: f64,
    /// Perturbation and learning-rate constants (Fishtest's c and a).
    c: f64,
    a: f64,
}

const ALPHA: f64 = 0.602;
const GAMMA: f64 = 0.101;

fn parse_spec(text: &str, steps: usize) -> Result<Vec<Param>, String> {
    let n = steps as f64;
    let big_a = 0.1 * n;
    let mut params = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let words: Vec<&str> = line.split_whitespace().collect();
        let [name, start, min, max, c_end, r_end] = words[..] else {
            return Err(format!("expected `name start min max c_end r_end`: {line}"));
        };
        let num = |s: &str| s.parse::<f64>().map_err(|e| format!("{line}: {e}"));
        let (c_end, r_end) = (num(c_end)?, num(r_end)?);
        let c = c_end * n.powf(GAMMA);
        let a_end = r_end * c_end * c_end;
        params.push(Param {
            name: name.to_string(),
            value: num(start)?,
            min: num(min)?,
            max: num(max)?,
            c,
            a: a_end * (big_a + n).powf(ALPHA),
        });
    }
    Ok(params)
}

/// `--params` text for values.
fn params_text(params: &[Param], values: &[f64]) -> String {
    params
        .iter()
        .zip(values)
        .map(|(p, v)| format!("{}={}", p.name, v.round() as i64))
        .collect::<Vec<_>>()
        .join(",")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let fail = |message: String| -> ! {
        eprintln!("{message}\n{USAGE}");
        std::process::exit(2)
    };
    let number = |name: &str, default: u64| {
        flag(name).map_or(default, |v| v.parse().unwrap_or_else(|_| fail(format!("bad {name}"))))
    };
    let brain = flag("--brain").unwrap_or_else(|| fail("--brain is required".into()));
    let spec = flag("--spec").unwrap_or_else(|| fail("--spec is required".into()));
    let pairs = number("--pairs", 10_000) as usize;
    let deadline = Instant::now() + Duration::from_secs(number("--minutes", 300) * 60);
    let concurrency = number(
        "--concurrency",
        std::thread::available_parallelism().map_or(1, |n| n.get()) as u64,
    ) as usize;
    let seed = number("--seed", 1);
    let rule = flag("--rule").map_or(Rule::Freestyle, |r| {
        Rule::parse(&r).unwrap_or_else(|| fail(format!("unknown rule {r}")))
    });
    let settings = Settings {
        size: number("--size", 15) as usize,
        rule,
        turn_ms: number("--turn-ms", 50),
        match_ms: 0,
        memory_mb: 160,
    };
    let text = std::fs::read_to_string(&spec).unwrap_or_else(|e| fail(format!("{spec}: {e}")));
    let params = parse_spec(&text, pairs).unwrap_or_else(|e| fail(e));

    let values = Mutex::new(params.iter().map(|p| p.value).collect::<Vec<f64>>());
    let score = Mutex::new((0i64, 0usize));
    let next = AtomicUsize::new(0);
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..concurrency {
            scope.spawn(|| {
                let mut engine = Engine::new(16);
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    if k >= pairs || Instant::now() >= deadline {
                        break;
                    }
                    let step = (k + 1) as f64;
                    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ k as u64);
                    let current = values.lock().unwrap().clone();
                    let signs: Vec<f64> = params
                        .iter()
                        .map(|_| if rng.below(2) == 0 { -1.0 } else { 1.0 })
                        .collect();
                    let shifted = |sign: f64| -> Vec<f64> {
                        params
                            .iter()
                            .zip(&current)
                            .zip(&signs)
                            .map(|((p, v), s)| (v + sign * s * p.c / step.powf(GAMMA)).clamp(p.min, p.max))
                            .collect()
                    };
                    let plus = format!("{brain} --params {}", params_text(&params, &shifted(1.0)));
                    let minus = format!("{brain} --params {}", params_text(&params, &shifted(-1.0)));
                    let stones = opening(settings.size, settings.rule, &mut rng, &mut engine);
                    // Wins minus losses of the plus side over the pair.
                    let points = |outcome: Outcome, plus_black: bool| match (outcome, plus_black) {
                        (Outcome::Draw, _) => 0,
                        (Outcome::BlackWins, true) | (Outcome::WhiteWins, false) => 1,
                        _ => -1,
                    };
                    let g1 = play_game(&settings, &plus, &minus, &stones);
                    let g2 = play_game(&settings, &minus, &plus, &stones);
                    if let Some(e) = g1.2.as_ref().or(g2.2.as_ref()) {
                        eprintln!("pair {k}: {e}");
                    }
                    let result = (points(g1.0, true) + points(g2.0, false)) as f64;

                    let mut v = values.lock().unwrap();
                    for ((p, value), s) in params.iter().zip(v.iter_mut()).zip(&signs) {
                        let c_k = p.c / step.powf(GAMMA);
                        let a_k = p.a / (0.1 * pairs as f64 + step).powf(ALPHA);
                        *value = (*value + a_k / c_k * result * s).clamp(p.min, p.max);
                    }
                    let mut total = score.lock().unwrap();
                    total.0 += result as i64;
                    total.1 += 1;
                    if total.1.is_multiple_of(50) {
                        eprintln!(
                            "{} pairs ({:.0}s): {}",
                            total.1,
                            started.elapsed().as_secs_f64(),
                            params_text(&params, &v)
                        );
                    }
                }
            });
        }
    });

    let values = values.into_inner().unwrap();
    let (_, done) = score.into_inner().unwrap();
    println!("## SPSA\n");
    println!(
        "{done} game pairs, rule {}, {} ms a move, {:.0} minutes.\n",
        settings.rule,
        settings.turn_ms,
        started.elapsed().as_secs_f64() / 60.0
    );
    println!("| parameter | start | tuned |");
    println!("|---|---|---|");
    for (p, v) in params.iter().zip(&values) {
        println!("| {} | {} | {:.1} |", p.name, p.value, v);
    }
    println!("\n```\n--params {}\n```", params_text(&params, &values));
}
