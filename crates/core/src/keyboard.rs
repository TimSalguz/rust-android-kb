//! Keyboard geometry → precomputed, id-indexed cost tables.
//!
//! Each key has a center in key-width units. Substitution cost between two keys
//! comes from a Gaussian touch model: aiming at key `i` and landing on `j` has
//! probability ∝ exp(−d²/2σ²), so `C_sub = d²/(2σ²) + base_sub` (capped at
//! `max_sub`). Confusions geometry can't see — phonetic pairs, е↔ё, ь↔ъ — take
//! the minimum with their own cost.
//!
//! Everything here is derived from a [`Config`] once, when the engine is built
//! or its config/layout changes, so the search hot path is a single array load.
//!
//! QWERTY and ЙЦУКЕН share physical positions on a hardware keyboard; φ maps
//! between them by position, which powers wrong-layout correction
//! (`ghbdtn`→`привет`).

use crate::alphabet::{self, char_to_id};
use crate::config::{Config, Latin, Profile};

/// Width of the id-indexed tables: ids run `1..=CHARSET.len()`, 0 = unsupported char.
pub const N: usize = alphabet::CHARSET.len() + 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Latin,
    Cyrillic,
}

const Q_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const C_ROWS_DESKTOP: [&str; 3] = ["йцукенгшщзхъ", "фывапролджэ", "ячсмитьбю"];
// Phone ЙЦУКЕН: 11 keys per row, ъ lives on a long-press of ь.
const C_ROWS_PHONE: [&str; 3] = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"];

// Desktop row offsets (ANSI stagger), in key widths.
const DESKTOP_OFFSETS: [f32; 3] = [0.5, 0.75, 1.25];
// Phone row offsets, in that layout's key widths (Gboard-like): ЙЦУКЕН's
// bottom row sits right of a 1-wide shift key (the Latin layouts' offsets are
// `Latin::phone_offsets`).
const PHONE_C_OFFSETS: [f32; 3] = [0.0, 0.0, 1.0];
/// Phone keys are taller than wide: row height in QWERTY key widths.
const PHONE_ROW_HEIGHT: f32 = 1.35;
/// ЙЦУКЕН has 11 keys across the width QWERTY spends on 10.
const PHONE_C_KEY_WIDTH: f32 = 10.0 / 11.0;

/// Touch-typing finger (0 = left pinky … 7 = right pinky), aligned with the
/// desktop row strings above.
const Q_FINGERS: [&[u8]; 3] = [
    &[0, 1, 2, 3, 3, 4, 4, 5, 6, 7],
    &[0, 1, 2, 3, 3, 4, 4, 5, 6],
    &[0, 1, 2, 3, 3, 4, 4],
];
const C_FINGERS: [&[u8]; 3] = [
    &[0, 1, 2, 3, 3, 4, 4, 5, 6, 7, 7, 7],
    &[0, 1, 2, 3, 3, 4, 4, 5, 6, 7, 7],
    &[0, 1, 2, 3, 3, 4, 4, 5, 6],
];

/// Keys within this squared distance (key widths²) can both be hit by one press.
const NEIGHBOR_D2: f32 = 2.25;
/// A finger held down (not fully lifted between letters) can roll onto keys
/// this close — diagonals included, even with a phone's tall keys.
const HOLD_REACH_D2: f32 = 3.5;

/// Pairs confusable by sound or spelling rather than by key position.
const PHONETIC: &[(char, char)] = &[
    // Unstressed vowels, и/ы after ц, э/е, ё after sibilants (пришол), ю/у (брошюра).
    ('о', 'а'),
    ('е', 'и'),
    ('е', 'я'),
    ('и', 'ы'),
    ('е', 'э'),
    ('о', 'ё'),
    ('ю', 'у'),
    // Voiced / voiceless consonants.
    ('б', 'п'),
    ('в', 'ф'),
    ('г', 'к'),
    ('д', 'т'),
    ('ж', 'ш'),
    ('з', 'с'),
    // English vowel and c/k/s/z spellings.
    ('a', 'e'),
    ('e', 'i'),
    ('i', 'y'),
    ('c', 'k'),
    ('c', 's'),
    ('s', 'z'),
];

const HOME_ROW: usize = 1;

