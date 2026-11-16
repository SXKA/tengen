//! Board representation with incrementally updated line patterns.
//!
//! Stones are kept as one bitmask per color, direction and line, padded by 4
//! bits on each side so a 9-cell window around any cell is a single shift.
//! Every empty cell caches, for both colors, its pattern in each direction,
//! the order-independent pattern code of those four, and their combination
//! (`Pattern4`). Placing or removing a stone only touches the 4 cells on each
//! side along the 4 lines through it.

use crate::eval;
use crate::pattern::{HALF_WINDOW, Pattern, Pattern4, PatternCode, Tables};

/// Largest supported board side.
pub const MAX_SIZE: usize = 20;
/// Row stride of [`Pos`]; a power of two so coordinates are shifts.
const STRIDE: usize = 32;
/// Number of cell slots, including the unused columns past the stride.
pub const CELLS: usize = STRIDE * MAX_SIZE;
/// Lines per direction (diagonals have `2 * size - 1`).
const LINES: usize = 2 * MAX_SIZE - 1;
/// Step between neighbouring cells for each direction, in [`Pos`] units.
const DELTA: [isize; 4] = [1, STRIDE as isize, STRIDE as isize + 1, 1 - STRIDE as isize];

/// A player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    White = 1,
}

impl Color {
    /// The other player.
    #[inline]
    pub fn opponent(self) -> Color {
        match self {
            Color::Black => Color::White,
            Color::White => Color::Black,
        }
    }

    #[inline]
    fn index(self) -> usize {
        self as usize
    }
}

/// A cell on the board, packed as `y * 32 + x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos(u16);

impl Pos {
    /// Placeholder for "no move", e.g. in empty table entries.
    pub const NONE: Pos = Pos(u16::MAX);

    /// The cell at column `x` and row `y`.
    #[inline]
    pub fn new(x: usize, y: usize) -> Pos {
        debug_assert!(x < MAX_SIZE && y < MAX_SIZE);
        Pos((y * STRIDE + x) as u16)
    }

    /// Column, starting at 0.
    #[inline]
    pub fn x(self) -> usize {
        self.0 as usize % STRIDE
    }

    /// Row, starting at 0.
    #[inline]
    pub fn y(self) -> usize {
        self.0 as usize / STRIDE
    }

    /// Index into per-cell arrays of length [`CELLS`].
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// Raw 16-bit value, for compact storage.
    #[inline]
    pub fn raw(self) -> u16 {
        self.0
    }

    /// Inverse of [`Pos::raw`].
    #[inline]
    pub fn from_raw(raw: u16) -> Pos {
        Pos(raw)
    }

    /// Chebyshev distance between two cells.
    pub fn distance(self, other: Pos) -> usize {
        self.x().abs_diff(other.x()).max(self.y().abs_diff(other.y()))
    }
}

/// Inclusive rectangle around the stones where candidate moves live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x0: u8,
    pub y0: u8,
    pub x1: u8,
    pub y1: u8,
}

impl Area {
    fn empty() -> Area {
        Area {
            x0: u8::MAX,
            y0: u8::MAX,
            x1: 0,
            y1: 0,
        }
    }
}

/// Cached per-cell state.
#[derive(Clone, Copy)]
struct Cell {
    /// 0 empty, 1 black, 2 white.
    stone: u8,
    /// Number of stones within candidate range.
    near: u8,
    /// Pattern code per color.
    pcode: [PatternCode; 2],
    /// Pattern combination per color.
    p4: [Pattern4; 2],
    /// Line pattern per color and direction.
    pattern: [[Pattern; 4]; 2],
}

const EMPTY_CELL: Cell = Cell {
    stone: 0,
    near: 0,
    pcode: [0; 2],
    p4: [Pattern4::None; 2],
    pattern: [[Pattern::Dead; 4]; 2],
};

/// Offsets (dx, dy) of cells that become candidates around a stone: the 5x5
/// square plus the cells three steps away along the eight lines.
const NEAR: [(i8, i8); 32] = {
    let mut out = [(0i8, 0i8); 32];
    let mut n = 0;
    let mut dy = -3i8;
    while dy <= 3 {
        let mut dx = -3i8;
        while dx <= 3 {
            let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
            let square = ax <= 2 && ay <= 2;
            let line = (ax == 3 || ay == 3) && (ax == 0 || ay == 0 || ax == ay);
            if !(dx == 0 && dy == 0) && (square || line) {
                out[n] = (dx, dy);
                n += 1;
            }
            dx += 1;
        }
        dy += 1;
    }
    assert!(n == 32);
    out
};

