//! The correction engine: a weighted Damerau-Levenshtein DP carried along a
//! depth-first walk of the mmap'd FST. Each FST node holds a DP row over the
//! input; costs come from [`Config`] + keyboard geometry.
//!
//! A branch is pruned when its lower bound exceeds the current bound: the row
//! minimum (all edit costs are non-negative) plus the smallest prior in the
//! subtree — the FST output accumulated so far, since outputs only add up.
//! Children are visited ordered by how well they continue the input (and by
//! frequency), so the walk dives along the intended word and reaches deep
//! matches before the node cap. The budget starts low and is **widened** while
//! close matches are scarce — this reconsiders the branches pruned at a tighter
//! budget, surfacing heavily-misspelled words only when nothing closer exists,
//! so the common case stays fast and precise.
//!
//! The input is read under several hypotheses (as typed, wrong layout, hands on
//! the home row), each with its own precomputed cost table and penalty.

use std::collections::{BinaryHeap, HashMap};
use std::fs::File;
use std::io;
use std::path::Path;

use fst::raw::{CompiledAddr, Fst, Node};
use fst::Map;
use memmap2::Mmap;

use crate::alphabet;
use crate::chooser::Chooser;
use crate::config::Config;
use crate::dict::DictFormat;
use crate::keyboard::{Keyboard, N};
use crate::lemmas::Lemmas;
use crate::store::Store;

#[derive(Clone, Debug)]
pub struct Candidate {
    pub word: String,
    /// Total cost: `channel + w_lm·(−log P(word))` (lower = more likely).
    pub cost: f32,
    /// The channel part alone: how much typing error (or untyped completion)
    /// this candidate assumes, hypothesis penalty included — independent of
    /// how rare the word is.
    pub edit: f32,
}

/// What the keyboard observed about how one input char was pressed. Letters
/// such a press may have swallowed are cheap to be missing next to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hint {
    /// Held still noticeably longer than the user's rhythm: the same letter
    /// again or a key within reach may have been meant too (`сс`, `мас`).
    pub held: bool,
    /// Keys the finger slid across, without lifting, before settling on this one.
    pub slid: Vec<char>,
    /// Pressed almost together with the previous letter (fingers overlapped,
    /// or within a roll-over window): the two may have registered swapped.
    pub rollover: bool,
    /// Substitution costs measured from the actual touch point: `(letter,
    /// cost)` for keys around the tap. A tap on the border of two keys makes
    /// both cheap; a tap dead-center makes the neighbors expensive. Letters
    /// not listed keep the engine's key-to-key geometry.
    pub spatial: Vec<(char, f32)>,
    /// The user saw this letter and kept it while editing the word: any edit
    /// that changes it (replace, drop, swap, a letter slipped in next to it)
    /// costs this much more. 0: an ordinary press.
    pub kept: f32,
}

/// Work done by one [`Engine::suggest_with_stats`] call.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// FST nodes visited across all hypotheses and widening passes.
    pub nodes: usize,
}

fn row_min(row: &[f32]) -> f32 {
    row.iter().copied().fold(f32::INFINITY, f32::min)
}

/// Where a node's words lie: the output gathered on the way to it, and —
/// in a ranked dictionary ([`DictFormat::ranked`]) — the rank just past its
/// last word (its first word's rank is the output's).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Span {
    pub(crate) acc: u64,
    pub(crate) hi: u64,
    /// A quick lower bound of its words' priors ([`Store::bound`]); the
    /// search asks for a tighter one ([`Priors::least`]) only where this
    /// doesn't prune.
    pub(crate) least: u64,
}

/// How a dictionary's priors read: from the outputs of its automaton, or
/// by rank from a [`Store`].
pub(crate) enum Priors<'a, D: AsRef<[u8]>> {
    Outputs(DictFormat),
    Ranked(DictFormat, &'a Store<D>),
}

impl<D: AsRef<[u8]>> Clone for Priors<'_, D> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<D: AsRef<[u8]>> Copy for Priors<'_, D> {}

impl<D: AsRef<[u8]>> Priors<'_, D> {
    /// Where all the words lie.
    pub(crate) fn root(&self) -> Span {
        match self {
            Priors::Outputs(_) => Span::default(),
            Priors::Ranked(_, store) => Span {
                acc: 0,
                hi: store.len() as u64,
                least: store.bound(0, store.len() as u64),
            },
        }
    }

    /// Where the words of `node`'s `i`-th child lie (`span` the node's).
    pub(crate) fn child(&self, node: &Node, i: usize, span: Span) -> Span {
        let acc = span.acc + node.transition(i).out.value();
        let next = match self {
            Priors::Ranked(..) if i + 1 < node.len() => {
                Some(span.acc + node.transition(i + 1).out.value())
            }
            _ => None,
        };
        self.span(acc, next, span)
    }

    /// The span of a child whose output so far is `acc`, the next child's
    /// `next` (None: it is the last), `parent` its parent's.
    pub(crate) fn span(&self, acc: u64, next: Option<u64>, parent: Span) -> Span {
        match self {
            Priors::Outputs(f) => Span {
                acc,
                hi: 0,
                least: f.prior(acc),
            },
            Priors::Ranked(f, store) => {
                let hi = next.map_or(parent.hi, |v| f.rank(v));
                Span {
                    acc,
                    hi,
                    least: store.bound(f.rank(acc), hi),
                }
            }
        }
    }

    /// A lower bound of the priors of the words in `span` (thousandths of a
    /// nat).
    pub(crate) fn least(&self, span: Span) -> u64 {
        match self {
            Priors::Outputs(_) => span.least,
            Priors::Ranked(f, store) => store.least(f.rank(span.acc), span.hi),
        }
    }

    /// Whether [`Priors::least`] may be tighter than the quick bound.
    pub(crate) fn refines(&self) -> bool {
        matches!(self, Priors::Ranked(..))
    }

    /// The prior of a word by its value (thousandths of a nat).
    pub(crate) fn word(&self, v: u64) -> u64 {
        match self {
            Priors::Outputs(f) => f.prior(v),
            Priors::Ranked(f, store) => store.prior(f.rank(v)),
        }
    }
}

/// Best-first completion frontier entry (a min-heap on `cost`).
struct Frontier {
    cost: f32,
    span: Span,
    path: Vec<u8>,
    addr: CompiledAddr,
    /// A completion's walk: the previous word's pairs down the same letters
    /// (the context model's node and outputs), and whether this is a word
    /// found, at its exact cost, rather than a node to go on from.
    pairs: Option<(CompiledAddr, u64)>,
    found: bool,
}

impl PartialEq for Frontier {
    fn eq(&self, o: &Self) -> bool {
        self.cost.total_cmp(&o.cost).is_eq()
    }
}
impl Eq for Frontier {}
impl PartialOrd for Frontier {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Frontier {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

/// One reading of the input with its per-query cost tables, so the DP inner
/// loop is pure array loads.
struct Query {
    input: Vec<u8>,
    /// `sub[c * m + j]`: cost of word char `c` aligned with input char `j`.
    sub: Vec<f32>,
    /// `ins[j]`: cost of input char `j` being an extra keypress.
    ins: Vec<f32>,
    /// `trans[j]` (j ≥ 2): cost of input chars `j−2`, `j−1` being swapped.
    trans: Vec<f32>,
    penalty: f32,
    /// Cap on this hypothesis' edit budget (no widening past it).
    max_budget: f32,
    /// `absorb[c * (m + 1) + j]`: word char `c` may be missing at input column
    /// `j` because an adjacent press swallowed it (see [`Hint`]). Empty: none.
    absorb: Vec<bool>,
    /// `kept_del[j]`: extra cost of a word char missing at input column `j`
    /// between two kept letters (see [`Hint::kept`]). Empty: none kept.
    kept_del: Vec<f32>,
    /// The ids of т, ь, с: ь between т and ся is a spelling slip of its own.
    tsya: Option<(u8, u8, u8)>,
}

impl Query {
    #[allow(clippy::too_many_arguments)]
    fn new(
        kb: &Keyboard,
        cfg: &Config,
        input: Vec<u8>,
        hints: &[Hint],
        home_hyp: bool,
        penalty: f32,
        max_budget: f32,
        taps: bool,
    ) -> Self {
        let m = input.len();
        let mut sub = vec![0.0f32; N * m];
        for c in 1..N {
            for (j, &x) in input.iter().enumerate() {
                sub[c * m + j] = kb.cost(home_hyp, x, c as u8);
            }
        }
        // Touch points refine the reading as typed (other hypotheses
        // reinterpret the keys, so the taps don't describe them).
        if taps {
            for (j, hint) in hints.iter().enumerate().take(m) {
                for &(ch, cost) in &hint.spatial {
                    let Some(c) = alphabet::char_to_id(ch.to_lowercase().next().unwrap_or(ch))
                    else {
                        continue;
                    };
                    if c != input[j] {
                        sub[c as usize * m + j] = cost.min(kb.confusion(input[j], c));
                    }
                }
            }
        }
        // Extra keypresses are cheap when they repeat the previous key (bounce)
        // or sit next to an adjacent typed key (one press hit two keys).
        let mut ins: Vec<f32> = (0..m)
            .map(|j| {
                let x = input[j];
                let mut cost = kb.ins_cost(x);
                if j > 0 && input[j - 1] == x {
                    cost = cost.min(cfg.c_ins_repeat);
                }
                let near = |k: usize| {
                    input
                        .get(k)
                        .is_some_and(|&y| y != x && kb.is_neighbor(x, y))
                };
                if (j > 0 && near(j - 1)) || near(j + 1) {
                    cost = cost.min(cfg.c_ins_neighbor);
                }
                cost
            })
            .collect();
        // ь typed between т and ся where the word has none (учиться for
        // учится): a spelling slip, cheap.
        let tsya = alphabet::char_to_id('т')
            .zip(alphabet::char_to_id('ь'))
            .zip(alphabet::char_to_id('с'))
            .map(|((t, soft), s)| (t, soft, s));
        let ya = alphabet::char_to_id('я');
        if let (Some((t, soft, s)), Some(ya)) = (tsya, ya) {
            for j in 1..m.saturating_sub(2) {
                if input[j] == soft && input[j - 1] == t && input[j + 1] == s && input[j + 2] == ya
                {
                    ins[j] = ins[j].min(cfg.c_tsya);
                }
            }
        }
        // Letters of different hands swap more easily; nearly simultaneous
        // presses from both hands swap most easily.
        let mut trans: Vec<f32> = (0..=m)
            .map(|j| {
                if j < 2 || !kb.cross_hand(input[j - 2], input[j - 1]) {
                    cfg.c_trans
                } else if hints.get(j - 1).is_some_and(|h| h.rollover) {
                    cfg.c_trans_rollover
                } else {
                    cfg.c_trans_cross
                }
            })
            .collect();
        // Letters the user kept while editing stay as typed (е may still be ё).
        let kept: Vec<f32> = (0..m)
            .map(|j| hints.get(j).map_or(0.0, |h| h.kept))
            .collect();
        let mut kept_del = Vec::new();
        if kept.iter().any(|&k| k > 0.0) {
            for (j, &k) in kept.iter().enumerate().filter(|(_, &k)| k > 0.0) {
                let x = input[j];
                for c in 1..N {
                    if c as u8 != x && kb.cost(false, x, c as u8) > cfg.c_yo.max(cfg.c_accent) {
                        sub[c * m + j] += k;
                    }
                }
                ins[j] += k;
            }
            for j in 2..=m {
                trans[j] += kept[j - 2].max(kept[j - 1]);
            }
            kept_del = (0..=m)
                .map(|j| match j {
                    0 => kept[0],
                    j if j == m => 0.0,
                    j => kept[j - 1].min(kept[j]),
                })
                .collect();
        }
        let mut absorb = Vec::new();
        if hints.iter().any(|h| h.held || !h.slid.is_empty()) {
            let w = m + 1;
            absorb = vec![false; N * w];
            let slid: Vec<Vec<u8>> = hints
                .iter()
                .map(|h| {
                    h.slid
                        .iter()
                        .filter_map(|&c| alphabet::char_to_id(c.to_lowercase().next().unwrap_or(c)))
                        .collect()
                })
                .collect();
            for c in 1..N as u8 {
                let swallowed = |k: usize| {
                    hints
                        .get(k)
                        .is_some_and(|h| h.held && kb.in_reach(input[k], c))
                        || slid.get(k).is_some_and(|s| s.contains(&c))
                };
                for j in 0..w {
                    absorb[c as usize * w + j] =
                        (j > 0 && swallowed(j - 1)) || (j < m && swallowed(j));
                }
            }
        }
        Query {
            input,
            sub,
            ins,
            trans,
            penalty,
            max_budget,
            absorb,
            kept_del,
            tsya,
        }
    }
}

/// The k best totals over *distinct* words — the branch-and-bound bound.
struct TopK {
    k: usize,
    /// Ascending.
    v: Vec<f32>,
}

impl TopK {
    fn bound(&self) -> f32 {
        if self.v.len() >= self.k {
            self.v[self.k - 1]
        } else {
            f32::INFINITY
        }
    }

