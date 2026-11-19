//! Board representation with incrementally updated line patterns.
//!
//! Stones are kept as one bitmask per color, direction and line, padded by 5
//! bits on each side so the window around any cell is a single shift.
//! Every empty cell caches, for both colors, its pattern in each direction,
//! the order-independent pattern code of those four, and their combination
//! (`Pattern4`). Placing or removing a stone only touches the cells within a
//! window's reach (4 in freestyle, 5 otherwise) along the 4 lines through
//! it.
//!
//! Under renju rules the cached combination of a cell where Black may not
//! play is [`Pattern4::Forbidden`]. Overlines and double fours are exact;
//! double threes are candidates until [`Board::is_forbidden`] checks that
//! both threes can really become straight fours.

use crate::eval;
use crate::pattern::{LINE_FORBIDDEN, LineTable, Pattern, Pattern4, PatternCode, Tables};
use crate::rule::Rule;

/// Largest supported board side.
pub const MAX_SIZE: usize = 20;
/// Bits of padding on each side of a line mask: half the widest window.
const PAD: usize = 5;
/// Row stride of [`Pos`]; a power of two so coordinates are shifts.
const STRIDE: usize = 32;
/// Number of cell slots, including the unused columns past the stride.
pub const CELLS: usize = STRIDE * MAX_SIZE;
/// Rows of off-board cells above and below the board in [`Board`]'s cell
/// array, so that walking up to 5 steps from any cell never leaves it (a
/// step left from column 0 lands at the end of the row above, hence one row
/// more than 5).
const PAD_ROWS: usize = 6;
/// Cell array length, padding included.
const SLOTS: usize = STRIDE * (MAX_SIZE + 2 * PAD_ROWS);

/// Index of `pos` in the padded cell array.
#[inline]
fn slot(pos: Pos) -> usize {
    pos.index() + STRIDE * PAD_ROWS
}
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
    pub(crate) fn index(self) -> usize {
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

/// Iterator over the cells of an [`Area`], row by row.
pub struct AreaCells {
    x: u8,
    y: u8,
    x0: u8,
    x1: u8,
    y1: u8,
}

impl Iterator for AreaCells {
    type Item = Pos;

    #[inline]
    fn next(&mut self) -> Option<Pos> {
        // An empty area has x0 > x1 and y0 > y1.
        if self.y > self.y1 || self.x > self.x1 {
            return None;
        }
        let pos = Pos::new(self.x as usize, self.y as usize);
        if self.x == self.x1 {
            self.x = self.x0;
            self.y += 1;
        } else {
            self.x += 1;
        }
        Some(pos)
    }
}

/// Iterator of [`Board::candidates`].
pub struct Candidates<'a> {
    cand: &'a [u32; MAX_SIZE],
    y: usize,
    y1: usize,
    xmask: u32,
    row: u32,
}

impl Iterator for Candidates<'_> {
    type Item = Pos;

    #[inline]
    fn next(&mut self) -> Option<Pos> {
        while self.row == 0 {
            if self.y >= self.y1 {
                return None;
            }
            self.y += 1;
            self.row = self.cand[self.y] & self.xmask;
        }
        let x = self.row.trailing_zeros() as usize;
        self.row &= self.row - 1;
        Some(Pos::new(x, self.y))
    }
}

/// Stone codes of [`Cell::stone`].
const EMPTY: u8 = 0;
const BLACK: u8 = 1;
const WALL: u8 = 3;
/// Off the board: never empty, never updated.
const OFF: u8 = 4;

/// Pattern of a line-table entry (low four bits; see [`LINE_FORBIDDEN`]).
const PATTERN_OF: [Pattern; 16] = {
    let mut out = [Pattern::Dead; 16];
    let mut i = 0;
    while i < Pattern::COUNT {
        out[i] = Pattern::ALL[i];
        i += 1;
    }
    out
};

/// Cached per-cell state.
#[derive(Clone, Copy)]
struct Cell {
    /// 0 empty, 1 black, 2 white, 3 a stone of neither (continuous games).
    stone: u8,
    /// Pattern code per color.
    pcode: [PatternCode; 2],
    /// Pattern combination per color.
    p4: [Pattern4; 2],
    /// Line pattern per color and direction.
    pattern: [[Pattern; 4]; 2],
    /// Per color, one bit per direction whose line alone forbids the move
    /// (renju Black only).
    flags: [u8; 2],
}

const EMPTY_CELL: Cell = Cell {
    stone: EMPTY,
    pcode: [0; 2],
    p4: [Pattern4::None; 2],
    pattern: [[Pattern::Dead; 4]; 2],
    flags: [0; 2],
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
    /// Length of [`Board::journal`] before the move.
    journal_len: u32,
    value: [i32; 2],
    value_late: [i32; 2],
    p4_count: [[u16; Pattern4::COUNT]; 2],
    windows: [[i16; 6]; 2],
    /// Candidate rows `pos.y - 3 ..= pos.y + 3` before the move.
    cand_rows: [u32; 7],
}

/// Zobrist keys, derived deterministically so every board agrees.
struct Zobrist {
    stone: [[u64; CELLS]; 2],
    side: u64,
    /// Per rule, so positions under different rules never share entries.
    /// Freestyle's is zero.
    rule: [u64; Rule::COUNT],
    wall: [u64; CELLS],
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
            rule: [0; Rule::COUNT],
            wall: [0; CELLS],
        });
        for key in keys.stone.iter_mut().flatten() {
            *key = next();
        }
        keys.side = next();
        for key in keys.rule.iter_mut().skip(1) {
            *key = next();
        }
        for key in keys.wall.iter_mut() {
            *key = next();
        }
        keys
    })
}

