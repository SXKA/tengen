//! Random positions per rule and the cells where a stone would win, for
//! comparison with the Gomocup judge.
use tengen_arena::openings::Rng;
use tengen_engine::pattern::Pattern4;
use tengen_engine::{Board, Color, Pos, Rule};
fn main() {
    let rule = Rule::parse(&std::env::args().nth(1).unwrap()).unwrap();
    let count: usize = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(200);
    let mut rng = Rng(rule.piskvork() as u64 + 99);
    for _ in 0..count {
        let mut board = Board::with_rule(15, rule);
        let stones = 20 + rng.below(80);
        let mut tries = 0;
        while board.ply() < stones && tries < 10_000 {
            tries += 1;
            // Edges included, to exercise caro's edge rule.
            let p = Pos::new(rng.below(15), rng.below(15));
            let c = board.side_to_move();
            if !board.is_empty(p) || board.p4(p, c) == Pattern4::AFive {
                continue;
            }
            board.play(p);
        }
        let stones: Vec<String> = board
            .moves()
            .enumerate()
            .map(|(i, p)| format!("{},{},{}", p.x(), p.y(), i % 2 + 1))
            .collect();
        let mut wins = Vec::new();
        for y in 0..15 {
            for x in 0..15 {
                for (c, color) in [(1, Color::Black), (2, Color::White)] {
                    let p = Pos::new(x, y);
                    if board.is_empty(p) && board.p4(p, color) == Pattern4::AFive {
                        wins.push(format!("{x},{y},{c}"));
                    }
                }
            }
        }
        println!("{} | {}", stones.join(" "), wins.join(" "));
    }
}
