//! Static evaluation.
//!
//! The evaluation is linear so it can be tuned by logistic regression on game
//! results (Texel tuning):
//!
//! * every empty cell contributes a value per color, looked up by its pattern
//!   code (the order-independent combination of its four line patterns);
//! * a threat term, indexed by which kinds of forcing moves each side has,
//!   captures what the side to move can do with its tempo.
//!
//! The tables live in `weights.txt`, written by `tengen-tune`. They start
//! from a factored model, where a code's value is the sum of its four line
//! patterns' weights plus a bonus for their combination; tuning refines each
//! code on its own.
//!
//! Scores are from the side to move's point of view, in units where
//! [`SCALE`] maps to a win probability of about 73%.

use std::sync::OnceLock;

use crate::board::{Board, Color, Pos};
use crate::pattern::{PCODE_COUNT, Pattern, Pattern4, PatternCode, Tables};
use crate::rule::Rule;

/// Evaluation score. Mate scores live above [`crate::search::MATE_BOUND`].
pub type Value = i32;

/// Eval units per logistic unit: `win_probability = 1 / (1 + exp(-v / SCALE))`.
pub const SCALE: f64 = 200.0;

/// Stones on the board at which the late-game table takes over fully.
pub const PHASE_PLIES: usize = 40;

/// Number of threat-mask entries.
pub const THREAT_COUNT: usize = 1 << 13;

/// Threat-mask entries of weights files from before the thirteen-bit mask.
const LEGACY_THREAT_COUNT: usize = 1 << 8;

/// Bits of the threat mask: which kinds of forcing move each side has.
/// Besides the four-three, fours that also make another four or two
/// (`FOUR_PLUS`) are told apart from plain fours, and double threes and
/// threes with more (`THREE_PLUS`) from single threes, as in Rapfi; the
/// opponent's five is the side to move's forced block.
mod bit {
    pub const SELF_FLEX4: usize = 1 << 0;
    pub const SELF_FOUR_THREE: usize = 1 << 1;
    pub const SELF_FOUR_PLUS: usize = 1 << 2;
    pub const SELF_FOUR: usize = 1 << 3;
    pub const SELF_THREE_PLUS: usize = 1 << 4;
    pub const SELF_THREE: usize = 1 << 5;
    pub const OPP_FLEX4: usize = 1 << 6;
    pub const OPP_FOUR_THREE: usize = 1 << 7;
    pub const OPP_FOUR_PLUS: usize = 1 << 8;
    pub const OPP_FOUR: usize = 1 << 9;
    pub const OPP_THREE_PLUS: usize = 1 << 10;
    pub const OPP_THREE: usize = 1 << 11;
    pub const OPP_FIVE: usize = 1 << 12;
    /// Shift from a side-to-move bit to the same opponent bit.
    pub const OPP_SHIFT: usize = 6;
}

/// Entry of an eight-bit (legacy) threat table that `mask` refines: flex
/// four, four-three, any other four and any three for each side.
fn legacy_threat_index(mask: usize) -> usize {
    let side = |m: usize| {
        (m & bit::SELF_FLEX4 != 0) as usize
            | ((m & bit::SELF_FOUR_THREE != 0) as usize) << 1
            | ((m & (bit::SELF_FOUR_PLUS | bit::SELF_FOUR) != 0) as usize) << 2
            | ((m & (bit::SELF_THREE_PLUS | bit::SELF_THREE) != 0) as usize) << 3
    };
    side(mask) | side(mask >> bit::OPP_SHIFT) << 4
}

/// Expand an eight-bit (legacy) threat table to the eleven-bit mask.
pub fn expand_legacy_threats(legacy: &[i32]) -> Vec<i32> {
    (0..THREAT_COUNT)
        .map(|mask| legacy[legacy_threat_index(mask)])
        .collect()
}