    /// A word's total improved from `old` (if it had one) to `new`.
    fn update(&mut self, old: Option<f32>, new: f32) {
        if let Some(i) = old.and_then(|o| self.v.iter().position(|&x| x == o)) {
            self.v.remove(i);
        }
        let i = self.v.partition_point(|&x| x <= new);
        self.v.insert(i, new);
        self.v.truncate(self.k);
    }
}

/// Search state shared by every hypothesis and widening pass of one query.
struct State<'c> {
    /// The place the word is read at: its costs are the context's.
    ctx: &'c Context,
    /// Best (total, channel) per word (as an id path).
    best: HashMap<Vec<u8>, (f32, f32)>,
    topk: TopK,
    nodes: usize,
    /// DP rows, one per depth, flattened with stride `m + 1`.
    rows: Vec<f32>,
    path: Vec<u8>,
}

pub struct Engine<D: AsRef<[u8]>> {
    pub(crate) map: Map<D>,
    /// How the dictionary's values read (its FST type field): the prior,
    /// and each word's grammar in a dictionary built with it.
    pub(crate) format: DictFormat,
    /// Optional context model: `previous\0word` → quantized `−log P(word |
    /// previous)`, same encoding as the dictionary (see index-builder).
    pub(crate) bigrams: Option<Map<D>>,
    /// Optional lemma vectors ([`crate::lemmas`]), each word's row under `0,
    /// 25, word` in the context model.
    pub(crate) lemmas: Option<Lemmas<D>>,
    /// Optional chooser ([`crate::chooser`]): the sentence so far read by a
    /// small transformer, in every weighing (with the lemma vectors).
    pub(crate) chooser: Option<Chooser<D>>,
    /// The priors by rank of a ranked dictionary ([`DictFormat::ranked`]).
    pub(crate) store: Option<Store<D>>,
    /// Optional casing list: lowercase word → 1 (always Capitalized) or 2 (in
    /// CAPITALS) — names, places, abbreviations (tools/proper_nouns.py).
    casing: Option<Map<D>>,
    /// Words the user added by hand, searched alongside the dictionary (a
    /// small FST built in memory).
    pub(crate) user: Option<Map<Vec<u8>>>,
    kb: Keyboard,
    pub(crate) cfg: Config,
}

/// How a word is written however it was typed (see [`Engine::casing`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Casing {
    AsTyped,
    /// Москва, Иван, I'm.
    Capital,
    /// США, МГУ, NASA.
    Upper,
}

mod commas;
mod context;
mod decode;
pub use context::Context;
pub use decode::Evidence;

/// Separator between the two words of a bigram key (ids start at 1).
const BIGRAM_SEP: u8 = 0;

/// The prior of a word the user added: as likely as a fairly common word
/// (−ln P × 1000).
const USER_PRIOR: u64 = 8_000;

/// PMI of two grammatical classes never seen together (too rare to count).
const UNSEEN_CLASS_PAIR: f32 = -1.5;
/// A word guessed with nothing typed ([`Evidence::Nothing`]): this many times
/// `top_k` down the context model's list — the grammar rules some out.
const PREDICT_POOL: usize = 4;
/// A guess the phrase weighs down this much (nats) is ruled out.
const PREDICT_RULED_OUT: f32 = -0.99;

/// Key of an ending pair in the bigram FST (tools/build_bigrams.py
/// --endings): `0, left class, 0, right class`, a class being `1` + a short
/// word itself (≤ `whole` letters: в, для) or `2` + the last two letters.
/// Word keys start with a letter id (≥ 1), so the two never meet.
pub fn ending_key(previous: &str, word: &str) -> Option<Vec<u8>> {
    let mut key = ending_prefix(previous)?;
    ending_class(word, 2, &mut key)?;
    Some(key)
}

/// The start of the ending pairs' keys of `previous`, the separator after
/// it too.
fn ending_prefix(previous: &str) -> Option<Vec<u8>> {
    let mut key = vec![BIGRAM_SEP];
    ending_class(previous, 3, &mut key)?;
    key.push(BIGRAM_SEP);
    Some(key)
}

/// A word's ending class: the whole word (1, ids) up to `whole` letters,
/// else its last two (2, ids).
fn ending_class(w: &str, whole: usize, key: &mut Vec<u8>) -> Option<()> {
    let ids = remap(w)?;
    if ids.is_empty() {
        return None;
    }
    if ids.len() <= whole {
        key.push(1);
        key.extend_from_slice(&ids);
    } else {
        key.push(2);
        key.extend_from_slice(&ids[ids.len() - 2..]);
    }
    Some(())
}

fn remap(word: &str) -> Option<Vec<u8>> {
    word.to_lowercase()
        .chars()
        .map(alphabet::char_to_id)
        .collect()
}

