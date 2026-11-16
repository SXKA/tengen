//! Tengen for the browser, as a WebAssembly module with a plain C ABI.
//!
//! One game lives inside the module. The host (a Web Worker) calls the
//! `tg_*` exports and provides two imports in the `env` module:
//!
//! * `tg_now() -> f64`: milliseconds from any epoch, for time limits;
//! * `tg_info(depth, seldepth, value, nodes, elapsed_ms, pv_ptr, pv_len)`:
//!   progress after each search iteration, `pv_ptr` pointing at `pv_len`
//!   packed moves (`x | y << 8`) in the module's memory.
//!
//! Moves cross the boundary packed as `x | y << 8`, with -1 for "none".

use std::sync::Mutex;

use tengen_engine::eval::{self, SCALE};
use tengen_engine::pattern::{Pattern4, Tables};
use tengen_engine::search::{MATE, MATE_BOUND};
use tengen_engine::{Board, Color, Engine, Info, Limits, Pos, Rule};

#[cfg(target_arch = "wasm32")]
mod host {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        pub fn tg_now() -> f64;
        pub fn tg_info(depth: i32, seldepth: i32, value: i32, nodes: f64, elapsed_ms: f64, pv: *const u16, pv_len: u32);
    }
}

/// Native stand-ins so the crate builds and tests off the web.
#[cfg(not(target_arch = "wasm32"))]
mod host {
    pub unsafe fn tg_now() -> f64 {
        tengen_engine::search::default_clock()
    }
    pub unsafe fn tg_info(_: i32, _: i32, _: i32, _: f64, _: f64, _: *const u16, _: u32) {}
}

fn now() -> f64 {
    // SAFETY: the host import takes no arguments and only reads a clock.
    unsafe { host::tg_now() }
}

fn report(info: &Info) {
    let pv: Vec<u16> = info.pv.iter().map(|&p| pack(p) as u16).collect();
    // SAFETY: the pointer and length describe `pv`, alive for the call.
    unsafe {
        host::tg_info(
            info.depth,
            info.seldepth as i32,
            info.value,
            info.nodes as f64,
            info.elapsed_ms,
            pv.as_ptr(),
            pv.len() as u32,
        );
    }
}

struct Game {
    board: Board,
    engine: Engine,
    /// Cells of the five that ended the game, if it has ended that way.
    five: Vec<Pos>,
}

static GAME: Mutex<Option<Game>> = Mutex::new(None);
static LINE: Mutex<[u16; 16]> = Mutex::new([0; 16]);

fn with_game<T>(default: T, f: impl FnOnce(&mut Game) -> T) -> T {
    let mut game = GAME.lock().unwrap_or_else(|e| e.into_inner());
    game.as_mut().map_or(default, f)
}

fn pack(pos: Pos) -> i32 {
    (pos.x() | pos.y() << 8) as i32
}

fn cell(board: &Board, x: u32, y: u32) -> Option<Pos> {
    let (x, y) = (x as usize, y as usize);
    (x < board.size() && y < board.size()).then(|| Pos::new(x, y))
}

/// The stones in a row with `pos` along `dir`, `pos` included, sorted.
fn line_through(board: &Board, pos: Pos, dir: usize) -> Vec<Pos> {
    let color = board.stone(pos);
    let mut line = vec![pos];
    for side in [-1isize, 1] {
        let mut k = side;
        while let Some(q) = board.step(pos, dir, k)
            && board.stone(q) == color
        {
            line.push(q);
            k += side;
        }
    }
    line.sort();
    line
}

/// Start a new game on a `size` board (5 to 20) under `rule`, given by its
/// Piskvork number (0 freestyle, 1 standard, 4 renju, 8 freestyle caro,
/// 9 caro). Returns 0, or -1 for a bad size. The search table is kept but
/// cleared.
#[unsafe(no_mangle)]
pub extern "C" fn tg_new(size: u32, rule: u32) -> i32 {
    if !(5..=tengen_engine::board::MAX_SIZE as u32).contains(&size) {
        return -1;
    }
    let mut game = GAME.lock().unwrap_or_else(|e| e.into_inner());
    let engine = match game.take() {
        Some(mut old) => {
            old.engine.clear();
            old.engine
        }
        None => {
            let mut engine = Engine::new(64);
            engine.set_clock(now);
            engine
        }
    };
    *game = Some(Game {
        board: Board::with_rule(size as usize, Rule::from_piskvork(rule)),
        engine,
        five: Vec::new(),
    });
    0
}