/// The evaluation tables.
#[derive(Clone, Debug, PartialEq)]
pub struct Weights {
    /// Value of an empty cell for one color, by pattern code, early in the
    /// game.
    pub pcode: Vec<i32>,
    /// The same from [`PHASE_PLIES`] stones on; the evaluation blends the
    /// two linearly in between.
    pub pcode_late: Vec<i32>,
    /// Threat term, indexed by [`threat_mask`].
    pub threat: Vec<i32>,
    /// Move-ordering score of playing a cell, by the mover's pattern code
    /// there (what the move builds).
    pub policy_own: Vec<i32>,
    /// Move-ordering score of playing a cell, by the opponent's pattern code
    /// there (what the move takes away).
    pub policy_opp: Vec<i32>,
    /// Value of each five-cell window still open to the side to move, less
    /// the same for the opponent, by how many of the color's stones it holds
    /// (1 to 4): early in the game, then late ([`WINDOW_WEIGHTS`] in all).
    /// Room to make five, which line patterns alone do not see across the
    /// board.
    pub window: Vec<i32>,
    /// Move-ordering correction for the combination a move makes for the
    /// mover and the one it takes from the opponent, together, by the
    /// position's [`PolicyContext`]: `[context][own][opponent]`
    /// ([`POLICY_CROSS_LEN`] in all), as in Rapfi's classical policy.
    pub policy_cross: Vec<i32>,
}

/// What the position demands, for the move-ordering cross table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PolicyContext {
    /// The opponent can make an open four.
    OpponentFlexFour = 0,
    /// The opponent can make a four-three.
    OpponentFourThree,
    /// Both sides can make an open three or more.
    MutualRace,
    /// Only the side to move can make an open three or more.
    SelfAttacking,
    /// Only the opponent can make an open three or more.
    OpponentAttacking,
    /// Neither can.
    Quiet,
}

impl PolicyContext {
    /// Number of contexts.
    pub const COUNT: usize = 6;
}

/// Entries of [`Weights::policy_cross`].
pub const POLICY_CROSS_LEN: usize = PolicyContext::COUNT * Pattern4::COUNT * Pattern4::COUNT;

/// Open-window counts the window term weighs: windows holding 1 to 4 stones.
pub const WINDOW_KINDS: usize = 4;
/// Entries of [`Weights::window`]: early, then late.
pub const WINDOW_WEIGHTS: usize = 2 * WINDOW_KINDS;

/// Default move-ordering bonus for what a move makes, by combination.
pub const DEFAULT_ATTACK: [i32; Pattern4::COUNT] = [0, 0, 4, 3, 10, 12, 30, 45, 120, 20, 60, 400, 2000, 10000];
/// Default move-ordering bonus for what a move prevents, by combination.
pub const DEFAULT_DEFEND: [i32; Pattern4::COUNT] = [0, 0, 3, 2, 8, 10, 24, 36, 100, 12, 40, 300, 1500, 8000];

/// The default move-ordering tables for an evaluation table: a combination
/// bonus plus the cell's value, three quarters of it for the opponent.
pub fn default_policy(pcode: &[i32]) -> (Vec<i32>, Vec<i32>) {
    let tables = Tables::get();
    let p4 = |code: usize| tables.pattern4(code as PatternCode) as usize;
    let own = pcode
        .iter()
        .enumerate()
        .map(|(c, &v)| DEFAULT_ATTACK[p4(c)] + v)
        .collect();
    let opp = pcode
        .iter()
        .enumerate()
        .map(|(c, &v)| DEFAULT_DEFEND[p4(c)] + v * 3 / 4)
        .collect();
    (own, opp)
}

/// The factored model that seeds the tables: per line pattern and per
/// combination, plus the threat term.
#[derive(Clone, Debug, PartialEq)]
pub struct Factored {
    pub pattern: [i32; Pattern::COUNT],
    pub pattern4: [i32; Pattern4::COUNT],
    pub threat: [i32; THREAT_COUNT],
}

/// Hand-set starting point of the factored model.
pub const DEFAULT_FACTORED: Factored = Factored {
    //        Dead B1 F1 B2 F2 F2A F2B B3 F3 F3S B4  F4   F5
    pattern: [0, 1, 2, 3, 8, 10, 12, 10, 30, 36, 24, 100, 250],
    //         None X  L  K  J   I  H   G   F  E   D    C    B    A
    pattern4: [0, 0, 0, 0, 8, 10, 0, 20, 80, 0, 25, 150, 300, 600],
    threat: default_threats(),
};

/// Default bonus for the side to move.
const TEMPO: i32 = 60;

