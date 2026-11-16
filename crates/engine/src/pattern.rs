//! Line patterns, their combinations, and the lookup tables behind them.
//!
//! For every empty cell, direction and color, the engine classifies what a
//! stone of that color would form on that line if it were placed there. In
//! freestyle only the 4 cells on each side matter, so the classification is
//! precomputed for every possible 9-cell window (9 bits of own stones and 9
//! bits of blocked cells: opponent stones or off-board), 2^18 entries.
//!
//! The other rules look one cell further, to tell a five from an overline
//! and, in caro, a stone from the board edge at the end of a five. Their
//! tables ([`LineTable`]) cover 11-cell windows in which each of the 10
//! cells around the center is empty, own, blocked by a stone, or off the
//! board: 2^20 entries per rule and color.
//!
//! The pattern vocabulary (F/B levels, the "S" and "A/B" refinements and the
//! A..L combination classes) follows the Rapfi engine's terminology; the
//! classification code is an independent implementation.

use std::sync::OnceLock;

use crate::rule::Rule;

/// What a stone placed on a cell would form along one line.
///
/// Ordered from weakest to strongest, so patterns compare directly. Flex
/// (`F`) patterns can still grow into an open shape; block (`B`) patterns can
/// only grow into a blocked one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Pattern {
    /// No five can ever be completed through this cell on this line.
    Dead = 0,
    /// One move from a blocked two.
    B1,
    /// One move from an open two.
    F1,
    /// One move from a blocked three.
    B2,
    /// One or two different moves make an open three.
    F2,
    /// Three different moves make an open three.
    F2A,
    /// Four or more different moves make an open three.
    F2B,
    /// One move from a four (blocked three).
    B3,
    /// One move makes an open four (open three).
    F3,
    /// Two or more different moves make an open four.
    F3S,
    /// One move makes five; the opponent can block it (four).
    B4,
    /// Two different moves make five; cannot be blocked (open four).
    F4,
    /// Five or more in a row.
    F5,
}

impl Pattern {
    /// Number of patterns.
    pub const COUNT: usize = 13;

    /// All patterns, weakest first.
    pub const ALL: [Pattern; Pattern::COUNT] = [
        Pattern::Dead,
        Pattern::B1,
        Pattern::F1,
        Pattern::B2,
        Pattern::F2,
        Pattern::F2A,
        Pattern::F2B,
        Pattern::B3,
        Pattern::F3,
        Pattern::F3S,
        Pattern::B4,
        Pattern::F4,
        Pattern::F5,
    ];

    /// Open three (one or more ways to an open four).
    pub fn is_flex3(self) -> bool {
        matches!(self, Pattern::F3 | Pattern::F3S)
    }

    /// Open two of any strength.
    pub fn is_flex2(self) -> bool {
        matches!(self, Pattern::F2 | Pattern::F2A | Pattern::F2B)
    }
}

/// Combination of a cell's four line patterns for one color, weakest first.
///
/// The letters follow the usual threat ranking: `A` wins immediately, `B`
/// wins next move, `C` (four-three) usually wins, and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Pattern4 {
    /// Nothing notable.
    None = 0,
    /// Renju only: Black may not play here (overline, two fours or two open
    /// threes). Double threes are only candidates until
    /// [`crate::Board::is_forbidden`] confirms them.
    Forbidden,
    /// An open two.
    LFlex2,
    /// A blocked three.
    KBlock3,
    /// Two open twos.
    JFlex2x2,
    /// Two blocked threes, or a blocked three and an open two.
    IBlock3Plus,
    /// An open three.
    HFlex3,
    /// An open three plus an open two or blocked three.
    GFlex3Plus,
    /// Two open threes (double three).
    FFlex3x2,
    /// A four.
    EBlock4,
    /// A four plus an open two or blocked three.
    DBlock4Plus,
    /// A four and an open three (four-three).
    CBlock4Flex3,
    /// An open four or two fours: wins next move.
    BFlex4,
    /// Five: wins now.
    AFive,
}

impl Pattern4 {
    /// Number of combinations.
    pub const COUNT: usize = 14;