fn map_file<P: AsRef<Path>>(path: P) -> io::Result<Map<Mmap>> {
    let file = File::open(path)?;
    // SAFETY: the blob is a read-only, prebuilt asset; we don't mutate it.
    let mmap = unsafe { Mmap::map(&file)? };
    Map::new(mmap).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

impl Engine<Mmap> {
    /// Open an FST dictionary blob by memory-mapping it (zero heap for the index).
    /// A ranked dictionary's priors ([`crate::store`]) are beside it, in
    /// the file of the same name ending `.bin`.
    pub fn open<P: AsRef<Path>>(path: P, cfg: Config) -> io::Result<Self> {
        let engine = Engine::new(map_file(&path)?, cfg);
        if engine.format.ranked {
            let store = Store::open(path.as_ref().with_extension("bin"))?;
            return Ok(engine.with_store(store));
        }
        Ok(engine)
    }

    /// [`Engine::open`] plus a context model (bigram FST), also mmap'd.
    pub fn open_with_bigrams<P: AsRef<Path>, Q: AsRef<Path>>(
        dict: P,
        bigrams: Q,
        cfg: Config,
    ) -> io::Result<Self> {
        Ok(Engine::open(dict, cfg)?.with_bigrams(map_file(bigrams)?))
    }

    /// Add the casing list (see [`Engine::casing`]), mmap'd.
    pub fn open_casing<P: AsRef<Path>>(self, path: P) -> io::Result<Self> {
        Ok(self.with_casing(map_file(path)?))
    }

    /// Add the chooser ([`crate::chooser`]), mmap'd.
    pub fn open_chooser<P: AsRef<Path>>(self, path: P) -> io::Result<Self> {
        Ok(self.with_chooser(Chooser::open(path)?))
    }

    /// Add the lemma vectors ([`crate::lemmas`]), mmap'd.
    pub fn open_lemmas<P: AsRef<Path>>(self, path: P) -> io::Result<Self> {
        Ok(self.with_lemmas(Lemmas::open(path)?))
    }
}

impl<D: AsRef<[u8]>> Engine<D> {
    pub fn new(map: Map<D>, cfg: Config) -> Self {
        Engine {
            format: DictFormat::from_type(map.as_fst().fst_type()),
            map,
            bigrams: None,
            lemmas: None,
            chooser: None,
            store: None,
            casing: None,
            user: None,
            kb: Keyboard::new(&cfg),
            cfg,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Replace the config (e.g. on a layout change); rebuilds the keyboard cost
    /// tables once, here, rather than per query.
    pub fn set_config(&mut self, cfg: Config) {
        self.kb = Keyboard::new(&cfg);
        self.cfg = cfg;
    }

    /// Whether `word` is a dictionary word (case-insensitive, exact) or one
    /// the user added.
    pub fn contains(&self, word: &str) -> bool {
        remap(word).is_some_and(|k| {
            self.map.contains_key(&k) || self.user.as_ref().is_some_and(|u| u.contains_key(&k))
        })
    }

    /// The words the user added by hand (replaces the previous set).
    pub fn set_user_words<S: AsRef<str>>(&mut self, words: &[S]) {
        let mut keys: Vec<Vec<u8>> = words
            .iter()
            .filter_map(|w| remap(w.as_ref()))
            .filter(|k| !k.is_empty())
            .collect();
        keys.sort();
        keys.dedup();
        self.user = if keys.is_empty() {
            None
        } else {
            Map::from_iter(keys.into_iter().map(|k| (k, USER_PRIOR))).ok()
        };
    }

    /// Attach a context model (a bigram FST built by index-builder).
    pub fn with_bigrams(mut self, bigrams: Map<D>) -> Self {
        self.bigrams = Some(bigrams);
        self
    }

    /// Attach lemma vectors (with a context model that holds the words'
    /// lemma ids).
    /// Attach a ranked dictionary's priors ([`crate::store`]).
    pub fn with_store(mut self, store: Store<D>) -> Self {
        self.store = Some(store);
        self
    }

    /// How the dictionary's priors read.
    pub(crate) fn priors(&self) -> Priors<'_, D> {
        match (&self.store, self.format.ranked) {
            (Some(store), true) => Priors::Ranked(self.format, store),
            _ => Priors::Outputs(self.format),
        }
    }

    pub fn with_lemmas(mut self, lemmas: Lemmas<D>) -> Self {
        self.lemmas = Some(lemmas);
        self
    }

    /// Attach the chooser (it reads the lemma vectors: attach those too).
    pub fn with_chooser(mut self, chooser: Chooser<D>) -> Self {
        self.chooser = Some(chooser);
        self
    }

    pub fn with_casing(mut self, casing: Map<D>) -> Self {
        self.casing = Some(casing);
        self
    }

    /// How `word` is written: names and places capitalized, abbreviations in
    /// capitals, everything else as typed.
    pub fn casing(&self, word: &str) -> Casing {
        let (Some(map), Some(key)) = (self.casing.as_ref(), remap(word)) else {
            return Casing::AsTyped;
        };
        match map.get(key).map(|v| v & 3) {
            Some(1) => Casing::Capital,
            Some(2) => Casing::Upper,
            _ => Casing::AsTyped,
        }
    }

    /// An obscene word (flagged in the casing map): the keyboard can keep it
    /// out of its suggestions and corrections.
    pub fn offensive(&self, word: &str) -> bool {
        let (Some(map), Some(key)) = (self.casing.as_ref(), remap(word)) else {
            return false;
        };
        map.get(key).is_some_and(|v| v & 4 != 0)
    }

    pub fn has_bigrams(&self) -> bool {
        self.bigrams.is_some()
    }

    /// `−log P(word | previous)` in nats, if the pair is in the context model.
    pub fn bigram_nats(&self, previous: &str, word: &str) -> Option<f32> {
        let mut key = remap(previous)?;
        key.push(BIGRAM_SEP);
        key.extend(remap(word)?);
        Some(self.bigrams.as_ref()?.get(key)? as f32 * self.cfg.prior_scale)
    }

    /// `word`'s grammar (tools/build_classes.py): its class and reading set.
    /// The dictionary value holds its grammar id ([`DictFormat`]), the id's
    /// pair is in the context model under `0, 10, id`; an older model keeps
    /// the pair itself under `0, 3, word`.
    fn word_grammar(&self, word: &str) -> Option<(u64, u64)> {
        let key = remap(word)?;
        let bigrams = self.bigrams.as_ref()?;
        let v = if self.format.shift > 0 {
            let id = self.format.grammar(self.map.get(&key)?);
            if id == 0 {
                return None;
            }
            bigrams.get([BIGRAM_SEP, 10, (id >> 8) as u8, id as u8])?
        } else {
            bigrams.get([vec![BIGRAM_SEP, 3], key].concat())?
        };
        Some((v & 0xFFFF, v >> 16))
    }

    /// `word`'s grammatical class (its likely readings), if known.
    fn word_class(&self, word: &str) -> Option<u64> {
        self.word_grammar(word).map(|g| g.0).filter(|&c| c != 0)
    }

    /// The grammemes ([`crate::gram::bit`]) of all of `word`'s readings, the
    /// rare ones too (empty when unknown): its reading set's, one each under
    /// `0, 9, set (2 bytes), index`.
    pub fn readings(&self, word: &str) -> Vec<u64> {
        self.set_readings(self.word_grammar(word).map_or(0, |g| g.1))
    }

    /// A reading set's readings (see [`Engine::readings`]; 0: none).
    pub(crate) fn set_readings(&self, set: u64) -> Vec<u64> {
        let Some(bigrams) = self.bigrams.as_ref().filter(|_| set != 0) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..=u8::MAX {
            match bigrams.get([BIGRAM_SEP, 9, (set >> 8) as u8, set as u8, i]) {
                Some(bits) => out.push(bits),
                None => break,
            }
        }
        out
    }

    /// Log ratios kept as bytes: steps of 0.05 nat around 128.
    fn log_ratios<const N: usize>(v: u64) -> [f32; N] {
        std::array::from_fn(|i| ((v >> (8 * i)) & 0xFF) as f32 * 0.05 - 6.4)
    }

    /// The log odds of a comma between `a` and `b` (tools/build_commas.py):
    /// the pair's own when it has them, else the odds anywhere moved by what
    /// each word says (a comma before «что», «но», «который»; after
    /// «например»). None without the model.
    pub fn comma_odds(&self, a: &str, b: &str) -> Option<f32> {
        let m = self.bigrams.as_ref()?;
        let odds = |key: Vec<u8>| m.get(key).map(|v| v as f32 * 0.05 - 6.4);
        let base = odds(vec![BIGRAM_SEP, 15, 0])?;
        let (ka, kb) = (remap(a)?, remap(b)?);
        let pair = [
            vec![BIGRAM_SEP, 15, 3],
            ka.clone(),
            vec![BIGRAM_SEP],
            kb.clone(),
        ]
        .concat();
        if let Some(v) = odds(pair) {
            return Some(v);
        }
        let before = odds([vec![BIGRAM_SEP, 15, 1], kb].concat()).map_or(0.0, |v| v - base);
        let after = odds([vec![BIGRAM_SEP, 15, 2], ka].concat()).map_or(0.0, |v| v - base);
        Some(base + before + after)
    }

    /// A word's sense class (tools/build_topics.py): `0, 13, word`.
    /// A word's row in the lemma vectors: its lemma's (`0, 25, word`), UNK
    /// for another Russian word — its lemma too rare for vectors of its own
    /// — and None for the rest (no vectors, or not a Russian word).
    pub(crate) fn lemma(&self, word: &str) -> Option<u32> {
        let lemmas = self.lemmas.as_ref()?;
        let key = [vec![BIGRAM_SEP, 25], remap(word)?].concat();
        match self.bigrams.as_ref()?.get(key) {
            Some(id) => Some(id as u32),
            None => word
                .chars()
                .all(|c| matches!(c, 'а'..='я' | 'ё' | '-'))
                .then(|| lemmas.unk()),
        }
    }

    fn topic(&self, word: &str) -> Option<u64> {
        let key = [vec![BIGRAM_SEP, 13], remap(word)?].concat();
        self.bigrams.as_ref()?.get(key)
    }

    /// How much likelier two sense classes share a sentence than by chance
    /// (nats; 0 when not kept): `0, 14, class, class`.
    fn topic_pmi(&self, a: u64, b: u64) -> f32 {
        let (x, y) = (
            (a.min(b) as u16).to_be_bytes(),
            (a.max(b) as u16).to_be_bytes(),
        );
        self.bigrams
            .as_ref()
            .and_then(|m| m.get([BIGRAM_SEP, 14, x[0], x[1], y[0], y[1]]))
            .map_or(0.0, |v| v as f32 * 0.05 - 6.4)
    }

    /// The learned frame of a governing word (`0, 11, word`).
    fn frame(&self, word: &str) -> Option<crate::gram::Frame> {
        self.frame_at(11, word)
    }

    /// A verb's clause frame: what comes after a noun phrase in its clause
    /// (`0, 16, verb`).
    fn clause_frame(&self, verb: &str) -> Option<crate::gram::Frame> {
        self.frame_at(16, verb)
    }

    /// A preposition's frame by the sense class of the verb it hangs on
    /// («бежала за [велосипедом]», «заплатила за [велосипед]»): `0, 17,
    /// class, preposition`.
    fn through_frame(&self, class: u64, prep: &str) -> Option<crate::gram::Frame> {
        let c = (class as u16).to_be_bytes();
        let key = [vec![BIGRAM_SEP, 17, c[0], c[1]], remap(prep)?].concat();
        let v: [f32; 8] = Self::log_ratios(self.bigrams.as_ref()?.get(key)?);
        Some(crate::gram::Frame {
            other: v[0],
            cases: [v[1], v[2], v[3], v[4], v[5], v[6]],
            infn: v[7],
        })
    }

    /// A preposition's frame by the word right before it («что за
    /// [головная боль]», «благодарю за [помощь]»): `0, 20, word, 0,
    /// preposition`.
    fn paired_frame(&self, word: &str, prep: &str) -> Option<crate::gram::Frame> {
        let key = [
            vec![BIGRAM_SEP, 20],
            remap(word)?,
            vec![BIGRAM_SEP],
            remap(prep)?,
        ]
        .concat();
        let v: [f32; 8] = Self::log_ratios(self.bigrams.as_ref()?.get(key)?);
        Some(crate::gram::Frame {
            other: v[0],
            cases: [v[1], v[2], v[3], v[4], v[5], v[6]],
            infn: v[7],
        })
    }

    /// An adjective's own agreement weights (`0, 23, word`): how much likelier
    /// than anywhere a noun phrase that agrees with it, and one that
    /// doesn't, comes next.
    fn attribute_weights(&self, word: &str) -> Option<(f32, f32)> {
        let key = [vec![BIGRAM_SEP, 23], remap(word)?].concat();
        let [agree, disagree] = Self::log_ratios::<2>(self.bigrams.as_ref()?.get(key)?);
        Some((agree, disagree))
    }

    /// A frame kept under `0, space, word`.
    fn frame_at(&self, space: u8, word: &str) -> Option<crate::gram::Frame> {
        let key = [vec![BIGRAM_SEP, space], remap(word)?].concat();
        let v: [f32; 8] = Self::log_ratios(self.bigrams.as_ref()?.get(key)?);
        Some(crate::gram::Frame {
            other: v[0],
            cases: [v[1], v[2], v[3], v[4], v[5], v[6]],
            // (0 in a model from before the infinitive was told apart: a
            // byte of 0 reads as -6.4.)
            infn: if v[7] < -6.3 { v[0] } else { v[7] },
        })
    }

    /// The learned weights after adjectives and a subject (`0, 12`); None
    /// in an older model (one without the subject's: a misfit -1).
    fn attr_weights(&self) -> Option<crate::gram::AttrWeights> {
        let raw = self.bigrams.as_ref()?.get([BIGRAM_SEP, 12])?;
        let v: [f32; 7] = Self::log_ratios(raw);
        let subject = raw >> 40 != 0;
        Some(crate::gram::AttrWeights {
            other: v[0],
            agree: v[1],
            disagree: v[2],
            base_other: v[3],
            base_infn: v[4],
            subject_fits: if subject { v[5] } else { 0.0 },
            subject_misfits: if subject { v[6] } else { -1.0 },
            after_subject: self
                .bigrams
                .as_ref()?
                .get([BIGRAM_SEP, 18])
                .map(Self::log_ratios::<8>),
            after_subject_other: self
                .bigrams
                .as_ref()?
                .get([BIGRAM_SEP, 21])
                .map(Self::log_ratios::<8>),
            coord: self.bigrams.as_ref()?.get([BIGRAM_SEP, 19]).map(|v| {
                let [same, other] = Self::log_ratios::<2>(v);
                (same, other)
            }),
            impersonal: self.bigrams.as_ref()?.get([BIGRAM_SEP, 22]).map(|v| {
                let [fit, misfit] = Self::log_ratios::<2>(v);
                (fit, misfit)
            }),
        })
    }

    /// The tags of a grammatical class's readings (a byte each, packed
    /// under `0, 5, class`).
    fn class_tags(&self, class: u64) -> Vec<u8> {
        let key = [BIGRAM_SEP, 5, (class >> 8) as u8, class as u8];
        let mut v = self.bigrams.as_ref().and_then(|b| b.get(key)).unwrap_or(0);
        let mut out = Vec::with_capacity(8);
        while v != 0 {
            out.push(v as u8);
            v >>= 8;
        }
        out
    }

    /// How much likelier `word`'s grammar is right after `previous`'s than
    /// anywhere (PMI, nats) — agreement in gender, number, case — for the
    /// best-fitting pair of their readings (a word fits if any reading does).
    /// `None` when either is unclassed; a pair of readings never seen counts
    /// as mildly unlikely.
    pub fn class_pmi(&self, previous: &str, word: &str) -> Option<f32> {
        self.class_pmi_tags(&self.class_tags(self.word_class(previous)?), word)
    }

    /// [`Engine::class_pmi`] with the previous word's class's tags known.
    fn class_pmi_tags(&self, ta: &[u8], word: &str) -> Option<f32> {
        let bigrams = self.bigrams.as_ref()?;
        let tb = self.class_tags(self.word_class(word)?);
        let mut best: Option<f32> = None;
        for &x in ta {
            for &y in &tb {
                if let Some(v) = bigrams.get([BIGRAM_SEP, 4, x, y]) {
                    let pmi = v as f32 * self.cfg.prior_scale - 8.0;
                    best = Some(best.map_or(pmi, |b| b.max(pmi)));
                }
            }
        }
        Some(best.unwrap_or(UNSEEN_CLASS_PAIR))
    }

    /// The confusion pair `word`/`other` (docs/rules-format.md): its id, and
    /// whether `word` is the pair's first member. Kept under `0, 7, word, 0,
    /// other` as `(id + 1)·2 + (word is second)`.
    fn pair(&self, word: &str, other: &str) -> Option<(u64, bool)> {
        let mut key = vec![BIGRAM_SEP, 7];
        key.extend(remap(word)?);
        key.push(BIGRAM_SEP);
        key.extend(remap(other)?);
        let v = self.bigrams.as_ref()?.get(key)?;
        Some((v / 2 - 1, v % 2 == 0))
    }

    /// The words `word` forms a confusion pair with (docs/rules-format.md):
    /// the next word may speak for one of them (к / ко мне, раненый /
    /// раненный осколком).
    pub fn partners(&self, word: &str) -> Vec<String> {
        use fst::{IntoStreamer, Streamer};
        let (Some(bigrams), Some(ids)) = (self.bigrams.as_ref(), remap(word)) else {
            return Vec::new();
        };
        let lo = [vec![BIGRAM_SEP, 7], ids, vec![BIGRAM_SEP]].concat();
        let mut hi = lo.clone();
        *hi.last_mut().unwrap() += 1;
        let mut out = Vec::new();
        let mut stream = bigrams.range().ge(&lo).lt(&hi).into_stream();
        while let Some((key, _)) = stream.next() {
            out.push(alphabet::ids_to_string(&key[lo.len()..]));
        }
        out
    }

    /// What the next word says about `typed` vs `alt` when the two are a
    /// confusion pair: the strongest matching rule's evidence (log
    /// likelihood ratio, nats) — positive for `alt`, negative for `typed`,
    /// 0 without a rule. Rules sit under `0, 6, pair id (3 bytes), feature`
    /// as `member·100000 + strength·1000`.
    pub fn next_evidence(&self, typed: &str, alt: &str, next: &str) -> f32 {
        // «о» before a vowel sound is «об» (об этом, об успехе) — by the
        // sound, not by the word: no list of words holds them all. (Back,
        // «об» → «о», is left to the rules: «об стену» is right too.)
        if (typed, alt) == ("о", "об") {
            let vowel = next
                .chars()
                .next()
                .and_then(|c| c.to_lowercase().next())
                .is_some_and(|c| "аоуэиы".contains(c));
            if vowel {
                return 6.0;
            }
        }
        let (Some(bigrams), Some((id, typed_first))) =
            (self.bigrams.as_ref(), self.pair(typed, alt))
        else {
            return 0.0;
        };
        let mut feats = vec![format!("w+1={}", next.to_lowercase())];
        if let Some(c) = self.word_class(next) {
            feats.push(format!("c+1={c}"));
        }
        let mut best: Option<(u64, f32)> = None;
        for f in feats {
            let mut key = vec![BIGRAM_SEP, 6, (id >> 16) as u8, (id >> 8) as u8, id as u8];
            key.extend_from_slice(f.as_bytes());
            if let Some(v) = bigrams.get(key) {
                let s = (v % 100_000) as f32 / 1000.0;
                if best.is_none_or(|b| s > b.1) {
                    best = Some((v / 100_000, s));
                }
            }
        }
        match best {
            // Member 0 is the pair's first word; `alt` is first when `typed` isn't.
            Some((member, s)) if (member == 0) != typed_first => s,
            Some((_, s)) => -s,
            None => 0.0,
        }
    }

    /// How much likelier `word`'s ending is right after `previous` than
    /// anywhere (PMI, nats; 0 when unknown).
    pub fn ending_pmi(&self, previous: &str, word: &str) -> f32 {
        let (Some(bigrams), Some(key)) = (self.bigrams.as_ref(), ending_key(previous, word)) else {
            return 0.0;
        };
        bigrams
            .get(key)
            .map_or(0.0, |v| v as f32 * self.cfg.prior_scale - 8.0)
    }

    /// Re-rank candidates for the word typed after `previous`, the word
    /// pairs alone ([`Engine::weigh`] with nothing else known).
    pub fn rerank(&self, previous: &str, cands: &mut [Candidate]) {
        self.weigh(&self.context(Some(previous), &[], &[]), cands);
    }

    /// The likeliest next words after `previous` (most likely first), from the
    /// context model — best-first over the `previous\0` subtree.
    pub fn predict(&self, previous: &str, k: usize) -> Vec<Candidate> {
        self.predict_from(previous, "", k)
    }

    /// Words the context model expects after `previous` that start with the
    /// typed `prefix` (and are longer), costed like [`Engine::complete`] so
    /// [`Engine::rerank`] weighs them with the rest.
    pub fn complete_after(&self, previous: &str, prefix: &str, k: usize) -> Vec<Candidate> {
        let typed = prefix.chars().count();
        self.predict_from(previous, prefix, k)
            .into_iter()
            .filter_map(|c| {
                let edit =
                    self.cfg.c_complete_char * c.word.chars().count().saturating_sub(typed) as f32;
                Some(Candidate {
                    cost: edit + self.word_cost(&c.word)?,
                    edit,
                    word: c.word,
                })
            })
            .collect()
    }

    /// [`Engine::predict`] among the words starting with `start`.
    fn predict_from(&self, previous: &str, start: &str, k: usize) -> Vec<Candidate> {
        let (Some(bigrams), Some(mut prefix), Some(start)) =
            (self.bigrams.as_ref(), remap(previous), remap(start))
        else {
            return Vec::new();
        };
        if prefix.is_empty() {
            return Vec::new(); // (keys under a bare separator are the endings)
        }
        prefix.push(BIGRAM_SEP);
        prefix.extend_from_slice(&start);
        let fst = bigrams.as_fst();
        let mut node = fst.root();
        let mut base = 0u64;
        for &b in &prefix {
            let Some(i) = node.find_input(b) else {
                return Vec::new();
            };
            let t = node.transition(i);
            base += t.out.value();
            node = fst.node(t.addr);
        }
        let cost = |acc: u64| self.cfg.w_lm * acc as f32 * self.cfg.prior_scale;
        let mut out = Vec::new();
        let mut heap: BinaryHeap<Frontier> = BinaryHeap::new();
        for t in node.transitions() {
            let acc = base + t.out.value();
            heap.push(Frontier {
                cost: cost(acc),
                span: Span {
                    acc,
                    ..Span::default()
                },
                path: [start.as_slice(), &[t.inp]].concat(),
                addr: t.addr,
                pairs: None,
                found: false,
            });
        }
        let mut visited = 0;
        while let Some(Frontier {
            span: Span { acc, .. },
            path,
            addr,
            ..
        }) = heap.pop()
        {
            visited += 1;
            if out.len() >= k || visited > self.cfg.max_nodes {
                break;
            }
            let n = fst.node(addr);
            if n.is_final() {
                let c = cost(acc + n.final_output().value());
                out.push(Candidate {
                    word: alphabet::ids_to_string(&path),
                    cost: c,
                    edit: 0.0,
                });
            }
            for t in n.transitions() {
                let mut p = path.clone();
                p.push(t.inp);
                let a = acc + t.out.value();
                heap.push(Frontier {
                    cost: cost(a),
                    span: Span {
                        acc: a,
                        ..Span::default()
                    },
                    path: p,
                    addr: t.addr,
                    pairs: None,
                    found: false,
                });
            }
        }
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost));
        out
    }

    /// `word` written with its ё (еще → ещё, пошел → пошёл) where the е-form
    /// is only a spelling of the ё-word (tools/yo_words.py, `0, 24, word`):
    /// None where there is none, or where е and ё make two words (небе /
    /// нёбе, все / всё — the context's to decide).
    pub fn yo_form(&self, word: &str) -> Option<String> {
        let key = [vec![BIGRAM_SEP, 24], remap(word)?].concat();
        let mask = self.bigrams.as_ref()?.get(key)?;
        Some(
            word.chars()
                .enumerate()
                .map(|(i, c)| {
                    if i < 64 && mask >> i & 1 == 1 {
                        'ё'
                    } else {
                        c
                    }
                })
                .collect(),
        )
    }

    /// The prior cost `w_lm·(−log P(word))` of a dictionary word, if it is one.
    pub fn word_cost(&self, word: &str) -> Option<f32> {
        let key = remap(word)?;
        let prior = match self.map.get(&key) {
            Some(v) => self.priors().word(v),
            None => self.user.as_ref()?.get(&key)?,
        };
        Some(self.cfg.w_lm * prior as f32 * self.cfg.prior_scale)
    }

    /// Whether some dictionary (or user) word starts with `prefix`
    /// (case-insensitive).
    pub fn is_prefix(&self, prefix: &str) -> bool {
        fn walks<F: AsRef<[u8]>>(fst: &Fst<F>, prefix: &str) -> bool {
            let mut node = fst.root();
            for c in prefix.to_lowercase().chars() {
                let Some(id) = alphabet::char_to_id(c) else {
                    return false;
                };
                let Some(i) = node.find_input(id) else {
                    return false;
                };
                node = fst.node(node.transition_addr(i));
            }
            true
        }
        walks(self.map.as_fst(), prefix)
            || self
                .user
                .as_ref()
                .is_some_and(|u| walks(u.as_fst(), prefix))
    }

    /// Rank dictionary words by probability of being the intended word for `raw`.
    ///
    /// Starts at the `max_cost` budget; if fewer than `min_results` candidates
    /// are found, the budget is widened by `widen_step` and the search re-run
    /// (up to `max_cost_ceiling`). This reconsiders the branches pruned earlier
    /// — i.e. surfaces words with many typos, but only when close matches are
    /// scarce, so precision is preserved in the common case.
    pub fn suggest(&self, raw: &str) -> Vec<Candidate> {
        self.suggest_with_stats(raw).0
    }

    /// [`Engine::suggest`] plus how much work it took.
    pub fn suggest_with_stats(&self, raw: &str) -> (Vec<Candidate>, Stats) {
        self.suggest_hinted(raw, &[])
    }

    /// [`Engine::suggest`] with per-char press hints (`hints[i]` for input char
    /// `i`): letters a press may have swallowed are cheap to be missing.
    pub fn suggest_hinted(&self, raw: &str, hints: &[Hint]) -> (Vec<Candidate>, Stats) {
        self.suggest_at(&Context::default(), raw, hints)
    }

    /// [`Engine::suggest_hinted`] at a place: the costs are the context's
    /// ([`Engine::weigh`]) all through the search — a word the place makes
    /// likely isn't cut off for being rare on its own.
    pub(crate) fn suggest_at(
        &self,
        ctx: &Context,
        raw: &str,
        hints: &[Hint],
    ) -> (Vec<Candidate>, Stats) {
        let cfg = &self.cfg;
        let input: Vec<u8> = raw
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| alphabet::char_to_id(c).unwrap_or(0))
            .collect();
        // Overly long tokens are not typos of any real word — skip correction.
        if input.is_empty() || input.len() > cfg.max_input_len {
            return (Vec::new(), Stats::default());
        }

        if cfg.one_row_input {
            // Each key is a column placeholder: read it only as such.
            let q = Query::new(&self.kb, cfg, input, hints, true, 0.0, f32::INFINITY, true);
            return self.run(ctx, vec![q], None, 1);
        }

        // Letters kept while editing were read on screen: no other reading.
        let reviewed = hints.iter().any(|h| h.kept > 0.0);
        let mut hyps = Vec::with_capacity(3);
        if cfg.layout_flip && !reviewed {
            let flipped = self.kb.flip_ids(&input);
            if flipped != input {
                hyps.push(Query::new(
                    &self.kb,
                    cfg,
                    flipped,
                    hints,
                    false,
                    cfg.layout_switch,
                    cfg.layout_max_cost,
                    false,
                ));
            }
        }
        // The one-row reading is a fallback: only when the ordinary reading
        // finds no convincing correction (checked after it ran) — the home row
        // holds the most frequent Russian letters, so plenty of ordinary words
        // are typed on it alone.
        let one_row = (!reviewed
            && cfg.home_row
            && input.len() >= 3
            && input.iter().all(|&c| self.kb.is_home_row(c)))
        .then(|| {
            Query::new(
                &self.kb,
                cfg,
                input.clone(),
                hints,
                true,
                cfg.home_row_penalty,
                f32::INFINITY,
                false,
            )
        });
        let m = input.len();
        hyps.insert(
            0,
            Query::new(&self.kb, cfg, input, hints, false, 0.0, f32::INFINITY, true),
        );
        self.run(ctx, hyps, one_row, m)
    }

    /// Search `hyps` with widening, then the one-row fallback if the ordinary
    /// readings found nothing convincing; rank and return the candidates.
    fn run(
        &self,
        ctx: &Context,
        hyps: Vec<Query>,
        one_row: Option<Query>,
        m: usize,
    ) -> (Vec<Candidate>, Stats) {
        let cfg = &self.cfg;

        // Widen the budget while close matches are scarce. Results accumulate
        // across passes (a word found at a tight budget is kept even if a later,
        // looser pass would starve before reaching it under the node cap). The
        // top-k bound persists across passes/hypotheses so branch-and-bound keeps
        // tightening. Each hypothesis has its own node budget shared by all its
        // passes — so one can't starve another, and garbage input still can't
        // blow up latency (at most `hypotheses × max_nodes`).
        let mut hyp_nodes = vec![0usize; hyps.len()];
        let mut hyp_budget = vec![f32::NEG_INFINITY; hyps.len()];
        let silent = Context::default();
        let ctx = if ctx.speaks() && self.bigrams.is_some() {
            ctx
        } else {
            &silent
        };
        let mut st = State {
            ctx,
            best: HashMap::new(),
            topk: TopK {
                k: cfg.top_k.max(1),
                v: Vec::with_capacity(cfg.top_k + 1),
            },
            nodes: 0,
            rows: Vec::new(),
            path: Vec::with_capacity(32),
        };
        let mut budget = cfg.max_cost;
        loop {
            for (h, q) in hyps.iter().enumerate() {
                // A capped hypothesis that already ran at its cap would only redo work.
                let b = budget.min(q.max_budget);
                if hyp_nodes[h] < cfg.max_nodes && b > hyp_budget[h] {
                    st.nodes = hyp_nodes[h];
                    self.search(q, b, &mut st);
                    hyp_nodes[h] = st.nodes.min(cfg.max_nodes);
                    hyp_budget[h] = b;
                }
            }
            let exhausted = hyp_nodes.iter().all(|&n| n >= cfg.max_nodes);
            if st.best.len() >= cfg.min_results || budget >= cfg.max_cost_ceiling || exhausted {
                break;
            }
            budget = (budget + cfg.widen_step).min(cfg.max_cost_ceiling);
        }

        // However mangled the input, offer something: one more pass, much
        // looser, with its own node budget — only when nothing was found.
        if st.best.is_empty() && cfg.rescue_cost > budget {
            if let Some(q) = hyps.first() {
                st.nodes = 0;
                self.search(q, cfg.rescue_cost, &mut st);
                hyp_nodes.push(st.nodes.min(cfg.max_nodes));
            }
        }

        if let Some(q) = one_row {
            let best_rate = st
                .best
                .values()
                .map(|&(_, channel)| channel)
                .fold(f32::INFINITY, f32::min)
                / m as f32;
            if best_rate > cfg.home_row_trigger {
                let (found, mut used, mut budget) = (st.best.len(), 0usize, cfg.max_cost);
                loop {
                    st.nodes = used;
                    self.search(&q, budget, &mut st);
                    used = st.nodes.min(cfg.max_nodes);
                    if st.best.len() > found
                        || budget >= cfg.max_cost_ceiling
                        || used >= cfg.max_nodes
                    {
                        break;
                    }
                    budget = (budget + cfg.widen_step).min(cfg.max_cost_ceiling);
                }
                hyp_nodes.push(used);
            }
        }

        let stats = Stats {
            nodes: hyp_nodes.iter().sum(),
        };
        let mut out: Vec<Candidate> = st
            .best
            .into_iter()
            .map(|(path, (cost, edit))| Candidate {
                word: alphabet::ids_to_string(&path),
                cost,
                edit,
            })
            .collect();
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
        out.truncate(cfg.top_k);
        (out, stats)
    }

    /// As-you-type completions of an *exactly* typed prefix, cheapest first.
    /// Returns up to `Config::top_k` words.
    ///
    /// A completion costs `w_lm·(−log P(word)) + c_complete_char·(untyped chars)`:
    /// a long word guessed from a short prefix pays for what the user hasn't
    /// typed, so it doesn't outrank a close correction (`ди`→`ли`, not
    /// `директор`). FST outputs are non-negative and accumulate to each word's
    /// prior, so that cost only grows along a path and a best-first walk visits
    /// completions in order.
    pub fn complete(&self, raw: &str) -> Vec<Candidate> {
        self.complete_at(&Context::default(), raw)
    }

    /// [`Engine::complete`] at a place: the best completions by what they
    /// cost there ([`Engine::weigh`]) — a word the previous one leads to
    /// comes up however rare on its own.
    pub(crate) fn complete_at(&self, ctx: &Context, raw: &str) -> Vec<Candidate> {
        let input: Vec<char> = raw.trim().to_lowercase().chars().collect();
        if input.is_empty() {
            return Vec::new();
        }
        let silent = Context::default();
        let ctx = if ctx.speaks() && self.bigrams.is_some() {
            ctx
        } else {
            &silent
        };
        let mut out = self.complete_in(ctx, self.map.as_fst(), self.priors(), &input);
        if let Some(user) = &self.user {
            let plain = Priors::Outputs(DictFormat::PLAIN);
            for c in self.complete_in(ctx, user.as_fst(), plain, &input) {
                if !out.iter().any(|o| o.word == c.word) {
                    out.push(c);
                }
            }
            out.sort_by(|a, b| a.cost.total_cmp(&b.cost));
            out.truncate(self.cfg.top_k);
        }
        out
    }

    /// [`Engine::complete`] over one FST, best first by what the words cost
    /// at the place: down the way by the least they may cost
    /// ([`Engine::prior_floor`]), a word found goes back into the queue at
    /// its exact cost — so the first `top_k` out are the best.
    fn complete_in<F: AsRef<[u8]>>(
        &self,
        ctx: &Context,
        fst: &Fst<F>,
        priors: Priors<'_, D>,
        input: &[char],
    ) -> Vec<Candidate> {
        let cfg = &self.cfg;
        let pair_fst = self.bigrams.as_ref().map(|b| b.as_fst());
        let down = |pairs: Option<(CompiledAddr, u64)>, c: u8| {
            pairs.zip(pair_fst).and_then(|((a, acc), f)| {
                let n = f.node(a);
                let t = n.transition(n.find_input(c)?);
                Some((t.addr, acc + t.out.value()))
            })
        };
        // Follow the exact prefix.
        let mut node = fst.root();
        let mut base = priors.root();
        let mut pairs = ctx.pairs;
        let mut prefix: Vec<u8> = Vec::with_capacity(input.len());
        for &c in input {
            let Some(id) = alphabet::char_to_id(c) else {
                return Vec::new();
            };
            let Some(ti) = node.find_input(id) else {
                return Vec::new();
            };
            let t = node.transition(ti);
            base = priors.child(&node, ti, base);
            pairs = down(pairs, id);
            prefix.push(id);
            node = fst.node(t.addr);
        }

        let speaks = ctx.speaks();
        let own = |v: u64| priors.word(v) as f32 * cfg.prior_scale;
        let least = |span: Span| priors.least(span) as f32 * cfg.prior_scale;
        let typing = |extra: usize| cfg.c_complete_char * extra as f32;
        // The least a word below costs; a word's exact cost.
        let floor = |span: Span, pairs: Option<(CompiledAddr, u64)>, extra: usize| {
            if speaks {
                let pair = pairs.map(|(_, a)| a as f32 * cfg.prior_scale);
                self.prior_floor(ctx, least(span), pair) + typing(extra)
            } else {
                cfg.w_lm * least(span) + typing(extra)
            }
        };
        let exact = |path: &[u8], v: u64, extra: usize| {
            let c = Candidate {
                word: alphabet::ids_to_string(path),
                cost: cfg.w_lm * own(v) + typing(extra),
                edit: typing(extra),
            };
            if speaks {
                self.shift(ctx, &c)
            } else {
                0.0
            }
        };
        let mut heap: BinaryHeap<Frontier> = BinaryHeap::new();
        let push = |heap: &mut BinaryHeap<Frontier>,
                    path: Vec<u8>,
                    span: Span,
                    addr: CompiledAddr,
                    pairs: Option<(CompiledAddr, u64)>| {
            let extra = path.len() - prefix.len();
            heap.push(Frontier {
                cost: floor(span, pairs, extra),
                span,
                path,
                addr,
                pairs,
                found: false,
            });
        };
        push(&mut heap, prefix.clone(), base, node.addr(), pairs);
        let mut out: Vec<Candidate> = Vec::new();
        let mut visited = 0usize;
        while let Some(f) = heap.pop() {
            let extra = f.path.len() - prefix.len();
            if f.found {
                out.push(Candidate {
                    word: alphabet::ids_to_string(&f.path),
                    cost: f.cost,
                    edit: typing(extra),
                });
                if out.len() >= cfg.top_k {
                    break;
                }
                continue;
            }
            visited += 1;
            if visited > cfg.max_nodes {
                // Out of steps: the words found so far, best first.
                let mut found: Vec<Frontier> = heap.into_iter().filter(|f| f.found).collect();
                found.sort_by(|a, b| a.cost.total_cmp(&b.cost));
                for f in found.into_iter().take(cfg.top_k.saturating_sub(out.len())) {
                    let extra = f.path.len() - prefix.len();
                    out.push(Candidate {
                        word: alphabet::ids_to_string(&f.path),
                        cost: f.cost,
                        edit: typing(extra),
                    });
                }
                break;
            }
            let n = fst.node(f.addr);
            if n.is_final() {
                let v = f.span.acc + n.final_output().value();
                let plain = cfg.w_lm * own(v) + typing(extra);
                let cost = plain + exact(&f.path, v, extra);
                heap.push(Frontier {
                    cost,
                    span: f.span,
                    path: f.path.clone(),
                    addr: f.addr,
                    pairs: None,
                    found: true,
                });
            }
            for (i, t) in n.transitions().enumerate() {
                let mut p = f.path.clone();
                p.push(t.inp);
                push(
                    &mut heap,
                    p,
                    priors.child(&n, i, f.span),
                    t.addr,
                    down(f.pairs, t.inp),
                );
            }
        }
        out
    }

    /// Threshold DFS over the dictionary — and the user's words — for one
    /// hypothesis at one budget.
    fn search(&self, q: &Query, budget: f32, st: &mut State) {
        self.search_in(self.map.as_fst(), self.priors(), q, budget, st);
        if let Some(user) = &self.user {
            self.search_in(
                user.as_fst(),
                Priors::Outputs(DictFormat::PLAIN),
                q,
                budget,
                st,
            );
        }
    }

    fn search_in<F: AsRef<[u8]>>(
        &self,
        fst: &Fst<F>,
        priors: Priors<'_, D>,
        q: &Query,
        budget: f32,
        st: &mut State,
    ) {
        let w = q.input.len() + 1;
        if st.rows.len() < 2 * w {
            st.rows.resize(2 * w, 0.0);
        }
        // Row 0: the empty word against the input = every typed char is extra.
        st.rows[0] = 0.0;
        for j in 1..w {
            st.rows[j] = st.rows[j - 1] + q.ins[j - 1];
        }
        st.path.clear();
        let pairs = st.ctx.pairs;
        let root = priors.root();
        self.walk(fst, priors, q, budget, st, fst.root(), 0, 0, root, pairs);
    }

    /// A final word is accepted when `w_ch·row[m] ≤ budget`. A branch is pruned
    /// when `w_ch·min(row)` exceeds the budget, or when that plus the least
    /// its words may cost at the place (their smallest prior, `out_acc`, and
    /// their pairs' with the previous word, down `pairs` — the context
    /// model's node and outputs so far) and the hypothesis penalty can't
    /// beat the k-th best total found so far.
    #[allow(clippy::too_many_arguments)]
    fn walk<F: AsRef<[u8]>>(
        &self,
        fst: &Fst<F>,
        priors: Priors<'_, D>,
        q: &Query,
        budget: f32,
        st: &mut State,
        node: Node,
        depth: usize,
        last: u8,
        span: Span,
        pairs: Option<(CompiledAddr, u64)>,
    ) {
        st.nodes += 1;
        if st.nodes > self.cfg.max_nodes {
            return;
        }
        let cfg = &self.cfg;
        let m = q.input.len();
        let w = m + 1;
        let row = &st.rows[depth * w..(depth + 1) * w];

        let pair_fst = self.bigrams.as_ref().map(|b| b.as_fst());
        if node.is_final() {
            let edit = row[m];
            if cfg.w_ch * edit <= budget {
                let prior =
                    priors.word(span.acc + node.final_output().value()) as f32 * cfg.prior_scale;
                let channel = cfg.w_ch * edit + q.penalty;
                let mut total = channel + cfg.w_lm * prior;
                if st.ctx.speaks() {
                    // The least it may cost at the place; the exact cost
                    // only for a word that may make the best.
                    let pair = pairs.zip(pair_fst).and_then(|((a, acc), f)| {
                        let n = f.node(a);
                        n.is_final()
                            .then(|| (acc + n.final_output().value()) as f32 * cfg.prior_scale)
                    });
                    let floor = channel + self.prior_floor(st.ctx, prior, pair);
                    total = if floor >= st.topk.bound() {
                        floor
                    } else {
                        let c = Candidate {
                            word: alphabet::ids_to_string(&st.path),
                            cost: total,
                            edit: channel,
                        };
                        total + self.shift(st.ctx, &c)
                    };
                }
                match st.best.get_mut(&st.path) {
                    Some(c) if total < c.0 => {
                        let old = c.0;
                        *c = (total, channel);
                        st.topk.update(Some(old), total);
                    }
                    Some(_) => {}
                    None => {
                        st.best.insert(st.path.clone(), (total, channel));
                        st.topk.update(None, total);
                    }
                }
            }
        }

        let lb = cfg.w_ch * row_min(row);
        if lb > budget {
            return;
        }
        let least = |own: u64| {
            let own = own as f32 * cfg.prior_scale;
            if st.ctx.speaks() {
                let pair = pairs.map(|(_, acc)| acc as f32 * cfg.prior_scale);
                self.prior_floor(st.ctx, own, pair)
            } else {
                cfg.w_lm * own
            }
        };
        // The quick bound first; the tight one only if that doesn't prune.
        if lb + least(span.least) + q.penalty > st.topk.bound() {
            return;
        }
        if priors.refines() && lb + least(priors.least(span)) + q.penalty > st.topk.bound() {
            return;
        }

        // Visit children cheapest-first: how well each continues the input at
        // this depth, plus how much rarer its best word is.
        let jkey = depth.min(m - 1);
        let mut kids = [(0.0f32, 0u8, 0 as CompiledAddr, Span::default()); 64];
        let mut n = 0;
        for t in node.transitions() {
            if n == kids.len() {
                break;
            }
            let acc = span.acc + t.out.value();
            kids[n] = (0.0, t.inp, t.addr, Span { acc, ..span });
            n += 1;
        }
        // Where each child's words end: where the next one's begin (the
        // transitions read once).
        let past = (n < node.len()).then(|| span.acc + node.transition(n).out.value());
        for i in 0..n {
            let next = if i + 1 < n {
                Some(kids[i + 1].3.acc)
            } else {
                past
            };
            let child = priors.span(kids[i].3.acc, next, span);
            // How much rarer the child's best word is (an older dictionary:
            // the transition's own output, as it always was).
            let rarer = match priors {
                Priors::Outputs(f) => f.prior(child.acc - span.acc),
                Priors::Ranked(..) => child.least,
            };
            let key =
                q.sub[kids[i].1 as usize * m + jkey] + cfg.w_lm * rarer as f32 * cfg.prior_scale;
            kids[i].0 = key;
            kids[i].3 = child;
        }
        kids[..n].sort_unstable_by(|a, b| a.0.total_cmp(&b.0));

        if st.rows.len() < (depth + 2) * w {
            st.rows.resize((depth + 2) * w, 0.0);
        }
        for &(_, c, addr, child) in &kids[..n] {
            self.step(q, depth, c, last, &mut st.rows);
            st.path.push(c);
            // Down the previous word's pairs by the same letter.
            let below = pairs.zip(pair_fst).and_then(|((a, acc), f)| {
                let n = f.node(a);
                let t = n.transition(n.find_input(c)?);
                Some((t.addr, acc + t.out.value()))
            });
            self.walk(
                fst,
                priors,
                q,
                budget,
                st,
                fst.node(addr),
                depth + 1,
                c,
                child,
                below,
            );
            st.path.pop();
        }
    }

    /// One DP step: extend the candidate word by char `c` (previous word char
    /// `last`), writing row `depth + 1` from rows `depth` and `depth − 1`.
    #[inline]
    fn step(&self, q: &Query, depth: usize, c: u8, last: u8, rows: &mut [f32]) {
        let cfg = &self.cfg;
        let m = q.input.len();
        let w = m + 1;
        // A missing letter that doubles the previous one is a common slip.
        let del = if c == last {
            self.kb.del_cost(c).min(cfg.c_del_double)
        } else {
            self.kb.del_cost(c)
        };
        let (head, tail) = rows.split_at_mut((depth + 1) * w);
        let cur = &head[depth * w..];
        let prev = if depth >= 1 {
            Some(&head[(depth - 1) * w..depth * w])
        } else {
            None
        };
        let out = &mut tail[..w];
        let crow = &q.sub[c as usize * m..(c as usize + 1) * m];
        // Letters swallowed by a held key are cheap to be missing.
        let absorb =
            (!q.absorb.is_empty()).then(|| &q.absorb[c as usize * w..(c as usize + 1) * w]);
        // ь left out between т and ся (учится typed for учиться).
        let tsya = q
            .tsya
            .filter(|&(t, soft, _)| c == soft && last == t)
            .map(|(_, _, s)| s);
        let del_at = |j: usize| {
            let d = if absorb.is_some_and(|a| a[j]) {
                del.min(cfg.c_del_held)
            } else {
                del
            };
            let d = if tsya.is_some_and(|s| q.input.get(j) == Some(&s)) {
                d.min(cfg.c_tsya)
            } else {
                d
            };
            d + q.kept_del.get(j).copied().unwrap_or(0.0)
        };

        // A letter typed as two (oe for œ, ss for ß): about as cheap as a
        // left-out accent.
        let digraph = self.kb.digraph(c);
        out[0] = cur[0] + del_at(0);
        for j in 1..=m {
            let mut b = (cur[j - 1] + crow[j - 1])
                .min(cur[j] + del_at(j))
                .min(out[j - 1] + q.ins[j - 1]);
            if let Some(pr) = prev {
                if j >= 2 && c == q.input[j - 2] && last == q.input[j - 1] {
                    b = b.min(pr[j - 2] + q.trans[j]);
                }
            }
            if let Some((x, y)) = digraph {
                if j >= 2 && q.input[j - 2] == x && q.input[j - 1] == y {
                    b = b.min(cur[j - 2] + cfg.c_accent);
                }
            }
            out[j] = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile;

    fn engine_with(words: &[(&str, u64)], cfg: Config) -> Engine<Vec<u8>> {
        let mut kv: Vec<(Vec<u8>, u64)> = words
            .iter()
            .map(|(w, v)| {
                (
                    w.chars()
                        .map(|c| alphabet::char_to_id(c).unwrap())
                        .collect(),
                    *v,
                )
            })
            .collect();
        kv.sort();
        kv.dedup_by(|a, b| a.0 == b.0);
        let map = Map::from_iter(kv.iter().map(|(k, v)| (k.as_slice(), *v))).unwrap();
        Engine::new(map, cfg)
    }

    /// Words for a ranked dictionary to read as the plain one does: some
    /// runs of forms longer than 16 (the store's minima), priors multiples
    /// of 50.
    fn many_words() -> Vec<(String, u64)> {
        let stems = [
            "кот", "кит", "ком", "кол", "кор", "мол", "мор", "дом", "дым", "сон",
        ];
        let ends = [
            "", "а", "у", "ом", "е", "ы", "ов", "ам", "ами", "ах", "ик", "ики", "ище",
        ];
        let mut out = Vec::new();
        for (i, s) in stems.iter().enumerate() {
            for (j, e) in ends.iter().enumerate() {
                let p = 50 * (40 + ((i * 37 + j * 53) % 200)) as u64;
                out.push((format!("{s}{e}"), p));
            }
        }
        out
    }

    #[test]
    fn a_ranked_dictionary_reads_as_the_plain_one() {
        let words = many_words();
        let refs: Vec<(&str, u64)> = words.iter().map(|(w, p)| (w.as_str(), *p)).collect();
        let plain = engine_with(&refs, Config::default());
        let (map, store) = crate::store::ranked(&refs);
        let ranked = Engine::new(map, Config::default()).with_store(store);
        assert!(ranked.format.ranked);
        let same = |a: Vec<Candidate>, b: Vec<Candidate>| {
            assert_eq!(a.len(), b.len(), "{a:?} {b:?}");
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.word, y.word, "{a:?} {b:?}");
                assert!((x.cost - y.cost).abs() < 1e-4, "{a:?} {b:?}");
            }
        };
        for typed in [
            "кот",
            "кто",
            "ктоами",
            "домх",
            "сно",
            "морик",
            "клм",
            "дымище",
            "аа",
        ] {
            same(
                plain.read(Evidence::Taps(typed, &[])),
                ranked.read(Evidence::Taps(typed, &[])),
            );
            same(
                plain.read(Evidence::Begun(typed)),
                ranked.read(Evidence::Begun(typed)),
            );
        }
        assert_eq!(plain.word_cost("котами"), ranked.word_cost("котами"));
        assert_eq!(ranked.word_cost("котище"), plain.word_cost("котище"));
        assert!(ranked.word_cost("котищи").is_none());
    }

    fn engine_from(words: &[&str]) -> Engine<Vec<u8>> {
        let kv: Vec<(&str, u64)> = words.iter().map(|w| (*w, 0)).collect();
        engine_with(&kv, Config::default())
    }

    fn engine_vals(words: &[(&str, u64)]) -> Engine<Vec<u8>> {
        engine_with(words, Config::default())
    }

    fn top(e: &Engine<Vec<u8>>, s: &str) -> String {
        e.suggest(s)
            .into_iter()
            .next()
            .map(|c| c.word)
            .unwrap_or_default()
    }

    #[test]
    fn exact_match_wins() {
        let e = engine_from(&["hello", "help", "world"]);
        assert_eq!(top(&e, "hello"), "hello");
    }

    #[test]
    fn neighbor_substitution() {
        // 'e' is adjacent to 'd' on QWERTY.
        let e = engine_from(&["world", "would", "word"]);
        assert_eq!(top(&e, "worle"), "world");
    }

    #[test]
    fn missing_letter() {
        let e = engine_from(&["hello", "world"]);
        assert_eq!(top(&e, "helo"), "hello");
    }

    #[test]
    fn extra_letter() {
        let e = engine_from(&["hello", "help"]);
        assert_eq!(top(&e, "helllo"), "hello");
    }

    #[test]
    fn a_letter_typed_as_two() {
        let e = engine_vals(&[
            ("vœux", 400),
            ("vieux", 3000),
            ("straße", 500),
            ("strasser", 10),
        ]);
        assert_eq!(top(&e, "voeux"), "vœux");
        assert_eq!(top(&e, "strasse"), "straße");
    }

    #[test]
    fn transposition() {
        let e = engine_from(&["hello", "world"]);
        assert_eq!(top(&e, "hlelo"), "hello");
    }

    #[test]
    fn wrong_layout() {
        let e = engine_from(&["привет", "пока"]);
        assert_eq!(top(&e, "ghbdtn"), "привет");
    }

    #[test]
    fn no_wrong_layout_on_phone() {
        // Phone keys are labelled: the φ hypothesis is off there.
        let e = engine_with(
            &[("привет", 0)],
            Config::default().with_profile(Profile::Phone),
        );
        assert!(e.suggest("ghbdtn").iter().all(|c| c.word != "привет"));
    }

    #[test]
    fn heavy_typos_surface_via_widening() {
        // Two deletions exceed the initial budget, so this only appears once
        // the budget widens because close candidates are scarce.
        let e = engine_from(&["information", "world"]);
        let got = e.suggest("infrmaton"); // missing 'o' and 'i'
        assert_eq!(got.first().map(|c| c.word.as_str()), Some("information"));
    }

    #[test]
    fn yo_is_almost_free() {
        let e = engine_vals(&[("ещё", 5000), ("её", 5000), ("еще", 20000)]);
        assert_eq!(top(&e, "ещо"), "ещё");
        assert!(e.suggest("еще").iter().any(|c| c.word == "ещё"));
    }

    #[test]
    fn phonetic_vowel() {
        let e = engine_from(&["корова", "карта", "крова"]);
        assert_eq!(top(&e, "карова"), "корова");
    }

    #[test]
    fn missing_double_letter() {
        let e = engine_from(&["корректный", "коректор"]);
        assert_eq!(top(&e, "коректный"), "корректный");
    }

    #[test]
    fn key_bounce() {
        let e = engine_from(&["привет", "приветы", "приведет"]);
        assert_eq!(top(&e, "приввет"), "привет");
    }

    #[test]
    fn missing_apostrophe() {
        let e = engine_from(&["don't", "done"]);
        assert_eq!(top(&e, "dont"), "don't");
    }

    #[test]
    fn held_key_absorbs_neighbors() {
        // «мастер» typed as «атер»: the finger stayed on «а» and rolled over м and с.
        let cfg = Config::default().with_profile(Profile::Phone);
        let e = engine_with(&[("мастер", 0), ("тер", 0), ("мотор", 0)], cfg);
        assert_eq!(top(&e, "атер"), "тер");
        let held = Hint {
            held: true,
            ..Hint::default()
        };
        let hints = [held, Hint::default(), Hint::default(), Hint::default()];
        let (got, _) = e.suggest_hinted("атер", &hints);
        assert_eq!(got.first().map(|c| c.word.as_str()), Some("мастер"));
    }

    #[test]
    fn kept_letters_are_not_rewritten() {
        let e = engine_vals(&[("умчались", 5000), ("мучились", 3000)]);
        let kept = |n: usize| -> Vec<Hint> {
            (0..8)
                .map(|i| Hint {
                    kept: if i < n { 1000.0 } else { 0.0 },
                    ..Hint::default()
                })
                .collect()
        };
        let words = |h: &[Hint]| -> Vec<String> {
            e.suggest_hinted("мучались", h)
                .0
                .into_iter()
                .map(|c| c.word)
                .collect()
        };
        assert!(words(&[]).contains(&"умчались".to_string()));
        // «муч» kept: the swap at the start is out, the tail may change.
        assert_eq!(words(&kept(3)), vec!["мучились".to_string()]);
        // «муча» kept: neither word starts that way.
        assert!(words(&kept(4)).is_empty());
    }

    #[test]
    fn kept_e_may_still_be_yo() {
        let e = engine_vals(&[("ещё", 1000)]);
        let hints = vec![
            Hint {
                kept: 1000.0,
                ..Hint::default()
            };
            3
        ];
        let (got, _) = e.suggest_hinted("еще", &hints);
        assert_eq!(got.first().map(|c| c.word.as_str()), Some("ещё"));
    }

    #[test]
    fn touch_points_override_key_geometry() {
        // «с» typed, but the tap was on the border with «м»: «мама» becomes cheap.
        let e = engine_from(&["мама", "сама"]);
        let cost = |spatial: Vec<(char, f32)>| {
            let hint = Hint {
                spatial,
                ..Hint::default()
            };
            let (got, _) = e.suggest_hinted(
                "сама",
                &[hint, Hint::default(), Hint::default(), Hint::default()],
            );
            got.iter()
                .find(|c| c.word == "мама")
                .map(|c| c.cost)
                .unwrap_or(f32::INFINITY)
        };
        assert!(cost(vec![('м', 0.6)]) + 1.0 < cost(vec![]));
        assert!(
            cost(vec![('м', 6.0)]) > cost(vec![]),
            "a dead-center tap makes the neighbor dearer"
        );
    }

    #[test]
    fn cross_hand_transpositions_are_cheaper() {
        // Desktop fingers: н (right index) ↔ а (left index) cross hands;
        // а ↔ п are both left index.
        let e = engine_from(&["напечатать"]);
        let cost = |typed: &str, hints: &[Hint]| e.suggest_hinted(typed, hints).0[0].cost;
        let cross = cost("анпечатать", &[]);
        let same = cost("нпаечатать", &[]);
        assert!(cross < same, "{cross} vs {same}");
        let mut hints = vec![Hint::default(); 10];
        hints[1].rollover = true;
        assert!(cost("анпечатать", &hints) < cross);
    }

    #[test]
    fn mangled_input_still_gets_suggestions() {
        let e = engine_from(&["проверяемый", "мир"]);
        assert!(!e.suggest("пркврчнмвф").is_empty());
    }

    fn bigram_map(pairs: &[(&str, &str, u64)]) -> Map<Vec<u8>> {
        let mut kv: Vec<(Vec<u8>, u64)> = pairs
            .iter()
            .map(|(a, b, v)| {
                let mut k = remap(a).unwrap();
                k.push(BIGRAM_SEP);
                k.extend(remap(b).unwrap());
                (k, *v)
            })
            .collect();
        kv.sort();
        Map::from_iter(kv.iter().map(|(k, v)| (k.as_slice(), *v))).unwrap()
    }

    #[test]
    fn context_decides_between_close_candidates() {
        // «как де»: «де» and «же» are equally plausible alone; after «как»,
        // «же» is far more likely.
        let e = engine_with(
            &[("де", 9000), ("же", 9200)],
            Config::default().with_profile(Profile::Phone),
        )
        .with_bigrams(bigram_map(&[("как", "же", 1500)]));
        let mut got = e.suggest("де");
        e.rerank("как", &mut got);
        assert_eq!(got[0].word, "же", "{got:?}");
    }

    #[test]
    fn predicts_likeliest_next_words() {
        let e = engine_from(&["как", "дела", "же", "раз"]).with_bigrams(bigram_map(&[
            ("как", "дела", 900),
            ("как", "же", 1200),
            ("как", "раз", 2500),
            ("раз", "два", 300),
        ]));
        let words: Vec<String> = e.predict("как", 2).into_iter().map(|c| c.word).collect();
        assert_eq!(words, vec!["дела", "же"]);
        assert!(e.predict("нет", 3).is_empty());
    }

    #[test]
    fn context_completes_the_typed_prefix() {
        let e = engine_vals(&[("быть", 3000), ("был", 1000), ("бы", 900), ("может", 2000)])
            .with_bigrams(bigram_map(&[("может", "быть", 500), ("может", "бы", 4000)]));
        let words: Vec<String> = e
            .complete_after("может", "б", 5)
            .into_iter()
            .map(|c| c.word)
            .collect();
        assert_eq!(words, vec!["быть".to_string(), "бы".to_string()]);
        assert!(e
            .complete_after("может", "бы", 5)
            .iter()
            .all(|c| c.word != "бы"));
    }

    #[test]
    fn endings_agree_without_the_word_pair() {
        let cfg = Config::default().with_profile(Profile::Phone);
        let e = engine_with(
            &[("дорожка", 2000), ("дорожках", 4000), ("неведомых", 9000)],
            cfg,
        );
        let mut kv: Vec<(Vec<u8>, u64)> =
            vec![(ending_key("неведомых", "дорожках").unwrap(), 12_000)];
        kv.push((ending_key("неведомых", "дорожка").unwrap(), 8_000));
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        let plain = e.suggest("дорожкдж");
        let mut ctx = plain.clone();
        e.rerank("неведомых", &mut ctx);
        let gap = |c: &[Candidate]| {
            let cost = |w: &str| c.iter().find(|x| x.word == w).map(|x| x.cost).unwrap();
            cost("дорожках") - cost("дорожка")
        };
        assert!(gap(&ctx) < gap(&plain) - 0.5, "{plain:?} {ctx:?}");
        assert_eq!(
            ctx[0].word, "дорожках",
            "after «неведомых»: the agreeing form"
        );
    }

    #[test]
    fn user_words_are_words() {
        let mut e = engine_vals(&[("кринж", 3000), ("кружок", 4000)]);
        assert!(!e.contains("кринжово"));
        e.set_user_words(&["кринжово"]);
        assert!(e.contains("Кринжово"));
        assert_eq!(top(&e, "кринжлво"), "кринжово");
        assert!(e.is_prefix("кринжо"));
        assert!(e.complete("кринжо").iter().any(|c| c.word == "кринжово"));
        e.set_user_words::<&str>(&[]);
        assert!(!e.contains("кринжово"));
    }

    #[test]
    fn grammar_agreement_without_the_word_pair() {
        // «пользовательский ввод / вод»: the same swipe path; «вод» is the
        // likelier word, but it's genitive plural after a nominative adjective.
        let e = engine_vals(&[("ввод", 5000), ("вод", 4300), ("пользовательский", 9000)]);
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let class = |w: &str, c: u64| ([vec![0, 3], id(w)].concat(), c);
        let mut kv = vec![
            class("пользовательский", 1),
            class("ввод", 2),
            class("вод", 3),
            (vec![0, 5, 0, 1], 1),      // class 1: tag 1 (adj masc sg nom)
            (vec![0, 5, 0, 2], 2),      // class 2: tag 2 (noun masc sg nom)
            (vec![0, 5, 0, 3], 3),      // class 3: tag 3 (noun femn pl gen)
            (vec![0, 4, 1, 2], 11_000), // adj → masc noun: likely
            (vec![0, 4, 1, 3], 5_000),  // adj → gen. plural: unlikely
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        let cost = |w: &str| {
            let mut c = vec![Candidate {
                word: w.into(),
                cost: e.word_cost(w).unwrap(),
                edit: 0.0,
            }];
            e.rerank("пользовательский", &mut c);
            c[0].cost
        };
        assert!(e.word_cost("вод").unwrap() < e.word_cost("ввод").unwrap());
        assert!(cost("ввод") < cost("вод"));
    }

    #[test]
    fn the_phrase_grammar_reads_every_reading() {
        // «с таким опытом»: «таким» is mostly plural dative, but also the
        // masculine instrumental — which «опытом» agrees with.
        let e = engine_vals(&[
            ("с", 3000),
            ("таким", 6000),
            ("опытом", 6000),
            ("опытам", 5500),
        ]);
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let parse = crate::gram::parse;
        // Class 1 (low bits) for all; reading sets 1–4 above.
        let word = |w: &str, set: u64| ([vec![0, 3], id(w)].concat(), 1 | set << 16);
        let mut kv = vec![
            word("с", 1),
            word("таким", 2),
            word("опытом", 3),
            word("опытам", 4),
            (vec![0, 9, 0, 1, 0], parse("PREP")),
            (vec![0, 9, 0, 2, 0], parse("ADJF,plur,datv")),
            (vec![0, 9, 0, 2, 1], parse("ADJF,masc,sing,ablt")),
            (vec![0, 9, 0, 3, 0], parse("NOUN,masc,sing,ablt")),
            (vec![0, 9, 0, 4, 0], parse("NOUN,masc,plur,datv")),
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        assert_eq!(e.readings("таким").len(), 2);
        let mut cands: Vec<Candidate> = ["опытам", "опытом"]
            .iter()
            .map(|w| Candidate {
                word: w.to_string(),
                cost: e.word_cost(w).unwrap(),
                edit: 0.0,
            })
            .collect();
        cands.sort_by(|a, b| a.cost.total_cmp(&b.cost));
        assert_eq!(cands[0].word, "опытам");
        let phrase = ["с".to_string(), "таким".to_string()];
        e.weigh(&e.context(None, &phrase, &[]), &mut cands);
        assert_eq!(cands[0].word, "опытом", "{cands:?}");
    }

    #[test]
    fn the_search_weighs_as_weigh_does() {
        // «ко…» after «рыжий»: on its own «ком» by far; after it «кот».
        let words = [("ком", 4_000), ("кот", 15_000), ("кит", 9_000)];
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let pairs = || {
            let kv = vec![([id("рыжий"), vec![0], id("кот")].concat(), 100u64)];
            Map::from_iter(kv).unwrap()
        };
        let narrow = Config {
            top_k: 1,
            ..Config::default()
        };
        let e = engine_with(&words, narrow).with_bigrams(pairs());
        let wide = engine_with(&words, Config::default()).with_bigrams(pairs());
        let ctx = e.context(Some("рыжий"), &["рыжий".to_string()], &[]);
        let taps = Evidence::Taps("ко", &[]);
        // Only one word kept: on its own, «ком»…
        assert_eq!(e.read(taps)[0].word, "ком");
        // …but the search at the place keeps «кот», rare on its own.
        let got = e.decode(&ctx, taps);
        assert_eq!(got[0].word, "кот", "{got:?}");
        // At the cost weighing all of them gives it.
        let mut all = wide.read(taps);
        wide.weigh(&ctx, &mut all);
        assert_eq!(all[0].word, "кот");
        assert!((all[0].cost - got[0].cost).abs() < 1e-3, "{all:?} {got:?}");
    }

    #[test]
    fn the_lemmas_speak_inside_the_search() {
        // «ча…» after «пить»: on its own «час» by far; the lemma vectors
        // know «чай» goes with «пить» — no pair of these words needed.
        let words = [("час", 4_000), ("чай", 6_000), ("чан", 12_000)];
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        // Lemma ids: пить 0, чай 1, час 2, чан 3; UNK 4, START 5.
        let (z, one) = (vec![0.0; 4], |x: f32| vec![x, 0.0, 0.0, 0.0]);
        let ctx = vec![
            one(3.0),
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
        ];
        let tgt = vec![z.clone(), one(2.0), z.clone(), z.clone(), z.clone()];
        let stems: Vec<Vec<u8>> = ["пить", "чай", "час", "чан"]
            .iter()
            .map(|w| id(w))
            .collect();
        let stems: Vec<&[u8]> = stems.iter().map(Vec::as_slice).collect();
        let blob =
            || Lemmas::new(crate::lemmas::tests::blob(&ctx, &tgt, &[0.0; 5], &stems)).unwrap();
        let ids = || {
            let mut kv: Vec<(Vec<u8>, u64)> = [("пить", 0), ("чай", 1), ("час", 2), ("чан", 3)]
                .iter()
                .map(|(w, i)| ([vec![0, 25], id(w)].concat(), *i))
                .collect();
            kv.sort();
            Map::from_iter(kv).unwrap()
        };
        let narrow = Config {
            top_k: 1,
            ..Config::default()
        };
        let e = engine_with(&words, narrow)
            .with_bigrams(ids())
            .with_lemmas(blob());
        let wide = engine_with(&words, Config::default())
            .with_bigrams(ids())
            .with_lemmas(blob());
        let ctx = e.context(Some("пить"), &["пить".to_string()], &[]);
        let taps = Evidence::Taps("ча", &[]);
        assert_eq!(e.read(taps)[0].word, "час");
        let got = e.decode(&ctx, taps);
        assert_eq!(got[0].word, "чай", "{got:?}");
        let mut all = wide.read(taps);
        wide.weigh(&ctx, &mut all);
        assert_eq!(all[0].word, "чай");
        assert!((all[0].cost - got[0].cost).abs() < 1e-3, "{all:?} {got:?}");
        // Nothing typed yet: the lemma predicts the word, no pair seen.
        let next = e.decode(&ctx, Evidence::Nothing);
        assert_eq!(
            next.first().map(|c| c.word.as_str()),
            Some("чай"),
            "{next:?}"
        );
        // Without their weight, the word's own odds again.
        let off = Config {
            w_lemma: 0.0,
            ..Config::default()
        };
        let off = engine_with(&words, off)
            .with_bigrams(ids())
            .with_lemmas(blob());
        let ctx = off.context(Some("пить"), &["пить".to_string()], &[]);
        assert_eq!(off.decode(&ctx, taps)[0].word, "час");
    }

    #[test]
    fn the_chooser_weighs_inside_the_search() {
        // A random chooser over the lemma test's blob: whatever it says, the
        // search at the place must weigh as weigh does (its bound holds).
        let words = [
            ("час", 4_000),
            ("чай", 6_000),
            ("чан", 12_000),
            ("чем", 3_000),
        ];
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let (z, one) = (vec![0.0; 4], |x: f32| vec![x, 0.0, 0.0, 0.0]);
        let ctx = vec![
            one(3.0),
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
        ];
        let tgt = vec![z.clone(), one(2.0), z.clone(), z.clone(), z.clone()];
        let stems: Vec<Vec<u8>> = ["пить", "чай", "час", "чан"]
            .iter()
            .map(|w| id(w))
            .collect();
        let stems: Vec<&[u8]> = stems.iter().map(Vec::as_slice).collect();
        let blob =
            || Lemmas::new(crate::lemmas::tests::blob(&ctx, &tgt, &[0.0; 5], &stems)).unwrap();
        let ids = || {
            let mut kv: Vec<(Vec<u8>, u64)> = [("пить", 0), ("чай", 1), ("час", 2), ("чан", 3)]
                .iter()
                .map(|(w, i)| ([vec![0, 25], id(w)].concat(), *i))
                .collect();
            kv.sort();
            Map::from_iter(kv).unwrap()
        };
        let chooser = || Chooser::new(crate::chooser::tests::blob(8, 1, 4, 3, 4)).unwrap();
        let make = |cfg: Config| {
            engine_with(&words, cfg)
                .with_bigrams(ids())
                .with_lemmas(blob())
                .with_chooser(chooser())
        };
        let narrow = make(Config {
            top_k: 1,
            ..Config::default()
        });
        let wide = make(Config::default());
        let sentence = ["я".to_string(), "хочу".to_string(), "пить".to_string()];
        for typed in ["ча", "чм", "чен"] {
            let taps = Evidence::Taps(typed, &[]);
            let ctx = narrow.context(Some("пить"), &sentence[2..], &sentence);
            let got = narrow.decode(&ctx, taps);
            let mut all = wide.read(taps);
            wide.weigh(&ctx, &mut all);
            assert_eq!(got[0].word, all[0].word, "{typed}: {got:?} {all:?}");
            assert!((all[0].cost - got[0].cost).abs() < 1e-3, "{all:?} {got:?}");
        }
        // Off, the chooser changes nothing.
        let off = make(Config {
            w_chooser: 0.0,
            ..Config::default()
        });
        let ctx = off.context(Some("пить"), &sentence[2..], &sentence);
        let bare = engine_with(&words, Config::default())
            .with_bigrams(ids())
            .with_lemmas(blob());
        let bctx = bare.context(Some("пить"), &sentence[2..], &sentence);
        let taps = Evidence::Taps("ча", &[]);
        let (a, b) = (off.decode(&ctx, taps), bare.decode(&bctx, taps));
        assert_eq!(a.len(), b.len());
        assert!(a
            .iter()
            .zip(&b)
            .all(|(x, y)| x.word == y.word && (x.cost - y.cost).abs() < 1e-6));
    }

    #[test]
    fn a_preposition_takes_its_case_from_the_verb() {
        // «бежала за [велосипедом]»: after verbs of running «за» takes the
        // instrumental (sense class 7), by itself it might be either.
        let e = engine_vals(&[
            ("бежала", 6000),
            ("за", 3000),
            ("велосипед", 8000),
            ("велосипедом", 9000),
        ]);
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let parse = crate::gram::parse;
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let frame = |v: [f32; 8]| {
            v.iter()
                .enumerate()
                .fold(0u64, |a, (i, x)| a | byte(*x) << (8 * i))
        };
        let word = |w: &str, set: u64| ([vec![0, 3], id(w)].concat(), 1 | set << 16);
        let mut kv = vec![
            word("бежала", 1),
            word("за", 2),
            word("велосипед", 3),
            word("велосипедом", 4),
            (vec![0, 9, 0, 1, 0], parse("VERB,femn,sing,past")),
            (vec![0, 9, 0, 2, 0], parse("PREP")),
            (vec![0, 9, 0, 3, 0], parse("NOUN,inan,masc,sing,nomn")),
            (vec![0, 9, 0, 3, 1], parse("NOUN,inan,masc,sing,accs")),
            (vec![0, 9, 0, 4, 0], parse("NOUN,inan,masc,sing,ablt")),
            ([vec![0, 13], id("бежала")].concat(), 7),
            (
                [vec![0, 17, 0, 7], id("за")].concat(),
                frame([-2.6, -0.7, -1.0, 0.4, 0.1, 2.6, -1.0, -3.0]),
            ),
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        let phrase = ["бежала".to_string(), "за".to_string()];
        let ctx = e.context(Some("за"), &phrase, &[]);
        let mut cands: Vec<Candidate> = ["велосипед", "велосипедом"]
            .iter()
            .map(|w| Candidate {
                word: w.to_string(),
                cost: e.word_cost(w).unwrap(),
                edit: 0.0,
            })
            .collect();
        e.weigh(&ctx, &mut cands);
        assert_eq!(cands[0].word, "велосипедом", "{cands:?}");
    }

    #[test]
    fn the_grammar_can_live_in_the_dictionary() {
        // Priors in steps of 50, grammar ids in 2 bits; id 1 is (class 7,
        // reading set 3).
        let format = DictFormat {
            quantum: 50,
            shift: 2,
            floor: 0,
            ranked: false,
        };
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let mut words = vec![
            (id("дом"), format.value(4_000, 1)),
            (id("дым"), format.value(6_000, 0)),
        ];
        words.sort();
        let mut b = fst::raw::Builder::new_type(Vec::new(), format.fst_type()).unwrap();
        for (k, v) in &words {
            b.insert(k, *v).unwrap();
        }
        let dict = Map::new(b.into_inner().unwrap()).unwrap();
        let mut kv = vec![
            (vec![0, 10, 0, 1], 7 | 3 << 16),
            (
                vec![0, 9, 0, 3, 0],
                crate::gram::parse("NOUN,masc,sing,nomn"),
            ),
        ];
        kv.sort();
        let e = Engine::new(dict, Config::default()).with_bigrams(Map::from_iter(kv).unwrap());
        assert_eq!(e.format, format);
        assert_eq!(e.word_class("дом"), Some(7));
        assert_eq!(e.readings("дом").len(), 1);
        assert_eq!(e.word_class("дым"), None);
        let prior = e.config().prior_scale * e.config().w_lm;
        assert!((e.word_cost("дом").unwrap() - 4_000.0 * prior).abs() < 1e-3);
        assert_eq!(top(&e, "дрм"), "дом");
    }

    #[test]
    fn the_sentence_says_what_it_is_about() {
        let e = engine_vals(&[("кошка", 7_000), ("крошка", 6_500), ("корм", 7_000)]);
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        // Sense classes: 1 pets, 2 bread; pets go with pets (+1.5 nats), not
        // with bread (-0.5).
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let mut kv = vec![
            ([vec![0, 13], id("кошка")].concat(), 1),
            ([vec![0, 13], id("крошка")].concat(), 2),
            ([vec![0, 13], id("корм")].concat(), 1),
            (vec![0, 14, 0, 1, 0, 1], byte(1.5)),
            (vec![0, 14, 0, 1, 0, 2], byte(-0.5)),
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        let mut cands: Vec<Candidate> = ["крошка", "кошка"]
            .iter()
            .map(|w| Candidate {
                word: w.to_string(),
                cost: e.word_cost(w).unwrap(),
                edit: 0.0,
            })
            .collect();
        assert_eq!(cands[0].word, "крошка");
        let sentence = ["корм".to_string(), "для".to_string()];
        e.weigh(&e.context(None, &[], &sentence), &mut cands);
        assert_eq!(cands[0].word, "кошка", "{cands:?}");
    }

    #[test]
    fn commas_by_the_words_around() {
        let e = engine_vals(&[("знаю", 5000), ("что", 3000), ("потому", 5000)]);
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let mut kv = vec![
            (vec![0, 15, 0], byte(-2.3)),
            ([vec![0, 15, 1], id("что")].concat(), byte(1.5)),
            (
                [vec![0, 15, 3], id("потому"), vec![0], id("что")].concat(),
                byte(-4.0),
            ),
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        assert!((e.comma_odds("знаю", "что").unwrap() - 1.5).abs() < 0.03);
        // «потому что» has its own: no comma between.
        assert!(e.comma_odds("потому", "что").unwrap() < -3.0);
    }

    #[test]
    fn rules_on_the_next_word() {
        let e = engine_vals(&[("в", 3000), ("а", 3000), ("принципе", 8000)]);
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let mut kv = vec![
            ([vec![0, 7], id("в"), vec![0], id("а")].concat(), 2), // pair 0, «в» first
            ([vec![0, 7], id("а"), vec![0], id("в")].concat(), 3),
            ([vec![0, 6, 0, 0, 0], b"w+1=\xd0\xbf".to_vec()].concat(), 1), // unrelated
            (
                [vec![0, 6, 0, 0, 0], "w+1=принципе".as_bytes().to_vec()].concat(),
                6_000,
            ), // for «в»
        ];
        kv.sort();
        let e = e.with_bigrams(Map::from_iter(kv).unwrap());
        assert_eq!(e.next_evidence("а", "в", "принципе"), 6.0);
        assert_eq!(e.next_evidence("в", "а", "принципе"), -6.0);
        assert_eq!(e.next_evidence("а", "в", "доме"), 0.0);
    }

    #[test]
    fn slid_keys_may_be_missing() {
        // The finger landed on «к», slid to «о» and lifted: «кот» typed as «от».
        let e = engine_from(&["кот", "от", "код"]);
        let cost = |hints: &[Hint]| {
            let (got, _) = e.suggest_hinted("от", hints);
            got.iter()
                .find(|c| c.word == "кот")
                .map(|c| c.cost)
                .unwrap()
        };
        let slide = Hint {
            slid: vec!['к'],
            ..Hint::default()
        };
        assert!(
            cost(&[slide, Hint::default()]) + 2.0 < cost(&[]),
            "a slid-over letter is cheap to miss"
        );
    }

    #[test]
    fn one_row_keyboard_reads_columns() {
        // «мастер» on a one-row ЙЦУКЕН: each tap is a column, typed here as the
        // column's home-row letter (м→п, с→а, т→о, е→п).
        let cfg = Config {
            one_row_input: true,
            ..Config::default().with_profile(Profile::Phone)
        };
        let e = engine_with(&[("мастер", 2000), ("пастер", 6000), ("ссора", 2000)], cfg);
        assert_eq!(top(&e, "пааопр"), "мастер");
    }

    #[test]
    fn home_row_typing() {
        // каракатица typed without leaving the home row (фыва олдж): every key
        // stands for its finger's column.
        let e = engine_from(&["каракатица", "карта", "кошка", "автомат"]);
        assert_eq!(top(&e, "ааоаааоаыа"), "каракатица");
    }

    #[test]
    fn completion_returns_only_prefixed() {
        let e = engine_from(&["app", "apple", "apply", "banana"]);
        let words: Vec<String> = e.complete("app").into_iter().map(|c| c.word).collect();
        assert!(words.contains(&"app".to_string()));
        assert!(words.contains(&"apple".to_string()));
        assert!(words.contains(&"apply".to_string()));
        assert!(!words.contains(&"banana".to_string()));
    }

    #[test]
    fn completion_pays_for_untyped_chars() {
        // A frequent long word no longer beats a slightly rarer short one.
        let e = engine_vals(&[("ди", 9000), ("дитя", 8000), ("директор", 7500)]);
        let words: Vec<String> = e.complete("ди").into_iter().map(|c| c.word).collect();
        assert_eq!(words, vec!["ди", "дитя", "директор"]);
    }

    #[test]
    fn completion_frequency_order() {
        // Lower stored value = more frequent = ranked first (here strongly
        // enough to outweigh the untyped-char cost).
        let e = engine_vals(&[("app", 9000), ("apple", 1000), ("apply", 3000)]);
        let words: Vec<String> = e.complete("app").into_iter().map(|c| c.word).collect();
        assert_eq!(words, vec!["apple", "apply", "app"]);
    }
}