const fn default_threats() -> [i32; THREAT_COUNT] {
    let mut out = [0; THREAT_COUNT];
    let mut mask = 0;
    while mask < THREAT_COUNT {
        let fours = bit::SELF_FLEX4 | bit::SELF_FOUR_THREE | bit::SELF_FOUR_PLUS | bit::SELF_FOUR;
        let opp_forcing = mask & (fours << bit::OPP_SHIFT | bit::OPP_FIVE) != 0;
        let self_forcing = mask & fours != 0;
        // Having the move is worth about half a stone's potential.
        let mut v = TEMPO;
        if mask & bit::SELF_FLEX4 != 0 {
            v += if opp_forcing { 300 } else { 1500 };
        }
        if mask & bit::SELF_FOUR_THREE != 0 {
            v += if opp_forcing { 100 } else { 400 };
        }
        if mask & (bit::SELF_FOUR_PLUS | bit::SELF_FOUR) != 0 {
            v += 60;
        }
        if mask & (bit::SELF_THREE_PLUS | bit::SELF_THREE) != 0 {
            v += if opp_forcing { 40 } else { 150 };
        }
        if mask & bit::OPP_FLEX4 != 0 {
            v -= if self_forcing { 200 } else { 600 };
        }
        if mask & bit::OPP_FOUR_THREE != 0 {
            v -= 150;
        }
        if mask & (bit::OPP_FOUR_PLUS | bit::OPP_FOUR) != 0 {
            v -= 30;
        }
        if mask & (bit::OPP_THREE_PLUS | bit::OPP_THREE) != 0 {
            v -= 50;
        }
        out[mask] = v;
        mask += 1;
    }
    out
}

impl Factored {
    /// The tables this model implies.
    pub fn weights(&self) -> Weights {
        let tables = Tables::get();
        let pcode = (0..PCODE_COUNT as PatternCode)
            .map(|code| {
                let lines: i32 = tables.patterns(code).iter().map(|&p| self.pattern[p as usize]).sum();
                lines + self.pattern4[tables.pattern4(code) as usize]
            })
            .collect::<Vec<i32>>();
        let (policy_own, policy_opp) = default_policy(&pcode);
        Weights {
            pcode_late: pcode.clone(),
            pcode,
            threat: self.threat.to_vec(),
            policy_own,
            policy_opp,
            window: vec![0; WINDOW_WEIGHTS],
            policy_cross: vec![0; POLICY_CROSS_LEN],
        }
    }
}

impl Default for Weights {
    fn default() -> Weights {
        Weights::built_in(Rule::Freestyle)
    }
}

impl Weights {
    /// The weights the engine ships with for `rule`: `weights-RULE.txt`
    /// (e.g. `weights-renju.txt`) if it exists, else the freestyle
    /// `weights.txt`.
    pub fn built_in(rule: Rule) -> Weights {
        Weights::parse(built_in_text(rule)).expect("built-in weights are valid")
    }

    /// Parse the text form written by [`Weights`]'s `Display`: one line per
    /// table, a name followed by whitespace-separated integers. Lines
    /// starting with `#` are comments. Missing move-ordering tables get
    /// their defaults, a missing window table zeros.
    pub fn parse(text: &str) -> Result<Weights, String> {
        let mut pcode = None;
        let mut pcode_late = None;
        let mut threat = None;
        let mut policy_own = None;
        let mut policy_opp = None;
        let mut window = None;
        let mut policy_cross = None;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let mut words = line.split_whitespace();
            let name = words.next().unwrap_or_default();
            let values: Vec<i32> = words
                .map(|v| v.parse().map_err(|e| format!("{name}: {e}")))
                .collect::<Result<_, _>>()?;
            let values = if name == "threat" && values.len() == LEGACY_THREAT_COUNT {
                expand_legacy_threats(&values)
            } else {
                values
            };
            let (slot, len) = match name {
                "pcode" => (&mut pcode, PCODE_COUNT),
                "pcode_late" => (&mut pcode_late, PCODE_COUNT),
                "threat" => (&mut threat, THREAT_COUNT),
                "policy_own" => (&mut policy_own, PCODE_COUNT),
                "policy_opp" => (&mut policy_opp, PCODE_COUNT),
                "window" => (&mut window, WINDOW_WEIGHTS),
                "policy_cross" => (&mut policy_cross, POLICY_CROSS_LEN),
                _ => return Err(format!("unknown table {name}")),
            };
            if values.len() != len {
                return Err(format!("{name}: expected {len} values, got {}", values.len()));
            }
            *slot = Some(values);
        }
        let pcode = pcode.ok_or("missing pcode table")?;
        let (default_own, default_opp) = default_policy(&pcode);
        Ok(Weights {
            pcode_late: pcode_late.unwrap_or_else(|| pcode.clone()),
            threat: threat.ok_or("missing threat table")?,
            policy_own: policy_own.unwrap_or(default_own),
            policy_opp: policy_opp.unwrap_or(default_opp),
            window: window.unwrap_or_else(|| vec![0; WINDOW_WEIGHTS]),
            policy_cross: policy_cross.unwrap_or_else(|| vec![0; POLICY_CROSS_LEN]),
            pcode,
        })
    }
}