/// Undo information for one move.
#[derive(Clone, Copy)]
struct Undo {
    pos: Pos,
    area: Area,
}

/// Zobrist keys, derived deterministically so every board agrees.
struct Zobrist {
    stone: [[u64; CELLS]; 2],
    side: u64,
}

fn zobrist() -> &'static Zobrist {
    static KEYS: std::sync::OnceLock<Box<Zobrist>> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| {
        let mut state = 0x7465_6e67_656e_0001u64; // "tengen" seed
        let mut next = || {
            // splitmix64
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let mut keys = Box::new(Zobrist {
            stone: [[0; CELLS]; 2],
            side: 0,
        });
        for color in 0..2 {
            for key in keys.stone[color].iter_mut() {
                *key = next();
            }
        }
        keys.side = next();
        keys
    })
}

/// A Gomoku position with the move history that led to it.
#[derive(Clone)]
pub struct Board {
    size: usize,
    cells: Box<[Cell; CELLS]>,
    /// Stones per color, direction and line, bit `pos + 4`.
    lines: [[[u32; LINES]; 4]; 2],
    /// Off-board bits per direction and line.
    edges: [[u32; LINES]; 4],
    history: Vec<Undo>,
    area: Area,
    hash: u64,
    /// Sum of [`eval::CellValues`] over empty cells, per color.
    value: [i32; 2],
    /// Number of empty cells with each combination, per color.
    p4_count: [[u16; Pattern4::COUNT]; 2],
}

impl Board {
    /// An empty board with `size` rows and columns (5 to 20).
    pub fn new(size: usize) -> Board {
        assert!((5..=MAX_SIZE).contains(&size), "unsupported board size {size}");
        let mut board = Board {
            size,
            cells: Box::new([EMPTY_CELL; CELLS]),
            lines: [[[0; LINES]; 4]; 2],
            edges: [[0; LINES]; 4],
            history: Vec::with_capacity(size * size),
            area: Area::empty(),
            hash: 0,
            value: [0; 2],
            p4_count: [[0; Pattern4::COUNT]; 2],
        };
        for dir in 0..4 {
            for line in 0..LINES {
                let mut edge = 0u32;
                for bit in 0..32 {
                    if board.cell_at(dir, line, bit as isize - HALF_WINDOW as isize).is_none() {
                        edge |= 1 << bit;
                    }
                }
                board.edges[dir][line] = edge;
            }
        }
        let tables = Tables::get();
        for y in 0..size {
            for x in 0..size {
                board.refresh_cell(Pos::new(x, y), tables);
            }
        }
        board.recount();
        board
    }

    /// Board side length.
    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Number of stones on the board.
    #[inline]
    pub fn ply(&self) -> usize {
        self.history.len()
    }

    /// The player to move. Black moves first.
    #[inline]
    pub fn side_to_move(&self) -> Color {
        if self.history.len().is_multiple_of(2) {
            Color::Black
        } else {
            Color::White
        }
    }

    /// Zobrist hash of the position, including the side to move.
    #[inline]
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Whether `pos` is on the board.
    #[inline]
    pub fn contains(&self, pos: Pos) -> bool {
        pos.x() < self.size && pos.y() < self.size
    }

    /// The stone on `pos`, if any.
    #[inline]
    pub fn stone(&self, pos: Pos) -> Option<Color> {
        match self.cells[pos.index()].stone {
            1 => Some(Color::Black),
            2 => Some(Color::White),
            _ => None,
        }
    }

    /// Whether `pos` is on the board and empty.
    #[inline]
    pub fn is_empty(&self, pos: Pos) -> bool {
        self.contains(pos) && self.cells[pos.index()].stone == 0
    }

    /// Whether `pos` is empty and close enough to a stone to be worth playing.
    #[inline]
    pub fn is_candidate(&self, pos: Pos) -> bool {
        let cell = &self.cells[pos.index()];
        cell.stone == 0 && cell.near > 0
    }

