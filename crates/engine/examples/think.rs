use tengen_engine::{Board, Engine, Limits, Pos};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ms: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3000.0);
    let threads: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut board = Board::new(15);
    for (x, y) in [(7, 7), (8, 8), (7, 8), (6, 6)] {
        board.play(Pos::new(x, y));
    }
    let mut engine = Engine::new(128);
    engine.set_threads(threads);
    let r = engine.search(
        &board,
        &Limits {
            soft_ms: Some(ms),
            hard_ms: Some(ms * 3.0),
            ..Default::default()
        },
        |i| {
            println!(
                "depth {} sel {} value {} nodes {} nps {} time {:.0} pv {:?}",
                i.depth,
                i.seldepth,
                i.value,
                i.nodes,
                i.nps(),
                i.elapsed_ms,
                i.pv.iter()
                    .map(|p| format!("{}{}", (b'a' + p.x() as u8) as char, p.y() + 1))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        },
    );
    println!("best {:?} value {}", r.best_move, r.value);
}