impl std::fmt::Display for Weights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "# Tengen evaluation weights, written by tengen-tune.")?;
        for (name, table) in [
            ("pcode", &self.pcode),
            ("pcode_late", &self.pcode_late),
            ("threat", &self.threat),
            ("policy_own", &self.policy_own),
            ("policy_opp", &self.policy_opp),
            ("window", &self.window),
            ("policy_cross", &self.policy_cross),
        ] {
            let values: Vec<String> = table.iter().map(i32::to_string).collect();
            writeln!(f, "{name} {}", values.join(" "))?;
        }
        Ok(())
    }
}

/// Weights chosen by [`install`], for every rule.
static INSTALLED: OnceLock<CellValues> = OnceLock::new();

include!(concat!(env!("OUT_DIR"), "/weights.rs"));
/// The built-in weights per rule, loaded on first use.
static BUILT_IN: [OnceLock<CellValues>; Rule::COUNT] = [const { OnceLock::new() }; Rule::COUNT];

/// Use `weights` instead of the built-in ones, under every rule, for the
/// rest of the process.
///
/// Must be called before the first board is created; fails afterwards.
pub fn install(weights: &Weights) -> Result<(), String> {
    if BUILT_IN.iter().any(|v| v.get().is_some()) {
        return Err("evaluation weights are already in use".to_string());
    }
    INSTALLED
        .set(CellValues::new(weights))
        .map_err(|_| "evaluation weights are already in use".to_string())
}

/// The tables in the form the board and search use them.
pub struct CellValues {
    values: Box<[i32; PCODE_COUNT]>,
    values_late: Box<[i32; PCODE_COUNT]>,
    threat: Box<[i32; THREAT_COUNT]>,
    policy_own: Box<[i32; PCODE_COUNT]>,
    policy_opp: Box<[i32; PCODE_COUNT]>,
    window: [i32; WINDOW_WEIGHTS],
    policy_cross: Box<[i32; POLICY_CROSS_LEN]>,
    /// Contexts in which some cross entry is not zero, one bit each.
    cross_active: u8,
}

impl CellValues {
    /// Tables for the weights in use under `rule`: the built-in ones unless
    /// [`install`] chose others.
    #[inline]
    pub fn for_rule(rule: Rule) -> &'static CellValues {
        if let Some(values) = INSTALLED.get() {
            return values;
        }
        BUILT_IN[rule.index()].get_or_init(|| CellValues::new(&Weights::built_in(rule)))
    }

    /// Tables for arbitrary weights.
    pub fn new(weights: &Weights) -> CellValues {
        CellValues {
            values: weights.pcode.clone().try_into().expect("pcode table size"),
            values_late: weights.pcode_late.clone().try_into().expect("pcode table size"),
            threat: weights.threat.clone().try_into().expect("threat table size"),
            policy_own: weights.policy_own.clone().try_into().expect("policy table size"),
            policy_opp: weights.policy_opp.clone().try_into().expect("policy table size"),
            window: weights.window.clone().try_into().expect("window table size"),
            policy_cross: weights
                .policy_cross
                .clone()
                .try_into()
                .expect("policy cross table size"),
            cross_active: weights
                .policy_cross
                .chunks(Pattern4::COUNT * Pattern4::COUNT)
                .enumerate()
                .filter(|(_, c)| c.iter().any(|&v| v != 0))
                .fold(0, |mask, (i, _)| mask | 1 << i),
        }
    }

    /// Whether the cross table has any entry for `context`.
    #[inline]
    pub fn cross_active(&self, context: PolicyContext) -> bool {
        self.cross_active & (1 << context as u8) != 0
    }

    /// Whether the cross table has any entry at all.
    #[inline]
    pub fn has_cross(&self) -> bool {
        self.cross_active != 0
    }

    /// Cross-table correction for a move making `own` for the mover and
    /// taking `opp` from the opponent, in `context`.
    #[inline]
    pub fn policy_cross(&self, context: PolicyContext, own: Pattern4, opp: Pattern4) -> i32 {
        self.policy_cross[(context as usize * Pattern4::COUNT + own as usize) * Pattern4::COUNT + opp as usize]
    }

    /// Move-ordering score of a cell with the mover's code `own` and the
    /// opponent's code `opp`.
    #[inline]
    pub fn policy(&self, own: PatternCode, opp: PatternCode) -> i32 {
        self.policy_own[own as usize] + self.policy_opp[opp as usize]
    }

    /// Value of a cell with pattern code `code`, early in the game.
    #[inline]
    pub fn value(&self, code: PatternCode) -> i32 {
        self.values[code as usize]
    }

    /// Value of a cell with pattern code `code`, late in the game.
    #[inline]
    pub fn value_late(&self, code: PatternCode) -> i32 {
        self.values_late[code as usize]
    }
}