    /// Combine the patterns of the four directions for Black under renju
    /// rules, where two fours or two open threes are forbidden (overlines
    /// are flagged separately; see [`LineTable`]).
    pub fn from_patterns_renju(patterns: [Pattern; 4]) -> Pattern4 {
        let count = |f: fn(Pattern) -> bool| patterns.iter().filter(|&&p| f(p)).count();
        if count(|p| p == Pattern::F5) > 0 {
            Pattern4::AFive
        } else if count(|p| p == Pattern::B4 || p == Pattern::F4) >= 2 || count(Pattern::is_flex3) >= 2 {
            Pattern4::Forbidden
        } else {
            Pattern4::from_patterns(patterns)
        }
    }

    /// Combine the patterns of the four directions.
    pub fn from_patterns(patterns: [Pattern; 4]) -> Pattern4 {
        let count = |f: fn(Pattern) -> bool| patterns.iter().filter(|&&p| f(p)).count();
        let fives = count(|p| p == Pattern::F5);
        let flex4 = count(|p| p == Pattern::F4);
        let block4 = count(|p| p == Pattern::B4);
        let flex3 = count(Pattern::is_flex3);
        let block3 = count(|p| p == Pattern::B3);
        let flex2 = count(Pattern::is_flex2);

        if fives > 0 {
            Pattern4::AFive
        } else if flex4 > 0 || block4 >= 2 {
            Pattern4::BFlex4
        } else if block4 > 0 {
            if flex3 > 0 {
                Pattern4::CBlock4Flex3
            } else if block3 > 0 || flex2 > 0 {
                Pattern4::DBlock4Plus
            } else {
                Pattern4::EBlock4
            }
        } else if flex3 >= 2 {
            Pattern4::FFlex3x2
        } else if flex3 > 0 {
            if block3 > 0 || flex2 > 0 {
                Pattern4::GFlex3Plus
            } else {
                Pattern4::HFlex3
            }
        } else if block3 >= 2 || (block3 > 0 && flex2 > 0) {
            Pattern4::IBlock3Plus
        } else if flex2 >= 2 {
            Pattern4::JFlex2x2
        } else if block3 > 0 {
            Pattern4::KBlock3
        } else if flex2 > 0 {
            Pattern4::LFlex2
        } else {
            Pattern4::None
        }
    }

    /// Whether playing here makes at least a four (a forcing move).
    pub fn is_four(self) -> bool {
        self >= Pattern4::EBlock4
    }
}

/// Order-independent code of the four line patterns at a cell.
pub type PatternCode = u16;

/// Number of pattern codes: multisets of 4 patterns out of 13.
pub const PCODE_COUNT: usize = 1820;

/// Cells on each side of the center that a window covers.
pub const HALF_WINDOW: u32 = 4;
/// Bits in a window, center included.
pub const WINDOW_BITS: u32 = 2 * HALF_WINDOW + 1;
const WINDOW_MASK: u32 = (1 << WINDOW_BITS) - 1;
const CENTER: u32 = HALF_WINDOW;

const LINE_ENTRIES: usize = 1 << (2 * WINDOW_BITS);

#[inline]
fn table_index(own: u32, blocked: u32) -> usize {
    ((own & WINDOW_MASK) | ((blocked & WINDOW_MASK) << WINDOW_BITS)) as usize
}

/// All precomputed pattern tables.
pub struct Tables {
    /// Window -> pattern of a stone on the empty center.
    line: Box<[Pattern; LINE_ENTRIES]>,
    /// Four patterns -> pattern code. Indexing by the enums lets the compiler
    /// drop bounds checks.
    pcode: Box<[[[[PatternCode; Pattern::COUNT]; Pattern::COUNT]; Pattern::COUNT]; Pattern::COUNT]>,
    /// Pattern code -> combination.
    pattern4: Vec<Pattern4>,
    /// Pattern code -> combination for Black under renju rules.
    pattern4_renju: Vec<Pattern4>,
    /// Pattern code -> the four patterns, weakest first.
    patterns: Vec<[Pattern; 4]>,
}