/// Accented letters and the letter they are typed as without the accent.
const ACCENTS: &[(char, char)] = &[
    ('à', 'a'),
    ('á', 'a'),
    ('â', 'a'),
    ('ã', 'a'),
    ('ä', 'a'),
    ('å', 'a'),
    ('æ', 'a'),
    ('ç', 'c'),
    ('è', 'e'),
    ('é', 'e'),
    ('ê', 'e'),
    ('ë', 'e'),
    ('ì', 'i'),
    ('í', 'i'),
    ('î', 'i'),
    ('ï', 'i'),
    ('ñ', 'n'),
    ('ò', 'o'),
    ('ó', 'o'),
    ('ô', 'o'),
    ('õ', 'o'),
    ('ö', 'o'),
    ('ø', 'o'),
    ('œ', 'o'),
    ('ù', 'u'),
    ('ú', 'u'),
    ('û', 'u'),
    ('ü', 'u'),
    ('ý', 'y'),
    ('ÿ', 'y'),
    ('ß', 's'),
];

#[derive(Clone, Copy)]
struct Key {
    x: f32,
    y: f32,
    layout: Layout,
    row: usize,
    finger: u8,
}

pub struct Keyboard {
    /// `sub[typed * N + intended]`: cost of typing `typed` when `intended` was meant.
    sub: Vec<f32>,
    /// Same, under the home-row hypothesis (a typed home-row key stands for any
    /// key of its finger on desktop / its column on a phone).
    home: Vec<f32>,
    /// Only the confusions geometry can't see (phonetic, ё, ь/ъ); ∞ elsewhere.
    confusion: Vec<f32>,
    /// `neighbor[a * N + b]`: one press can plausibly hit both keys.
    neighbor: Vec<bool>,
    /// `reach[a * N + b]`: a held press on `a` can also have produced `b`.
    reach: Vec<bool>,
    /// Cost of a missing (deleted) word char, before the doubled-letter rule.
    del: [f32; N],
    /// Cost of an extra (inserted) typed char, before the context rules.
    ins: [f32; N],
    home_row: [bool; N],
    /// Which hand types each key: 0 left, 1 right, 2 unknown.
    hand: [u8; N],
    /// φ: the id at the same physical position in the other layout (0 = none).
    flip: [u8; N],
    /// A letter people type as two (œ as oe, æ as ae, ß as ss): those two
    /// ids (0 = none).
    digraph: [(u8, u8); N],
    /// Cost of a substitution with no shared geometry (cross-alphabet / unknown key).
    far: f32,
}

fn id(c: char) -> usize {
    char_to_id(c).expect("keyboard char outside the alphabet") as usize
}

fn place(
    keys: &mut [Option<Key>; N],
    rows: &[&str; 3],
    fingers: &[&[u8]; 3],
    layout: Layout,
    pos: impl Fn(usize, usize) -> (f32, f32),
) {
    for (r, row) in rows.iter().enumerate() {
        for (i, c) in row.chars().enumerate() {
            let (x, y) = pos(r, i);
            keys[id(c)] = Some(Key {
                x,
                y,
                layout,
                row: r,
                finger: fingers[r].get(i).copied().unwrap_or(0),
            });
        }
    }
}

/// Key centers for a profile and Latin layout, in QWERTY key widths.
fn geometry(profile: Profile, latin: Latin) -> [Option<Key>; N] {
    let mut keys = [None; N];
    let rows = latin.rows();
    match profile {
        Profile::Desktop => {
            let pos = |r: usize, i: usize| (DESKTOP_OFFSETS[r] + i as f32, r as f32);
            place(&mut keys, &rows, &Q_FINGERS, Layout::Latin, pos);
            place(
                &mut keys,
                &C_ROWS_DESKTOP,
                &C_FINGERS,
                Layout::Cyrillic,
                pos,
            );
            // ё sits on the backtick key, left of the number row.
            keys[id('ё')] = Some(Key {
                x: 0.0,
                y: -1.0,
                layout: Layout::Cyrillic,
                row: 3,
                finger: 0,
            });
        }
        Profile::Phone => {
            let y = |r: usize| (r as f32 + 0.5) * PHONE_ROW_HEIGHT;
            // Layouts with 11 keys a row (QWERTZ) have narrower keys.
            let widest = rows.iter().map(|r| r.chars().count()).max().unwrap_or(10);
            let unit = 10.0 / widest as f32;
            let offsets = latin.phone_offsets();
            place(&mut keys, &rows, &Q_FINGERS, Layout::Latin, |r, i| {
                ((offsets[r] + i as f32 + 0.5) * unit, y(r))
            });
            place(
                &mut keys,
                &C_ROWS_PHONE,
                &C_FINGERS,
                Layout::Cyrillic,
                |r, i| {
                    (
                        (PHONE_C_OFFSETS[r] + i as f32 + 0.5) * PHONE_C_KEY_WIDTH,
                        y(r),
                    )
                },
            );
            // Long-press keys: pressed on their base key, but never "typed by
            // accident" from the home row.
            keys[id('ё')] = keys[id('е')].map(|k| Key { row: 3, ..k });
            keys[id('ъ')] = keys[id('ь')].map(|k| Key { row: 3, ..k });
        }
    }
    // Accented letters without a key of their own: a long press on their
    // letter's key (é on e).
    for &(accented, base) in ACCENTS {
        if keys[id(accented)].is_none() {
            keys[id(accented)] = keys[id(base)].map(|k| Key { row: 3, ..k });
        }
    }
    keys
}