/// Which kinds of forcing moves each side has, for the threat term.
pub fn threat_mask(board: &Board, us: Color) -> usize {
    use Pattern4::*;
    let side = |c: Color| {
        let any = |kinds: &[Pattern4]| kinds.iter().any(|&k| board.p4_count(c, k) > 0);
        let flag = |on: bool, b: usize| if on { b } else { 0 };
        flag(any(&[BFlex4]), bit::SELF_FLEX4)
            | flag(any(&[CBlock4Flex3]), bit::SELF_FOUR_THREE)
            | flag(any(&[DBlock4Plus]), bit::SELF_FOUR_PLUS)
            | flag(any(&[EBlock4]), bit::SELF_FOUR)
            | flag(any(&[FFlex3x2, GFlex3Plus]), bit::SELF_THREE_PLUS)
            | flag(any(&[HFlex3]), bit::SELF_THREE)
    };
    let them = us.opponent();
    let five = if board.p4_count(them, AFive) > 0 {
        bit::OPP_FIVE
    } else {
        0
    };
    side(us) | side(them) << bit::OPP_SHIFT | five
}

/// Static evaluation from the side to move's point of view.
#[inline]
pub fn evaluate(board: &Board) -> Value {
    let us = board.side_to_move();
    let them = us.opponent();
    let values = board.values();
    let (mine, theirs) = (board.open_windows(us), board.open_windows(them));
    let (mut window_early, mut window_late) = (0, 0);
    for k in 0..WINDOW_KINDS {
        let d = (mine[k + 1] - theirs[k + 1]) as i32;
        window_early += values.window[k] * d;
        window_late += values.window[WINDOW_KINDS + k] * d;
    }
    let early = board.pattern_value(us) - board.pattern_value(them) + window_early;
    let late = board.pattern_value_late(us) - board.pattern_value_late(them) + window_late;
    blend(early, late, board.ply()) + values.threat[threat_mask(board, us)]
}

/// Mix early- and late-game values by the number of stones played.
#[inline]
fn blend(early: i32, late: i32, ply: usize) -> i32 {
    let t = ply.min(PHASE_PLIES) as i32;
    (early * (PHASE_PLIES as i32 - t) + late * t) / PHASE_PLIES as i32
}

/// Win probability of the side to move for an evaluation.
pub fn win_probability(value: Value) -> f64 {
    1.0 / (1.0 + (-(value as f64) / SCALE).exp())
}

/// Linear features of a position, for tuning: `evaluate` equals the dot
/// product of these with the weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Features {
    /// Own minus opponent count of empty cells per pattern code, nonzero
    /// entries only.
    pub pcode: Vec<(PatternCode, i32)>,
    /// Active threat-mask entry.
    pub threat: usize,
    /// Stones on the board, for the game-phase blend.
    pub ply: usize,
    /// Own minus opponent open five-cell windows holding 1 to 4 stones.
    pub window: [i32; WINDOW_KINDS],
}