impl Tables {
    /// The shared tables, built on first use (a few milliseconds).
    pub fn get() -> &'static Tables {
        static TABLES: OnceLock<Tables> = OnceLock::new();
        TABLES.get_or_init(Tables::build)
    }

    /// Pattern a stone on the (empty) center of the window would form.
    ///
    /// `own` and `blocked` hold the 9 window cells, bit 4 being the center.
    #[inline]
    pub fn line(&self, own: u32, blocked: u32) -> Pattern {
        self.line[table_index(own, blocked)]
    }

    /// Pattern code of four line patterns, in any order.
    #[inline]
    pub fn pcode(&self, p: [Pattern; 4]) -> PatternCode {
        self.pcode[p[0] as usize][p[1] as usize][p[2] as usize][p[3] as usize]
    }

    /// Combination class of a pattern code.
    #[inline]
    pub fn pattern4(&self, pcode: PatternCode) -> Pattern4 {
        self.pattern4[pcode as usize]
    }

    /// Combination class of a pattern code for Black under renju rules.
    #[inline]
    pub fn pattern4_renju(&self, pcode: PatternCode) -> Pattern4 {
        self.pattern4_renju[pcode as usize]
    }

    /// The four patterns of a pattern code, weakest first.
    pub fn patterns(&self, pcode: PatternCode) -> [Pattern; 4] {
        self.patterns[pcode as usize]
    }

    fn build() -> Tables {
        let line = build_line_table();

        // Number the multisets by enumerating sorted 4-tuples, then map every
        // ordered tuple to the number of its sorted form.
        let mut patterns = Vec::with_capacity(PCODE_COUNT);
        let mut index_of = std::collections::HashMap::new();
        for a in 0..Pattern::COUNT {
            for b in a..Pattern::COUNT {
                for c in b..Pattern::COUNT {
                    for d in c..Pattern::COUNT {
                        index_of.insert([a, b, c, d], patterns.len() as PatternCode);
                        patterns.push([a, b, c, d].map(|i| Pattern::ALL[i]));
                    }
                }
            }
        }
        assert_eq!(patterns.len(), PCODE_COUNT);
        let mut pcode = Box::new([[[[0; Pattern::COUNT]; Pattern::COUNT]; Pattern::COUNT]; Pattern::COUNT]);
        for (a, slab) in pcode.iter_mut().enumerate() {
            for (b, plane) in slab.iter_mut().enumerate() {
                for (c, row) in plane.iter_mut().enumerate() {
                    for (d, code) in row.iter_mut().enumerate() {
                        let mut key = [a, b, c, d];
                        key.sort_unstable();
                        *code = index_of[&key];
                    }
                }
            }
        }
        let pattern4 = patterns.iter().map(|&p| Pattern4::from_patterns(p)).collect();
        let pattern4_renju = patterns.iter().map(|&p| Pattern4::from_patterns_renju(p)).collect();
        Tables {
            line,
            pcode,
            pattern4,
            pattern4_renju,
            patterns,
        }
    }
}

/// Whether `own` holds five consecutive stones that include the center.
fn has_five_through_center(own: u32) -> bool {
    (CENTER - 4..=CENTER).any(|start| {
        let run = 0b11111 << start;
        own & run == run
    })
}

/// Classify a window whose center holds an own stone. `placed` gives the
/// pattern of a window with one more own stone (the center still included).
fn classify_placed(own: u32, blocked: u32, placed: impl Fn(u32) -> Pattern) -> Pattern {
    if has_five_through_center(own) {
        return Pattern::F5;
    }
    let (mut wins, mut flex4, mut block4, mut flex3, mut block3, mut flex2, mut block2) = (0, 0, 0, 0, 0, 0, 0);
    let mut cells = WINDOW_MASK & !own & !blocked;
    while cells != 0 {
        let bit = cells.isolate_lowest_one();
        cells &= cells - 1;
        let next = own | bit;
        if has_five_through_center(next) {
            wins += 1;
            continue;
        }
        match placed(next) {
            Pattern::F4 => flex4 += 1,
            Pattern::B4 => block4 += 1,
            Pattern::F3 | Pattern::F3S => flex3 += 1,
            Pattern::B3 => block3 += 1,
            Pattern::F2 | Pattern::F2A | Pattern::F2B => flex2 += 1,
            Pattern::B2 => block2 += 1,
            _ => {}
        }
    }
    match () {
        _ if wins >= 2 => Pattern::F4,
        _ if wins == 1 => Pattern::B4,
        _ if flex4 >= 2 => Pattern::F3S,
        _ if flex4 == 1 => Pattern::F3,
        _ if block4 > 0 => Pattern::B3,
        _ if flex3 >= 4 => Pattern::F2B,
        _ if flex3 == 3 => Pattern::F2A,
        _ if flex3 > 0 => Pattern::F2,
        _ if block3 > 0 => Pattern::B2,
        _ if flex2 > 0 => Pattern::F1,
        _ if block2 > 0 => Pattern::B1,
        _ => Pattern::Dead,
    }
}