/// Use `threads` search threads. With more than one, the host must have
/// started `threads - 1` helpers with [`tg_helper`]; builds without shared
/// memory always search with one.
#[unsafe(no_mangle)]
pub extern "C" fn tg_set_threads(threads: u32) {
    let threads = if cfg!(target_feature = "atomics") {
        threads.max(1)
    } else {
        1
    };
    with_game((), |g| g.engine.set_threads(threads as usize));
}

/// Whether this build can search with several threads (shared memory).
#[unsafe(no_mangle)]
pub extern "C" fn tg_threaded() -> i32 {
    cfg!(target_feature = "atomics") as i32
}

/// Run helper thread `id` (1 or more) forever. Call it from a separate
/// instance of the module sharing this one's memory, after giving that
/// instance its own stack (`__stack_pointer`) and thread-local storage
/// (`__wasm_init_tls`).
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
#[unsafe(no_mangle)]
pub extern "C" fn tg_helper(id: u32) {
    tengen_engine::search::pool::helper(id.max(1) as usize)
}

/// Allocate `size` bytes aligned to `align` that are never freed, for helper
/// stacks and thread-local blocks. Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn tg_alloc(size: u32, align: u32) -> *mut u8 {
    match std::alloc::Layout::from_size_align(size.max(1) as usize, align.max(1) as usize) {
        // SAFETY: the layout has a non-zero size.
        Ok(layout) => unsafe { std::alloc::alloc_zeroed(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Resize the search table.
#[unsafe(no_mangle)]
pub extern "C" fn tg_set_hash(megabytes: u32) {
    with_game((), |g| g.engine.set_hash(megabytes.clamp(1, 1024) as usize));
}

/// Play a stone for the side to move.
///
/// Returns -1 if illegal (occupied, off the board, forbidden for Black
/// under renju rules, or the game is over), 0 if the game goes on, 1 or 2
/// if Black or White just won, and 3 if the board is full.
#[unsafe(no_mangle)]
pub extern "C" fn tg_play(x: u32, y: u32) -> i32 {
    with_game(-1, |g| {
        let Some(pos) = cell(&g.board, x, y) else { return -1 };
        if !g.five.is_empty() || !g.board.is_legal(pos) {
            return -1;
        }
        let color = g.board.side_to_move();
        let wins = g.board.p4(pos, color) == Pattern4::AFive;
        let dir = g.board.five_direction(pos, color);
        g.board.play(pos);
        if let (true, Some(dir)) = (wins, dir) {
            g.five = line_through(&g.board, pos, dir);
            return color as i32 + 1;
        }
        if g.board.is_full() { 3 } else { 0 }
    })
}

/// Take back the last move. Returns 0, or -1 if there is none.
#[unsafe(no_mangle)]
pub extern "C" fn tg_undo() -> i32 {
    with_game(-1, |g| {
        if g.board.ply() == 0 {
            return -1;
        }
        g.board.undo();
        g.five.clear();
        0
    })
}

/// Search for the side to move for about `ms` milliseconds (and at most
/// `max_depth` plies if positive). Returns the packed best move, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn tg_think(ms: f64, max_depth: i32) -> i32 {
    with_game(-1, |g| {
        if !g.five.is_empty() || g.board.is_full() {
            return -1;
        }
        let mut limits = Limits::movetime(ms.max(1.0));
        if max_depth > 0 {
            limits.depth = Some(max_depth);
        }
        g.engine.search(&g.board, &limits, report).best_move.map_or(-1, pack)
    })
}

/// Number of stones played.
#[unsafe(no_mangle)]
pub extern "C" fn tg_ply() -> u32 {
    with_game(0, |g| g.board.ply() as u32)
}

/// The `index`-th move played (from 0), packed, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn tg_move(index: u32) -> i32 {
    with_game(-1, |g| g.board.moves().nth(index as usize).map_or(-1, pack))
}

/// 0 for an empty cell, 1 for Black, 2 for White, -1 off the board.
#[unsafe(no_mangle)]
pub extern "C" fn tg_stone(x: u32, y: u32) -> i32 {
    with_game(-1, |g| match cell(&g.board, x, y).map(|p| g.board.stone(p)) {
        Some(Some(Color::Black)) => 1,
        Some(Some(Color::White)) => 2,
        Some(None) => 0,
        None => -1,
    })
}

/// What a stone of `color` (1 Black, 2 White) would make on an empty cell,
/// as a combination class from 0 (nothing) to 12 (five). -1 if occupied,
/// -2 where Black may not play under renju rules.
#[unsafe(no_mangle)]
pub extern "C" fn tg_threat(x: u32, y: u32, color: u32) -> i32 {
    with_game(-1, |g| {
        let color = if color == 1 { Color::Black } else { Color::White };
        match cell(&g.board, x, y) {
            Some(p) if g.board.is_empty(p) => match g.board.p4(p, color) {
                Pattern4::Forbidden if g.board.is_forbidden(p) => -2,
                // A double three that turned out legal.
                Pattern4::Forbidden => class(Tables::get().pattern4(g.board.pcode(p, color))),
                p4 => class(p4),
            },
            _ => -1,
        }
    })
}

/// A combination as 0 (nothing) to 12 (five).
fn class(p4: Pattern4) -> i32 {
    match p4 {
        Pattern4::None | Pattern4::Forbidden => 0,
        p4 => p4 as i32 - 1,
    }
}

/// Static evaluation for the side to move.
#[unsafe(no_mangle)]
pub extern "C" fn tg_eval() -> i32 {
    with_game(0, |g| eval::evaluate(&g.board))
}

/// Write the winning five's cells (packed) to an internal buffer and return
/// how many there are; read them at [`tg_line_ptr`].
#[unsafe(no_mangle)]
pub extern "C" fn tg_five_line() -> u32 {
    with_game(0, |g| {
        let mut line = LINE.lock().unwrap_or_else(|e| e.into_inner());
        let n = g.five.len().min(line.len());
        for (slot, &p) in line.iter_mut().zip(&g.five[..n]) {
            *slot = pack(p) as u16;
        }
        n as u32
    })
}

/// Address of the buffer filled by [`tg_five_line`].
#[unsafe(no_mangle)]
pub extern "C" fn tg_line_ptr() -> *const u16 {
    LINE.lock().unwrap_or_else(|e| e.into_inner()).as_ptr()
}

/// Score of a win on the next ply; scores within 1000 of it are mates.
#[unsafe(no_mangle)]
pub extern "C" fn tg_mate() -> i32 {
    debug_assert_eq!(MATE - MATE_BOUND, 1000);
    MATE
}

/// Evaluation units per logistic unit, to turn scores into win rates.
#[unsafe(no_mangle)]
pub extern "C" fn tg_scale() -> f64 {
    SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_through_the_c_abi() {
        assert_eq!(tg_new(3, 0), -1);
        assert_eq!(tg_new(15, 0), 0);
        assert_eq!(tg_play(7, 7), 0);
        assert_eq!(tg_play(7, 7), -1);
        assert_eq!(tg_stone(7, 7), 1);
        assert_eq!(tg_ply(), 1);
        let reply = tg_think(50.0, 4);
        assert!(reply >= 0);
        assert_eq!(tg_play(reply as u32 & 0xff, reply as u32 >> 8), 0);
        assert_eq!(tg_undo(), 0);
        assert_eq!(tg_undo(), 0);
        assert_eq!(tg_undo(), -1);
        // Black makes five on row 0 while White plays row 2.
        for x in 0..4 {
            assert_eq!(tg_play(x, 0), 0);
            assert_eq!(tg_play(x, 2), 0);
        }
        assert_eq!(tg_threat(4, 0, 1), 12);
        assert_eq!(tg_play(4, 0), 1);
        assert_eq!(tg_play(5, 5), -1);
        assert_eq!(tg_five_line(), 5);
        assert_eq!(tg_think(10.0, 1), -1);
        assert_eq!(tg_move(0), 0);
        assert!(tg_scale() > 0.0 && tg_mate() == MATE);

        // Renju: Black's double three at (8, 7) cannot be played, and
        // White's overline wins.
        assert_eq!(tg_new(15, 4), 0);
        for (x, y) in [(6, 7), (0, 0), (7, 7), (0, 14), (8, 5), (14, 0), (8, 6), (14, 14)] {
            assert_eq!(tg_play(x, y), 0);
        }
        assert_eq!(tg_threat(8, 7, 1), -2);
        assert_eq!(tg_play(8, 7), -1);
        // Standard: an overline does not win.
        assert_eq!(tg_new(15, 1), 0);
        for x in [0, 1, 2, 4, 5] {
            assert_eq!(tg_play(x, 0), 0);
            assert_eq!(tg_play(x, 10), 0);
        }
        assert_eq!(tg_play(3, 0), 0);
        assert_eq!(tg_five_line(), 0);
    }
}