impl Keyboard {
    /// Build all cost tables for `cfg.profile`. Call again only when the layout
    /// or the config changes.
    pub fn new(cfg: &Config) -> Self {
        let keys = geometry(cfg.profile, cfg.latin);
        let two_sigma2 = 2.0 * cfg.sigma * cfg.sigma;
        let far = cfg.base_sub + cfg.cross_alphabet_penalty;
        let geo = |d2: f32| (d2 / two_sigma2).min(cfg.max_sub) + cfg.base_sub;

        let mut sub = vec![far; N * N];
        let mut home = vec![far; N * N];
        let mut neighbor = vec![false; N * N];
        let mut reach = vec![false; N * N];
        let mut home_row = [false; N];
        let mut hand = [2u8; N];
        // Two-thumb typing on a phone splits the keyboard down the middle;
        // touch typing splits it by finger.
        let middle = |k: &Key| match (cfg.profile, k.layout) {
            (Profile::Phone, Layout::Latin) => 5.0,
            (Profile::Phone, Layout::Cyrillic) => 5.5 * PHONE_C_KEY_WIDTH,
            (Profile::Desktop, _) => f32::NAN,
        };
        for a in 0..N {
            if let Some(ka) = keys[a] {
                home_row[a] = ka.row == HOME_ROW;
                hand[a] = match cfg.profile {
                    Profile::Desktop => (ka.finger >= 4) as u8,
                    Profile::Phone => (ka.x >= middle(&ka)) as u8,
                };
            }
            for b in 0..N {
                let i = a * N + b;
                if a == b {
                    sub[i] = 0.0;
                    home[i] = 0.0;
                    continue;
                }
                let (Some(ka), Some(kb)) = (keys[a], keys[b]) else {
                    continue;
                };
                if ka.layout != kb.layout {
                    continue;
                }
                let (dx, dy) = (ka.x - kb.x, ka.y - kb.y);
                let d2 = dx * dx + dy * dy;
                sub[i] = geo(d2);
                neighbor[i] = d2 <= NEIGHBOR_D2;
                reach[i] = d2 <= HOLD_REACH_D2;
                home[i] = sub[i];
                let typed_row_ok =
                    ka.row == HOME_ROW || (cfg.profile == Profile::Phone && ka.row < 3);
                if typed_row_ok && kb.row < 3 {
                    let reach = match cfg.profile {
                        // Hands never leave the home row: the key stands for
                        // every key its finger covers.
                        Profile::Desktop if ka.finger == kb.finger => cfg.c_home_finger,
                        Profile::Desktop => f32::INFINITY,
                        // One-row keyboard: the key stands for its column —
                        // only the horizontal position counts.
                        Profile::Phone => {
                            cfg.c_home_finger + (dx * dx / two_sigma2).min(cfg.max_sub)
                        }
                    };
                    home[i] = home[i].min(reach);
                }
            }
        }

        // Confusions that geometry can't see.
        let mut confusion = vec![f32::INFINITY; N * N];
        let mut cheaper = |x: char, y: char, cost: f32| {
            let (a, b) = (id(x), id(y));
            for i in [a * N + b, b * N + a] {
                sub[i] = sub[i].min(cost);
                home[i] = home[i].min(cost);
                confusion[i] = confusion[i].min(cost);
            }
        };
        for &(x, y) in PHONETIC {
            cheaper(x, y, cfg.c_phonetic);
        }
        cheaper('ь', 'ъ', cfg.c_hard_soft);
        // е/ё is directional: skipping ё is routine, typing it by mistake is not.
        let (e, yo) = (id('е'), id('ё'));
        for (typed, intended, cost) in [(e, yo, cfg.c_yo), (yo, e, cfg.c_yo_reverse)] {
            let i = typed * N + intended;
            sub[i] = sub[i].min(cost);
            home[i] = home[i].min(cost);
            confusion[i] = confusion[i].min(cost);
        }
        // So is an accent: left out routinely (e for é, n for ñ). Typed
        // where none or another one was meant, it was chosen — a long press
        // on the same key, which geometry alone would call free.
        for &(x, bx) in ACCENTS {
            let (a, b) = (id(x), id(bx));
            let long_press = keys[a].is_some_and(|k| k.row == 3);
            let mut set = |typed: usize, intended: usize, cost: f32, force: bool| {
                let i = typed * N + intended;
                for t in [&mut sub, &mut home, &mut confusion] {
                    t[i] = if force { cost } else { t[i].min(cost) };
                }
            };
            set(b, a, cfg.c_accent, false);
            set(a, b, cfg.c_accent_reverse, long_press);
            for &(y, by) in ACCENTS {
                if by == bx && y != x {
                    set(a, id(y), cfg.c_accent_reverse, long_press);
                }
            }
        }

        let mut del = [cfg.c_del; N];
        let mut ins = [cfg.c_ins; N];
        for c in ['ь', 'ъ'] {
            del[id(c)] = cfg.c_del_sign;
            ins[id(c)] = cfg.c_ins_sign;
        }
        for c in ['\'', '-'] {
            del[id(c)] = cfg.c_del_punct;
        }

        // φ pairs keys by desktop position (row + column).
        let mut flip = [0u8; N];
        for (q, c) in Q_ROWS.iter().zip(C_ROWS_DESKTOP.iter()) {
            for (l, k) in q.chars().zip(c.chars()) {
                flip[id(l)] = id(k) as u8;
                flip[id(k)] = id(l) as u8;
            }
        }

        let mut digraph = [(0u8, 0u8); N];
        for (letter, a, b) in [('œ', 'o', 'e'), ('æ', 'a', 'e'), ('ß', 's', 's')] {
            digraph[id(letter)] = (id(a) as u8, id(b) as u8);
        }

        Keyboard {
            sub,
            home,
            confusion,
            neighbor,
            reach,
            del,
            ins,
            home_row,
            hand,
            flip,
            digraph,
            far,
        }
    }