fn build_line_table() -> Box<[Pattern; LINE_ENTRIES]> {
    let size = 1usize << (2 * WINDOW_BITS);
    let center = 1u32 << CENTER;

    // Windows with an own stone on the center, most stones first, so every
    // window can read the windows with one more stone.
    let mut states = Vec::new();
    for blocked in 0..(1u32 << WINDOW_BITS) {
        if blocked & center != 0 {
            continue;
        }
        let free = WINDOW_MASK & !blocked & !center;
        let mut sub = free;
        loop {
            states.push((sub | center, blocked));
            if sub == 0 {
                break;
            }
            sub = (sub - 1) & free;
        }
    }
    states.sort_by_key(|&(own, _)| std::cmp::Reverse(own.count_ones()));

    let mut placed = vec![Pattern::Dead; size];
    for &(own, blocked) in &states {
        let pattern = classify_placed(own, blocked, |next| placed[table_index(next, blocked)]);
        placed[table_index(own, blocked)] = pattern;
    }

    // Built on the heap: the table is too large for small (wasm) stacks.
    let mut table: Box<[Pattern; LINE_ENTRIES]> = vec![Pattern::Dead; LINE_ENTRIES]
        .into_boxed_slice()
        .try_into()
        .expect("table size");
    for own in 0..(1u32 << WINDOW_BITS) {
        for blocked in 0..(1u32 << WINDOW_BITS) {
            if own & blocked == 0 && (own | blocked) & center == 0 {
                table[table_index(own, blocked)] = placed[table_index(own | center, blocked)];
            }
        }
    }
    table
}

/// Bit of a [`LineTable`] entry set when this line alone forbids Black's
/// move on the center under renju rules: an overline, or two fours on the
/// same line.
pub const LINE_FORBIDDEN: u8 = 1 << 4;

/// Cells on each side of the center in a [`LineTable`] window.
pub const LARGE_HALF: u32 = 5;
const LARGE_CELLS: usize = 2 * LARGE_HALF as usize + 1;
const LARGE_CENTER: usize = LARGE_HALF as usize;
const LARGE_ENTRIES: usize = 1 << 20;
const UNKNOWN: u8 = 0xff;

/// The pattern in a [`LineTable`] entry.
#[inline]
pub fn entry_pattern(entry: u8) -> Pattern {
    Pattern::ALL[(entry & 0x0f) as usize]
}

/// What wins on a line, for one rule and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Semantics {
    /// Five or more.
    FiveOrMore,
    /// Exactly five.
    Exact,
    /// Exactly five; an overline is forbidden.
    RenjuBlack,
    /// Six or more, or five not blocked by stones at both ends.
    FreeCaro,
    /// Exactly five, not blocked by stones at both ends.
    Caro,
}

/// A cell of a line window as the classifier sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Empty,
    Own,
    /// An opponent stone, or a stone that belongs to nobody.
    Stone,
    /// Off the board.
    Edge,
}

/// Window -> pattern table for the rules that need 11-cell windows.
///
/// Each entry holds a [`Pattern`] in its low four bits and possibly
/// [`LINE_FORBIDDEN`].
pub struct LineTable {
    entries: Box<[u8; LARGE_ENTRIES]>,
}

