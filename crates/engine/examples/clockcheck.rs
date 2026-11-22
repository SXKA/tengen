//! Print time budgets for a grid of clocks, to compare builds bit for bit.
use tengen_engine::search::Limits;
fn main() {
    for left in [500.0, 3000.0, 20000.0, 180000.0] {
        for turn in [0.0, 100.0, 1000.0, 5000.0] {
            for (empty, ply) in [(220, 5), (150, 40), (80, 90), (10, 200)] {
                let l = Limits::for_clock(left, turn, empty, ply);
                println!("{:?} {:?} {} {:?}", l.soft_ms.map(f64::to_bits), l.hard_ms.map(f64::to_bits), l.dynamic, Limits::movetime(turn).soft_ms.map(f64::to_bits));
            }
        }
    }
}