    /// Moves played so far, oldest first.
    pub fn moves(&self) -> impl ExactSizeIterator<Item = Pos> + '_ {
        self.history.iter().map(|u| u.pos)
    }

    /// The most recent move, if any.
    #[inline]
    pub fn last_move(&self) -> Option<Pos> {
        self.history.last().map(|u| u.pos)
    }

    /// The move `back` plies ago (0 is the last move).
    #[inline]
    pub fn recent_move(&self, back: usize) -> Option<Pos> {
        self.history.len().checked_sub(back + 1).map(|i| self.history[i].pos)
    }

    /// Whether every cell is occupied.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.history.len() == self.size * self.size
    }

    /// Pattern a stone of `color` would form on `pos` along `dir`.
    #[inline]
    pub fn pattern(&self, pos: Pos, color: Color, dir: usize) -> Pattern {
        self.cells[pos.index()].pattern[color.index()][dir]
    }

    /// Pattern code of `pos` for `color`.
    #[inline]
    pub fn pcode(&self, pos: Pos, color: Color) -> PatternCode {
        self.cells[pos.index()].pcode[color.index()]
    }

    /// What a stone of `color` would form on `pos` (empty cells only).
    #[inline]
    pub fn p4(&self, pos: Pos, color: Color) -> Pattern4 {
        self.cells[pos.index()].p4[color.index()]
    }

    /// Number of empty cells where `color` would form `p4`.
    #[inline]
    pub fn p4_count(&self, color: Color, p4: Pattern4) -> u32 {
        self.p4_count[color.index()][p4 as usize] as u32
    }

    /// Number of empty cells where `color` would form at least `p4`.
    pub fn p4_count_at_least(&self, color: Color, p4: Pattern4) -> u32 {
        self.p4_count[color.index()][p4 as usize..]
            .iter()
            .map(|&n| n as u32)
            .sum()
    }

    /// Sum of empty-cell values for `color` (see [`eval::CellValues`]).
    #[inline]
    pub fn pattern_value(&self, color: Color) -> i32 {
        self.value[color.index()]
    }

    /// Rectangle containing every candidate move.
    #[inline]
    pub fn area(&self) -> Area {
        self.area
    }

    /// Every cell of the candidate area, row by row. Empty when no stone has
    /// been played.
    pub fn area_cells(&self) -> impl Iterator<Item = Pos> + use<> {
        let a = self.area;
        // An empty area has x0 > x1 and y0 > y1, so both ranges are empty.
        let (xs, ys) = (a.x0..=a.x1, a.y0..=a.y1);
        ys.flat_map(move |y| xs.clone().map(move |x| Pos::new(x as usize, y as usize)))
    }

    /// Empty cells near stones, row by row.
    pub fn candidates(&self) -> impl Iterator<Item = Pos> + '_ {
        self.area_cells().filter(|&p| self.is_candidate(p))
    }

    /// The cell `k` steps from `pos` along `dir`, if on the board.
    ///
    /// Directions: 0 horizontal, 1 vertical, 2 diagonal (down-right) and 3
    /// anti-diagonal (up-right).
    #[inline]
    pub fn step(&self, pos: Pos, dir: usize, k: isize) -> Option<Pos> {
        const DX: [isize; 4] = [1, 0, 1, 1];
        const DY: [isize; 4] = [0, 1, 1, -1];
        let n = self.size as isize;
        let x = pos.x() as isize + k * DX[dir];
        let y = pos.y() as isize + k * DY[dir];
        ((0..n).contains(&x) && (0..n).contains(&y)).then(|| Pos::new(x as usize, y as usize))
    }

    /// The first empty cell where `color` would make five, if any.
    pub fn five_point(&self, color: Color) -> Option<Pos> {
        if self.p4_count(color, Pattern4::AFive) == 0 {
            return None;
        }
        self.area_cells()
            .find(|&p| self.is_empty(p) && self.p4(p, color) == Pattern4::AFive)
    }

    /// The board center, the usual first move.
    pub fn center(&self) -> Pos {
        Pos::new(self.size / 2, self.size / 2)
    }

    /// Place a stone for the side to move on an empty cell.
    pub fn play(&mut self, pos: Pos) {
        debug_assert!(self.is_empty(pos), "illegal move {pos:?}");
        let color = self.side_to_move();
        self.history.push(Undo { pos, area: self.area });
        self.hash ^= zobrist().stone[color.index()][pos.index()] ^ zobrist().side;
        self.set_stone(pos, Some(color));
        self.update_near(pos, 1);
        self.extend_area(pos);
    }

    /// Take back the last move.
    pub fn undo(&mut self) {
        let Undo { pos, area } = self.history.pop().expect("no move to undo");
        let color = self.side_to_move();
        self.hash ^= zobrist().stone[color.index()][pos.index()] ^ zobrist().side;
        self.set_stone(pos, None);
        self.update_near(pos, -1);
        self.area = area;
    }

    /// Pass the turn without playing, for null-move style probes. Undo with
    /// [`Board::undo_pass`].
    pub fn pass(&mut self) {
        self.history.push(Undo {
            pos: Pos::NONE,
            area: self.area,
        });
        self.hash ^= zobrist().side;
    }

    /// Undo a [`Board::pass`].
    pub fn undo_pass(&mut self) {
        let undo = self.history.pop().expect("no pass to undo");
        debug_assert_eq!(undo.pos, Pos::NONE);
        self.hash ^= zobrist().side;
    }

    /// The cell at `pos` along line `line` of `dir`, if on the board.
    fn cell_at(&self, dir: usize, line: usize, pos: isize) -> Option<Pos> {
        let n = self.size as isize;
        let line = line as isize;
        let (x, y) = match dir {
            0 => (pos, line),
            1 => (line, pos),
            2 => (pos + line - (n - 1), pos),
            _ => (pos, line - pos),
        };
        (0..n).contains(&x).then_some(())?;
        (0..n).contains(&y).then_some(())?;
        Some(Pos::new(x as usize, y as usize))
    }

    /// Line index and position along the line of `pos` in direction `dir`.
    #[inline]
    fn line_of(&self, pos: Pos, dir: usize) -> (usize, usize) {
        let (x, y) = (pos.x(), pos.y());
        match dir {
            0 => (y, x),
            1 => (x, y),
            2 => (x + self.size - 1 - y, y),
            _ => (x + y, x),
        }
    }

    /// Recompute the patterns of an empty `pos` from the line masks.
    #[inline]
    fn compute_patterns(&self, pos: Pos, tables: &Tables) -> [[Pattern; 4]; 2] {
        let pairs = [0, 1, 2, 3].map(|dir| {
            let (line, at) = self.line_of(pos, dir);
            let black = self.lines[0][dir][line] >> at;
            let white = self.lines[1][dir][line] >> at;
            let edge = self.edges[dir][line] >> at;
            (tables.line(black, white | edge), tables.line(white, black | edge))
        });
        [pairs.map(|p| p.0), pairs.map(|p| p.1)]
    }

    /// Remove an empty cell's contribution to the running totals.
    #[inline]
    fn detach(&mut self, pos: Pos, values: &eval::CellValues) {
        let cell = self.cells[pos.index()];
        if cell.stone == 0 {
            for c in 0..2 {
                self.value[c] -= values.value(cell.pcode[c]);
                self.p4_count[c][cell.p4[c] as usize] -= 1;
            }
        }
    }

    /// Add an empty cell's contribution to the running totals.
    #[inline]
    fn attach(&mut self, pos: Pos, values: &eval::CellValues) {
        let cell = self.cells[pos.index()];
        if cell.stone == 0 {
            for c in 0..2 {
                self.value[c] += values.value(cell.pcode[c]);
                self.p4_count[c][cell.p4[c] as usize] += 1;
            }
        }
    }

    /// Recompute the cached patterns of `pos` (no totals).
    #[inline]
    fn refresh_cell(&mut self, pos: Pos, tables: &Tables) {
        let patterns = self.compute_patterns(pos, tables);
        let cell = &mut self.cells[pos.index()];
        cell.pattern = patterns;
        cell.pcode = patterns.map(|p| tables.pcode(p));
        cell.p4 = cell.pcode.map(|code| tables.pattern4(code));
    }

    /// Recompute the running totals from scratch.
    fn recount(&mut self) {
        let values = eval::CellValues::standard();
        self.value = [0; 2];
        self.p4_count = [[0; Pattern4::COUNT]; 2];
        for y in 0..self.size {
            for x in 0..self.size {
                self.attach(Pos::new(x, y), values);
            }
        }
    }

    /// Put or remove a stone and update every cell whose patterns depend on it.
    fn set_stone(&mut self, pos: Pos, stone: Option<Color>) {
        let tables = Tables::get();
        let values = eval::CellValues::standard();
        self.detach(pos, values);
        let bits = [0usize, 1, 2, 3].map(|dir| {
            let (line, at) = self.line_of(pos, dir);
            (line, 1u32 << (at + HALF_WINDOW as usize))
        });
        let (color, value) = match stone {
            Some(c) => (c, c.index() as u8 + 1),
            None => {
                let c = self.stone(pos).expect("removing from an empty cell");
                (c, 0)
            }
        };
        for (dir, &(line, bit)) in bits.iter().enumerate() {
            self.lines[color.index()][dir][line] ^= bit;
        }
        self.cells[pos.index()].stone = value;
        self.refresh_cell(pos, tables);
        self.attach(pos, values);

        for delta in DELTA {
            for side in [-1isize, 1] {
                let mut q = pos.index() as isize;
                for _ in 0..HALF_WINDOW {
                    q += side * delta;
                    let Some(next) = self.on_board(q) else { break };
                    if self.cells[next.index()].stone != 0 {
                        continue;
                    }
                    self.detach(next, values);
                    self.refresh_cell(next, tables);
                    self.attach(next, values);
                }
            }
        }
    }

    /// `q` as a board cell if it is on the board. Every step changes the
    /// column by at most one, so leaving the board always lands in a padding
    /// column (`x >= size`), a row past the last, or a negative index.
    #[inline]
    fn on_board(&self, q: isize) -> Option<Pos> {
        if q < 0 || q as usize >= CELLS {
            return None;
        }
        let p = Pos(q as u16);
        (p.x() < self.size && p.y() < self.size).then_some(p)
    }

    fn update_near(&mut self, pos: Pos, delta: i8) {
        let n = self.size as i32;
        for &(dx, dy) in NEAR.iter() {
            let (x, y) = (pos.x() as i32 + dx as i32, pos.y() as i32 + dy as i32);
            if (0..n).contains(&x) && (0..n).contains(&y) {
                let cell = &mut self.cells[Pos::new(x as usize, y as usize).index()];
                cell.near = cell.near.wrapping_add_signed(delta);
            }
        }
    }

    fn extend_area(&mut self, pos: Pos) {
        let r = 3;
        let max = self.size - 1;
        let (x, y) = (pos.x(), pos.y());
        let a = &mut self.area;
        a.x0 = a.x0.min(x.saturating_sub(r) as u8);
        a.y0 = a.y0.min(y.saturating_sub(r) as u8);
        a.x1 = a.x1.max((x + r).min(max) as u8);
        a.y1 = a.y1.max((y + r).min(max) as u8);
    }
}