impl LineTable {
    /// The table for `rule` and Black (`black`) or White, built on first
    /// use. `None` for freestyle, which uses the 9-cell table of [`Tables`].
    pub fn get(rule: Rule, black: bool) -> Option<&'static LineTable> {
        static TABLES: [OnceLock<LineTable>; 5] = [const { OnceLock::new() }; 5];
        let (slot, semantics) = match (rule, black) {
            (Rule::Freestyle, _) => return None,
            (Rule::Standard, _) => (0, Semantics::Exact),
            (Rule::Renju, true) => (1, Semantics::RenjuBlack),
            (Rule::Renju, false) => (2, Semantics::FiveOrMore),
            (Rule::FreestyleCaro, _) => (3, Semantics::FreeCaro),
            (Rule::Caro, _) => (4, Semantics::Caro),
        };
        Some(TABLES[slot].get_or_init(|| LineTable::build(semantics)))
    }

    /// Entry for an empty center, from 11-bit windows centered on bit 5:
    /// `own` holds own stones and off-board cells, `blocked` every other
    /// stone and off-board cells.
    #[inline]
    pub fn entry(&self, own: u32, blocked: u32) -> u8 {
        self.entries[large_index(own, blocked)]
    }

    fn build(semantics: Semantics) -> LineTable {
        let mut memo: Box<[u8; LARGE_ENTRIES]> = vec![UNKNOWN; LARGE_ENTRIES]
            .into_boxed_slice()
            .try_into()
            .expect("table size");
        // Every window the board can produce: off-board cells only at the
        // ends, the rest empty, own or blocked.
        for left in 0..=LARGE_CENTER {
            for right in 0..=LARGE_CENTER {
                let mut line = [Cell::Edge; LARGE_CELLS];
                line[LARGE_CENTER] = Cell::Own;
                let free: Vec<usize> = (left..LARGE_CELLS - right).filter(|&i| i != LARGE_CENTER).collect();
                for code in 0..3usize.pow(free.len() as u32) {
                    let mut c = code;
                    for &i in &free {
                        line[i] = [Cell::Empty, Cell::Own, Cell::Stone][c % 3];
                        c /= 3;
                    }
                    classify(semantics, &line, &mut memo[..]);
                }
            }
        }
        for entry in memo.iter_mut().filter(|e| **e == UNKNOWN) {
            *entry = Pattern::Dead as u8;
        }
        LineTable { entries: memo }
    }
}

/// Index of a [`LineTable`] entry: both 11-bit windows without the center.
#[inline]
fn large_index(own: u32, blocked: u32) -> usize {
    let squeeze = |m: u32| (m & 0x1f) | ((m >> 1) & 0x3e0);
    (squeeze(own) | (squeeze(blocked) << 10)) as usize
}

fn line_key(line: &[Cell; LARGE_CELLS]) -> usize {
    let (mut own, mut blocked) = (0, 0);
    for (i, &c) in line.iter().enumerate() {
        match c {
            Cell::Own => own |= 1 << i,
            Cell::Stone => blocked |= 1 << i,
            Cell::Edge => {
                own |= 1 << i;
                blocked |= 1 << i;
            }
            Cell::Empty => {}
        }
    }
    large_index(own, blocked)
}

/// Classify a window whose center holds an own stone, memoized by window.
fn classify(semantics: Semantics, line: &[Cell; LARGE_CELLS], memo: &mut [u8]) -> u8 {
    let key = line_key(line);
    if memo[key] == UNKNOWN {
        memo[key] = classify_uncached(semantics, line, memo);
    }
    memo[key]
}

