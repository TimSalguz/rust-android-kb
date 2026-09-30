//! Tunable weights for the noisy-channel scoring. All costs are in nats
//! (negative log-probabilities), so they add along an edit alignment and are
//! directly comparable to the `−log P(word)` prior.
//!
//! The keyboard cost tables are derived from this config once, when the engine
//! is built or [`crate::Engine::set_config`] is called — never per query.

/// Physical keyboard the input comes from. Selects key geometry and which
/// correction hypotheses make sense.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Hardware keyboard: square keys, ANSI row stagger, identical QWERTY/ЙЦУКЕН
    /// positions (wrong-layout typing is common), touch-typing finger zones.
    Desktop,
    /// Phone on-screen keyboard (Gboard-like): keys taller than wide, 10 keys per
    /// row for QWERTY and 11 (narrower) for ЙЦУКЕН, ё/ъ on long-press.
    Phone,
}

/// Where the Latin letters sit: the layout of the language typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Latin {
    Qwerty,
    /// German: z and y swapped, ü ö ä keys.
    Qwertz,
    /// French: a/q, z/w swapped, m on the home row, an apostrophe key.
    Azerty,
    /// QWERTY with ñ.
    Spanish,
    /// QWERTY with ç.
    Portuguese,
}

impl Latin {
    /// The letter rows, top to bottom.
    pub fn rows(self) -> [&'static str; 3] {
        match self {
            Latin::Qwerty => ["qwertyuiop", "asdfghjkl", "zxcvbnm"],
            Latin::Qwertz => ["qwertzuiopü", "asdfghjklöä", "yxcvbnm"],
            Latin::Azerty => ["azertyuiop", "qsdfghjklm", "wxcvbn'"],
            Latin::Spanish => ["qwertyuiop", "asdfghjklñ", "zxcvbnm"],
            Latin::Portuguese => ["qwertyuiop", "asdfghjklç", "zxcvbnm"],
        }
    }

