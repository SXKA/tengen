//! The game rules the engine plays, as numbered by the Piskvork protocol.
//!
//! The protocol's `INFO rule` value is a bitmask: 1 means only exactly five in
//! a row wins, 2 a continuous game, 4 renju and 8 caro. The continuous-game
//! bit only changes what the manager sends (see [`crate::Board::place_wall`]),
//! so it is not part of [`Rule`].

/// How a game is won, and which moves are forbidden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rule {
    /// Five or more in a row wins (`INFO rule 0`).
    #[default]
    Freestyle,
    /// Exactly five in a row wins; six or more does not (`INFO rule 1`).
    Standard,
    /// Black wins with exactly five and may not make an overline, two fours
    /// or two open threes at once; White wins with five or more (`INFO rule 4`).
    Renju,
    /// Six or more wins; exactly five wins unless both ends are blocked by
    /// stones, the board edge not counting (`INFO rule 8`).
    FreestyleCaro,
    /// Exactly five wins unless both ends are blocked by stones, the board
    /// edge not counting (`INFO rule 9`, the Gomocup caro rule).
    Caro,
}

impl Rule {
    /// Number of rules.
    pub const COUNT: usize = 5;

    /// Every rule.
    pub const ALL: [Rule; Rule::COUNT] = [
        Rule::Freestyle,
        Rule::Standard,
        Rule::Renju,
        Rule::FreestyleCaro,
        Rule::Caro,
    ];

    /// The rule for a Piskvork `INFO rule` bitmask. Renju takes precedence
    /// over caro, and caro over exactly-five, as in the Gomocup manager.
    pub fn from_piskvork(bits: u32) -> Rule {
        if bits & 4 != 0 {
            Rule::Renju
        } else if bits & 8 != 0 {
            if bits & 1 != 0 { Rule::Caro } else { Rule::FreestyleCaro }
        } else if bits & 1 != 0 {
            Rule::Standard
        } else {
            Rule::Freestyle
        }
    }

    /// The Piskvork `INFO rule` value of this rule.
    pub fn piskvork(self) -> u32 {
        match self {
            Rule::Freestyle => 0,
            Rule::Standard => 1,
            Rule::Renju => 4,
            Rule::FreestyleCaro => 8,
            Rule::Caro => 9,
        }
    }

    /// Short lowercase name, as accepted by [`Rule::parse`].
    pub fn name(self) -> &'static str {
        match self {
            Rule::Freestyle => "freestyle",
            Rule::Standard => "standard",
            Rule::Renju => "renju",
            Rule::FreestyleCaro => "freestyle-caro",
            Rule::Caro => "caro",
        }
    }

    /// A rule by name or by its Piskvork number.
    pub fn parse(text: &str) -> Option<Rule> {
        let text = text.trim().to_ascii_lowercase();
        if let Ok(bits) = text.parse::<u32>() {
            return Some(Rule::from_piskvork(bits));
        }
        Rule::ALL.into_iter().find(|r| r.name() == text)
    }

    /// Index into per-rule arrays.
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piskvork_numbers_round_trip() {
        for rule in Rule::ALL {
            assert_eq!(Rule::from_piskvork(rule.piskvork()), rule);
            assert_eq!(Rule::parse(rule.name()), Some(rule));
            assert_eq!(Rule::parse(&rule.piskvork().to_string()), Some(rule));
        }
        // The continuous-game bit does not change the rule.
        assert_eq!(Rule::from_piskvork(2), Rule::Freestyle);
        assert_eq!(Rule::from_piskvork(3), Rule::Standard);
        assert_eq!(Rule::from_piskvork(6), Rule::Renju);
        assert_eq!(Rule::from_piskvork(11), Rule::Caro);
        assert_eq!(Rule::parse("gomoku"), None);
    }
}