/// A Gomoku position with the move history that led to it.
#[derive(Clone)]
pub struct Board {
    size: usize,
    rule: Rule,
    /// Per-cell state, with off-board cells around the board.
    cells: Box<[Cell; SLOTS]>,
    /// Stones per color, direction and line, bit `pos + 5`.
    lines: [[[u32; LINES]; 4]; 2],
    /// Stones of neither color, the same way.
    walls: [[u32; LINES]; 4],
    /// Off-board bits per direction and line.
    edges: [[u32; LINES]; 4],
    /// Per-color 11-cell tables, or `None` for freestyle's 9-cell table.
    large: Option<[&'static LineTable; 2]>,
    /// Whether Black has forbidden moves.
    renju: bool,
    /// Evaluation tables for the rule.
    values: &'static eval::CellValues,
    walls_placed: usize,
    history: Vec<Undo>,
    /// Cells as they were before the moves in `history` changed them, so
    /// undoing a move copies them back instead of recomputing patterns.
    journal: Vec<(u16, Cell)>,
    area: Area,
    /// Candidate cells ([`Board::is_candidate`]), one bit per column, by
    /// row: kept with each move so listing them skips the scan.
    cand: [u32; MAX_SIZE],
    hash: u64,
    /// Sum of [`eval::CellValues`] over empty cells, per color.
    value: [i32; 2],
    /// The same with the late-game table.
    value_late: [i32; 2],
    /// Number of empty cells with each combination, per color.
    p4_count: [[u16; Pattern4::COUNT]; 2],
    /// Five-cell windows on the board still open to each color (none of the
    /// opponent's stones or walls in them), by how many of its own stones
    /// they hold: room the color has left to make five.
    windows: [[i16; 6]; 2],
}

impl Board {
    /// An empty freestyle board with `size` rows and columns (5 to 20).
    pub fn new(size: usize) -> Board {
        Board::with_rule(size, Rule::Freestyle)
    }

