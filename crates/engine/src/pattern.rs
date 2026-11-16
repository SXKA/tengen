//! Line patterns, their combinations, and the lookup tables behind them.
//!
//! For every empty cell, direction and color, the engine classifies what a
//! stone of that color would form on that line if it were placed there. In
//! freestyle only the 4 cells on each side matter, so the classification is
//! precomputed for every possible 9-cell window (9 bits of own stones and 9
//! bits of blocked cells: opponent stones or off-board), 2^18 entries.
//!
//! The pattern vocabulary (F/B levels, the "S" and "A/B" refinements and the
//! A..L combination classes) follows the Rapfi engine's terminology; the
//! classification code is an independent implementation.

use std::sync::OnceLock;

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
    pub const COUNT: usize = 13;

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

#[inline]
fn table_index(own: u32, blocked: u32) -> usize {
    ((own & WINDOW_MASK) | ((blocked & WINDOW_MASK) << WINDOW_BITS)) as usize
}

/// All precomputed pattern tables.
pub struct Tables {
    /// Window -> pattern of a stone on the empty center.
    line: Vec<Pattern>,
    /// Four patterns (as a base-13 number) -> pattern code.
    pcode: Vec<PatternCode>,
    /// Pattern code -> combination.
    pattern4: Vec<Pattern4>,
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
        let n = Pattern::COUNT;
        let index = ((p[0] as usize * n + p[1] as usize) * n + p[2] as usize) * n + p[3] as usize;
        self.pcode[index]
    }

    /// Combination class of a pattern code.
    #[inline]
    pub fn pattern4(&self, pcode: PatternCode) -> Pattern4 {
        self.pattern4[pcode as usize]
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
        let n = Pattern::COUNT;
        let pcode = (0..n.pow(4))
            .map(|index| {
                let mut key = [index / (n * n * n), index / (n * n) % n, index / n % n, index % n];
                key.sort_unstable();
                index_of[&key]
            })
            .collect();
        let pattern4 = patterns.iter().map(|&p| Pattern4::from_patterns(p)).collect();
        Tables {
            line,
            pcode,
            pattern4,
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
        let bit = cells & cells.wrapping_neg();
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

fn build_line_table() -> Vec<Pattern> {
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

    let mut table = vec![Pattern::Dead; size];
    for own in 0..(1u32 << WINDOW_BITS) {
        for blocked in 0..(1u32 << WINDOW_BITS) {
            if own & blocked == 0 && (own | blocked) & center == 0 {
                table[table_index(own, blocked)] = placed[table_index(own | center, blocked)];
            }
        }
    }
    table
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
            let bit = cells & cells.wrapping_neg();
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

    #[test]
    fn pattern_codes_are_order_independent() {
        use Pattern::*;
        let t = Tables::get();
        let code = t.pcode([B4, F3, Dead, F2]);
        assert_eq!(code, t.pcode([F2, Dead, F3, B4]));
        assert_eq!(t.patterns(code), [Dead, F2, F3, B4]);
        assert_eq!(t.pattern4(code), Pattern4::CBlock4Flex3);
        let distinct: std::collections::HashSet<_> = t.pcode.iter().collect();
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
    }
}
