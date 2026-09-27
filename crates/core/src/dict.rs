//! How a dictionary FST's values read: each word's prior and, in the same
//! value, its grammar — one structure for the words, their frequency and
//! their readings.
//!
//! The prior (`−ln P × 1000`) is kept in steps of `quantum` thousandths of a
//! nat: fewer distinct values, more word tails shared (0.05 nat: the same
//! corrections, 1.3 MB less). Below it, `shift` bits hold the word's grammar
//! id — a (class, reading set) pair, numbered from the commonest, looked up
//! in the context model (`0, 10, id`). With the prior on top, the output
//! the FST gathers on the way to a node (the smallest value below it) still
//! bounds the prior of every word below from underneath: the search prunes
//! on it. The format is kept in the FST's type field; type 0 is the plain
//! prior in thousandths.
//!
//! A word's sense class (tools/build_topics.py) is kept apart, in the
//! context model: it is the same for all forms of a lemma, so there it sits
//! on the stem and the endings are shared (0.6 MB); here, next to the
//! form's own prior, it cost 3.1 MB.
//!
//! Sizes (docs/DESIGN.md): the words alone 2.5 MB; with the grammar alone
//! 3.4 MB (the grammar goes with the ending: tails are shared); with the
//! prior alone 6.3 MB (a word's frequency doesn't — the ending «-ила» of
//! «любила» and of «носила» carries different remainders); with both
//! 9.0 MB, against 6.3 MB plus a 3.5 MB second copy of the words with their
//! grammar.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DictFormat {
    /// Thousandths of a nat per step of the stored prior.
    pub quantum: u32,
    /// Bits of grammar id below the prior (0: none).
    pub shift: u32,
}

/// Marks a type field that holds a [`DictFormat`].
const MARK: u64 = 0xD1C7 << 32;

impl DictFormat {
    /// The prior alone, in thousandths of a nat.
    pub const PLAIN: DictFormat = DictFormat {
        quantum: 1,
        shift: 0,
    };

    pub fn from_type(ty: u64) -> DictFormat {
        if ty & (0xFFFF << 32) != MARK {
            return Self::PLAIN;
        }
        DictFormat {
            quantum: ((ty & 0xFFFF) as u32).max(1),
            shift: ((ty >> 16) & 0x3F) as u32,
        }
    }

    pub fn fst_type(self) -> u64 {
        if self == Self::PLAIN {
            return 0;
        }
        MARK | (self.shift as u64) << 16 | self.quantum as u64
    }

    /// The value of a word: its prior (thousandths of a nat) and grammar id.
    pub fn value(self, prior: u64, grammar: u64) -> u64 {
        let q = self.quantum as u64;
        (((prior + q / 2) / q) << self.shift) | (grammar & mask(self.shift))
    }

    /// The prior (thousandths of a nat) of a value — or, of the output
    /// gathered on the way to a node, a lower bound of its words' priors.
    pub fn prior(self, v: u64) -> u64 {
        (v >> self.shift) * self.quantum as u64
    }

    /// The grammar id of a value (0: none).
    pub fn grammar(self, v: u64) -> u64 {
        v & mask(self.shift)
    }
}

fn mask(bits: u32) -> u64 {
    (1u64 << bits) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_holds_the_prior_and_the_grammar() {
        let f = DictFormat {
            quantum: 50,
            shift: 12,
        };
        assert_eq!(DictFormat::from_type(f.fst_type()), f);
        assert_eq!(DictFormat::from_type(0), DictFormat::PLAIN);
        let v = f.value(12_345, 3143);
        assert_eq!(f.prior(v), 12_350);
        assert_eq!(f.grammar(v), 3143);
        // The smallest of several values bounds each one's prior from below.
        let a = f.value(9_000, 4000);
        let b = f.value(9_100, 1);
        assert!(f.prior(a.min(b)) <= 9_000);
        assert_eq!(DictFormat::PLAIN.prior(8_000), 8_000);
        assert_eq!(DictFormat::PLAIN.grammar(8_000), 0);
    }
}
