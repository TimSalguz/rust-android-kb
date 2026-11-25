use std::collections::HashSet;

use super::{Candidate, Engine, Evidence};

const PREFIXES: &[&str] = &[
    "дигидро",
    "диметил",
    "триметил",
    "триокси",
    "диокси",
    "тетра",
    "гидро",
    "моно",
    "нитро",
    "нитрозо",
    "сульфо",
    "фосфо",
    "хлор",
    "бром",
    "фтор",
    "иод",
    "оксо",
    "тио",
    "азо",
    "циано",
    "амино",
    "метил",
    "этил",
    "пропил",
    "бутил",
    "сверх",
    "супер",
    "мега",
    "ультра",
    "гипер",
    "гипо",
    "макро",
    "микро",
    "мульти",
    "псевдо",
    "квази",
    "анти",
    "контр",
    "архи",
    "интер",
    "пост",
    "пред",
    "пере",
    "недо",
    "полу",
    "мини",
    "макси",
    "экс",
    "наи",
    "вне",
    "внутри",
    "общерус",
    "общеслав",
];
const SUFFIXES: &[&str] = &[
    "нутый",
    "нутого",
    "нутому",
    "нутым",
    "нутом",
    "нутая",
    "нутой",
    "нутую",
    "нутое",
    "нутые",
    "нутых",
    "нутыми",
    "нуть",
    "нул",
    "нула",
    "нули",
    "ированный",
    "ированного",
    "ированная",
    "ированное",
    "ированные",
    "ировать",
    "ирует",
    "ируют",
    "ование",
    "овка",
];
const MAX_PREFIXES: usize = 4;
const MAX_STATES: usize = 8;
const PREFIX_PENALTY: f32 = 0.65;

#[derive(Clone, Debug, Default)]
struct State {
    at: usize,
    depth: usize,
    word: String,
    cost: f32,
    edit: f32,
}

fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<usize> = (0..=b.chars().count()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.chars().enumerate() {
            let old = row[j + 1];
            row[j + 1] = if x == y {
                diagonal
            } else {
                1 + diagonal.min(row[j]).min(row[j + 1])
            };
            diagonal = old;
        }
    }
    row[b.chars().count()]
}

fn near_prefix(word: &str) -> bool {
    PREFIXES.iter().any(|p| distance(word, p) <= 1)
}

fn join(state: &State, part: &Candidate, chars: &[char]) -> State {
    State {
        at: state.at + chars.len(),
        depth: state.depth + 1,
        word: format!("{}{}", state.word, part.word),
        cost: state.cost + part.cost + PREFIX_PENALTY,
        edit: state.edit + part.edit + PREFIX_PENALTY,
    }
}

impl<D: AsRef<[u8]>> Engine<D> {
    fn prefix_parts(&self, typed: &str, correct: bool) -> Vec<Candidate> {
        if PREFIXES.contains(&typed) {
            return vec![Candidate {
                word: typed.to_string(),
                cost: if correct {
                    self.word_cost(typed).unwrap_or(7.0)
                } else {
                    0.0
                },
                edit: 0.0,
            }];
        }
        if !correct || typed.chars().count() < 3 || !near_prefix(typed) {
            return Vec::new();
        }
        let exact_cost = self.word_cost(typed);
        self.suggest(typed)
            .into_iter()
            .filter(|c| PREFIXES.contains(&c.word.as_str()))
            .filter(|c| c.edit <= 3.5 && c.edit <= 0.85 * typed.chars().count() as f32)
            .filter(|c| exact_cost.is_none_or(|cost| c.cost <= cost + 3.0))
            .take(2)
            .collect()
    }