impl std::fmt::Display for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let last = self.last_move();
        write!(f, "  ")?;
        for x in 0..self.size {
            write!(f, " {}", (b'a' + x as u8) as char)?;
        }
        writeln!(f)?;
        for y in 0..self.size {
            write!(f, "{:2}", y + 1)?;
            for x in 0..self.size {
                let p = Pos::new(x, y);
                let mark = match self.stone(p) {
                    Some(Color::Black) => 'X',
                    Some(Color::White) => 'O',
                    None => '.',
                };
                let sep = if Some(p) == last { '[' } else { ' ' };
                write!(f, "{sep}{mark}")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny deterministic generator for tests.
    pub(crate) struct Rng(u64);
    impl Rng {
        pub(crate) fn new(seed: u64) -> Rng {
            Rng(seed)
        }
        pub(crate) fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    /// Rebuild a board from scratch by replaying its moves.
    fn replay(board: &Board) -> Board {
        let mut fresh = Board::new(board.size());
        for pos in board.moves().collect::<Vec<_>>() {
            fresh.play(pos);
        }
        fresh
    }

    fn assert_consistent(board: &Board) {
        let mut fresh = Board::new(board.size());
        // Place stones directly, then recompute everything from the masks.
        for (i, pos) in board.moves().enumerate() {
            let color = if i % 2 == 0 { Color::Black } else { Color::White };
            fresh.lines_set(pos, color);
        }
        for y in 0..board.size {
            for x in 0..board.size {
                let p = Pos::new(x, y);
                if board.is_empty(p) {
                    let t = Tables::get();
                    assert_eq!(fresh.compute_patterns(p, t), board.cells[p.index()].pattern, "{p:?}");
                }
            }
        }
        let mut recount = board.clone();
        recount.recount();
        assert_eq!(recount.value, board.value);
        assert_eq!(recount.p4_count, board.p4_count);
        assert_eq!(replay(board).hash, board.hash);
    }

    impl Board {
        fn lines_set(&mut self, pos: Pos, color: Color) {
            for dir in 0..4 {
                let (line, at) = self.line_of(pos, dir);
                self.lines[color.index()][dir][line] |= 1 << (at + HALF_WINDOW as usize);
            }
            self.cells[pos.index()].stone = color.index() as u8 + 1;
        }
    }

    #[test]
    fn random_play_and_undo_stay_consistent() {
        let mut rng = Rng::new(0x1234_5678);
        for &size in &[15, 20, 7] {
            let mut board = Board::new(size);
            let empty_hash = board.hash();
            for _ in 0..3 {
                let n = size * size * 2 / 3;
                for _ in 0..n {
                    let empties: Vec<Pos> = (0..size * size)
                        .map(|i| Pos::new(i % size, i / size))
                        .filter(|&p| board.is_empty(p))
                        .collect();
                    board.play(empties[rng.below(empties.len())]);
                }
                assert_consistent(&board);
                for _ in 0..n / 2 {
                    board.undo();
                }
                assert_consistent(&board);
                while board.ply() > 0 {
                    board.undo();
                }
                assert_eq!(board.hash(), empty_hash);
                assert_eq!(board.area(), Area::empty());
            }
        }
    }

    #[test]
    fn edges_block_lines() {
        let mut board = Board::new(15);
        // Three stones against the left edge: the edge blocks one side, so
        // both (3, y) and (4, y) only make a four, never an open four.
        for x in 0..3 {
            board.play(Pos::new(x, 0));
            board.play(Pos::new(x, 5));
        }
        assert_eq!(board.pattern(Pos::new(3, 0), Color::Black, 0), Pattern::B4);
        assert_eq!(board.p4(Pos::new(3, 0), Color::Black), Pattern4::EBlock4);
        assert_eq!(board.p4(Pos::new(4, 0), Color::Black), Pattern4::EBlock4);
        assert_eq!(board.pattern(Pos::new(3, 5), Color::White, 0), Pattern::B4);
        assert_eq!(board.p4_count(Color::Black, Pattern4::EBlock4), 2);
        assert_eq!(board.p4_count_at_least(Color::Black, Pattern4::BFlex4), 0);
    }

    #[test]
    fn diagonals_wrap_correctly() {
        let mut board = Board::new(15);
        // A diagonal from (11,0) to (14,3) must not continue onto the next row.
        for i in 0..4 {
            board.play(Pos::new(11 + i, i));
            board.play(Pos::new(0, 14 - i));
        }
        // The diagonal is capped by the right edge after (14,3): only (10,-1)
        // would extend it, which is off the board, so there is no five point.
        assert_eq!(board.p4_count_at_least(Color::Black, Pattern4::AFive), 0);
        // White's vertical four on column 0 from row 11 to 14 has one five
        // point at (0,10).
        assert_eq!(board.p4(Pos::new(0, 10), Color::White), Pattern4::AFive);
        assert_eq!(board.p4_count(Color::White, Pattern4::AFive), 1);
        assert!(board.to_string().contains('['));
    }

    #[test]
    fn candidates_surround_stones() {
        let mut board = Board::new(15);
        assert_eq!(board.candidates().count(), 0);
        board.play(Pos::new(7, 7));
        assert_eq!(board.candidates().count(), 32);
        board.play(Pos::new(0, 0));
        // Corner stone adds the in-board part of its neighbourhood.
        assert!(board.is_candidate(Pos::new(2, 2)));
        assert!(board.is_candidate(Pos::new(3, 0)));
        assert!(!board.is_candidate(Pos::new(3, 1)));
        board.undo();
        assert!(!board.is_candidate(Pos::new(2, 2)));
    }
}