    /// Phone row offsets, in that layout's key widths (the bottom row sits
    /// right of the shift key).
    pub fn phone_offsets(self) -> [f32; 3] {
        match self {
            Latin::Qwerty => [0.0, 0.5, 1.5],
            Latin::Qwertz => [0.0, 0.0, 2.0],
            Latin::Azerty | Latin::Spanish | Latin::Portuguese => [0.0, 0.0, 1.5],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub profile: Profile,
    /// The Latin letters' layout.
    pub latin: Latin,

    // --- geometry ---
    /// Touch spread in key-widths (Gaussian σ). Larger = more permissive.
    pub sigma: f32,
    /// Floor cost for any substitution (so same-key-different-glyph isn't free).
    pub base_sub: f32,
    /// Extra cost when substituting across alphabets / unknown-geometry keys.
    pub cross_alphabet_penalty: f32,
    /// Ceiling on a same-layout substitution cost, so a distant-but-real single
    /// substitution stays reachable (the frequency prior then re-ranks it).
    pub max_sub: f32,

    // --- substitutions that geometry can't see ---
    /// Phonetic / orthographic confusions (о↔а, е↔и, д↔т, з↔с, …).
    pub c_phonetic: f32,
    /// е typed where ё was meant: people routinely skip ё, so almost free.
    pub c_yo: f32,
    /// ь missing or extra between т and ся (учится / учиться): a spelling
    /// slip, one of the commonest — cheap, the context decides.
    pub c_tsya: f32,
    /// ё typed where е was meant: the user chose ё on purpose, so rarely a slip.
    pub c_yo_reverse: f32,
    /// ь↔ъ (ъ is a long-press / rare key).
    pub c_hard_soft: f32,
    /// A letter typed without its accent (e for é, n for ñ, u for ü):
    /// routine, so almost free — like е for ё.
    pub c_accent: f32,
    /// An accent typed where none (or another one) was meant: a choice.
    pub c_accent_reverse: f32,

    // --- edit operation costs (−log P of each edit) ---
    /// Missing letter (word has a char the user didn't type).
    pub c_del: f32,
    /// Missing letter that doubles the previous one (`коректный`, `occured`).
    pub c_del_double: f32,
    /// Missing ь/ъ (`обявление`, `учица`).
    pub c_del_sign: f32,
    /// Missing apostrophe / hyphen (`dont`, `ктото`).
    pub c_del_punct: f32,
    /// Missing letter next to a key that was held noticeably longer than the
    /// typing rhythm — the finger stayed down and rolled over it (`атер` with
    /// a long «а» → `мастер`), or pressed a doubled letter once.
    pub c_del_held: f32,
    /// Extra letter (user typed a char not in the word).
    pub c_ins: f32,
    /// Extra letter repeating the previous one — key bounce (`приввет`).
    pub c_ins_repeat: f32,
    /// Extra letter next to an adjacent typed key — fat finger hit two keys.
    pub c_ins_neighbor: f32,
    /// Extra ь/ъ (`учиться` typed for `учится`).
    pub c_ins_sign: f32,
    /// Adjacent transposition (`ab`→`ba`) of two letters typed by the same hand.
    pub c_trans: f32,
    /// Transposition of letters typed by different hands (two-thumb typing:
    /// the hands drift out of sync — `анпечатать`).
    pub c_trans_cross: f32,
    /// Cross-hand transposition where the second press came almost together
    /// with the first (fingers overlapped): the order itself is unreliable.
    pub c_trans_rollover: f32,

    // --- hypotheses / mixing ---
    /// Try the input re-read under the other layout (`ghbdtn`→`привет`). Useful
    /// on a hardware keyboard; on a phone the keys are labelled, so it mostly
    /// produces false positives.
    pub layout_flip: bool,
    /// Penalty added to the wrong-layout (φ) hypothesis.
    pub layout_switch: f32,
    /// Edit budget cap for the φ hypothesis: a wrong-layout word is typed
    /// systematically, so allow about one typo on top — no widening.
    pub layout_max_cost: f32,
    /// When every typed key is on the home row, also try reading each key as
    /// "any key of that column/finger" — typing without moving the hands off
    /// the home row, or a one-row (Minuum-style) keyboard.
    pub home_row: bool,
    /// One-row (compact, Minuum-style) keyboard: each typed key stands for its
    /// whole column; only the horizontal position is known. The input is read
    /// that way exclusively (no as-typed reading).
    pub one_row_input: bool,
    /// Penalty added to the home-row hypothesis.
    pub home_row_penalty: f32,
    /// The home-row reading is tried only when the ordinary reading's best
    /// candidate assumes more typing error than this per letter.
    pub home_row_trigger: f32,
    /// Home-row hypothesis: cost of a key reached by the same finger
    /// (desktop) / in the same column (phone).
    pub c_home_finger: f32,
    /// Completion cost per character the user hasn't typed yet.
    pub c_complete_char: f32,
    /// Context model mix: `P = λ·P(word | previous) + (1 − λ)·P(word)`.
    pub ctx_lambda: f32,
    /// How much the ending model moves the `P(word)` part (a factor
    /// `e^(w·PMI)` of the endings, PMI clamped to ±4): unseen word pairs still
    /// agree in case and number (неведомых → дорожках).
    pub w_endings: f32,
    /// How much grammar moves `P(word)`: `e^(w·PMI)` of the two words'
    /// grammatical classes (tools/build_classes.py); where both are known it
    /// replaces the endings.
    pub w_classes: f32,
    /// Weight of the context rules on the next word (docs/rules-format.md):
    /// their log likelihood ratio, in nats, times `w_lm`.
    pub w_rules: f32,
    /// Weight of the phrase grammar ([`crate::gram::phrase_score`]): a word in a
    /// case its preposition doesn't govern, or that doesn't agree with the
    /// words before it, costs this much more (nats).
    pub w_phrase: f32,
    /// Weight of a predicate's agreement with its subject («девочка быстро
    /// побежала»), learned log ratios against chance.
    pub w_subject: f32,
    /// Weight of the sense classes ([`crate::Engine::weigh`]): how
    /// well a word's class goes with those of the sentence so far (nats).
    pub w_topic: f32,
    /// Weight of the lemma vectors ([`crate::lemmas`]): how much likelier the
    /// word's lemma comes after the previous word's than on its own (PMI,
    /// nats), on the word's own odds.
    pub w_lemma: f32,
    /// Weight of the chooser ([`crate::chooser`]): what the sentence so far
    /// says of a word beyond the word right before (its f, over τ, off the
    /// cost); 0: off.
    pub w_chooser: f32,
    /// The most the chooser moves a word's cost either way (nats), and the
    /// share of that the search allows for before it knows the word (1: all
    /// — no word it would raise is cut; 0: it re-weighs what the search
    /// found — the same words on the phrase set, a third faster to type).
    pub chooser_clamp: f32,
    pub chooser_bound: f32,
    /// The sentence's graph (the parser) names the subject (1) or not (0): a
    /// noun that may be in the nominative, waiting for its head as the
    /// subject — weighed by that chance («нос» before «оторвали» may be the
    /// object: 0.67), not taken for sure.
    pub graph_subject: f32,
    /// Gesture typing: weights of the full comparison with a word's path —
    /// location (squared mean distance, key widths) and shape (scale-free) —
    /// and of the per-letter costs that choose which words get compared.
    pub gesture_location: f32,
    pub gesture_shape: f32,
    pub gesture_walk: f32,
    /// A gesture whose best-fitting word is farther than this from it (mean
    /// distance, key widths) was drawn sloppily: its geometry counts that
    /// much less, squared (0: always fully). Swiped sentences, corners off
    /// by 0.4 key: 83.5 → 85.0% right; by 0.2: the same 97.4%.
    pub gesture_trust: f32,
    /// What a drawn word never seen in the counts costs more (nats): a
    /// sloppy path fits some rare name or term («вадидах») as well as the
    /// word meant, and such words are hardly ever drawn.
    pub gesture_rare: f32,
    /// Weight of the finger's pace along a gesture (a stop no letter
    /// explains, a letter flown past), when the path comes with its times;
    /// 0: the path alone.
    pub gesture_pace: f32,
    /// Weight on the language prior term `−log P(word)`.
    pub w_lm: f32,
    /// Weight on the channel (edit) term.
    pub w_ch: f32,
    /// Scale applied to the stored per-word prior value before mixing.
    pub prior_scale: f32,

    // --- search bounds ---
    /// Initial edit-cost budget: finals with `w_ch·row[m] ≤ budget` are accepted,
    /// branches with `w_ch·min(row) > budget` are pruned.
    pub max_cost: f32,
    /// If a pass yields fewer than this many candidates, the budget is widened
    /// and the search re-run — surfacing heavily-misspelled words only when
    /// close matches are scarce, so precision is preserved in the common case.
    pub min_results: usize,
    /// Budget increment per widening pass.
    pub widen_step: f32,
    /// Ceiling for the budget; widening stops here (tunable "max typos" knob).
    pub max_cost_ceiling: f32,
    /// If nothing at all was found, one more pass at this budget (with a fresh
    /// node budget) so there are always some suggestions, however mangled the
    /// input.
    pub rescue_cost: f32,
    /// Hard cap on FST nodes visited per hypothesis in a `suggest` call (shared
    /// across its widening passes), bounding worst-case latency on garbage input.
    pub max_nodes: usize,
    /// Inputs longer than this (in chars) skip correction — no real word is that
    /// long, so it's certainly not a typo of one. Avoids pathological cost.
    pub max_input_len: usize,
    /// Number of candidates returned.
    pub top_k: usize,
}

impl Default for Config {
    /// Balanced preset (see [`Config::balanced`]).
    fn default() -> Self {
        Config::balanced()
    }
}

impl Config {
    /// Weights shared by every preset; only the search-effort knobs differ.
    fn base() -> Self {
        Config {
            profile: Profile::Desktop,
            latin: Latin::Qwerty,
            sigma: 0.45,
            base_sub: 0.5,
            cross_alphabet_penalty: 8.0,
            max_sub: 6.0,
            c_phonetic: 3.0,
            c_yo: 0.2,
            c_tsya: 0.5,
            c_yo_reverse: 2.0,
            c_hard_soft: 1.0,
            c_accent: 0.2,
            c_accent_reverse: 2.0,
            c_del: 3.25,
            c_del_double: 1.5,
            c_del_sign: 2.0,
            c_del_punct: 1.0,
            c_del_held: 1.0,
            c_ins: 4.0,
            c_ins_repeat: 4.0,
            c_ins_neighbor: 4.0,
            c_ins_sign: 2.0,
            c_trans: 2.0,
            c_trans_cross: 1.5,
            c_trans_rollover: 0.8,
            layout_flip: true,
            layout_switch: 3.0,
            layout_max_cost: 3.5,
            home_row: true,
            one_row_input: false,
            home_row_penalty: 3.0,
            home_row_trigger: 1.3,
            c_home_finger: 0.4,
            c_complete_char: 0.7,
            ctx_lambda: 0.5,
            w_endings: 1.0,
            w_classes: 1.5,
            w_rules: 1.0,
            w_phrase: 0.5,
            w_subject: 1.0,
            w_topic: 0.25,
            w_lemma: 0.5,
            w_chooser: 1.0,
            chooser_clamp: 2.0,
            chooser_bound: 0.0,
            graph_subject: 1.0,
            gesture_location: 7.0,
            gesture_shape: 6.0,
            gesture_walk: 8.0,
            gesture_trust: 0.3,
            gesture_rare: 1.5,
            gesture_pace: 1.0,
            w_lm: 0.4,
            w_ch: 1.0,
            // Matches PRIOR_INT_SCALE in index-builder: stored value ÷ 1000 = nats.
            prior_scale: 0.001,
            // Overridden per preset below.
            max_cost: 7.0,
            min_results: 2,
            widen_step: 4.0,
            max_cost_ceiling: 21.0,
            rescue_cost: 40.0,
            max_nodes: 20_000,
            max_input_len: 32,
            top_k: 8,
        }
    }

    /// Fastest: least search effort, best for low-power / background use.
    /// Slightly lower recall on multi-error words.
    pub fn fast() -> Self {
        Config {
            max_cost: 6.0,
            max_nodes: 10_000,
            ..Config::base()
        }
    }

    /// Default: good speed/quality mix.
    pub fn balanced() -> Self {
        Config {
            max_cost: 7.0,
            max_nodes: 20_000,
            ..Config::base()
        }
    }

    /// Highest recall for hard/heavy typos. More widening headroom and a larger
    /// node budget.
    pub fn accurate() -> Self {
        Config {
            max_cost: 9.0,
            min_results: 3,
            max_cost_ceiling: 25.0,
            max_nodes: 60_000,
            ..Config::base()
        }
    }

    /// Set a numeric weight by field name (tuning harnesses, settings screens).
    pub fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        macro_rules! fields {
            (f32: $($f:ident),*; usize: $($u:ident),*) => {
                match name {
                    $(stringify!($f) => self.$f = value,)*
                    $(stringify!($u) => self.$u = value as usize,)*
                    _ => return Err(format!("unknown config parameter `{name}`")),
                }
            };
        }
        fields!(
            f32: sigma, base_sub, cross_alphabet_penalty, max_sub, c_phonetic, c_yo, c_tsya, c_yo_reverse,
                c_hard_soft, c_accent, c_accent_reverse, c_del, c_del_double, c_del_sign, c_del_punct, c_del_held, c_ins, c_ins_repeat,
                c_ins_neighbor, c_ins_sign, c_trans, c_trans_cross, c_trans_rollover, layout_switch, layout_max_cost, home_row_penalty, home_row_trigger, c_home_finger, c_complete_char, ctx_lambda, w_endings, w_classes, w_rules, w_phrase, w_subject, w_topic, w_lemma, w_chooser, chooser_clamp, chooser_bound, graph_subject, gesture_location, gesture_shape, gesture_walk, gesture_trust, gesture_rare, gesture_pace,
                w_lm, w_ch, prior_scale, max_cost, widen_step, max_cost_ceiling, rescue_cost;
            usize: min_results, max_nodes, max_input_len, top_k
        );
        Ok(())
    }

    /// Switch the keyboard profile, adjusting the touch spread and the
    /// hypotheses that only make sense on one kind of keyboard. Call it before
    /// hand-tuning `sigma`, which it overrides.
    pub fn with_profile(self, profile: Profile) -> Self {
        let (sigma, layout_flip) = match profile {
            Profile::Desktop => (0.45, true),
            // A thumb is wide relative to phone keys (tools/eval.py touch set).
            Profile::Phone => (0.6, false),
        };
        Config {
            profile,
            sigma,
            layout_flip,
            ..self
        }
    }
}