    /// Cost of typing `typed` when `intended` was meant (ids), normal or under
    /// the home-row hypothesis.
    #[inline]
    pub fn cost(&self, home_hyp: bool, typed: u8, intended: u8) -> f32 {
        let i = typed as usize * N + intended as usize;
        if home_hyp {
            self.home[i]
        } else {
            self.sub[i]
        }
    }

    /// Cost of the non-geometric confusions only (phonetic, ё, ь/ъ); ∞ if none.
    #[inline]
    pub fn confusion(&self, typed: u8, intended: u8) -> f32 {
        self.confusion[typed as usize * N + intended as usize]
    }

    /// [`Keyboard::cost`] for chars; unsupported chars get the far cost.
    pub fn sub_cost(&self, typed: char, intended: char) -> f32 {
        if typed == intended {
            return 0.0;
        }
        match (char_to_id(typed), char_to_id(intended)) {
            (Some(a), Some(b)) => self.cost(false, a, b),
            _ => self.far,
        }
    }

    #[inline]
    pub fn is_neighbor(&self, a: u8, b: u8) -> bool {
        self.neighbor[a as usize * N + b as usize]
    }

    /// A held press on `held` may also have typed `c` (same key or within reach).
    #[inline]
    pub fn in_reach(&self, held: u8, c: u8) -> bool {
        held == c || self.reach[held as usize * N + c as usize]
    }

    /// The two keys are typed by different hands.
    #[inline]
    pub fn cross_hand(&self, a: u8, b: u8) -> bool {
        let (ha, hb) = (self.hand[a as usize], self.hand[b as usize]);
        ha != 2 && hb != 2 && ha != hb
    }