    /// An empty board with `size` rows and columns (5 to 20) for `rule`.
    pub fn with_rule(size: usize, rule: Rule) -> Board {
        assert!((5..=MAX_SIZE).contains(&size), "unsupported board size {size}");
        let large = LineTable::get(rule, true)
            .zip(LineTable::get(rule, false))
            .map(|(b, w)| [b, w]);
        let mut board = Board {
            size,
            rule,
            cells: Box::new(
                [Cell {
                    stone: OFF,
                    ..EMPTY_CELL
                }; SLOTS],
            ),
            lines: [[[0; LINES]; 4]; 2],
            walls: [[0; LINES]; 4],
            edges: [[0; LINES]; 4],
            large,
            renju: rule == Rule::Renju,
            values: eval::CellValues::for_rule(rule),
            walls_placed: 0,
            history: Vec::with_capacity(size * size),
            journal: Vec::with_capacity(40 * size * size),
            area: Area::empty(),
            cand: [0; MAX_SIZE],
            hash: zobrist().rule[rule.index()],
            value: [0; 2],
            value_late: [0; 2],
            p4_count: [[0; Pattern4::COUNT]; 2],
            windows: [[0; 6]; 2],
        };
        for dir in 0..4 {
            for line in 0..LINES {
                let mut edge = 0u32;
                for bit in 0..32 {
                    if board.cell_at(dir, line, bit as isize - PAD as isize).is_none() {
                        edge |= 1 << bit;
                    }
                }
                board.edges[dir][line] = edge;
                // Every window on an empty board is open to both colors.
                for start in 0..=32 - 5 {
                    if edge & (0x1f << start) == 0 {
                        board.windows[0][0] += 1;
                        board.windows[1][0] += 1;
                    }
                }
            }
        }
        let tables = Tables::get();
        for y in 0..size {
            for x in 0..size {
                board.cells[slot(Pos::new(x, y))].stone = EMPTY;
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

    /// The rules of the game on this board.
    #[inline]
    pub fn rule(&self) -> Rule {
        self.rule
    }

    /// The evaluation tables in use on this board.
    #[inline]
    pub fn values(&self) -> &'static eval::CellValues {
        self.values
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

    /// The stone on `pos`, if it belongs to a player.
    #[inline]
    pub fn stone(&self, pos: Pos) -> Option<Color> {
        match self.cells[slot(pos)].stone {
            1 => Some(Color::Black),
            2 => Some(Color::White),
            _ => None,
        }
    }

    /// Whether `pos` holds a stone of neither player (see
    /// [`Board::place_wall`]).
    #[inline]
    pub fn is_wall(&self, pos: Pos) -> bool {
        self.contains(pos) && self.cells[slot(pos)].stone == WALL
    }

    /// Whether `pos` is on the board and empty.
    #[inline]
    pub fn is_empty(&self, pos: Pos) -> bool {
        self.contains(pos) && self.cells[slot(pos)].stone == EMPTY
    }

    /// Whether `pos` is empty and close enough to a stone to be worth playing.
    #[inline]
    pub fn is_candidate(&self, pos: Pos) -> bool {
        pos.y() < MAX_SIZE && (self.cand[pos.y()] >> pos.x()) & 1 == 1
    }

    /// Moves played so far, oldest first.
    pub fn moves(&self) -> impl ExactSizeIterator<Item = Pos> + '_ {
        self.history.iter().map(|u| u.pos)
    }

    /// Cells holding a stone of neither player, row by row.
    pub fn walls(&self) -> impl Iterator<Item = Pos> + '_ {
        let n = self.size;
        (0..n * n)
            .map(move |i| Pos::new(i % n, i / n))
            .filter(|&p| self.is_wall(p))
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
        self.history.len() + self.walls_placed == self.size * self.size
    }

    /// Pattern a stone of `color` would form on `pos` along `dir`.
    #[inline]
    pub fn pattern(&self, pos: Pos, color: Color, dir: usize) -> Pattern {
        self.cells[slot(pos)].pattern[color.index()][dir]
    }

    /// What `color` would form on the empty `f` if its opponent also held
    /// the empty `q`, without playing it. `None` with the large tables
    /// (every rule but freestyle), where the move must be played to know.
    #[inline]
    pub fn p4_if_opponent_at(&self, f: Pos, color: Color, q: Pos) -> Option<Pattern4> {
        if self.large.is_some() {
            return None;
        }
        let dx = q.x() as i32 - f.x() as i32;
        let dy = q.y() as i32 - f.y() as i32;
        let d = dx.abs().max(dy.abs());
        let dir = if d == 0 || d > 4 {
            None
        } else if dy == 0 {
            Some(0)
        } else if dx == 0 {
            Some(1)
        } else if dx == dy {
            Some(2)
        } else if dx == -dy {
            Some(3)
        } else {
            None
        };
        let Some(dir) = dir else {
            return Some(self.p4(f, color));
        };
        let tables = Tables::get();
        let (line, at) = self.line_of(f, dir);
        let (_, at_q) = self.line_of(q, dir);
        let c = color.index();
        let own = self.lines[c][dir][line] >> (at + 1);
        let blocked =
            (self.lines[1 - c][dir][line] | 1u32 << (at_q + PAD) | self.edges[dir][line] | self.walls[dir][line])
                >> (at + 1);
        let mut patterns = self.cells[slot(f)].pattern[c];
        patterns[dir] = tables.line(own, blocked);
        Some(tables.pattern4(tables.pcode(patterns)))
    }

    /// Pattern code of `pos` for `color`.
    #[inline]
    pub fn pcode(&self, pos: Pos, color: Color) -> PatternCode {
        self.cells[slot(pos)].pcode[color.index()]
    }

    /// What a stone of `color` would form on `pos` (empty cells only).
    ///
    /// Under renju rules, [`Pattern4::Forbidden`] for Black may still be a
    /// legal double three; see [`Board::is_forbidden`].
    #[inline]
    pub fn p4(&self, pos: Pos, color: Color) -> Pattern4 {
        self.cells[slot(pos)].p4[color.index()]
    }

    /// The direction in which a stone of `color` on `pos` would complete a
    /// winning line, if any.
    pub fn five_direction(&self, pos: Pos, color: Color) -> Option<usize> {
        (0..4).find(|&dir| self.pattern(pos, color, dir) == Pattern::F5)
    }

    /// Whether Black may not play on `pos` under renju rules: an overline,
    /// two fours or two real open threes at once, unless the move makes
    /// five. Always false under other rules, for occupied cells, and where
    /// [`Board::p4`] is not [`Pattern4::Forbidden`].
    ///
    /// A double three is real only if both threes can become straight fours
    /// by moves that are not forbidden themselves, which is checked by
    /// trying the moves (the board is restored before returning).
    pub fn is_forbidden(&mut self, pos: Pos) -> bool {
        if !self.renju || !self.is_empty(pos) {
            return false;
        }
        let cell = self.cells[slot(pos)];
        if cell.p4[0] != Pattern4::Forbidden {
            return false;
        }
        if cell.flags[0] != 0 {
            return true;
        }
        let fours = cell.pattern[0]
            .iter()
            .filter(|&&p| p == Pattern::B4 || p == Pattern::F4)
            .count();
        if fours >= 2 {
            return true;
        }
        self.set_stone(pos, BLACK);
        let mut threes = 0;
        for dir in 0..4 {
            if cell.pattern[0][dir].is_flex3() && self.straight_four_follows(pos, dir) {
                threes += 1;
                if threes == 2 {
                    break;
                }
            }
        }
        self.set_stone(pos, EMPTY);
        threes >= 2
    }

    /// With a black stone on `pos`, whether Black can make a straight four
    /// through it along `dir` with a move that is not forbidden. Such a move
    /// is next to the black stones joined to `pos`, on either side.
    fn straight_four_follows(&mut self, pos: Pos, dir: usize) -> bool {
        for side in [-1isize, 1] {
            for k in 1..=4 {
                let Some(q) = self.step(pos, dir, side * k) else { break };
                match self.cells[slot(q)].stone {
                    BLACK => continue,
                    EMPTY => {
                        if self.pattern(q, Color::Black, dir) == Pattern::F4 && !self.is_forbidden(q) {
                            return true;
                        }
                        break;
                    }
                    _ => break,
                }
            }
        }
        false
    }

    /// Whether the side to move may play on `pos`: empty, and not forbidden.
    pub fn is_legal(&mut self, pos: Pos) -> bool {
        self.is_empty(pos) && !(self.side_to_move() == Color::Black && self.is_forbidden(pos))
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

    /// Sum of empty-cell values for `color` with the late-game table.
    #[inline]
    pub fn pattern_value_late(&self, color: Color) -> i32 {
        self.value_late[color.index()]
    }

    /// Rectangle containing every candidate move.
    #[inline]
    pub fn area(&self) -> Area {
        self.area
    }

    /// Every cell of the candidate area, row by row. Empty when no stone has
    /// been played.
    pub fn area_cells(&self) -> AreaCells {
        let a = self.area;
        AreaCells {
            x: a.x0,
            y: a.y0,
            x0: a.x0,
            x1: a.x1,
            y1: a.y1,
        }
    }

    /// Empty cells near stones, row by row.
    pub fn candidates(&self) -> Candidates<'_> {
        let a = self.area;
        if a.y0 > a.y1 || a.x0 > a.x1 {
            return Candidates {
                cand: &self.cand,
                y: 1,
                y1: 0,
                xmask: 0,
                row: 0,
            };
        }
        let xmask = ((1u64 << (a.x1 as u32 + 1)) - (1u64 << a.x0)) as u32;
        Candidates {
            cand: &self.cand,
            y: a.y0 as usize,
            y1: a.y1 as usize,
            xmask,
            row: self.cand[a.y0 as usize] & xmask,
        }
    }

    /// After a stone on `pos`: every empty cell of its [`NEAR`] stencil is
    /// now a candidate, and `pos` no longer is.
    #[inline]
    fn add_candidates(&mut self, pos: Pos) {
        // NEAR as columns by row offset -3..=3, bit dx + 3.
        const ROWS: [u64; 7] = {
            let mut rows = [0u64; 7];
            let mut i = 0;
            while i < NEAR.len() {
                let (dx, dy) = NEAR[i];
                rows[(dy + 3) as usize] |= 1 << (dx + 3);
                i += 1;
            }
            rows
        };
        let (px, py) = (pos.x(), pos.y() as isize);
        let full = (1u64 << self.size) - 1;
        for (i, &stencil) in ROWS.iter().enumerate() {
            let y = py + i as isize - 3;
            if !(0..self.size as isize).contains(&y) {
                continue;
            }
            let y = y as usize;
            let occupied = (self.lines[0][0][y] | self.lines[1][0][y] | self.walls[0][y]) as u64 >> PAD;
            let mask = ((stencil << px) >> 3) & full & !occupied;
            self.cand[y] |= mask as u32;
        }
        self.cand[pos.y()] &= !(1u32 << px);
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

    /// What undoing the next move (or pass) restores.
    fn undo_record(&self, pos: Pos) -> Undo {
        Undo {
            pos,
            area: self.area,
            journal_len: self.journal.len() as u32,
            value: self.value,
            value_late: self.value_late,
            p4_count: self.p4_count,
            windows: self.windows,
            cand_rows: if pos == Pos::NONE {
                [0; 7]
            } else {
                std::array::from_fn(|i| {
                    let y = pos.y() as isize + i as isize - 3;
                    if (0..self.size as isize).contains(&y) {
                        self.cand[y as usize]
                    } else {
                        0
                    }
                })
            },
        }
    }

    /// Place a stone for the side to move on an empty cell.
    pub fn play(&mut self, pos: Pos) {
        debug_assert!(self.is_empty(pos), "illegal move {pos:?}");
        let color = self.side_to_move();
        self.history.push(self.undo_record(pos));
        self.hash ^= zobrist().stone[color.index()][pos.index()] ^ zobrist().side;
        self.place::<true>(pos, color.index() as u8 + 1);
        self.extend_area(pos);
        self.add_candidates(pos);
    }

    /// Take back the last move: copy back the cells it changed and the
    /// totals from before it. The emptied cell's own patterns were left
    /// as they were while it was occupied, which is right again now.
    pub fn undo(&mut self) {
        let undo = self.history.pop().expect("no move to undo");
        let pos = undo.pos;
        let color = self.side_to_move();
        self.hash ^= zobrist().stone[color.index()][pos.index()] ^ zobrist().side;
        while self.journal.len() > undo.journal_len as usize {
            let (i, cell) = self.journal.pop().expect("journal entry");
            self.cells[i as usize] = cell;
        }
        for dir in 0..4 {
            let (line, at) = self.line_of(pos, dir);
            self.lines[color.index()][dir][line] ^= 1u32 << (at + PAD);
        }
        self.cells[slot(pos)].stone = EMPTY;
        self.value = undo.value;
        self.value_late = undo.value_late;
        self.p4_count = undo.p4_count;
        self.windows = undo.windows;
        self.area = undo.area;
        for (i, &row) in undo.cand_rows.iter().enumerate() {
            let y = pos.y() as isize + i as isize - 3;
            if (0..self.size as isize).contains(&y) {
                self.cand[y as usize] = row;
            }
        }
    }

    /// Put a stone of neither player on an empty cell. In a continuous game
    /// (Piskvork `INFO rule` bit 2) the stones of a completed five, and in
    /// renju a forbidden move, stay on the board but belong to nobody: they
    /// block lines for both sides. Not a move, and never taken back.
    pub fn place_wall(&mut self, pos: Pos) {
        debug_assert!(self.is_empty(pos), "wall on occupied {pos:?}");
        self.hash ^= zobrist().wall[pos.index()];
        self.set_stone(pos, WALL);
        self.walls_placed += 1;
        self.cand[pos.y()] &= !(1u32 << pos.x());
    }

    /// Pass the turn without playing, for null-move style probes. Undo with
    /// [`Board::undo_pass`].
    pub fn pass(&mut self) {
        self.history.push(self.undo_record(Pos::NONE));
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

    /// Line-table entries of an empty `pos` along `dir`, per color.
    #[inline]
    fn entries(&self, pos: Pos, dir: usize, tables: &Tables) -> [u8; 2] {
        let (line, at) = self.line_of(pos, dir);
        let black = self.lines[0][dir][line] >> at;
        let white = self.lines[1][dir][line] >> at;
        let edge = self.edges[dir][line] >> at;
        let block = edge | (self.walls[dir][line] >> at);
        match self.large {
            None => [
                tables.line(black >> 1, (white | block) >> 1) as u8,
                tables.line(white >> 1, (black | block) >> 1) as u8,
            ],
            Some([b, w]) => [
                b.entry(black | edge, white | block),
                w.entry(white | edge, black | block),
            ],
        }
    }

    /// Recompute the patterns and flags of an empty `pos` from the masks.
    #[inline]
    fn compute_patterns(&self, pos: Pos, tables: &Tables) -> ([[Pattern; 4]; 2], [u8; 2]) {
        let entries = [0, 1, 2, 3].map(|dir| self.entries(pos, dir, tables));
        let mut flags = [0u8; 2];
        let patterns = [0, 1].map(|c| {
            entries
                .map(|e| e[c])
                .iter()
                .enumerate()
                .fold([Pattern::Dead; 4], |mut out, (dir, &entry)| {
                    out[dir] = PATTERN_OF[(entry & 0x0f) as usize];
                    flags[c] |= ((entry & LINE_FORBIDDEN != 0) as u8) << dir;
                    out
                })
        });
        (patterns, flags)
    }

    /// Combination of color `c`'s pattern code and line flags at a cell.
    #[inline]
    fn combine(&self, c: usize, code: PatternCode, flags: u8, tables: &Tables) -> Pattern4 {
        let p4 = tables.pattern4(code);
        if !self.renju || c != 0 || p4 == Pattern4::AFive {
            p4
        } else if flags != 0 {
            Pattern4::Forbidden
        } else {
            tables.pattern4_renju(code)
        }
    }

    /// Remove an empty cell's contribution to the running totals.
    #[inline]
    fn detach(&mut self, pos: Pos, values: &eval::CellValues) {
        let cell = self.cells[slot(pos)];
        if cell.stone == 0 {
            for c in 0..2 {
                self.value[c] -= values.value(cell.pcode[c]);
                self.value_late[c] -= values.value_late(cell.pcode[c]);
                self.p4_count[c][cell.p4[c] as usize] -= 1;
            }
        }
    }

    /// Add an empty cell's contribution to the running totals.
    #[inline]
    fn attach(&mut self, pos: Pos, values: &eval::CellValues) {
        let cell = self.cells[slot(pos)];
        if cell.stone == 0 {
            for c in 0..2 {
                self.value[c] += values.value(cell.pcode[c]);
                self.value_late[c] += values.value_late(cell.pcode[c]);
                self.p4_count[c][cell.p4[c] as usize] += 1;
            }
        }
    }

    /// Recompute the cached patterns of `pos` (no totals).
    #[inline]
    fn refresh_cell(&mut self, pos: Pos, tables: &Tables) {
        let (patterns, flags) = self.compute_patterns(pos, tables);
        let pcode = patterns.map(|p| tables.pcode(p));
        let p4 = [0, 1].map(|c| self.combine(c, pcode[c], flags[c], tables));
        let cell = &mut self.cells[slot(pos)];
        cell.pattern = patterns;
        cell.flags = flags;
        cell.pcode = pcode;
        cell.p4 = p4;
    }

    /// Recompute the running totals from scratch.
    fn recount(&mut self) {
        let values = self.values;
        self.value = [0; 2];
        self.value_late = [0; 2];
        self.p4_count = [[0; Pattern4::COUNT]; 2];
        for y in 0..self.size {
            for x in 0..self.size {
                self.attach(Pos::new(x, y), values);
            }
        }
    }

    /// Put a stone (code 1 to 3) on an empty cell, or empty an occupied one
    /// (code 0), and update every cell whose patterns depend on it.
    fn set_stone(&mut self, pos: Pos, stone: u8) {
        self.place::<false>(pos, stone);
    }

    /// Add `sign` times the windows through `pos` to the open-window counts:
    /// called with -1 before a stone changes and +1 after.
    #[inline]
    fn count_windows(&mut self, pos: Pos, sign: i16) {
        for dir in 0..4 {
            let (line, at) = self.line_of(pos, dir);
            let edge = self.edges[dir][line];
            let black = self.lines[0][dir][line];
            let white = self.lines[1][dir][line];
            let wall = self.walls[dir][line];
            for start in at + PAD - 4..=at + PAD {
                let mask = 0x1f << start;
                if edge & mask != 0 {
                    continue;
                }
                let (b, w, x) = ((black & mask).count_ones(), (white & mask).count_ones(), wall & mask);
                if w == 0 && x == 0 {
                    self.windows[0][b as usize] += sign;
                }
                if b == 0 && x == 0 {
                    self.windows[1][w as usize] += sign;
                }
            }
        }
    }

    /// Five-cell windows still open to `color`, by how many of its stones
    /// they hold (0 to 5). Kept up to date only while the evaluation weighs
    /// windows (see [`eval::CellValues::has_window`]); otherwise use
    /// [`Board::count_open_windows`].
    #[inline]
    pub fn open_windows(&self, color: Color) -> &[i16; 6] {
        &self.windows[color.index()]
    }

    /// Open windows of both colors counted from scratch, by color index.
    pub fn count_open_windows(&self) -> [[i16; 6]; 2] {
        let mut out = [[0i16; 6]; 2];
        for dir in 0..4 {
            for line in 0..LINES {
                let edge = self.edges[dir][line];
                let black = self.lines[0][dir][line];
                let white = self.lines[1][dir][line];
                let wall = self.walls[dir][line];
                for start in 0..=32 - 5 {
                    let mask = 0x1f << start;
                    if edge & mask != 0 || wall & mask != 0 {
                        continue;
                    }
                    let (b, w) = ((black & mask).count_ones(), (white & mask).count_ones());
                    if w == 0 {
                        out[0][b as usize] += 1;
                    }
                    if b == 0 {
                        out[1][w as usize] += 1;
                    }
                }
            }
        }
        out
    }

    /// [`Board::set_stone`], saving every changed cell to the journal first
    /// when `JOURNAL`.
    #[inline]
    fn place<const JOURNAL: bool>(&mut self, pos: Pos, stone: u8) {
        let tables = Tables::get();
        let values = self.values;
        let old = self.cells[slot(pos)].stone;
        debug_assert!((old == EMPTY) != (stone == EMPTY), "{pos:?}: {old} -> {stone}");
        if old == EMPTY {
            self.detach(pos, values);
        }
        let windows = values.has_window();
        if windows {
            self.count_windows(pos, -1);
        }
        for dir in 0..4 {
            let (line, at) = self.line_of(pos, dir);
            let bit = 1u32 << (at + PAD);
            match old | stone {
                BLACK => self.lines[0][dir][line] ^= bit,
                WALL => self.walls[dir][line] ^= bit,
                _ => self.lines[1][dir][line] ^= bit,
            }
        }
        self.cells[slot(pos)].stone = stone;
        if windows {
            self.count_windows(pos, 1);
        }
        // An occupied cell's patterns are never read and go stale; bring them
        // up to date when it is emptied again.
        if stone == EMPTY {
            self.refresh_cell(pos, tables);
            self.attach(pos, values);
        }

        // Only the patterns along the line through `pos` change.
        match self.large {
            None => self.update_lines::<false, JOURNAL>(pos, tables, values),
            Some(_) => self.update_lines::<true, JOURNAL>(pos, tables, values),
        }
    }

    /// Update the empty cells within reach of `pos` along its four lines;
    /// `LARGE` for the 11-cell tables.
    #[inline]
    fn update_lines<const LARGE: bool, const JOURNAL: bool>(
        &mut self,
        pos: Pos,
        tables: &Tables,
        values: &eval::CellValues,
    ) {
        let reach = if LARGE { 5 } else { 4 };
        for (dir, delta) in DELTA.into_iter().enumerate() {
            for side in [-1isize, 1] {
                let mut q = slot(pos) as isize;
                for _ in 0..reach {
                    q += side * delta;
                    // Off-board cells, padding included, are never empty.
                    if self.cells[q as usize].stone != EMPTY {
                        continue;
                    }
                    let next = Pos((q as usize - slot(Pos(0))) as u16);
                    if LARGE {
                        self.update_direction::<JOURNAL>(next, dir, tables, values);
                    } else {
                        self.update_direction_small::<JOURNAL>(next, dir, tables, values);
                    }
                }
            }
        }
    }

    /// [`Board::update_direction`] for the freestyle table, which has no
    /// flags and no forbidden moves.
    #[inline]
    fn update_direction_small<const JOURNAL: bool>(
        &mut self,
        pos: Pos,
        dir: usize,
        tables: &Tables,
        values: &eval::CellValues,
    ) {
        let (line, at) = self.line_of(pos, dir);
        let black = self.lines[0][dir][line] >> (at + 1);
        let white = self.lines[1][dir][line] >> (at + 1);
        let block = (self.edges[dir][line] | self.walls[dir][line]) >> (at + 1);
        let new = [tables.line(black, white | block), tables.line(white, black | block)];
        let i = slot(pos);
        let old = self.cells[i];
        if old.pattern[0][dir] == new[0] && old.pattern[1][dir] == new[1] {
            return;
        }
        if JOURNAL {
            self.journal.push((i as u16, old));
        }
        for (c, &pattern) in new.iter().enumerate() {
            let cell = &mut self.cells[i];
            if cell.pattern[c][dir] == pattern {
                continue;
            }
            cell.pattern[c][dir] = pattern;
            let (old_code, old_p4) = (cell.pcode[c], cell.p4[c]);
            let code = tables.pcode(cell.pattern[c]);
            let p4 = tables.pattern4(code);
            cell.pcode[c] = code;
            cell.p4[c] = p4;
            self.value[c] += values.value(code) - values.value(old_code);
            self.value_late[c] += values.value_late(code) - values.value_late(old_code);
            self.p4_count[c][old_p4 as usize] -= 1;
            self.p4_count[c][p4 as usize] += 1;
        }
    }

    /// Recompute the patterns of an empty `pos` along `dir`, and for each
    /// color whose pattern changed, its code, combination and the running
    /// totals. Often a distant stone changes nothing, and then nothing else
    /// is looked up.
    #[inline]
    fn update_direction<const JOURNAL: bool>(
        &mut self,
        pos: Pos,
        dir: usize,
        tables: &Tables,
        values: &eval::CellValues,
    ) {
        let new = self.entries(pos, dir, tables);
        if JOURNAL {
            let old = &self.cells[slot(pos)];
            let same = |c: usize| {
                old.pattern[c][dir] == PATTERN_OF[(new[c] & 0x0f) as usize]
                    && old.flags[c] & (1 << dir) == ((new[c] & LINE_FORBIDDEN != 0) as u8) << dir
            };
            if same(0) && same(1) {
                return;
            }
            self.journal.push((slot(pos) as u16, *old));
        }
        for (c, &entry) in new.iter().enumerate() {
            let pattern = PATTERN_OF[(entry & 0x0f) as usize];
            let flag = ((entry & LINE_FORBIDDEN != 0) as u8) << dir;
            let cell = &mut self.cells[slot(pos)];
            if cell.pattern[c][dir] == pattern && cell.flags[c] & (1 << dir) == flag {
                continue;
            }
            cell.pattern[c][dir] = pattern;
            cell.flags[c] = (cell.flags[c] & !(1 << dir)) | flag;
            let (old_code, old_p4, flags) = (cell.pcode[c], cell.p4[c], cell.flags[c]);
            let code = tables.pcode(cell.pattern[c]);
            let p4 = self.combine(c, code, flags, tables);
            let cell = &mut self.cells[slot(pos)];
            cell.pcode[c] = code;
            cell.p4[c] = p4;
            self.value[c] += values.value(code) - values.value(old_code);
            self.value_late[c] += values.value_late(code) - values.value_late(old_code);
            self.p4_count[c][old_p4 as usize] -= 1;
            self.p4_count[c][p4 as usize] += 1;
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
                let mark = match self.cells[slot(p)].stone {
                    BLACK => 'X',
                    EMPTY => '.',
                    WALL => '#',
                    _ => 'O',
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
pub(crate) mod tests {

    /// Open windows counted from scratch.
    fn recount_windows(board: &Board) -> [[i16; 6]; 2] {
        let n = board.size() as isize;
        let mut out = [[0i16; 6]; 2];
        let stone = |x: isize, y: isize| board.stone(Pos::new(x as usize, y as usize));
        for (dx, dy) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
            for y in 0..n {
                for x in 0..n {
                    let (ex, ey) = (x + 4 * dx, y + 4 * dy);
                    if !(0..n).contains(&ex) || !(0..n).contains(&ey) {
                        continue;
                    }
                    let mut count = [0usize; 2];
                    for i in 0..5 {
                        if let Some(c) = stone(x + i * dx, y + i * dy) {
                            count[c.index()] += 1;
                        }
                    }
                    if count[1] == 0 {
                        out[0][count[0]] += 1;
                    }
                    if count[0] == 0 {
                        out[1][count[1]] += 1;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn open_windows_follow_moves_and_undos() {
        let mut board = Board::new(15);
        assert_eq!(board.count_open_windows(), recount_windows(&board));
        let moves = [
            (7, 7),
            (8, 8),
            (7, 8),
            (0, 0),
            (14, 14),
            (6, 6),
            (7, 9),
            (0, 14),
            (7, 10),
            (3, 11),
        ];
        for &(x, y) in &moves {
            board.play(Pos::new(x, y));
            assert_eq!(board.count_open_windows(), recount_windows(&board));
        }
        for _ in 0..4 {
            board.undo();
            assert_eq!(board.count_open_windows(), recount_windows(&board));
        }
    }
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

    #[test]
    fn p4_if_opponent_at_matches_playing_the_stone() {
        let mut rng = Rng::new(0x5eed_f00d);
        for _ in 0..40 {
            let mut board = Board::new(15);
            board.play(board.center());
            for _ in 0..(10 + rng.below(30)) {
                let empty: Vec<Pos> = board.candidates().collect();
                if empty.is_empty() || board.p4_count(board.side_to_move(), Pattern4::AFive) > 0 {
                    break;
                }
                board.play(empty[rng.below(empty.len())]);
                if board.p4_count(board.side_to_move().opponent(), Pattern4::AFive) > 0 {
                    break;
                }
            }
            let us = board.side_to_move();
            let them = us.opponent();
            let cells: Vec<Pos> = board.candidates().collect();
            for &q in &cells {
                board.play(q);
                for &f in &cells {
                    if f == q {
                        continue;
                    }
                    let after = board.p4(f, them);
                    board.undo();
                    assert_eq!(
                        board.p4_if_opponent_at(f, them, q),
                        Some(after),
                        "f {f:?} q {q:?}\n{board}"
                    );
                    board.play(q);
                }
                board.undo();
            }
        }
    }

    /// Rebuild a board from scratch by replaying its moves.
    fn replay(board: &Board) -> Board {
        let mut fresh = Board::with_rule(board.size(), board.rule());
        for pos in board.moves().collect::<Vec<_>>() {
            fresh.play(pos);
        }
        fresh
    }

    fn assert_consistent(board: &Board) {
        let mut fresh = Board::with_rule(board.size(), board.rule());
        // Place stones directly, then recompute everything from the masks.
        for (i, pos) in board.moves().enumerate() {
            let color = if i % 2 == 0 { Color::Black } else { Color::White };
            fresh.lines_set(pos, color);
        }
        fresh.walls = board.walls;
        for y in 0..board.size {
            for x in 0..board.size {
                let p = Pos::new(x, y);
                if board.is_empty(p) {
                    let t = Tables::get();
                    let (patterns, flags) = fresh.compute_patterns(p, t);
                    assert_eq!(patterns, board.cells[slot(p)].pattern, "{p:?}");
                    assert_eq!(flags, board.cells[slot(p)].flags, "{p:?}");
                }
            }
        }
        for y in 0..board.size {
            for x in 0..board.size {
                let p = Pos::new(x, y);
                let near = board.moves().any(|m| {
                    let (dx, dy) = (p.x() as i8 - m.x() as i8, p.y() as i8 - m.y() as i8);
                    NEAR.contains(&(dx, dy))
                });
                assert_eq!(board.is_candidate(p), board.is_empty(p) && near, "candidate {p:?}");
            }
        }
        let mut recount = board.clone();
        recount.recount();
        assert_eq!(recount.value, board.value);
        assert_eq!(recount.value_late, board.value_late);
        assert_eq!(recount.p4_count, board.p4_count);
        if board.walls_placed == 0 {
            assert_eq!(replay(board).hash, board.hash);
        }
    }

    impl Board {
        fn lines_set(&mut self, pos: Pos, color: Color) {
            for dir in 0..4 {
                let (line, at) = self.line_of(pos, dir);
                self.lines[color.index()][dir][line] |= 1 << (at + PAD);
            }
            self.cells[slot(pos)].stone = color.index() as u8 + 1;
        }
    }

    #[test]
    fn random_play_and_undo_stay_consistent() {
        let mut rng = Rng::new(0x1234_5678);
        for (size, rule) in [
            (15, Rule::Freestyle),
            (20, Rule::Freestyle),
            (7, Rule::Freestyle),
            (15, Rule::Standard),
            (15, Rule::Renju),
            (20, Rule::Renju),
            (15, Rule::Caro),
            (9, Rule::FreestyleCaro),
        ] {
            let mut board = Board::with_rule(size, rule);
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

    /// A board where Black and White have played these stones, in order,
    /// with filler moves along the bottom and right edges when one side has
    /// more.
    pub(crate) fn position(rule: Rule, black: &[(usize, usize)], white: &[(usize, usize)]) -> Board {
        let mut board = Board::with_rule(15, rule);
        // Far from the rows the tests use, and never four in a line.
        let mut fill = [14, 12]
            .into_iter()
            .flat_map(|y| (0..5).map(move |i| (3 * i, y)))
            .chain((0..4).map(|i| (14, 3 * i)));
        let n = black.len().max(white.len() + 1);
        for i in 0..n {
            let b = black.get(i).copied().unwrap_or_else(|| fill.next().unwrap());
            board.play(Pos::new(b.0, b.1));
            if i + 1 < n || white.len() >= n {
                let w = white.get(i).copied().unwrap_or_else(|| fill.next().unwrap());
                board.play(Pos::new(w.0, w.1));
            }
        }
        board
    }

    #[test]
    fn renju_overlines_and_double_fours_are_forbidden() {
        let overline = position(Rule::Renju, &[(3, 7), (4, 7), (5, 7), (7, 7), (8, 7)], &[]);
        let mut board = overline.clone();
        assert_eq!(board.p4(Pos::new(6, 7), Color::Black), Pattern4::Forbidden);
        assert!(board.is_forbidden(Pos::new(6, 7)));
        // White's overline wins.
        let white = position(Rule::Renju, &[(0, 7)], &[(3, 7), (4, 7), (5, 7), (7, 7), (8, 7)]);
        assert_eq!(white.p4(Pos::new(6, 7), Color::White), Pattern4::AFive);
        // Standard: the overline is no win, but not forbidden either.
        let mut standard = position(Rule::Standard, &[(3, 7), (4, 7), (5, 7), (7, 7), (8, 7)], &[]);
        assert!(standard.p4(Pos::new(6, 7), Color::Black) < Pattern4::EBlock4);
        assert!(!standard.is_forbidden(Pos::new(6, 7)));

        let mut double_four = position(Rule::Renju, &[(5, 7), (6, 7), (7, 7), (8, 4), (8, 5), (8, 6)], &[]);
        assert!(double_four.is_forbidden(Pos::new(8, 7)));
        // Two fours on one line.
        let mut in_line = position(Rule::Renju, &[(4, 7), (5, 7), (6, 7), (10, 7), (11, 7), (12, 7)], &[]);
        assert!(in_line.is_forbidden(Pos::new(8, 7)), "{in_line}");
        assert!(!in_line.is_forbidden(Pos::new(7, 7)));
    }

    #[test]
    fn renju_double_threes_must_be_real() {
        // Two open threes through (8, 7).
        let mut board = position(Rule::Renju, &[(6, 7), (7, 7), (8, 5), (8, 6)], &[]);
        assert_eq!(board.p4(Pos::new(8, 7), Color::Black), Pattern4::Forbidden);
        assert!(board.is_forbidden(Pos::new(8, 7)));
        // Only Black is bound by it.
        assert_eq!(board.side_to_move(), Color::White);
        assert!(board.is_legal(Pos::new(8, 7)));
        board.pass();
        assert!(!board.is_legal(Pos::new(8, 7)));
        board.undo_pass();
        // A four-three is allowed.
        let mut four_three = position(Rule::Renju, &[(5, 7), (6, 7), (7, 7), (8, 5), (8, 6)], &[(4, 7)]);
        assert_eq!(four_three.p4(Pos::new(8, 7), Color::Black), Pattern4::CBlock4Flex3);
        assert!(!four_three.is_forbidden(Pos::new(8, 7)));

        // The horizontal three through (7, 7) could only become a straight
        // four at (4, 7) or (8, 7), and both are overlines: it is no three.
        let black = [
            (5, 7),
            (6, 7),
            (7, 5),
            (7, 6),
            (4, 4),
            (4, 5),
            (4, 6),
            (4, 8),
            (4, 9),
            (8, 4),
            (8, 5),
            (8, 6),
            (8, 8),
            (8, 9),
        ];
        let mut fake = position(Rule::Renju, &black, &[]);
        assert!(fake.is_forbidden(Pos::new(4, 7)) && fake.is_forbidden(Pos::new(8, 7)));
        assert_eq!(fake.p4(Pos::new(7, 7), Color::Black), Pattern4::Forbidden);
        let before = fake.hash();
        assert!(!fake.is_forbidden(Pos::new(7, 7)), "{fake}");
        assert_eq!(fake.hash(), before);
        assert_consistent(&fake);
    }

    #[test]
    fn five_beats_a_forbidden_shape() {
        // (7, 7) makes five across and an overline down.
        let black = [(3, 7), (4, 7), (5, 7), (6, 7), (7, 4), (7, 5), (7, 6), (7, 8), (7, 9)];
        let mut board = position(Rule::Renju, &black, &[]);
        assert_eq!(board.p4(Pos::new(7, 7), Color::Black), Pattern4::AFive);
        assert!(!board.is_forbidden(Pos::new(7, 7)));
        assert_eq!(board.five_direction(Pos::new(7, 7), Color::Black), Some(0));
    }

    #[test]
    fn caro_fives_blocked_at_both_ends_do_not_win() {
        let mut board = position(Rule::Caro, &[(3, 7), (4, 7), (5, 7), (6, 7)], &[(2, 7), (8, 7)]);
        assert!(board.p4(Pos::new(7, 7), Color::Black) < Pattern4::AFive);
        // Against the edge the five wins.
        board = position(Rule::Caro, &[(0, 7), (1, 7), (2, 7), (3, 7)], &[(5, 7)]);
        assert_eq!(board.p4(Pos::new(4, 7), Color::Black), Pattern4::AFive);
        assert_eq!(board.five_point(Color::Black), Some(Pos::new(4, 7)));
        let free = position(Rule::Freestyle, &[(3, 7), (4, 7), (5, 7), (6, 7)], &[(2, 7), (8, 7)]);
        assert_eq!(free.p4(Pos::new(7, 7), Color::Black), Pattern4::AFive);
    }

    #[test]
    fn walls_block_both_colors() {
        for rule in Rule::ALL {
            let mut board = position(rule, &[(3, 7), (4, 7), (5, 7)], &[(3, 9), (4, 9), (5, 9)]);
            assert_eq!(board.p4(Pos::new(6, 7), Color::Black), Pattern4::BFlex4, "{rule}");
            board.place_wall(Pos::new(2, 7));
            board.place_wall(Pos::new(6, 9));
            assert!(board.is_wall(Pos::new(2, 7)) && !board.is_empty(Pos::new(2, 7)));
            assert_eq!(board.stone(Pos::new(2, 7)), None);
            assert_eq!(board.p4(Pos::new(6, 7), Color::Black), Pattern4::EBlock4, "{rule}");
            assert!(board.p4(Pos::new(7, 9), Color::White) < Pattern4::EBlock4, "{rule}");
            assert_consistent(&board);
            assert!(board.to_string().contains('#'));
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
