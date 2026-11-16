//! Random renju positions and Tengen's forbidden points, for comparison with
//! the Gomocup judge.
use tengen_arena::openings::Rng;
use tengen_engine::pattern::Pattern4;
use tengen_engine::{Board, Pos, Rule};
fn main() {
    let count: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(200);
    let seed: u64 = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(42);
    let mut rng = Rng(seed);
    for _ in 0..count {
        let mut board = Board::with_rule(15, Rule::Renju);
        let stones = 20 + rng.below(50);
        let mut tries = 0;
        while board.ply() < stones && tries < 10_000 {
            tries += 1;
            // Mostly near the center, so shapes form.
            let (x, y) = (3 + rng.below(9), 3 + rng.below(9));
            let p = Pos::new(x, y);
            let c = board.side_to_move();
            if !board.is_empty(p) || board.p4(p, c) == Pattern4::AFive {
                continue;
            }
            // Let Black play forbidden shapes too: the judge only cares about
            // the final position.
            board.play(p);
        }
        // With a five point Black just wins, and the judge then disagrees
        // about threes completed by it; such positions never need the
        // check.
        if board.p4_count(tengen_engine::Color::Black, Pattern4::AFive) > 0 {
            continue;
        }
        let mut stones = Vec::new();
        for (i, p) in board.moves().enumerate() {
            stones.push(format!("{},{},{}", p.x(), p.y(), i % 2 + 1));
        }
        let mut forbidden = Vec::new();
        for y in 0..15 {
            for x in 0..15 {
                let p = Pos::new(x, y);
                if board.is_forbidden(p) {
                    forbidden.push(format!("{x},{y}"));
                }
            }
        }
        println!("{} | {}", stones.join(" "), forbidden.join(" "));
    }
}