    #[inline]
    pub fn is_home_row(&self, id: u8) -> bool {
        self.home_row[id as usize]
    }

    /// Cost of the word char `c` missing from the input.
    #[inline]
    pub fn del_cost(&self, c: u8) -> f32 {
        self.del[c as usize]
    }

    /// Cost of the typed char `c` being an extra keypress (no context).
    #[inline]
    pub fn ins_cost(&self, c: u8) -> f32 {
        self.ins[c as usize]
    }

    /// The two letters `c` is typed as when it isn't on the keyboard (œ: o e).
    #[inline]
    pub fn digraph(&self, c: u8) -> Option<(u8, u8)> {
        let d = self.digraph[c as usize];
        (d.0 != 0).then_some(d)
    }

    /// Reinterpret ids under the other layout (φ); keys without a counterpart
    /// stay unchanged.
    pub fn flip_ids(&self, input: &[u8]) -> Vec<u8> {
        input
            .iter()
            .map(|&c| {
                if self.flip[c as usize] != 0 {
                    self.flip[c as usize]
                } else {
                    c
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb(profile: Profile) -> Keyboard {
        Keyboard::new(&Config::default().with_profile(profile))
    }

    #[test]
    fn neighbor_cheaper_than_far() {
        let kb = kb(Profile::Desktop);
        // On QWERTY, 's' is adjacent to 'a'; 'p' is far.
        assert!(kb.sub_cost('s', 'a') < kb.sub_cost('p', 'a'));
    }

    #[test]
    fn phone_rows_are_farther_than_columns() {
        // Phone keys are taller than wide: q→w (sideways) beats q→a (down).
        let kb = kb(Profile::Phone);
        assert!(kb.sub_cost('q', 'w') < kb.sub_cost('q', 'a'));
    }

    #[test]
    fn non_geometric_confusions_are_cheap() {
        let kb = kb(Profile::Desktop);
        let cfg = Config::default();
        assert!(kb.sub_cost('е', 'ё') <= cfg.c_yo);
        assert!(kb.sub_cost('а', 'о') <= cfg.c_phonetic);
        assert!(kb.sub_cost('д', 'т') <= cfg.c_phonetic);
    }

    #[test]
    fn accents_left_out_are_almost_free() {
        let kb = kb(Profile::Phone);
        assert!(kb.sub_cost('e', 'é') < 0.5);
        assert!(kb.sub_cost('n', 'ñ') < 0.5);
        assert!(kb.sub_cost('é', 'e') > 1.0, "an accent typed is a choice");
        assert!(kb.sub_cost('é', 'è') > 1.0);
    }

    #[test]
    fn each_language_has_its_own_latin_layout() {
        let qwertz = Keyboard::new(&Config {
            latin: Latin::Qwertz,
            ..Config::balanced().with_profile(Profile::Phone)
        });
        let id = |c| char_to_id(c).unwrap();
        // z sits between t and u on QWERTZ, next to a and x on QWERTY.
        assert!(qwertz.is_neighbor(id('z'), id('u')));
        assert!(!qwertz.is_neighbor(id('z'), id('a')));
        assert!(!qwertz.is_neighbor(id('z'), id('s')));
        assert!(kb(Profile::Phone).is_neighbor(id('z'), id('s')));
        assert!(qwertz.is_neighbor(id('ü'), id('p')), "ü has its own key");
        let azerty = Keyboard::new(&Config {
            latin: Latin::Azerty,
            ..Config::balanced().with_profile(Profile::Phone)
        });
        assert!(azerty.is_neighbor(id('a'), id('z')));
        assert!(azerty.is_neighbor(id('m'), id('l')));
    }

    #[test]
    fn cross_layout_roundtrip() {
        let kb = kb(Profile::Desktop);
        let word: Vec<u8> = "ghbdtn".chars().map(|c| char_to_id(c).unwrap()).collect();
        assert_eq!(alphabet::ids_to_string(&kb.flip_ids(&word)), "привет");
    }

    #[test]
    fn home_row_finger_reach() {
        // Left index finger covers к/е/а/п/м/и on desktop ЙЦУКЕН.
        let kb = kb(Profile::Desktop);
        let (a, k) = (char_to_id('а').unwrap(), char_to_id('к').unwrap());
        assert!(kb.is_home_row(a));
        assert!(kb.cost(true, a, k) < kb.cost(false, a, k));
    }
}