impl Features {
    /// Extract the features of `board` for the side to move.
    pub fn of(board: &Board) -> Features {
        let us = board.side_to_move();
        let mut counts = vec![0i32; PCODE_COUNT];
        let n = board.size();
        for pos in (0..n * n)
            .map(|i| Pos::new(i % n, i / n))
            .filter(|&p| board.is_empty(p))
        {
            counts[board.pcode(pos, us) as usize] += 1;
            counts[board.pcode(pos, us.opponent()) as usize] -= 1;
        }
        let pcode = counts
            .iter()
            .enumerate()
            .filter(|(_, n)| **n != 0)
            .map(|(code, &n)| (code as PatternCode, n))
            .collect();
        let (mine, theirs) = (board.open_windows(us), board.open_windows(us.opponent()));
        Features {
            pcode,
            threat: threat_mask(board, us),
            ply: board.ply(),
            window: std::array::from_fn(|k| (mine[k + 1] - theirs[k + 1]) as i32),
        }
    }

    /// Dot product with `weights`, i.e. the evaluation those weights give.
    pub fn dot(&self, weights: &Weights) -> i32 {
        let early: i32 = self
            .pcode
            .iter()
            .map(|&(code, n)| n * weights.pcode[code as usize])
            .sum();
        let late: i32 = self
            .pcode
            .iter()
            .map(|&(code, n)| n * weights.pcode_late[code as usize])
            .sum();
        let window_early: i32 = (0..WINDOW_KINDS).map(|k| weights.window[k] * self.window[k]).sum();
        let window_late: i32 = (0..WINDOW_KINDS)
            .map(|k| weights.window[WINDOW_KINDS + k] * self.window[k])
            .sum();
        blend(early + window_early, late + window_late, self.ply) + weights.threat[self.threat]
    }

    /// Weight of the late-game table for this position, from 0 to 1.
    pub fn phase(&self) -> f64 {
        self.ply.min(PHASE_PLIES) as f64 / PHASE_PLIES as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_reproduce_the_evaluation() {
        let weights = Weights::default();
        let mut board = Board::new(15);
        for (x, y) in [(7, 7), (8, 8), (7, 8), (6, 6), (8, 6), (9, 9), (6, 8), (0, 1)] {
            board.play(Pos::new(x, y));
            assert_eq!(Features::of(&board).dot(&weights), evaluate(&board));
        }
    }

    #[test]
    fn open_three_favors_its_owner() {
        let mut board = Board::new(15);
        for (x, y) in [(7, 7), (0, 14), (8, 7), (14, 14), (9, 7)] {
            board.play(Pos::new(x, y));
        }
        // White to move against Black's open three.
        assert!(evaluate(&board) < -150, "{}", evaluate(&board));
        assert_eq!(win_probability(0), 0.5);
    }

    #[test]
    fn weights_round_trip_through_text() {
        let weights = DEFAULT_FACTORED.weights();
        assert_eq!(Weights::parse(&weights.to_string()).unwrap(), weights);
        assert!(Weights::parse("pcode 1 2").is_err());
        assert!(Weights::parse("bogus 1").is_err());
        assert!(Weights::parse("threat 1").is_err());
        // Move-ordering tables default when absent.
        let text: String = weights
            .to_string()
            .lines()
            .filter(|l| !l.starts_with("policy"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(Weights::parse(&text).unwrap(), weights);
    }

    #[test]
    fn legacy_threats_expand_to_the_entries_they_cover() {
        let legacy: Vec<i32> = (0..LEGACY_THREAT_COUNT as i32).collect();
        let expanded = expand_legacy_threats(&legacy);
        assert_eq!(expanded[0], 0);
        assert_eq!(expanded[bit::SELF_FOUR_THREE], 0b10);
        assert_eq!(expanded[bit::SELF_FOUR_PLUS], 0b100);
        assert_eq!(expanded[bit::SELF_FOUR | bit::SELF_FOUR_PLUS], 0b100);
        assert_eq!(expanded[bit::SELF_THREE_PLUS], 0b1000);
        assert_eq!(expanded[bit::OPP_FLEX4 | bit::OPP_THREE], 0b1001_0000);
        assert_eq!(expanded[bit::OPP_FIVE], 0);
    }

    #[test]
    fn threat_mask_sees_each_side() {
        let mut board = Board::new(15);
        for (x, y) in [(7, 7), (0, 14), (8, 7), (14, 14), (9, 7)] {
            board.play(Pos::new(x, y));
        }
        // White to move; Black (the opponent) has open-four points.
        assert_ne!(threat_mask(&board, Color::White) & bit::OPP_FLEX4, 0);
        assert_eq!(threat_mask(&board, Color::White) & bit::SELF_FLEX4, 0);
        assert_ne!(threat_mask(&board, Color::Black) & bit::SELF_FLEX4, 0);
    }
}