    fn derived_parts(&self, typed: &str, begun: bool) -> Vec<Candidate> {
        let chars: Vec<char> = typed.chars().collect();
        let mut out = Vec::new();
        for i in 3..chars.len() {
            let stem: String = chars[..i].iter().collect();
            let Some(stem_cost) = self.word_cost(&stem).filter(|_| self.contains(&stem)) else {
                continue;
            };
            let ending: String = chars[i..].iter().collect();
            let ending_len = chars.len() - i;
            for &suffix in SUFFIXES {
                let suffix_len = suffix.chars().count();
                let candidate = if begun {
                    if !suffix.starts_with(&ending) || suffix == ending {
                        continue;
                    }
                    let edit =
                        0.8 + self.config().c_complete_char * (suffix_len - ending_len) as f32;
                    Candidate {
                        word: format!("{stem}{suffix}"),
                        cost: stem_cost + edit + 0.5,
                        edit,
                    }
                } else {
                    let d = distance(&ending, suffix);
                    if d > 2 || d > ending_len.max(suffix_len) / 3 {
                        continue;
                    }
                    let preference = if d == 0 || suffix == "нутый" {
                        0.0
                    } else {
                        1.0
                    };
                    let edit = 0.8 + 1.2 * d as f32 + preference;
                    Candidate {
                        word: format!("{stem}{suffix}"),
                        cost: stem_cost + edit + 0.5,
                        edit,
                    }
                };
                out.push(candidate);
            }
        }
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost));
        out.truncate(8);
        out
    }

    pub(crate) fn loan_derivation(&self, word: &str) -> bool {
        ["ный", "но"].iter().any(|suffix| {
            let Some(stem) = word.strip_suffix(suffix) else {
                return false;
            };
            if stem.chars().count() < 3 {
                return false;
            }
            crate::morph::transliterations(stem)
                .into_iter()
                .take(8)
                .any(|latin| {
                    self.suggest(&latin)
                        .into_iter()
                        .take(2)
                        .any(|c| c.edit <= 3.5 && c.word.is_ascii())
                })
        })
    }

    pub(crate) fn compound_adjective_parts(&self, raw: &str) -> Vec<Candidate> {
        let chars: Vec<char> = raw.chars().collect();
        let mut out = Vec::new();
        for i in 3..chars.len().saturating_sub(4) {
            let left: String = chars[..i].iter().collect();
            let Some(stem) = left.strip_suffix(['о', 'е']) else {
                continue;
            };
            let right: String = chars[i..].iter().collect();
            for ending in ["ой", "ий", "ый"] {
                let adjective = format!("{stem}{ending}");
                let Some(left_cost) = self
                    .word_cost(&adjective)
                    .filter(|_| self.contains(&adjective))
                else {
                    continue;
                };
                for suffix in ["ный", "ий", "ый"] {
                    let Some(noun) = right.strip_suffix(suffix) else {
                        continue;
                    };
                    if noun.chars().count() < 2 {
                        continue;
                    }
                    let noun = if self.contains(noun) {
                        noun.to_string()
                    } else {
                        format!("{}я", noun.strip_suffix('й').unwrap_or(noun))
                    };
                    let Some(right_cost) = self.word_cost(&noun).filter(|_| self.contains(&noun))
                    else {
                        continue;
                    };
                    out.push(Candidate {
                        word: raw.to_string(),
                        cost: left_cost + right_cost + 1.0,
                        edit: 1.0,
                    });
                }
            }
        }
        out
    }

    fn compound_parts(&self, raw: &str) -> Vec<Candidate> {
        let chars: Vec<char> = raw.chars().collect();
        let mut out = Vec::new();
        for i in 4..chars.len().saturating_sub(3) {
            let left: String = chars[..i].iter().collect();
            let Some(stem) = left.strip_suffix(['о', 'е']) else {
                continue;
            };
            let right: String = chars[i..].iter().collect();
            let adjective = format!("{stem}{}", if left.ends_with('о') { "ый" } else { "ий" });
            let Some(left_cost) = self
                .word_cost(&adjective)
                .filter(|_| self.contains(&adjective))
            else {
                continue;
            };
            {
                let right_candidates = if self.contains(&right) {
                    vec![Candidate {
                        word: right.clone(),
                        cost: self.word_cost(&right).unwrap_or(7.0),
                        edit: 0.0,
                    }]
                } else {
                    self.suggest(&right)
                };
                for c in right_candidates.into_iter().take(2) {
                    if c.word.chars().next() != right.chars().next() || c.edit > 1.5 {
                        continue;
                    }
                    out.push(Candidate {
                        word: format!("{left}{}", c.word),
                        cost: left_cost + c.cost + 1.0,
                        edit: c.edit + 1.0,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost));
        out.truncate(4);
        out
    }

    fn exact_incomplete(&self, raw: &str) -> bool {
        let chars: Vec<char> = raw.chars().collect();
        let mut at = 0;
        let mut depth = 0;
        while depth < MAX_PREFIXES {
            let rest: String = chars[at..].iter().collect();
            let Some(prefix) = PREFIXES
                .iter()
                .filter(|p| rest.starts_with(**p))
                .max_by_key(|p| p.chars().count())
            else {
                break;
            };
            at += prefix.chars().count();
            depth += 1;
        }
        depth > 0 && chars.len() - at < 3
    }

    fn exact_parts(&self, raw: &str, begun: bool) -> Vec<Candidate> {
        let chars: Vec<char> = raw.chars().collect();
        let mut state = State::default();
        while state.depth < MAX_PREFIXES {
            let rest: String = chars[state.at..].iter().collect();
            let Some(prefix) = PREFIXES
                .iter()
                .filter(|p| rest.starts_with(**p))
                .max_by_key(|p| p.chars().count())
            else {
                break;
            };
            let plen = prefix.chars().count();
            if chars.len() - state.at - plen < if begun { 1 } else { 3 } {
                break;
            }
            let part = Candidate {
                word: prefix.to_string(),
                cost: if begun {
                    0.0
                } else {
                    self.word_cost(prefix).unwrap_or(7.0)
                },
                edit: 0.0,
            };
            state = join(&state, &part, &chars[state.at..state.at + plen]);
        }
        if state.depth == 0 || chars.len() - state.at < if begun { 1 } else { 3 } {
            return Vec::new();
        }
        let rest: String = chars[state.at..].iter().collect();
        let base = if begun {
            self.complete_n(&rest, 32)
        } else if self.contains(&rest) {
            vec![Candidate {
                word: rest.clone(),
                cost: self.word_cost(&rest).unwrap_or(7.0),
                edit: 0.0,
            }]
        } else {
            self.suggest(&rest)
        };
        base.into_iter()
            .take(4)
            .filter(|c| begun || c.word.chars().next() == rest.chars().next())
            .filter(|c| !begun || c.word.chars().count() > rest.chars().count())
            .map(|c| Candidate {
                word: format!("{}{}", state.word, c.word),
                cost: state.cost + c.cost,
                edit: state.edit + c.edit,
            })
            .collect()
    }

    fn productive_parts(&self, raw: &str, begun: bool) -> Vec<Candidate> {
        let chars: Vec<char> = raw.to_lowercase().chars().collect();
        let n = chars.len();
        if !(6..=32).contains(&n)
            || !chars
                .iter()
                .all(|c| "абвгдеёжзийклмнопрстуфхцчшщъыьэюя".contains(*c))
            || self.contains(raw)
        {
            return Vec::new();
        }
        let mut out = self.derived_parts(raw, begun);
        let exact = self.exact_parts(raw, begun);
        if !exact.is_empty() {
            out.extend(exact);
            out.retain(|c| !begun || c.word.starts_with(raw));
            out.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
            let mut seen = HashSet::new();
            out.retain(|c| seen.insert(c.word.clone()));
            out.truncate(32);
            return out;
        }
        if self.exact_incomplete(raw) {
            return out;
        }
        let mut states = vec![State::default()];
        if !begun {
            out.extend(self.compound_adjective_parts(raw));
            out.extend(self.compound_parts(raw));
        }
        for _ in 0..=MAX_PREFIXES {
            let mut next = Vec::new();
            for state in states {
                if state.depth > 0 && n - state.at >= if begun { 1 } else { 3 } {
                    let rest: String = chars[state.at..].iter().collect();
                    let mut base = if begun {
                        self.complete_n(&rest, 256)
                    } else {
                        self.suggest(&rest)
                    };
                    base.extend(self.derived_parts(&rest, begun));
                    base.sort_by(|a, b| a.cost.total_cmp(&b.cost));
                    for mut c in base.into_iter().take(if begun { 256 } else { 8 }) {
                        let base_len = rest.chars().count();
                        if begun && c.word.chars().count() <= base_len {
                            continue;
                        }
                        if !begun && c.edit > 0.85 * base_len.max(4) as f32 {
                            continue;
                        }
                        if !begun && c.word.chars().next() != rest.chars().next() {
                            continue;
                        }
                        if begun
                            && self.readings(&c.word).iter().any(|r| {
                                r & (crate::gram::bit::ADJF
                                    | crate::gram::bit::ADVB
                                    | crate::gram::bit::PRTF)
                                    != 0
                            })
                        {
                            c.cost -= 4.0;
                        }
                        out.push(Candidate {
                            word: format!("{}{}", state.word, c.word),
                            cost: state.cost + c.cost,
                            edit: state.edit + c.edit,
                        });
                    }
                }
                if state.depth == MAX_PREFIXES || state.at >= n {
                    continue;
                }
                let left = n - state.at - if begun { 1 } else { 3 };
                for len in 2..=8.min(left) {
                    let part: String = chars[state.at..state.at + len].iter().collect();
                    for c in self.prefix_parts(&part, !begun) {
                        let mut joined = join(&state, &c, &chars[state.at..state.at + len]);
                        joined.depth = state.depth + 1;
                        next.push(joined);
                    }
                }
            }
            next.sort_by(|a, b| a.cost.total_cmp(&b.cost));
            let mut seen = HashSet::new();
            next.retain(|s| seen.insert((s.at, s.word.clone())));
            next.truncate(MAX_STATES);
            if next.is_empty() {
                break;
            }
            states = next;
        }
        out.retain(|c| !begun || c.word.starts_with(raw));
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
        let mut seen = HashSet::new();
        out.retain(|c| seen.insert(c.word.clone()));
        out.truncate(32);
        out
    }

    pub(crate) fn productive(&self, evidence: Evidence) -> Vec<Candidate> {
        let (raw, begun) = match evidence {
            Evidence::Taps(raw, _) => (raw, false),
            Evidence::Begun(raw) => (raw, true),
            _ => return Vec::new(),
        };
        let cache = if begun {
            &self.productive_begun
        } else {
            &self.productive_taps
        };
        if cache.borrow().0 == raw {
            return cache.borrow().1.clone();
        }
        let out = self.productive_parts(raw, begun);
        *cache.borrow_mut() = (raw.to_string(), out.clone());
        out
    }

    /// The dictionary root of a generated compound and its prior, for showing
    /// more than one form of the same derivation in the suggestion strip.
    pub fn productive_root(&self, word: &str) -> Option<(String, f32)> {
        let mut rest = word;
        for _ in 0..=MAX_PREFIXES {
            if let Some(cost) = self.word_cost(rest).filter(|_| self.contains(rest)) {
                return Some((rest.to_string(), cost));
            }
            let suffix = PREFIXES.iter().find_map(|p| rest.strip_prefix(p))?;
            rest = suffix;
        }
        None
    }

    pub(crate) fn merge_productive(
        &self,
        ordinary: Vec<Candidate>,
        generated: Vec<Candidate>,
        evidence: Evidence,
    ) -> Vec<Candidate> {
        let raw = match evidence {
            Evidence::Taps(raw, _) | Evidence::Begun(raw) => Some(raw),
            _ => None,
        };
        let mut out: Vec<Candidate> = ordinary.iter().take(4).cloned().collect();
        if let Some(exact) = raw.and_then(|raw| {
            generated
                .iter()
                .find(|c| c.word == raw && !self.contains(&c.word))
        }) {
            if !out.iter().any(|c| c.word == exact.word) {
                out.push(exact.clone());
            }
        }
        let mut all = ordinary;
        all.extend(generated);
        all.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
        let limit = if matches!(evidence, Evidence::Begun(_)) {
            self.config().top_k * 2
        } else {
            self.config().top_k
        };
        for c in all {
            if out.len() == limit {
                break;
            }
            if !out.iter().any(|o| o.word == c.word) {
                out.push(c);
            }
        }
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
        out
    }

    pub(crate) fn extend_productive(&self, out: &mut Vec<Candidate>, evidence: Evidence) {
        let generated = self.productive(evidence);
        *out = self.merge_productive(std::mem::take(out), generated, evidence);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Latin, Profile};

    fn engine(words: &[(&str, u64)]) -> Engine<Vec<u8>> {
        let mut words: Vec<(Vec<u8>, u64)> = words
            .iter()
            .map(|(w, n)| {
                (
                    w.chars()
                        .map(|c| crate::alphabet::char_to_id(c).unwrap())
                        .collect(),
                    *n,
                )
            })
            .collect();
        words.sort();
        let map = fst::Map::from_iter(words.iter().map(|(w, n)| (w.as_slice(), *n))).unwrap();
        Engine::new(
            map,
            Config {
                home_row: false,
                layout_flip: false,
                latin: Latin::Qwerty,
                ..Config::balanced().with_profile(Profile::Phone)
            },
        )
    }

    #[test]
    fn reads_prefixes_and_a_misspelled_base_separately() {
        let e = engine(&[
            ("свер", 100),
            ("сверх", 500),
            ("мега", 500),
            ("меша", 80),
            ("продуктивный", 500),
            ("сверхрегенеративный", 500),
            ("защищенный", 500),
        ]);
        let got = e.productive_parts("сверзмешапродктивныы", false);
        assert_eq!(
            got.first().map(|c| c.word.as_str()),
            Some("сверхмегапродуктивный"),
            "{got:?}"
        );
        let got = e.productive_parts("сверзсупермешаультразащищеный", false);
        assert!(
            got.iter()
                .any(|c| c.word == "сверхсупермегаультразащищенный"),
            "{got:?}"
        );
    }

    #[test]
    fn completes_an_exact_productive_prefix() {
        let e = engine(&[
            ("сверх", 500),
            ("широко", 1000),
            ("широкий", 900),
            ("широкоформатный", 100),
            ("школа", 800),
        ]);
        let got = e.productive_parts("сверхш", true);
        assert!(got.iter().any(|c| c.word == "сверхшироко"), "{got:?}");
        assert!(got.iter().any(|c| c.word == "сверхширокий"), "{got:?}");
        assert!(
            got.iter().any(|c| c.word == "сверхширокоформатный"),
            "{got:?}"
        );
    }

    #[test]
    fn derives_and_completes_suffixes_of_a_known_root() {
        let e = engine(&[("девайс", 500), ("другой", 1000)]);
        let got = e.productive_parts("девайснутыы", false);
        assert!(got.iter().any(|c| c.word == "девайснутый"), "{got:?}");
        let got = e.productive_parts("девайснутый", false);
        assert!(got.iter().any(|c| c.word == "девайснутый"), "{got:?}");
        let got = e.productive_parts("девайсн", true);
        assert!(got.iter().any(|c| c.word == "девайснутый"), "{got:?}");
        assert!(got.iter().any(|c| c.word == "девайснуть"), "{got:?}");
    }

    #[test]
    fn joins_an_adjective_stem_and_a_dictionary_root() {
        let e = engine(&[("кривошейный", 500), ("зерновой", 500)]);
        assert!(e.contains("кривошейный") && e.contains("зерновой"));
        let got = e.compound_parts("кривошейнозерновой");
        assert!(
            got.iter().any(|c| c.word == "кривошейнозерновой"),
            "{got:?}"
        );
        let got = e.productive_parts("кривошейнозернвой", false);
        assert!(
            !got.iter().any(|c| c.word == "кривошейнозеревой"),
            "{got:?}"
        );
    }

    #[test]
    fn builds_an_adjective_from_an_adjective_and_a_noun() {
        let e = engine(&[("кривой", 500), ("шея", 500)]);
        assert!(e.contains("кривой") && e.contains("шея"));
        let got = e.compound_adjective_parts("кривошейный");
        assert!(got.iter().any(|c| c.word == "кривошейный"), "{got:?}");
    }

    #[test]
    fn recognizes_a_borrowed_adjective_and_adverb() {
        let e = engine(&[("volatile", 500), ("кривой", 500), ("шея", 500)]);
        assert!(e.loan_derivation("волатильный"));
        assert!(e.is_combining_compound("Волатильно-кривошейный"));
        assert!(e.loan_derivation("волатильно"));
        assert!(!e.loan_derivation("фывапрол"));
    }

    #[test]
    fn random_text_has_no_productive_reading() {
        let e = engine(&[("сверх", 500), ("мега", 500), ("продуктивный", 500)]);
        assert!(e.productive_parts("фывапролджэ", false).is_empty());
        assert!(e.productive_parts("сверхфывапрол", false).is_empty());
        let e = engine(&[("полу", 500), ("получил", 500), ("сил", 500)]);
        assert!(!e
            .productive_parts("получил", false)
            .iter()
            .any(|c| c.word == "полусил"));
    }
}