fn classify_uncached(semantics: Semantics, line: &[Cell; LARGE_CELLS], memo: &mut [u8]) -> u8 {
    const C: usize = LARGE_CENTER;
    const LAST: usize = LARGE_CELLS - 1;
    let dead = Pattern::Dead as u8;

    // The run of own stones through the center decides wins and overlines.
    let (mut lo, mut hi) = (C, C);
    while lo > 0 && line[lo - 1] == Cell::Own {
        lo -= 1;
    }
    while hi < LAST && line[hi + 1] == Cell::Own {
        hi += 1;
    }
    let len = hi - lo + 1;
    if len >= 5 {
        // A run of exactly five never reaches the window's ends.
        let before = if lo > 0 { line[lo - 1] } else { Cell::Empty };
        let after = if hi < LAST { line[hi + 1] } else { Cell::Empty };
        let blocked = before == Cell::Stone && after == Cell::Stone;
        let five = Pattern::F5 as u8;
        return match semantics {
            Semantics::FiveOrMore => five,
            Semantics::Exact if len == 5 => five,
            Semantics::RenjuBlack if len == 5 => five,
            Semantics::RenjuBlack => dead | LINE_FORBIDDEN,
            Semantics::FreeCaro if len >= 6 || !blocked => five,
            Semantics::Caro if len == 5 && !blocked => five,
            _ => dead,
        };
    }

    // Otherwise see what one more stone in reach would make.
    let (mut start, mut end) = (C, C);
    while start > 0 && matches!(line[start - 1], Cell::Empty | Cell::Own) {
        start -= 1;
    }
    while end < LAST && matches!(line[end + 1], Cell::Empty | Cell::Own) {
        end += 1;
    }
    if end - start + 1 < 5 {
        return dead;
    }
    let mut fives = Vec::with_capacity(2);
    let (mut flex4, mut block4, mut flex3, mut block3, mut flex2, mut block2) = (0, 0, 0, 0, 0, 0);
    for i in start..=end {
        if line[i] != Cell::Empty {
            continue;
        }
        let mut next = *line;
        next[i] = Cell::Own;
        let entry = classify(semantics, &next, memo);
        if entry & LINE_FORBIDDEN != 0 {
            continue;
        }
        match entry_pattern(entry) {
            Pattern::F5 => fives.push(i),
            Pattern::F4 => flex4 += 1,
            Pattern::B4 => block4 += 1,
            Pattern::F3 | Pattern::F3S => flex3 += 1,
            Pattern::B3 => block3 += 1,
            Pattern::F2 | Pattern::F2A | Pattern::F2B => flex2 += 1,
            Pattern::B2 => block2 += 1,
            _ => {}
        }
    }
    let pattern = match () {
        _ if fives.len() >= 2 => {
            // Two five points five cells apart are the ends of one straight
            // four; closer ones are two fours on the line, which renju
            // forbids Black.
            let straight = fives.len() == 2 && fives[1] - fives[0] == 5;
            if semantics == Semantics::RenjuBlack && !straight {
                return dead | LINE_FORBIDDEN;
            }
            Pattern::F4
        }
        _ if fives.len() == 1 => Pattern::B4,
        _ if flex4 >= 2 => Pattern::F3S,
        _ if flex4 == 1 => Pattern::F3,
        _ if block4 > 0 => Pattern::B3,
        _ if flex3 >= 4 => Pattern::F2B,
        _ if flex3 == 3 => Pattern::F2A,
        _ if flex3 > 0 => Pattern::F2,
        _ if block3 > 0 => Pattern::B2,
        _ if flex2 > 0 => Pattern::F1,
        _ if block2 > 0 => Pattern::B1,
        _ => Pattern::Dead,
    };
    pattern as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Parse a 9-character window: `x` own, `o` blocked, `.` empty, `_` the
    /// empty center.
    fn window(s: &str) -> (u32, u32) {
        assert_eq!(s.len(), 9);
        assert_eq!(s.as_bytes()[4], b'_');
        let (mut own, mut blocked) = (0, 0);
        for (i, c) in s.chars().enumerate() {
            match c {
                'x' => own |= 1 << i,
                'o' => blocked |= 1 << i,
                '.' | '_' => {}
                _ => panic!("bad window {s}"),
            }
        }
        (own, blocked)
    }

    fn pattern(s: &str) -> Pattern {
        let (own, blocked) = window(s);
        Tables::get().line(own, blocked)
    }

    #[test]
    fn classifies_common_shapes() {
        use Pattern::*;
        let cases = [
            ("..xx_xx..", F5),
            ("xxxx_....", F5),
            ("x.xx_xx.x", F5), // overlines win in freestyle
            ("..xx_x...", F4),
            ("xx.x_.xxx", F4), // two different completions in one line
            (".oxx_x...", B4),
            ("..x._xx..", B4), // split four
            ("...x_x...", F3S),
            ("..x._x...", F3), // split three: only the middle makes an open four
            ("..ox_x...", B3),
            ("o.xx_o...", Dead), // only four cells available
            ("...._x...", F2B),
            ("...._....", F1),
            ("oooo_....", B1),
            ("..oo_o...", Dead),
        ];
        for (shape, expected) in cases {
            assert_eq!(pattern(shape), expected, "{shape}");
        }
    }

    /// Reference: the same rule evaluated by memoized recursion instead of the
    /// table's bottom-up order.
    fn recursive(own: u32, blocked: u32, memo: &mut HashMap<u32, Pattern>) -> Pattern {
        if let Some(&p) = memo.get(&own) {
            return p;
        }
        let mut children = HashMap::new();
        let mut cells = WINDOW_MASK & !own & !blocked;
        while cells != 0 {
            let bit = cells.isolate_lowest_one();
            cells &= cells - 1;
            if !has_five_through_center(own | bit) {
                children.insert(own | bit, recursive(own | bit, blocked, memo));
            }
        }
        let p = classify_placed(own, blocked, |next| children[&next]);
        memo.insert(own, p);
        p
    }

    #[test]
    fn table_matches_recursive_definition() {
        let tables = Tables::get();
        let center = 1 << CENTER;
        // Every 3-state window around an empty center: 3^8 combinations.
        let mut memos: HashMap<u32, HashMap<u32, Pattern>> = HashMap::new();
        for code in 0..3u32.pow(8) {
            let (mut own, mut blocked, mut c) = (0, 0, code);
            for i in (0..WINDOW_BITS).filter(|&i| i != CENTER) {
                match c % 3 {
                    1 => own |= 1 << i,
                    2 => blocked |= 1 << i,
                    _ => {}
                }
                c /= 3;
            }
            let memo = memos.entry(blocked).or_default();
            assert_eq!(
                tables.line(own, blocked),
                recursive(own | center, blocked, memo),
                "own={own:09b} blocked={blocked:09b}"
            );
        }
    }

    /// Parse an 11-character window for a [`LineTable`]: `x` own, `o`
    /// opponent, `|` off the board, `.` empty, `_` the empty center.
    fn large(rule: Rule, black: bool, s: &str) -> (Pattern, bool) {
        assert_eq!(s.len(), 11);
        assert_eq!(s.as_bytes()[5], b'_');
        let (mut own, mut blocked) = (0, 0);
        for (i, c) in s.chars().enumerate() {
            match c {
                'x' => own |= 1 << i,
                'o' => blocked |= 1 << i,
                '|' => {
                    own |= 1 << i;
                    blocked |= 1 << i;
                }
                '.' | '_' => {}
                _ => panic!("bad window {s}"),
            }
        }
        let entry = LineTable::get(rule, black).expect("large table").entry(own, blocked);
        (entry_pattern(entry), entry & LINE_FORBIDDEN != 0)
    }

    #[test]
    fn exact_five_rules_reject_overlines() {
        use Pattern::*;
        for rule in [Rule::Standard, Rule::Caro] {
            assert_eq!(large(rule, true, "...xx_xx..."), (F5, false), "{rule}");
            assert_eq!(large(rule, true, "..xxx_xx..."), (Dead, false), "{rule}");
            // The left five point would make six: only one way to five.
            assert_eq!(large(rule, false, ".x.xx_x...."), (B4, false), "{rule}");
            assert_eq!(large(rule, false, "....x_xx..."), (F4, false), "{rule}");
            // Both open-four points of this three would make six.
            assert_eq!(large(rule, true, "x..xx_..x.."), (B3, false), "{rule}");
        }
        assert_eq!(large(Rule::FreestyleCaro, true, "..xxx_xx..."), (F5, false));
        // Renju: White's overline wins, Black's is forbidden.
        assert_eq!(large(Rule::Renju, false, "..xxx_xx..."), (F5, false));
        assert_eq!(large(Rule::Renju, true, "..xxx_xx..."), (Dead, true));
        assert_eq!(large(Rule::Renju, true, ".x.xx_x...."), (B4, false));
        assert_eq!(large(Rule::Renju, true, "x..xx_..x.."), (B3, false));
        assert_eq!(large(Rule::Renju, false, "x..xx_..x.."), (F3S, false));
    }

    #[test]
    fn renju_flags_two_fours_on_one_line() {
        use Pattern::*;
        // Placing the center makes xxx.x.xxx: a five point on each side.
        assert_eq!(large(Rule::Renju, true, "oxxx._.xxxo"), (Dead, true));
        assert_eq!(large(Rule::Standard, true, "oxxx._.xxxo"), (F4, false));
        assert_eq!(large(Rule::Renju, false, "oxxx._.xxxo"), (F4, false));
        // A straight four is not two fours.
        assert_eq!(large(Rule::Renju, true, "o..xx_x...o"), (F4, false));
    }

    #[test]
    fn caro_fives_need_an_open_end() {
        use Pattern::*;
        for rule in [Rule::Caro, Rule::FreestyleCaro] {
            assert_eq!(large(rule, true, "oxxxx_o...."), (Dead, false), "{rule}");
            assert_eq!(large(rule, true, "oxxxx_....."), (F5, false), "{rule}");
            // The board edge does not block.
            assert_eq!(large(rule, true, "|xxxx_o...."), (F5, false), "{rule}");
            assert_eq!(large(rule, true, "..xxx_o...."), (B4, false), "{rule}");
            // Five cells between two stones: any five there is blocked.
            assert_eq!(large(rule, true, "o.xxx_o...."), (Dead, false), "{rule}");
            assert_eq!(large(rule, true, "ooxxx_.o..."), (Dead, false), "{rule}");
        }
        assert_eq!(large(Rule::Standard, true, "oxxxx_o...."), (F5, false));
        assert_eq!(large(Rule::Standard, true, "o.xxx_o...."), (B4, false));
    }

    #[test]
    fn five_or_more_matches_the_freestyle_table() {
        // Renju's White table classifies like freestyle, one cell further.
        let tables = Tables::get();
        let white = LineTable::get(Rule::Renju, false).unwrap();
        let mut rng = 0x9e37_79b9_u64;
        for code in 0..3u32.pow(8) {
            let (mut own, mut blocked, mut c) = (0u32, 0u32, code);
            for i in (0..WINDOW_BITS).filter(|&i| i != CENTER) {
                match c % 3 {
                    1 => own |= 1 << i,
                    2 => blocked |= 1 << i,
                    _ => {}
                }
                c /= 3;
            }
            // Random outer cells: they cannot matter for five or more.
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let outer = |r: u64| match r % 3 {
                1 => (1, 0),
                2 => (0, 1),
                _ => (0, 0),
            };
            let (lo, lb) = outer(rng);
            let (ho, hb) = outer(rng >> 8);
            let own11 = lo | (own << 1) | (ho << 10);
            let blocked11 = lb | (blocked << 1) | (hb << 10);
            assert_eq!(
                entry_pattern(white.entry(own11, blocked11)),
                tables.line(own, blocked),
                "own={own:09b} blocked={blocked:09b}"
            );
        }
    }

    #[test]
    fn pattern_codes_are_order_independent() {
        use Pattern::*;
        let t = Tables::get();
        let code = t.pcode([B4, F3, Dead, F2]);
        assert_eq!(code, t.pcode([F2, Dead, F3, B4]));
        assert_eq!(t.patterns(code), [Dead, F2, F3, B4]);
        assert_eq!(t.pattern4(code), Pattern4::CBlock4Flex3);
        let distinct: std::collections::HashSet<_> = t.pcode.iter().flatten().flatten().flatten().collect();
        assert_eq!(distinct.len(), PCODE_COUNT);
    }

    #[test]
    fn pattern4_combinations() {
        use Pattern::*;
        let cases = [
            ([F5, Dead, Dead, Dead], Pattern4::AFive),
            ([B4, B4, Dead, Dead], Pattern4::BFlex4),
            ([F4, Dead, Dead, Dead], Pattern4::BFlex4),
            ([B4, F3, Dead, Dead], Pattern4::CBlock4Flex3),
            ([B4, B3, Dead, Dead], Pattern4::DBlock4Plus),
            ([B4, Dead, Dead, Dead], Pattern4::EBlock4),
            ([F3, F3S, Dead, Dead], Pattern4::FFlex3x2),
            ([F3, F2, Dead, Dead], Pattern4::GFlex3Plus),
            ([F3S, Dead, Dead, Dead], Pattern4::HFlex3),
            ([B3, F2A, Dead, Dead], Pattern4::IBlock3Plus),
            ([F2, F2B, Dead, Dead], Pattern4::JFlex2x2),
            ([B3, B2, Dead, Dead], Pattern4::KBlock3),
            ([F2, F1, B1, Dead], Pattern4::LFlex2),
            ([F1, F1, B2, B1], Pattern4::None),
        ];
        for (patterns, expected) in cases {
            assert_eq!(Pattern4::from_patterns(patterns), expected, "{patterns:?}");
        }
        let renju = [
            ([F5, B4, B4, Dead], Pattern4::AFive),
            ([B4, B4, Dead, Dead], Pattern4::Forbidden),
            ([F4, B4, Dead, Dead], Pattern4::Forbidden),
            ([F3, F3S, Dead, Dead], Pattern4::Forbidden),
            ([B4, F3, Dead, Dead], Pattern4::CBlock4Flex3),
            ([F4, F3, Dead, Dead], Pattern4::BFlex4),
        ];
        for (patterns, expected) in renju {
            assert_eq!(Pattern4::from_patterns_renju(patterns), expected, "{patterns:?}");
        }
    }
}
