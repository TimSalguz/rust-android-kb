//! Where commas go, by the words themselves and by their kinds and the
//! sentence so far (tools/build_commas.py): a gerund or participle opens a
//! phrase, the predicate after it closes it, «и…, и…», an address, noun
//! phrases side by side in one case.

use super::{remap, Engine, BIGRAM_SEP};
use crate::gram::bit::*;
use crate::gram::cases;

/// What a comma around a word depends on beyond the word itself: its kind
/// (by all its readings; `MIX` when they differ, `UNK` with none) and the
/// cases its noun readings may be in.
fn kind(readings: &[u64]) -> (&'static str, u64) {
    let mut kinds: Vec<&'static str> = Vec::new();
    let mut all_cases = 0;
    for &r in readings {
        let k = if r & VERB != 0 {
            if r & (PAST | PRES | FUTR) != 0 {
                "VERB"
            } else {
                "IMPR"
            }
        } else if r & (NOUN | NPRO | ADJF) != 0 {
            let c = cases(r) & !VOCT;
            all_cases |= c;
            match (c == NOMN, r & ADJF != 0) {
                (true, false) => "NOM",
                (true, true) => "NOMA",
                (false, false) => "OBL",
                (false, true) => "OBLA",
            }
        } else if r & GRND != 0 {
            "GRND"
        } else if r & PRTF != 0 {
            "PRTF"
        } else if r & (PRTS | ADJS) != 0 {
            "PRED"
        } else if r & INFN != 0 {
            "INFN"
        } else if r & (ADVB | PRCL | PRED | COMP) != 0 {
            "ADV"
        } else if r & PREP != 0 {
            "PREP"
        } else if r & CONJ != 0 {
            "CONJ"
        } else if r & INTJ != 0 {
            "INTJ"
        } else if r & NUMR != 0 {
            "NUM"
        } else {
            "X"
        };
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let k = match kinds.as_slice() {
        [] => "UNK",
        [k] => k,
        _ => "MIX",
    };
    (k, all_cases)
}

/// The sentence as far as the word before the gap: whether that word
/// starts it; a gerund or participle phrase opened by a comma, no comma
/// since (and whether a noun phrase in the nominative came before it); an
/// «и» among the three words before that word since the last comma.
#[derive(Default)]
struct State {
    start: bool,
    open: Option<bool>,
    recent_and: bool,
}

impl<D: AsRef<[u8]>> Engine<D> {
    /// The log odds of a comma between `a` and `b`, `text` the text up to
    /// and with `a`: the pair's own odds when it has them; else what each
    /// word says — or its kind, where the word says nothing — and the kinds
    /// as a pair; an open phrase and the kind after it; «и» after an «и»;
    /// at the sentence's start, the pair's kinds there (an address).
    pub fn comma_odds_in(&self, text: &str, a: &str, b: &str) -> Option<f32> {
        let m = self.bigrams.as_ref()?;
        let odds = |key: Vec<u8>| m.get(key).map(|v| v as f32 * 0.05 - 6.4);
        let feature = |f: &str| odds([vec![BIGRAM_SEP, 15, 4], f.as_bytes().to_vec()].concat());
        let base = odds(vec![BIGRAM_SEP, 15, 0])?;
        let (ia, ib) = (remap(a)?, remap(b)?);
        let pair = [
            vec![BIGRAM_SEP, 15, 3],
            ia.clone(),
            vec![BIGRAM_SEP],
            ib.clone(),
        ]
        .concat();
        if let Some(v) = odds(pair) {
            return Some(v);
        }
        let ((ka, ca), (kb, cb)) = (kind(&self.readings(a)), kind(&self.readings(b)));
        let state = self.comma_state(text);
        let kind_b = feature(&format!(">{kb}"));
        let kind_a = feature(&format!("<{ka}"));
        let mut ev_b = odds([vec![BIGRAM_SEP, 15, 1], ib].concat())
            .or(kind_b)
            .unwrap_or(base);
        if let Some(subject) = state.open {
            let o = if subject { "openS" } else { "open" };
            if let Some(v) = feature(&format!("{o}>{kb}")) {
                ev_b = v;
            }
        }
        if matches!(kb, "GRND" | "PRTF") {
            if let Some(v) = feature(&format!("{ka}>{kb}")) {
                ev_b = v;
            }
        }
        if b == "и" && state.recent_and {
            if let Some(v) = feature("и>и") {
                ev_b = v;
            }
        }
        let ev_a = odds([vec![BIGRAM_SEP, 15, 2], ia].concat())
            .or(kind_a)
            .unwrap_or(base);
        let guess = base + (ev_b - base) + (ev_a - base);
        let kguess = base + (kind_b.unwrap_or(base) - base) + (kind_a.unwrap_or(base) - base);
        let shared = ka.get(..3) == kb.get(..3)
            && matches!(ka.get(..3), Some("NOM" | "OBL"))
            && ca & cb != 0;
        let pk = format!("{ka}{}{kb}", if shared { "=" } else { " " });
        let together = if state.start {
            feature(&format!("^{pk}")).or_else(|| feature(&pk))
        } else {
            feature(&pk)
        };
        Some(match together {
            Some(v) => guess + (v - kguess),
            None => guess,
        })
    }

    /// The sentence's state as far as the last word of `text`.
    fn comma_state(&self, text: &str) -> State {
        let lower = text.to_lowercase();
        let mut tokens: Vec<&str> = Vec::new();
        let mut rest = lower.as_str();
        while let Some(c) = rest.chars().next() {
            if c.is_alphabetic() || c == '-' {
                let end = rest
                    .char_indices()
                    .find(|(_, c)| !(c.is_alphabetic() || *c == '-'))
                    .map_or(rest.len(), |(i, _)| i);
                tokens.push(&rest[..end]);
                rest = &rest[end..];
            } else {
                if matches!(c, ',' | '.' | '!' | '?') {
                    tokens.push(&rest[..c.len_utf8()]);
                }
                rest = &rest[c.len_utf8()..];
            }
        }
        let last = tokens
            .iter()
            .rposition(|t| t.chars().next().is_some_and(char::is_alphabetic));
        let Some(last) = last else {
            return State::default();
        };
        let (mut start, mut open, mut subject_seen) = (true, None, false);
        let mut since_comma: Vec<&str> = Vec::new();
        let mut comma = false;
        for (i, &t) in tokens.iter().enumerate() {
            match t {
                "." | "!" | "?" => {
                    start = true;
                    open = None;
                    subject_seen = false;
                    since_comma.clear();
                    comma = false;
                }
                "," => comma = true,
                w => {
                    let (k, _) = kind(&self.readings(w));
                    if comma {
                        // A comma: a phrase opens before a gerund or a
                        // participle, else one open closes.
                        open = matches!(k, "GRND" | "PRTF").then_some(subject_seen);
                        since_comma.clear();
                        comma = false;
                    }
                    if i == last {
                        return State {
                            start,
                            open,
                            recent_and: since_comma.iter().rev().take(3).any(|&w| w == "и"),
                        };
                    }
                    start = false;
                    since_comma.push(w);
                    if matches!(k, "NOM" | "NOMA") {
                        subject_seen = true;
                    }
                }
            }
        }
        State::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{alphabet, Config};
    use fst::Map;

    #[test]
    fn a_phrase_opened_after_a_subject_closes_before_its_predicate() {
        // «мальчик, читающий книгу, улыбнулся»: the open phrase asks for
        // the comma before the predicate.
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let mut words: Vec<(Vec<u8>, u64)> = ["мальчик", "читающий", "книгу", "улыбнулся"]
            .iter()
            .map(|w| (id(w), 5000u64))
            .collect();
        words.sort();
        let e = Engine::new(Map::from_iter(words).unwrap(), Config::default());
        let parse = crate::gram::parse;
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let word = |w: &str, set: u64| ([vec![0, 3], id(w)].concat(), 1 | set << 16);
        let feat = |f: &str| [vec![0, 15, 4], f.as_bytes().to_vec()].concat();
        let mut kv = vec![
            word("мальчик", 1),
            word("читающий", 2),
            word("книгу", 3),
            word("улыбнулся", 4),
            (vec![0, 9, 0, 1, 0], parse("NOUN,anim,masc,sing,nomn")),
            (vec![0, 9, 0, 2, 0], parse("PRTF,masc,sing,nomn")),
            (vec![0, 9, 0, 3, 0], parse("NOUN,inan,femn,sing,accs")),
            (vec![0, 9, 0, 4, 0], parse("VERB,masc,sing,past")),
            (vec![0, 15, 0], byte(-2.3)),
            (feat("openS>VERB"), byte(2.5)),
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        let open = e
            .comma_odds_in("мальчик, читающий книгу", "книгу", "улыбнулся")
            .unwrap();
        let closed = e
            .comma_odds_in("мальчик читает книгу", "книгу", "улыбнулся")
            .unwrap();
        assert!(open > 2.0 && closed < 0.0, "{open} {closed}");
    }
}
