//! The keyboard itself, platform-free: layouts, touches, the word being
//! composed, suggestions/autocorrect, and a draw list. The Android shim only
//! forwards events, applies text operations and paints the draw list — so all
//! of this is testable on a desktop.

use std::collections::VecDeque;

use kbcore::engine::proof::tokens;
use kbcore::marks::Mark;
use kbcore::{Candidate, Casing, Context, Engine, Evidence, Hint};

use crate::grip::{phrase_for, Calibration, Grip, Grips, Sense, Step};

use crate::i18n::t;
use crate::layout::{self, Action, Hand, Key, Lang, Layer, EMOJI_KEY};
use crate::panel::{self, Hit, Panel, Stash, Tab};
use crate::settings::{LangSwitch, Settings, Strip, Theme};

/// Response flags returned to the shim (timer delay in ms in the high 16 bits).
pub const REDRAW: i32 = 1;
pub const OUTPUT: i32 = 2;
pub const HAPTIC: i32 = 4;
/// The keyboard's height changed: the shim must re-measure the view.
pub const RELAYOUT: i32 = 8;

/// Touch actions as sent by the shim.
pub const DOWN: i32 = 0;
pub const MOVE: i32 = 1;
pub const UP: i32 = 2;
pub const CANCEL: i32 = 3;

const LONG_PRESS_MS: i32 = 350;
/// A space tap read as a letter above the space bar pays this plus the
/// distance to that letter (Gaussian touch model).
const SPACE_SLIP: f32 = 1.0;
/// Space taps this cheap as a letter get a second reading — anywhere on the
/// bar right under the letter (whether the joined word wins is decided by
/// comparing it with the two separate words).
const SPACE_SLIP_MAX: f32 = 4.5;
/// "The space was a letter" must beat "two separate words" by this much.
const MERGE_MARGIN: f32 = 1.0;
/// The two-word window's margin for a spelling pair (в / во что, учится /
/// учиться): what the next word says settles it.
const SPELLING_MARGIN: f32 = 0.5;
/// A word read as the other of its confusion pair (к / ко, раненый /
/// раненный): what that costs as a slip, for the next word to outweigh.
const PAIR_SLIP: f32 = 1.0;
/// Two words that are one with a hyphen (кто то → кто-то, по русски →
/// по-русски): the hyphenated word must beat the two apart by this much.
const HYPHEN_MARGIN: f32 = 1.0;
/// Marks the strip suggestion that joins the current word with the previous.
const MERGE_MARK: &str = "↶ ";
/// Reading the previous word again in the light of the current one (а
/// принципе → в принципе) must win by this much: it changes a word already
/// in the text.
const WINDOW_MARGIN: f32 = 1.5;
/// How many readings of the previous word are weighed.
const WINDOW_ALTS: usize = 5;
/// A typed word that is a word itself gives way only to a slip this cheap: a
/// tap off the center toward the key meant — not a dead-center hit on a
/// neighbor, a missing or an extra letter (сестра приготовит → приготовить
/// would be wrong). Swept with tools/eval_typing.py: 1.0 / 1.3 / 1.6 change
/// 1 / 2 / 2 words of 2225 typed carefully, and get 95.2 / 95.6 / 96.0 % of
/// words right from sloppy taps.
const KNOWN_SLIP: f32 = 1.6;
/// …unless the grammar speaks against it and for another form of it, as far
/// as a missing or an extra letter at its end (девочка быстро побежал →
/// побежала, могут выдержат → выдержать): by this many nats (log ratios:
/// after a subject a predicate that doesn't agree is 110 times rarer than
/// one that does), and the other form still clearly likelier in context.
const GRAMMAR_SLIP: f32 = 3.5;
const GRAMMAR_GAP: f32 = 2.5;
/// Space after a whole real word, read as a longer form not finished: that
/// slip of its own costs this much (nats) — space after a word usually means
/// the word is done («он всем друг», not «другом»; «эта девочка бежа», a
/// gerund nobody writes, still «бежала»).
const SPACE_EARLY: f32 = 2.5;
/// A tap this close to the border with a neighbor (its touch cost at most
/// this over the key's own, in the engine's units) may be read as the
/// neighbor when no word goes on with the key's own letter.
const ZONE_NEAR: f32 = 0.25;
/// A missing space inside the typed word (`приветкак`).
const MISSING_SPACE: f32 = 3.0;
/// A letter just above the space bar typed instead of the space (`приветькак`).
const SPACE_AS_LETTER: f32 = 2.0;
/// Two words must beat the best single word by this much.
const SPLIT_MARGIN: f32 = 1.0;
/// French elision: typed before an apostrophe, these end a word of their own
/// (l'homme = l' + homme; aujourd'hui keeps its apostrophe inside).
const ELIDED: &[&str] = &[
    "c", "ç", "d", "j", "l", "m", "n", "s", "t", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu",
];

/// One-letter words a split may produce, per language.
fn one_letter_words(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru | Lang::En => "вксиаоуяai",
        Lang::De => "",
        Lang::Fr => "aày",
        Lang::Es => "aeouy",
        Lang::Pt => "aeoàé",
    }
}
/// Vowels: a short token without any is an abbreviation or a typo.
const VOWELS: &str = "аеёиоуыэюяaeiouyàáâãäåæèéêëìíîïòóôõöøœùúûüýÿ";
/// Channel cost of about one slipped key (a neighbor substitution, a missing
/// or extra letter): an unknown vowel-less token this close to a word is a typo.
const ONE_SLIP: f32 = 3.5;
/// Two letter presses closer than this (or overlapping) may have registered
/// in swapped order when they come from different hands.
const ROLLOVER_MS: i64 = 90;
/// A finger that drifts less than this (dp) across a key border is jitter,
/// not a slide: the press stays on the key it landed on.
const SLIDE_DP: f32 = 8.0;
/// A press from a letter that travels this far — in keys: widths across,
/// row heights down — over at least three keys, or twice as far, is a
/// gesture: a word drawn across the keys (a slide to a neighbor, two keys,
/// stays a correction).
const GESTURE_KEYS: f32 = 1.4;
const GESTURE_FAR: f32 = 2.5;
/// A tap on the top of Enter this soon after ⌫ is another ⌫ that drifted
/// down (a stray Enter can send the message).
const BKSP_ENTER_MS: i64 = 600;
/// Enter this soon after a letter on the other side of the keyboard (farther
/// than [`BRUSH_SPAN`] of its width) was brushed, not pressed: one thumb can't
/// cross that fast, and nobody means Enter that quickly.
const BRUSH_MS: i64 = 100;
const BRUSH_SPAN: f32 = 0.4;
/// Typing on after ⌫: letters left on screen weigh this much extra to change
/// (one letter erased — likely a quick fix of the last press)…
const KEPT_SOFT: f32 = 1.5;
/// …or are pinned (several erased, or the word was brought back whole).
const KEPT_HARD: f32 = 1000.0;
/// Punctuation that returns from the symbols to the letters once typed. Not
/// opening brackets: a digit may follow as well as a word — (2 + 3) · (см.).
const BACK_TO_LETTERS: &str = ".,!?;:'\")]}»”-—@";
/// Closing brackets and quotes: they hold on to the word before them — a
/// space typed after the word moves after the bracket (привет) · (см. выше)).
const CLOSING: &str = ")]}»”";
/// Holding the space bar this long offers the keyboard picker and settings.
const SPACE_MENU_MS: i32 = 700;
/// The space bar's menu: other keyboards, this keyboard's settings.
const MENU_KEYBOARDS: char = '⌨';
const MENU_SETTINGS: char = '⚙';
/// Lowercase short forms that end with a period mid-sentence (plus any lone
/// lowercase letter: т. е., и т. п., г.).
const ABBREVIATIONS: &[&str] = &[
    "гг", "вв", "см", "ср", "др", "пр", "ул", "кв", "стр", "рис", "им", "тыс", "млн", "млрд",
    "руб", "коп", "напр", "англ", "etc", "vs", "approx",
];
/// While a word is drawn, it is read again every this many trail points
/// (≈ every 30–60 ms) to show what it makes so far.
const GESTURE_PREVIEW_POINTS: usize = 4;
/// How many readings a swipe up on the space bar goes round (the strip's
/// and the next likeliest).
const VARIANTS: usize = 8;
/// Sliding this far along the space bar switches the language.
const SPACE_SWIPE_DP: f32 = 40.0;
/// Drag the left suggestion (the word as typed) up this far to add it to the
/// user's dictionary.
const DRAG_ADD_DP: f32 = 30.0;
/// A capitalized selected word goes in as a name (always capitalized) or as
/// an ordinary word: the two choices start with this (their texts: i18n).
const DICT_PLAIN: &str = "＋ ";
/// How much text before the cursor we keep (the shim asks for this many
/// UTF-16 units too): a whole sentence, for the proofreading.
const BEFORE_CHARS: usize = 400;
const REPEAT_START_MS: i32 = 400;
const REPEAT_MS: i32 = 60;
/// Held ⌫: after this many single chars it takes whole words…
const CHAR_REPEATS: u32 = 12;
/// …first this far apart, each next one sooner, down to the floor.
const WORD_REPEAT_MS: f32 = 300.0;
const WORD_REPEAT_FASTER: f32 = 0.8;
const WORD_REPEAT_MIN_MS: f32 = 120.0;

// Sizes in dp.
const STRIP_DP: f32 = 44.0;
const ROW_DP: f32 = 54.0;
const BOTTOM_PAD_DP: f32 = 6.0;
const GAP_X_DP: f32 = 3.0;
const GAP_Y_DP: f32 = 5.0;
const RADIUS_DP: f32 = 6.0;

/// The keyboard's colors (ARGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: i32,
    pub key: i32,
    pub key_special: i32,
    pub key_pressed: i32,
    pub accent: i32,
    pub text: i32,
    pub text_dim: i32,
    pub text_on_accent: i32,
}

impl Palette {
    /// From 8 colors in the field order (the shim's wallpaper palettes).
    fn from_slice(c: &[i32]) -> Option<Palette> {
        let &[bg, key, key_special, key_pressed, accent, text, text_dim, text_on_accent] = c else {
            return None;
        };
        Some(Palette {
            bg,
            key,
            key_special,
            key_pressed,
            accent,
            text,
            text_dim,
            text_on_accent,
        })
    }
}

const fn argb(c: u32) -> i32 {
    c as i32
}

const DARK: Palette = Palette {
    bg: argb(0xFF1B1C1F),
    key: argb(0xFF3A3B40),
    key_special: argb(0xFF2A2B2F),
    key_pressed: argb(0xFF5C5E66),
    accent: argb(0xFF8AB4F8),
    text: argb(0xFFECECEC),
    text_dim: argb(0xFF9A9CA3),
    text_on_accent: argb(0xFF1B1C1F),
};

const LIGHT: Palette = Palette {
    bg: argb(0xFFE3E5E8),
    key: argb(0xFFFFFFFF),
    key_special: argb(0xFFC8CCD2),
    key_pressed: argb(0xFFAEB3BA),
    accent: argb(0xFF1A73E8),
    text: argb(0xFF1F1F1F),
    text_dim: argb(0xFF5F6368),
    text_on_accent: argb(0xFFFFFFFF),
};

// Draw-list opcodes: every op is 7 ints.
pub const OP_RECT: i32 = 1; // x, y, w, h, color, radius
pub const OP_TEXT: i32 = 2; // center x, baseline y, size, color, text index, bold
pub const OP_ROTATE: i32 = 3; // center x, center y, millidegrees clockwise: until OP_RESTORE
pub const OP_RESTORE: i32 = 4;

/// A text operation for the editor.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Commit(String),
    Composing(String),
    Finish,
    /// Delete this many chars before the cursor.
    Delete(usize),
    Enter,
    /// Move the cursor one char on (over a space already there).
    Skip,
    /// Replace this many chars before and after the cursor by the text (a
    /// word re-corrected where the cursor was put).
    Replace(usize, usize, String),
    /// Show the system's keyboard picker.
    Picker,
    /// Open this keyboard's settings.
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shift {
    Off,
    Once,
    Caps,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Key(usize),
    Slot(usize),
    /// The emoji and clipboard panel, above its bottom row.
    Panel(Hit),
}

/// Space may finish a word not yet whole — of at least this many letters —
/// with a completion at most this many letters longer, when that is what
/// it most likely is («такого исхо» → «исхода»; not «мас» → «мастер»).
const COMPLETE_FROM: usize = 4;
const COMPLETE_ON_SPACE: usize = 2;

/// A comma goes in by itself when the model's log odds are at least this
/// (0.9: on held-out sentences 98–99% of such commas are right, and they
/// are 40% of all commas).
const COMMA_SURE: f32 = 2.2;

/// A finished sentence read whole (Engine::sentence_marks): a comma goes in
/// where its graph is at least this sure of one and the comma model's log
/// odds for the gap are at least [`PROOF_ODDS`] — of such commas on
/// held-out sentences typed without commas (data/parse_m long_valid,
/// tools/../proof_check) 97% are right; with the ones put in as typed
/// ([`COMMA_SURE`]) the keyboard finds 39% of the commas (27% without).
const PROOF_SURE: f32 = 0.8;
const PROOF_ODDS: f32 = 0.0;

/// A text just copied, offered in the strip (a tap pastes it) for this long.
const CLIP_OFFER_MS: i64 = 90_000;
/// What marks it there.
const CLIP_MARK: &str = "📋 ";

struct Pointer {
    id: i32,
    target: Option<Target>,
    /// Where the finger landed (the space bar handles horizontal swipes).
    origin: Option<Target>,
    start_x: f32,
    start_y: f32,
    down_ms: i64,
    /// Keys under the finger during the press, in order (slides).
    path: Vec<usize>,
    /// Landed while another letter press was still down, or right after one.
    rolled: bool,
    /// Already acted on (roll-over, long-press, backspace) — ignore the release.
    done: bool,
    /// Where the finger went, and how far (in keys, see [`GESTURE_KEYS`]),
    /// while it may be a gesture.
    trail: Vec<(f32, f32)>,
    /// When the finger was at each trail point (ms).
    trail_ms: Vec<i64>,
    travel: f32,
    /// Drawing a word across the keys (see [`GESTURE_KEYS`]).
    gesture: bool,
    /// Trail points already read for the live preview of a drawn word.
    previewed: usize,
    /// Where the finger likely aimed: the landing point moved back by the
    /// calibrated grip's usual offset. It only weighs the letters for the
    /// correction; the key pressed is the one under the finger.
    aim: (f32, f32),
}

/// The word before the last space, as it was typed — so a space that was
/// meant to be a letter can be undone once the next word shows it.
struct PrevWord {
    /// As it stood while composed (its case).
    typed: String,
    input: Input,
    /// What went into the text (maybe autocorrected).
    committed: String,
    /// Cost of the best reading of `typed` on its own.
    alone: f32,
    /// Letters the space tap may have been meant for, with the slip cost.
    letters: Vec<(char, f32)>,
    /// What came after it: a space, comma or period.
    sep: char,
    /// Its readings as the space-up gesture cycles them (filled on the first
    /// swipe), and which one is in the text.
    variants: Vec<String>,
    variant: usize,
    /// Its likeliest readings on its own (the word before it counted), to
    /// be weighed again with the next word; empty when it can't change (the
    /// user edited it, or a comma or period came between).
    alts: Vec<Candidate>,
}

/// Right after an autocorrect + space (or the punctuation that finished the
/// word): what was typed (and how) and what replaced it — ⌫ brings the
/// typed word back.
struct Undo {
    typed: String,
    fixed: String,
    /// What finished the word: a space (⌫ takes it too and the word is
    /// typed on), or a mark (it stays: «созвон,» — only the change goes).
    sep: char,
    hints: Vec<Hint>,
    /// Text before the word that the change replaced too (a joined or
    /// re-read previous word with its separator), put back by ⌫.
    restore: Option<String>,
    /// A change that made two words of the text (a re-read pair, a split):
    /// its readings, so a swipe up after ⌫ can bring the pair back.
    pairs: Vec<String>,
}

/// A finished sentence's proofreading: the text from its first new comma
/// on as it was and as it is now, and the word pairs the commas went
/// between (⌫ right after puts it back; those commas don't come again).
struct ProofUndo {
    was: String,
    now: String,
    pairs: Vec<(String, String)>,
}

/// What the finger gave for a word: letters tapped (lowercase) with their
/// touches, or a way drawn across the keys with the time of each point (ms).
/// Every reading of the word — as it is typed, again later in context, its
/// variants — comes from it ([`Ime::decode_input`]).
#[derive(Clone, Debug)]
enum Input {
    Typed {
        letters: String,
        hints: Vec<Hint>,
    },
    Drawn {
        trail: Vec<(f32, f32)>,
        ms: Vec<i64>,
    },
}

impl Input {
    fn typed(letters: &str, hints: &[Hint]) -> Self {
        Input::Typed {
            letters: letters.to_lowercase(),
            hints: hints.to_vec(),
        }
    }
}

/// A way drawn across the keys: its points, and the time of each (ms).
type Way = (Vec<(f32, f32)>, Vec<i64>);

/// A word as it went into the text (lowercase), and what the finger gave
/// for it.
struct Written {
    text: String,
    input: Input,
}

/// How many of the field's words keep their touches.
const WRITTEN: usize = 64;

/// ⌫ undid a change that made two words: `prefix` (the previous word and
/// its space, put back) and the composing word as typed. A swipe up while
/// they stand puts back the pair's readings, one by one.
struct PairUndo {
    prefix: String,
    typed: String,
    variants: Vec<String>,
}

/// A confident "that space was a letter": the joined word, and how many chars
/// before the composing word to replace (previous word + the space).
#[derive(Clone, Debug, PartialEq)]
struct Merge {
    word: String,
    /// The readings a swipe up goes through, `word` first (empty: just it).
    variants: Vec<String>,
    delete: usize,
}

/// The typed word read as two: `left right` (cased as typed).
#[derive(Clone, Debug, PartialEq)]
struct Split {
    left: String,
    right: String,
    /// The pair's readings (the right word's alternatives after the left
    /// one), this one first.
    variants: Vec<String>,
}

/// The alternatives of a held key (е: ё 5), shown in a row above it.
struct Popup {
    pointer: i32,
    key: usize,
    options: Vec<char>,
    selected: usize,
    /// Where the row sits: this many options' widths of it lie left of the
    /// key's center (the lit option over the finger).
    anchor: f32,
}

/// How one letter was pressed: hints for the engine, and the duration of a
/// finished stationary tap (for the rhythm).
type PressInfo = (Hint, Option<f32>);

#[derive(Clone, Copy, PartialEq)]
enum Timer {
    None,
    LongPress(i32),
    Repeat(i32),
}

pub struct Ime<D: AsRef<[u8]>> {
    /// The engine of the current language's data pack…
    engine: Engine<D>,
    engine_pack: &'static str,
    /// …and those of the other packs, swapped in with their language.
    spare: Vec<(&'static str, Engine<D>)>,
    /// Packs the shim has no data for (not asked for again).
    missing: Vec<&'static str>,
    dp: f32,
    width: f32,
    lang: Lang,
    layer: Layer,
    shift: Shift,
    keys: Vec<Key>,
    /// The word being typed, case as typed.
    word: String,
    /// What the composing region in the field currently shows: `word`, or its
    /// correction with live correction on.
    shown: String,
    /// Per char of `word`: how it was pressed (for the engine)…
    hints: Vec<Hint>,
    /// …and, for a stationary tap, how long (for the rhythm).
    durs: Vec<Option<f32>>,
    /// The word was typed straight through: no ⌫ inside, not resumed.
    clean: bool,
    rhythm: Rhythm,
    /// The press being activated right now (consumed by `type_char`).
    pending: Option<PressInfo>,
    /// When the previous letter key went down.
    last_down: Option<(i64, f32)>,
    /// Where the space key being activated was touched.
    /// Where the space, comma or period key being activated was touched — a
    /// tap meant for a letter above it?
    slip_tap: Option<(f32, f32)>,
    prev_word: Option<PrevWord>,
    /// Join the previous word with the current one (shown as `↶ word`).
    merge: Option<Merge>,
    /// The editor has a non-empty selection (⌫ deletes it).
    selected: bool,
    /// Debug logging is on (the shim enables it only in the app's own test field).
    log_on: bool,
    /// Debug log lines (JSON), drained by the shim into a local file.
    log: Vec<String>,
    /// The latest event time seen, for log lines.
    now_ms: i64,
    /// Top candidates of the last update, for the log.
    last_cands: Vec<(String, f32)>,
    /// The word being typed read at its place, every candidate (for tools
    /// that learn from the keyboard's choices: `decoded`).
    decoded: Vec<Candidate>,
    /// The next word expected before any letter of it: every candidate (as
    /// `decoded`, for tools).
    decoded_next: Vec<Candidate>,
    /// Read the current word as two (missing or mistyped space).
    split: Option<Split>,
    /// Suggestion strip: left, center, right.
    slots: [Option<String>; 3],
    /// Calibrated grips, the running guess of the current one, and a
    /// calibration in progress (see `grip`).
    grips: Grips,
    sense: Sense,
    calib: Option<Calibration>,
    /// A calibration was asked for: it starts in the app's own test field.
    calib_wanted: Option<Grip>,
    /// Shown in the strip until the next touch.
    notice: Option<String>,
    /// The emoji and clipboard panel, when open, and what it remembers (in
    /// memory only).
    panel: Option<Panel>,
    stash: Stash,
    /// The newest emoji the phone's font draws (see [`panel::emoji_max`]).
    emoji_max: u16,
    /// When a text was copied that the strip offers to paste.
    clip_offer: Option<i64>,
    /// Words the user added by hand (as they wrote them), and whether the
    /// list changed since saved (`take_user_words`).
    user_words: Vec<String>,
    user_dirty: bool,
    /// A single word is selected in the editor: per strip slot, a word to add
    /// to the user's dictionary (true) or take out (false), as it'd be stored.
    dict_actions: [Option<(String, bool)>; 3],
    /// The current word's likeliest readings (for the space-up gesture), and
    /// the one it picked, if any — space then commits that.
    alternatives: Vec<String>,
    chosen: Option<String>,
    /// A held key's alternatives on show: the finger picks one and lifts.
    popup: Option<Popup>,
    /// The way the word being typed was drawn along, if it was and it
    /// wasn't edited since: ⌫ takes it whole, punctuation after it brings a
    /// space, and it goes into the text with it.
    drawn: Option<Way>,
    /// The strip shows the readings of the word just drawn (in the text with
    /// its space): a tap puts one in its place, ⌫ takes the word whole.
    variants_shown: bool,
    /// The app's own test field has focus.
    own_field: bool,
    /// The calibration / reset stamps last seen in the settings (None before
    /// the first settings).
    seen_stamps: Option<([u64; 4], [u64; 4], u64)>,
    /// The grips changed: the shim saves them (`take_grips`).
    grips_dirty: bool,
    /// See [`WINDOW_MARGIN`] and [`KNOWN_SLIP`]; tunable for benchmarks.
    window_margin: f32,
    known_slip: f32,
    /// Next words shown before this word was begun: one stays first while the
    /// typed letters still match it.
    predicted: Vec<String>,
    /// What space will turn `word` into.
    autocorrect: Option<String>,
    /// Right after an autocorrect + space — backspace reverts.
    undo: Option<Undo>,
    /// Corrections the user undid at this spot, as (typed, fix), lowercase:
    /// the fix isn't offered here again, and typing the same word again keeps
    /// it. Forgotten once the word is committed or the cursor moves.
    rejected: Vec<(String, String)>,
    /// A comma just put in before this word (⌫ takes it out), and the word
    /// pairs whose comma the user took out in this field.
    comma_undo: Option<(String, String)>,
    commas_rejected: Vec<(String, String)>,
    /// The commas a finished sentence's proofreading just put in (⌫ takes
    /// them out).
    proof_undo: Option<ProofUndo>,
    /// `before` begins where the field does (the editor sent less than it
    /// was asked for, and nothing has been cut since).
    whole: bool,
    /// Letters erased from the word since one was last typed.
    erased: usize,
    /// The word came back whole (⌫ undid a correction or reached into a word).
    restored: bool,
    /// When ⌫ was last pressed.
    last_bksp: Option<i64>,
    /// Repeats of the ⌫ being held (0: not held). Nothing is looked up in
    /// the dictionary meanwhile — once, where it stops.
    held_bksp: u32,
    pointers: Vec<Pointer>,
    timer: Timer,
    out: Vec<Op>,
    /// Tail of the text before the cursor (excluding the composing word), for
    /// auto-capitalization and for resuming a word by backspacing into it.
    before: String,
    /// The field allows suggestions/autocorrect (not a password, URL, …).
    suggest: bool,
    /// ⌫ was used inside the word: show and keep exactly what was typed until
    /// the next letter (live correction pauses while the user edits).
    editing: bool,
    /// The last thing committed is a space we inserted after punctuation.
    auto_space: bool,
    /// The start of the text after the cursor (mid-text: a space there is
    /// stepped over, not doubled).
    after: String,
    /// The app asked not to learn from this field (a private window).
    private_field: bool,
    /// The one-handed layout on screen (rows squeezed within the thumb's
    /// reach), for which thumb (set by `relayout`)…
    arc: Option<Hand>,
    /// …and one forced by a swipe up on ?123 / Enter (`Some(None)`: forced
    /// off); `None` follows the setting and the grip.
    arc_forced: Option<Option<Hand>>,
    /// A two-word change undone by ⌫, for the swipe up (see [`PairUndo`]).
    pair_undo: Option<PairUndo>,
    /// The words committed in this field, as typed and with the touches
    /// (only in memory, for this field): a word the cursor is put in is
    /// read again from how it was typed.
    written: Vec<Written>,
    /// The cursor is in a word: its letters before and after the cursor,
    /// which a pick from the strip replaces.
    recorrect: Option<(usize, usize)>,
    /// The languages the app says the field is in (`EditorInfo.hintLocales`,
    /// e.g. Duolingo's lesson language), likeliest first.
    hint_langs: Vec<Lang>,
    /// The language of the keyboard's own words (the phone's).
    ui: Lang,
    /// The phone is in dark mode…
    system_dark: bool,
    /// …and its wallpaper's colors (Android 12+): light and dark palettes.
    wallpaper: Option<(Palette, Palette)>,
    auto_caps: bool,
    settings: Settings,
}

/// The user's normal tap duration, learned only from presses in *clean* words
/// (typed straight through, in the dictionary, left uncorrected), so typos and
/// edits — where the anomalies are — don't define "normal". Median and median
/// absolute deviation, so odd presses don't move it; pauses between keys don't
/// count at all (it measures press durations). Memory only, recent presses only.
#[derive(Default)]
struct Rhythm {
    samples: VecDeque<f32>,
}

impl Rhythm {
    const KEEP: usize = 40;
    const WARM_UP: usize = 12;

    fn add(&mut self, ms: f32) {
        if self.samples.len() == Self::KEEP {
            self.samples.pop_front();
        }
        self.samples.push_back(ms);
    }

    /// Stationary presses longer than this are "held"; `None` while warming up.
    fn held_above(&self) -> Option<f32> {
        if self.samples.len() < Self::WARM_UP {
            return None;
        }
        let median = |mut v: Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        let med = median(self.samples.iter().copied().collect());
        let mad = median(self.samples.iter().map(|x| (x - med).abs()).collect());
        Some((med * 1.8).max(med + 4.0 * mad).max(med + 60.0))
    }
}

fn timer_flag(ms: i32) -> i32 {
    ms.clamp(0, 0x7FFF) << 16
}

/// Match `cand`'s case to how `typed` was typed (Capitalized / ALL CAPS).
fn apply_case(typed: &str, cand: &str) -> String {
    let mut t = typed.chars();
    let first_upper = t.next().is_some_and(char::is_uppercase);
    if first_upper && typed.chars().count() > 1 && typed.chars().all(|c| !c.is_lowercase()) {
        return cand.to_uppercase();
    }
    if first_upper {
        let mut c = cand.chars();
        return c
            .next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default();
    }
    cand.to_string()
}

/// The record of a change that the swipe up can go round: `committed` is in
/// the text; `variants` (it first) and at the end `was`, what stood before.
fn unit(was: &str, committed: &str, variants: &[String]) -> PrevWord {
    let mut all: Vec<String> = variants.to_vec();
    if all.is_empty() {
        all.push(committed.to_string());
    }
    if !all.iter().any(|v| v == was) {
        all.push(was.to_string());
    }
    let variant = all.iter().position(|v| v == committed).unwrap_or(0);
    PrevWord {
        typed: was.to_string(),
        input: Input::typed(was, &[]),
        committed: committed.to_string(),
        alone: f32::INFINITY,
        letters: Vec::new(),
        sep: ' ',
        variants: all,
        variant,
        alts: Vec::new(),
    }
}

/// Now, ms since the epoch (the settings buttons' stamps).
fn stamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A JSON string literal.
fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Words spelled one for the other by mistake rather than mistyped: a
/// preposition and its form with о (к / ко мне, о / об этом), -тся / -ться.
fn spelling_pair(a: &str, b: &str) -> bool {
    const PREPOSITIONS: [(&str, &str); 6] = [
        ("в", "во"),
        ("с", "со"),
        ("к", "ко"),
        ("о", "об"),
        ("об", "обо"),
        ("о", "обо"),
    ];
    let tsya = |x: &str, y: &str| {
        x.strip_suffix("тся")
            .is_some_and(|stem| y.strip_suffix("ться") == Some(stem))
    };
    PREPOSITIONS
        .iter()
        .any(|&(x, y)| (a, b) == (x, y) || (a, b) == (y, x))
        || tsya(a, b)
        || tsya(b, a)
}

/// Two forms of one word, as far as the typed one tells: the shorter one
/// is the other's start but for its last two letters at most («побежал»,
/// «побежала»; «дом», «доме»; «учится», «учиться»; «бежа», «бежала»).
fn same_stem(a: &str, b: &str) -> bool {
    let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let shorter = a.chars().count().min(b.chars().count());
    common >= 2 && common + 2 >= shorter
}

/// One letter repeated (аа, ммм) or any letter three times in a row (аааа).
fn stretched(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    (chars.len() >= 2 && chars.iter().all(|&c| c == chars[0]))
        || chars.windows(3).any(|w| w[0] == w[1] && w[1] == w[2])
}

/// Keys a tap meant for a letter above may land on instead.
/// How many chars the last user-perceived character of `text` takes: an
/// emoji with its joiners, variation selectors, skin tones, keycap or tags,
/// a flag's two regional indicators; else 1.
fn last_cluster(text: &str) -> usize {
    let rev: Vec<char> = text.chars().rev().take(64).collect();
    let extends = |c: char| matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}');
    let regional = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    let mut i = 0;
    while i < rev.len() && extends(rev[i]) {
        i += 1;
    }
    if i >= rev.len() {
        return rev.len();
    }
    if regional(rev[i]) && rev.get(i + 1).is_some_and(|&c| regional(c)) {
        return i + 2;
    }
    i += 1;
    while rev.get(i) == Some(&'\u{200D}') && i + 1 < rev.len() {
        i += 1;
        while i < rev.len() && extends(rev[i]) {
            i += 1;
        }
        i += 1;
    }
    i.min(rev.len())
}

fn slip_key(action: Action) -> bool {
    matches!(
        action,
        Action::Space | Action::Char(',') | Action::Char('.')
    )
}

fn sentence_start(before: &str) -> bool {
    // Opening marks keep a sentence's start: ¿Qué, ¡Hola, «Да, „Hallo, (См.
    let open = before.trim_end_matches(|c| "¿¡«„“(\"'".contains(c));
    if open.len() != before.len() {
        return sentence_start(open);
    }
    if before.is_empty() || before.ends_with('\n') {
        return true;
    }
    let t = before.trim_end_matches(' ');
    if t.len() == before.len() {
        return false;
    }
    if t.is_empty() || t.ends_with('!') || t.ends_with('?') || t.ends_with("..") {
        return true;
    }
    let Some(body) = t.strip_suffix('.') else {
        return false;
    };
    // A period after a short form doesn't end the sentence: a lone lowercase
    // letter (т. п., г.), letters with periods inside (т.е.), a known short
    // form (ул.). Initials (А. С.) and the word «я» do.
    let token: Vec<char> = body
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic() || *c == '.')
        .collect();
    let token: String = token.into_iter().rev().collect();
    let letters = token.chars().filter(|c| c.is_alphabetic()).count();
    let short = token.chars().all(|c| !c.is_uppercase())
        && (letters == 1 && token != "я"
            || token.contains('.')
            || ABBREVIATIONS.contains(&token.as_str()));
    !short
}

impl<D: AsRef<[u8]>> Ime<D> {
    pub fn new(engine: Engine<D>, density: f32) -> Self {
        let mut ime = Ime {
            engine,
            engine_pack: "base",
            spare: Vec::new(),
            missing: Vec::new(),
            dp: density.max(0.5),
            width: 0.0,
            lang: Lang::Ru,
            layer: Layer::Letters,
            shift: Shift::Off,
            keys: Vec::new(),
            word: String::new(),
            shown: String::new(),
            hints: Vec::new(),
            durs: Vec::new(),
            clean: true,
            rhythm: Rhythm::default(),
            pending: None,
            last_down: None,
            slip_tap: None,
            prev_word: None,
            merge: None,
            selected: false,
            log_on: false,
            log: Vec::new(),
            now_ms: 0,
            last_cands: Vec::new(),
            decoded: Vec::new(),
            decoded_next: Vec::new(),
            split: None,
            slots: [None, None, None],
            grips: Grips::default(),
            sense: Sense::default(),
            calib: None,
            calib_wanted: None,
            notice: None,
            panel: None,
            stash: Stash::default(),
            emoji_max: panel::emoji_max(0),
            clip_offer: None,
            user_words: Vec::new(),
            user_dirty: false,
            dict_actions: [None, None, None],
            alternatives: Vec::new(),
            chosen: None,
            drawn: None,
            variants_shown: false,
            popup: None,
            own_field: false,
            seen_stamps: None,
            grips_dirty: false,
            window_margin: WINDOW_MARGIN,
            known_slip: KNOWN_SLIP,
            predicted: Vec::new(),
            autocorrect: None,
            undo: None,
            rejected: Vec::new(),
            comma_undo: None,
            proof_undo: None,
            whole: false,
            commas_rejected: Vec::new(),
            erased: 0,
            restored: false,
            last_bksp: None,
            held_bksp: 0,
            pointers: Vec::new(),
            timer: Timer::None,
            out: Vec::new(),
            before: String::new(),
            suggest: true,
            editing: false,
            auto_space: false,
            after: String::new(),
            private_field: false,
            hint_langs: Vec::new(),
            pair_undo: None,
            written: Vec::new(),
            recorrect: None,
            arc: None,
            arc_forced: None,
            ui: Lang::Ru,
            system_dark: true,
            wallpaper: None,
            auto_caps: true,
            settings: Settings::default(),
        };
        ime.measure(1080);
        ime
    }

    fn strip_h(&self) -> f32 {
        STRIP_DP * self.dp
    }

    fn row_h(&self) -> f32 {
        ROW_DP * self.dp * self.settings.height as f32 / 100.0
    }

    /// One-row letters: the compact layout is showing.
    fn compact(&self) -> bool {
        self.settings.one_row
            && self.layer == Layer::Letters
            && self.calib.is_none()
            && self.panel.is_none()
    }

    fn height(&self) -> f32 {
        let rows = if self.compact() { 2.0 } else { 4.0 };
        self.strip_h() + rows * self.row_h() + BOTTOM_PAD_DP * self.dp
    }

    /// The thumb the arc layout is for, if it is on: forced by a swipe, or
    /// (the setting) the grip showing one thumb.
    fn arc_hand(&self) -> Option<Hand> {
        if self.calib.is_some() || self.settings.one_row {
            return None;
        }
        match self.arc_forced {
            Some(forced) => forced,
            None if self.settings.arc_layout => match self.sense.shown() {
                Some(Grip::Left) => Some(Hand::Left),
                Some(Grip::Right) => Some(Hand::Right),
                _ => None,
            },
            None => None,
        }
    }

    /// Each key row's stretch the calibrated thumb reaches, px — from the
    /// painted area, or (an older calibration) from its arc. None without one.
    fn reach_rows(&self, hand: Hand) -> Option<Vec<(f32, f32)>> {
        let grip = match hand {
            Hand::Left => Grip::Left,
            Hand::Right => Grip::Right,
        };
        let w = self.width;
        if let Some(rows) = self.grips.rows(grip) {
            return Some(rows.iter().map(|&(a, b)| (a * w, b * w)).collect());
        }
        let (pu, pv, r) = self.grips.reach(grip)?;
        let row_v = self.row_h() / w;
        Some(
            (0..4)
                .map(|i| {
                    let dv = pv - (i as f32 + 0.5) * row_v;
                    let half = (r * r - dv * dv).max(0.0).sqrt();
                    match hand {
                        Hand::Left => (0.0, (pu + half).min(1.0) * w),
                        Hand::Right => ((pu - half).max(0.0) * w, w),
                    }
                })
                .collect(),
        )
    }

    /// A swipe up on ?123 (left) or Enter (right): the one-handed layout for
    /// that thumb, or back to the usual one if it is on already.
    fn toggle_arc(&mut self, hand: Hand) -> i32 {
        if self.arc != Some(hand) && self.reach_rows(hand).is_none() {
            self.notice = Some(t(self.ui, "arc.need_calibration").into());
            self.slots = [None, None, None];
            return REDRAW | HAPTIC;
        }
        self.arc_forced = Some(if self.arc == Some(hand) {
            None
        } else {
            Some(hand)
        });
        self.relayout();
        REDRAW | RELAYOUT | HAPTIC
    }

    fn relayout(&mut self) {
        if self.panel.is_some() {
            // The panel's page over the letter rows, its keys below.
            self.arc = None;
            let top = self.strip_h() + 3.0 * self.row_h();
            self.keys = layout::build_panel_row(self.width, top, self.row_h());
            return;
        }
        self.arc = if self.compact() {
            None
        } else {
            self.arc_hand()
        };
        // Only with a calibrated reach for that thumb.
        self.arc = self.arc.filter(|&h| self.reach_rows(h).is_some());
        let calib_rows = self
            .calib
            .as_ref()
            .filter(|c| {
                self.settings.arc_layout
                    && matches!(c.step(), Some(Step::Type(Grip::Left | Grip::Right)))
            })
            .and_then(|c| c.rows_now(self.row_h() / self.width))
            .map(|rows| {
                rows.iter()
                    .map(|&(a, b)| (a * self.width, b * self.width))
                    .collect()
            });
        if let Some(rows) = calib_rows.or_else(|| self.arc.and_then(|h| self.reach_rows(h))) {
            self.keys = layout::build_reach(
                self.lang,
                self.layer,
                self.width,
                self.strip_h(),
                self.row_h(),
                self.lang_key(),
                |r| rows[r.min(rows.len() - 1)],
            );
            return;
        }
        self.keys = if self.compact() {
            layout::build_compact(
                self.lang,
                self.width,
                self.strip_h(),
                self.row_h(),
                self.lang_key(),
            )
        } else {
            layout::build(
                self.lang,
                self.layer,
                self.width,
                self.strip_h(),
                self.row_h(),
                self.lang_key(),
            )
        };
    }

    fn lang_key(&self) -> bool {
        self.settings.lang_switch != LangSwitch::Swipe
    }

    /// Allow debug logging in this editor field (the shim allows it only in
    /// the app's own test field); it also has to be on in the settings.
    pub fn set_logging(&mut self, on: bool) {
        self.log_on = on;
    }

    fn logging(&self) -> bool {
        self.log_on && self.settings.debug_log
    }

    /// An obscene word the settings keep out of suggestions and corrections
    /// — unless it is exactly what was typed.
    fn blocked(&self, word: &str, typed: &str) -> bool {
        self.settings.block_offensive && word != typed && self.engine.offensive(word)
    }

    /// The language of the keyboard's own words (strip prompts, notices).
    pub fn set_ui(&mut self, ui: Lang) {
        self.ui = ui;
    }

    /// The phone's dark mode and wallpaper palettes (8 colors light, then 8
    /// dark).
    pub fn set_system_colors(&mut self, dark: bool, wallpaper: Option<&[i32]>) {
        self.system_dark = dark;
        self.wallpaper = wallpaper.and_then(|c| {
            let (l, d) = c.split_at(c.len().min(8));
            Some((Palette::from_slice(l)?, Palette::from_slice(d)?))
        });
    }

    /// The colors to draw with: the chosen theme, from the wallpaper if asked.
    pub fn palette(&self) -> Palette {
        let dark = match self.settings.theme {
            Theme::System => self.system_dark,
            Theme::Light => false,
            Theme::Dark => true,
        };
        match self.wallpaper.filter(|_| self.settings.wallpaper_colors) {
            Some((light, night)) => {
                if dark {
                    night
                } else {
                    light
                }
            }
            None if dark => DARK,
            None => LIGHT,
        }
    }

    /// The background color (the shim paints the navigation bar with it).
    pub fn background(&self) -> i32 {
        self.palette().bg
    }

    /// The languages the app suggests for the field (`EditorInfo.hintLocales`
    /// as codes, `es,en`): set before [`Ime::start_input`].
    pub fn set_hint_languages(&mut self, codes: &str) {
        self.hint_langs = codes
            .split(',')
            .filter_map(|c| Lang::from_code(c.trim().split(['-', '_']).next().unwrap_or("")))
            .collect();
    }

    /// The language for a new field: the one the app asks for, if it is
    /// enabled; else the one of the text at the cursor.
    fn lang_for_field(&mut self) {
        let hinted = self
            .hint_langs
            .iter()
            .find(|l| self.settings.languages.contains(l))
            .copied();
        match hinted {
            Some(l) => self.set_lang(l),
            None => {
                self.lang_from_text();
            }
        }
    }

    /// The cursor is on (or right after) a word of another script than the
    /// layout's: switch to that script's language. Between Latin languages
    /// only when the current one doesn't know the word and another does.
    fn lang_from_text(&mut self) -> bool {
        if !self.settings.lang_from_text || self.calib.is_some() || !self.suggest {
            return false;
        }
        let tail = self.before.trim_end_matches(' ');
        let word: String = tail
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic() || *c == '\'' || *c == '-')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
            .to_lowercase();
        let word = word.trim_matches(['\'', '-']);
        if word.chars().count() < 2 {
            return false;
        }
        let cyrillic = word.chars().any(|c| matches!(c, 'а'..='я' | 'ё'));
        let latin = word.chars().all(|c| !matches!(c, 'а'..='я' | 'ё'));
        let enabled = self.settings.languages.clone();
        let want = if cyrillic && self.lang != Lang::Ru {
            enabled.contains(&Lang::Ru).then_some(Lang::Ru)
        } else if latin && (self.lang == Lang::Ru || !self.knows(self.lang, word)) {
            let latin_langs: Vec<Lang> = enabled.into_iter().filter(|&l| l != Lang::Ru).collect();
            latin_langs
                .iter()
                .find(|&&l| self.knows(l, word))
                .or(latin_langs.first().filter(|_| self.lang == Lang::Ru))
                .copied()
        } else {
            None
        };
        match want {
            Some(l) if l != self.lang => {
                self.set_lang(l);
                true
            }
            _ => false,
        }
    }

    /// Whether the dictionary of `lang`'s pack (open or not yet) has `word`.
    fn knows(&self, lang: Lang, word: &str) -> bool {
        let pack = lang.pack();
        if pack == self.engine_pack {
            self.engine.contains(word)
        } else {
            self.spare
                .iter()
                .find(|(p, _)| *p == pack)
                .is_some_and(|(_, e)| e.contains(word))
        }
    }

    /// Type in `lang` from now on (its layout and dictionary), on the same page.
    fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
        self.use_pack();
        self.relayout();
    }

    /// The field is private (`EditorInfo.imeOptions` has
    /// `IME_FLAG_NO_PERSONALIZED_LEARNING`): set before [`Ime::start_input`].
    pub fn set_private(&mut self, private: bool) {
        self.private_field = private;
    }

    /// The start of the text after the cursor (the shim passes it on focus
    /// and on every cursor move).
    pub fn set_after(&mut self, text: &str) {
        self.after = text.chars().take(32).collect();
    }

    /// How much likelier a re-read previous word must be (benchmarks;
    /// infinity turns the two-word window off).
    pub fn set_window_margin(&mut self, margin: f32) {
        self.window_margin = margin;
    }

    /// How cheap a slip must be to replace a typed real word (benchmarks).
    pub fn set_known_slip(&mut self, slip: f32) {
        self.known_slip = slip;
    }

    /// Whether the focused field is the app's own test field (calibration
    /// runs only there).
    pub fn set_own_field(&mut self, own: bool) {
        self.own_field = own;
    }

    /// The user's own words, as saved (`user_words.txt`, one per line).
    pub fn load_user_words(&mut self, text: &str) {
        self.user_words = text
            .lines()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect();
        self.engine.set_user_words(&self.user_words);
    }

    /// The user's words to save, when they changed.
    pub fn take_user_words(&mut self) -> Option<String> {
        std::mem::take(&mut self.user_dirty)
            .then(|| self.user_words.iter().map(|w| format!("{w}\n")).collect())
    }

    /// A single word the dictionary could take: letters (and inner ' or -)
    /// of the keyboard's alphabets, at most 32.
    fn dictionary_word(text: &str) -> Option<&str> {
        let w = text.trim();
        let ok = (1..=32).contains(&w.chars().count())
            && w.chars().any(char::is_alphabetic)
            && w.chars().all(|c| {
                kbcore::alphabet::char_to_id(c.to_lowercase().next().unwrap_or(c)).is_some()
                    && (c.is_alphabetic() || c == '-' || c == '\'')
            });
        ok.then_some(w)
    }

    /// The strip's left slot shows the word exactly as typed (hold or drag
    /// it up to add it to the dictionary).
    fn left_is_typed(&self) -> bool {
        !self.word.is_empty()
            && self.slots[0]
                .as_deref()
                .is_some_and(|s| s.to_lowercase() == self.word.to_lowercase())
    }

    fn is_user_word(&self, w: &str) -> bool {
        let lower = w.to_lowercase();
        self.user_words.iter().any(|u| u.to_lowercase() == lower)
    }

    /// Add a word to the user's dictionary (only ever on an explicit request).
    fn add_user_word(&mut self, text: &str) -> bool {
        let Some(w) = Self::dictionary_word(text).map(String::from) else {
            return false;
        };
        if self.is_user_word(&w) {
            self.notice = Some(t(self.ui, "dict.already").replace("{}", &w));
            return false;
        }
        self.log_event("dict-add", &[("word", jstr(&w))]);
        self.user_words.push(w.clone());
        self.engine.set_user_words(&self.user_words);
        self.user_dirty = true;
        self.notice = Some(t(self.ui, "dict.added").replace("{}", &w));
        true
    }

    fn remove_user_word(&mut self, w: &str) {
        let lower = w.to_lowercase();
        self.user_words.retain(|u| u.to_lowercase() != lower);
        self.engine.set_user_words(&self.user_words);
        self.user_dirty = true;
        self.notice = Some(t(self.ui, "dict.removed").replace("{}", w));
    }

    /// The calibrated grips, as saved (`grip.conf`).
    pub fn load_grips(&mut self, text: &str) {
        self.grips = Grips::from_text(text);
        self.sense.reset();
    }

    /// The grips to save, when they changed.
    pub fn take_grips(&mut self) -> Option<String> {
        std::mem::take(&mut self.grips_dirty).then(|| self.grips.to_text())
    }

    /// An accelerometer sample (the shim sends them while the keyboard is up).
    pub fn tilt(&mut self, x: f32, y: f32, z: f32) -> i32 {
        let changed = self.sense.on_tilt(&self.grips, [x, y, z]);
        let mut flags = if changed && self.settings.grip_indicator {
            REDRAW
        } else {
            0
        };
        // The arc follows the grip — between words, with no finger down.
        if changed
            && self.pointers.is_empty()
            && self.word.is_empty()
            && self.arc_hand() != self.arc
        {
            self.relayout();
            flags |= REDRAW | RELAYOUT;
        }
        flags
    }

    /// Lay out for a view `width_px` wide; returns the keyboard height in px.
    pub fn measure(&mut self, width_px: i32) -> i32 {
        self.width = width_px.max(1) as f32;
        self.relayout();
        self.height().ceil() as i32
    }

    /// Apply user settings (from the settings screen).
    pub fn set_settings(&mut self, settings: Settings) {
        if settings.one_row != self.engine.config().one_row_input {
            let mut cfg = self.engine.config().clone();
            cfg.one_row_input = settings.one_row;
            self.engine.set_config(cfg);
        }
        let now = stamp_ms();
        let seen = self.seen_stamps;
        // Reset all: a request newer than the last one applied.
        let reset_seen = seen.map_or(self.grips.stamp, |s| s.2);
        if settings.grip_reset > reset_seen && settings.grip_reset > self.grips.stamp {
            self.grips = Grips::default();
            self.grips.stamp = settings.grip_reset;
            self.grips_dirty = true;
            self.sense.reset();
        }
        for g in Grip::ALL {
            let i = g.index();
            // Undo: a request newer than the last one applied to this grip.
            let undo_seen = seen.map_or(0, |s| s.1[i]).max(self.grips.undone(g));
            if settings.grip_undo[i] > undo_seen {
                self.grips.undo(g, settings.grip_undo[i]);
                self.grips_dirty = true;
                self.sense.reset();
            }
            // Calibrate: a newer request — or, just started (the keyboard may
            // have been asleep when the button was pressed), one from the last
            // minutes that no calibration answered yet.
            let asked = settings.calibrate[i];
            let wanted = match seen {
                Some(s) => asked > s.0[i],
                None => asked > self.grips.made(g) && now.saturating_sub(asked) < 3 * 60 * 1000,
            };
            if wanted {
                self.calib_wanted = Some(g);
            }
        }
        self.seen_stamps = Some((settings.calibrate, settings.grip_undo, settings.grip_reset));
        if !settings.clipboard {
            self.stash.clips.clear();
            self.clip_offer = None;
        }
        self.settings = settings;
        // A language taken out of the list: on to the first one left.
        if !self.settings.languages.contains(&self.lang) && self.calib.is_none() {
            self.lang = self.settings.languages.first().copied().unwrap_or(Lang::Ru);
            self.use_pack();
        }
        self.relayout();
    }

    /// Data packs the enabled languages need that aren't open yet (the
    /// shim installs them and calls [`Ime::add_pack`]).
    pub fn wanted_packs(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for l in &self.settings.languages {
            let p = l.pack();
            let open = p == self.engine_pack || self.spare.iter().any(|(s, _)| *s == p);
            if !open && !self.missing.contains(&p) && !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// A language's data pack, opened (`None`: the app doesn't have it —
    /// that language types with the base dictionary).
    pub fn add_pack(&mut self, pack: &'static str, engine: Option<Engine<D>>) {
        match engine {
            Some(e) => {
                self.spare.retain(|(p, _)| *p != pack);
                self.spare.push((pack, e));
                self.use_pack();
            }
            None => self.missing.push(pack),
        }
    }

    /// Swap in the current language's engine, if it is open.
    fn use_pack(&mut self) {
        let want = self.lang.pack();
        if want == self.engine_pack {
            return;
        }
        let Some(i) = self.spare.iter().position(|(p, _)| *p == want) else {
            return;
        };
        let (pack, engine) = self.spare.swap_remove(i);
        let old = std::mem::replace(&mut self.engine, engine);
        self.spare.push((self.engine_pack, old));
        self.engine_pack = pack;
        if self.settings.one_row != self.engine.config().one_row_input {
            let mut cfg = self.engine.config().clone();
            cfg.one_row_input = self.settings.one_row;
            self.engine.set_config(cfg);
        }
        self.engine.set_user_words(&self.user_words);
        self.refresh_predictions();
    }

    /// Start a calibration that was asked for, once the own test field has
    /// focus. Returns whether it started.
    fn maybe_calibrate(&mut self) -> bool {
        if !self.own_field || self.calib.is_some() {
            return false;
        }
        let Some(grip) = self.calib_wanted.take() else {
            return false;
        };
        self.commit_word(false);
        self.notice = None;
        self.lang = self.settings.languages.first().copied().unwrap_or(Lang::Ru);
        let known = Grip::ALL.map(|g| self.grips.reach(g));
        self.calib = Some(Calibration::new(phrase_for(self.lang), grip, known));
        self.use_pack();
        self.layer = Layer::Letters;
        self.pointers.clear();
        self.timer = Timer::None;
        self.slots = [None, None, None];
        self.relayout();
        true
    }

    /// `(x, y)` in keyboard widths from the top-left of the keys area.
    fn to_uv(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.width, (y - self.strip_h()) / self.width)
    }

    /// A touch during the calibration: arcs are drawn, the phrase is typed
    /// (nothing reaches the text field); the strip's right end cancels.
    fn calibration_touch(&mut self, action: i32, x: f32, y: f32) -> i32 {
        self.notice = None;
        // Only a touch that starts in the strip is for the strip: an arc may
        // run up into it.
        let in_strip = y < self.strip_h();
        if let (DOWN, Some(cal)) = (action, self.calib.as_mut()) {
            cal.strip_touch = in_strip;
        }
        if self.calib.as_ref().is_some_and(|c| c.strip_touch) {
            if action == UP && in_strip && x > self.width * 2.0 / 3.0 {
                // "Done" with the painted area: the phrase next — on the
                // one-handed rows it makes, if they are in use.
                if let Some(cal) = self
                    .calib
                    .as_mut()
                    .filter(|c| matches!(c.step(), Some(Step::Area(_))))
                {
                    cal.area_done();
                    self.relayout();
                    return REDRAW | RELAYOUT;
                }
                self.calib = None;
                self.notice = Some(t(self.ui, "cal.cancelled").into());
                self.relayout();
                return REDRAW | RELAYOUT;
            }
            return REDRAW;
        }
        let (u, v) = self.to_uv(x, y);
        let tilt = self.sense.tilt();
        let key = match self.hit(x, y) {
            Some(Target::Key(k)) => Some(self.keys[k].action),
            _ => None,
        };
        let key_u = self
            .keys
            .iter()
            .find(|k| matches!(k.action, Action::Char(_)))
            .map_or(1.0 / 11.0, |k| k.w / self.width);
        let centers = self.phrase_centers();
        let Some(cal) = self.calib.as_mut() else {
            return 0;
        };
        let mut phrase_done = false;
        match (cal.step(), action) {
            (Some(Step::Area(_)), DOWN) => {
                cal.arc_start();
                cal.arc_point(u, v, tilt);
            }
            (Some(Step::Area(_)), MOVE) => cal.arc_point(u, v, tilt),
            (Some(Step::Area(_)), UP) => {
                cal.arc_point(u, v, tilt);
                cal.arc_end();
            }
            (Some(Step::Type(_)), UP) => match key {
                Some(Action::Backspace) => cal.back(&centers, key_u),
                Some(Action::Char(_) | Action::Space) => {
                    phrase_done = cal.tap(u, v, tilt, &centers, key_u)
                }
                _ => {}
            },
            _ => {}
        }
        if phrase_done {
            if let Some(cal) = self.calib.as_mut() {
                cal.phrase_done(&centers, key_u);
            }
        }
        if self.calib.as_ref().is_some_and(Calibration::done) {
            return self.calibration_finished();
        }
        REDRAW
    }

    fn calibration_finished(&mut self) -> i32 {
        if !self.settings.languages.contains(&self.lang) {
            self.lang = self.settings.languages.first().copied().unwrap_or(Lang::Ru);
            self.use_pack();
        }
        let row_v = self.row_h() / self.width;
        if let Some(cal) = self.calib.take() {
            if let (Some(g), Some(profile)) = (cal.grip(), cal.finish(row_v)) {
                self.grips.set(g, profile, stamp_ms());
            }
        }
        self.grips_dirty = true;
        self.sense.reset();
        self.notice = Some(t(self.ui, "cal.saved").into());
        self.relayout();
        REDRAW | RELAYOUT
    }

    /// Key centers of the calibration phrase's chars, in keyboard widths.
    fn phrase_centers(&self) -> std::collections::HashMap<char, (f32, f32)> {
        let phrase = self.calib.as_ref().map_or("", Calibration::phrase);
        phrase
            .chars()
            .filter_map(|c| {
                let action = if c == ' ' {
                    Action::Space
                } else {
                    Action::Char(c)
                };
                let (x, y) = self.key_center(action)?;
                Some((c, self.to_uv(x, y)))
            })
            .collect()
    }

    /// What the strip says instead of suggestions, if anything: the
    /// calibration prompt (with a way out), or a notice.
    fn strip_prompt(&self) -> Option<(String, Option<&'static str>)> {
        let Some(cal) = &self.calib else {
            return self.notice.clone().map(|n| (n, None));
        };
        let text = match cal.step()? {
            Step::Area(_) if cal.retry => t(self.ui, "cal.more").to_string(),
            Step::Area(g) => t(self.ui, "cal.area").replace("{}", t(self.ui, g.hands())),
            Step::Type(g) => {
                let rest: String = cal.phrase().chars().skip(cal.typed()).take(24).collect();
                format!("{}: {rest}", t(self.ui, g.hands()))
            }
        };
        let out = t(
            self.ui,
            match cal.step()? {
                Step::Area(_) => "cal.done",
                _ => "cal.cancel",
            },
        );
        Some((text, Some(out)))
    }

    /// Where a tap landing at `(x, y)` was likely aimed: moved back by where
    /// the calibrated grip usually lands (for the letters' odds only).
    fn grip_shift(&self, x: f32, y: f32) -> (f32, f32) {
        if !self.settings.grip_offsets || !self.grips.calibrated() || y < self.strip_h() {
            return (x, y);
        }
        let (u, v) = self.to_uv(x, y);
        // The one-handed rows keep every key within reach: the offset field
        // (where on the screen taps land), not the fall short past the reach.
        let (du, dv) = if self.arc.is_some() {
            self.grips.shift_near(self.sense.probs(), u, v)
        } else {
            self.grips.shift(self.sense.probs(), u, v)
        };
        (x - du * self.width, y - dv * self.width)
    }

    /// A new editor field got focus. `input_type` is Android's `EditorInfo.inputType`.
    pub fn start_input(&mut self, text_before: &str, input_type: i32) -> i32 {
        self.selected = false;
        let class = input_type & 0xF;
        let variation = input_type & 0xFF0;
        let flags = input_type & !0xFFF;
        const TEXT_CLASS: i32 = 1;
        // Passwords, e-mail addresses, URIs: no suggestions, no autocorrect.
        const NO_SUGGEST_VARIATIONS: [i32; 6] = [0x80, 0x90, 0xE0, 0x20, 0xD0, 0x10];
        const FLAG_NO_SUGGESTIONS: i32 = 0x80000;
        const FLAG_CAPS: i32 = 0x1000 | 0x2000 | 0x4000;
        // A private window asks keyboards not to learn — this one learns
        // nothing anyway, so it keeps its suggestions and corrections there,
        // and ignores what apps add to keep other keyboards from learning
        // (no suggestions, a "visible password" that isn't one).
        const VISIBLE_PASSWORD: i32 = 0x90;
        let private = self.private_field;
        self.suggest = class == TEXT_CLASS
            && !(NO_SUGGEST_VARIATIONS.contains(&variation)
                && !(private && variation == VISIBLE_PASSWORD))
            && (flags & FLAG_NO_SUGGESTIONS == 0 || private);
        self.auto_caps = class == TEXT_CLASS && flags & FLAG_CAPS != 0 && self.settings.auto_caps;
        // Numbers and phone numbers open on the symbols layer.
        self.layer = if class == 2 || class == 3 {
            Layer::Symbols
        } else {
            Layer::Letters
        };
        self.word.clear();
        self.shown.clear();
        self.hints.clear();
        self.durs.clear();
        self.autocorrect = None;
        self.undo = None;
        self.editing = false;
        self.leave_word();
        self.prev_word = None;
        self.merge = None;
        self.slots = [None, None, None];
        self.pointers.clear();
        self.timer = Timer::None;
        if self.panel.take().is_some() {
            self.stash.settle();
        }
        self.editor_before(text_before);
        self.written.clear();
        self.comma_undo = None;
        self.proof_undo = None;
        self.commas_rejected.clear();
        self.recorrect = None;
        self.lang_for_field();
        self.shift = Shift::Off;
        self.update_shift();
        self.relayout();
        self.refresh_predictions();
        self.maybe_calibrate();
        REDRAW | RELAYOUT
    }

    fn set_before(&mut self, text: &str) {
        let n = text.chars().count();
        if n > BEFORE_CHARS {
            self.whole = false;
        }
        self.before = text.chars().skip(n.saturating_sub(BEFORE_CHARS)).collect();
    }

    /// The text before the cursor as the editor sent it: less than asked
    /// for, it starts where the field does.
    fn editor_before(&mut self, text: &str) {
        self.whole = text.encode_utf16().count() < BEFORE_CHARS;
        self.set_before(text);
    }

    /// The editor's selection changed; `text_before` is the text before the
    /// cursor (it includes our composing word, if any). If the cursor left the
    /// word being composed (the user tapped elsewhere), drop it.
    #[allow(clippy::too_many_arguments)]
    pub fn selection(
        &mut self,
        sel_start: i32,
        sel_end: i32,
        cand_start: i32,
        cand_end: i32,
        text_before: &str,
        text_after: &str,
        selected_text: Option<&str>,
    ) -> i32 {
        let _ = cand_start;
        self.set_after(text_after);
        self.selected = sel_start != sel_end;
        let flags = self.selection_inner(sel_start, sel_end, cand_end, text_before);
        // A selected word: offer to add it to (or take it out of) the user's
        // dictionary, in the strip's center.
        // Only a word the keyboard doesn't know yet (to add), or one the
        // user added (to take out) — a dictionary word needs nothing.
        let word = selected_text
            .filter(|_| self.selected && self.suggest)
            .and_then(Self::dictionary_word)
            .map(String::from)
            .filter(|w| self.is_user_word(w) || !self.engine.contains(w));
        let had = self.dict_actions.iter().any(Option::is_some);
        self.dict_actions = [None, None, None];
        if let Some(w) = word {
            if self.is_user_word(&w) {
                self.dict_actions[1] = Some((w.clone(), false));
                self.slots = [
                    None,
                    Some(format!("{}{w}", t(self.ui, "dict.remove"))),
                    None,
                ];
            } else if w.chars().any(char::is_uppercase) {
                // A capital: a name (always written so) or an ordinary word
                // that began a sentence — the likelier one in the center.
                let plain = w.to_lowercase();
                let name = (
                    Some((w.clone(), true)),
                    Some(format!("{DICT_PLAIN}{w}{}", t(self.ui, "dict.as_name"))),
                );
                let word = (
                    Some((plain.clone(), true)),
                    Some(format!("{DICT_PLAIN}{plain}")),
                );
                let (side, center) = if sentence_start(text_before) {
                    (name, word)
                } else {
                    (word, name)
                };
                self.dict_actions = [side.0, center.0, None];
                self.slots = [side.1, center.1, None];
            } else {
                self.dict_actions[1] = Some((w.clone(), true));
                self.slots = [None, Some(format!("{}{w}", t(self.ui, "dict.add"))), None];
            }
            return flags | REDRAW;
        }
        if had {
            self.refresh_predictions();
            return flags | REDRAW;
        }
        flags
    }

    fn selection_inner(
        &mut self,
        sel_start: i32,
        sel_end: i32,
        cand_end: i32,
        text_before: &str,
    ) -> i32 {
        if !self.word.is_empty() && (cand_end < 0 || sel_start != sel_end || sel_start != cand_end)
        {
            self.word.clear();
            self.shown.clear();
            self.hints.clear();
            self.durs.clear();
            self.autocorrect = None;
            self.slots = [None, None, None];
            self.editing = false;
            self.forget_spot();
            self.out.push(Op::Finish);
            self.editor_before(text_before);
            return REDRAW | OUTPUT;
        }
        let text = text_before
            .strip_suffix(self.shown.as_str())
            .unwrap_or(text_before);
        // The text before the cursor isn't what we left there: the user
        // moved the cursor (or the app changed the text). Only the tail is
        // compared — the shim's text may cut an emoji at its start.
        let tail = |t: &str| t.chars().rev().take(16).collect::<String>();
        let moved = self.word.is_empty() && tail(text) != tail(&self.before);
        self.editor_before(text);
        if moved {
            self.forget_spot();
            self.lang_from_text();
            self.refresh_predictions();
            self.offer_recorrect();
            self.update_shift();
            return REDRAW;
        }
        0
    }

    /// The cursor was put in (or right after) a word: its other readings in
    /// the strip, the likeliest in the center — from how it was typed when
    /// this field saw it typed (letters and touches), else from its letters.
    /// A pick replaces the whole word.
    fn offer_recorrect(&mut self) {
        self.recorrect = None;
        if !self.suggest || self.selected || !self.word.is_empty() || self.calib.is_some() {
            return;
        }
        let part = |c: &char| c.is_alphabetic() || *c == '\'' || *c == '-';
        let left: String = {
            let rev: Vec<char> = self.before.chars().rev().take_while(part).collect();
            rev.into_iter().rev().collect()
        };
        let right: String = self.after.chars().take_while(part).collect();
        let word = format!("{left}{right}");
        if word.chars().count() < 2 {
            return;
        }
        let lower = word.to_lowercase();
        let input = self
            .written
            .iter()
            .rev()
            .find(|w| w.text == lower)
            .map_or_else(|| Input::typed(&lower, &[]), |w| w.input.clone());
        // What the finger gave, as letters (a drawn word: the word read).
        let typed = match &input {
            Input::Typed { letters, .. } => letters.clone(),
            Input::Drawn { .. } => lower.clone(),
        };
        let head = self.before.chars().count() - left.chars().count();
        let prefix: String = self.before.chars().take(head).collect();
        let mut cands = self.decode_input(&self.context_at(&prefix), &input);
        cands.retain(|c| !self.blocked(&c.word, &typed));
        let mut options: Vec<String> = Vec::new();
        for w in cands
            .into_iter()
            .map(|c| c.word)
            .chain(std::iter::once(typed))
        {
            if w != lower && !options.contains(&w) {
                options.push(w);
            }
        }
        if options.is_empty() || self.settings.strip == Strip::Off {
            return;
        }
        let cased: Vec<String> = options
            .iter()
            .take(3)
            .map(|w| self.cased(&word, w))
            .collect();
        self.slots = [
            cased.get(1).cloned(),
            cased.first().cloned(),
            cased.get(2).cloned(),
        ];
        self.recorrect = Some((left.chars().count(), right.chars().count()));
    }

    /// The cursor left the spot: an undo, a join with the previous word or
    /// the word before it no longer refer to the text around it.
    fn forget_spot(&mut self) {
        self.undo = None;
        self.prev_word = None;
        self.merge = None;
        self.split = None;
        self.auto_space = false;
        self.leave_word();
    }

    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if y < 0.0 {
            return None;
        }
        if let Some(panel) = &self.panel {
            let frame = self.panel_frame();
            if y < frame.page_bottom {
                return frame.hit(panel.tab, x, y).map(Target::Panel);
            }
        }
        if y < self.strip_h() {
            let slot = ((x / self.width) * 3.0).floor().clamp(0.0, 2.0) as usize;
            return self.slots[slot].as_ref().map(|_| Target::Slot(slot));
        }
        // Nearest key: taps in the gaps between keys still land somewhere.
        self.keys
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.dist2(x, y).total_cmp(&b.1.dist2(x, y)))
            .map(|(i, _)| Target::Key(i))
    }

    /// Where a finger landing at `(x, y)` presses: [`Self::hit`], except that
    /// the top half of Enter right after ⌫ is ⌫ again.
    fn hit_down(&self, x: f32, y: f32, time_ms: i64) -> Option<Target> {
        let t = self.hit(x, y);
        let Some(Target::Key(k)) = t else {
            return t;
        };
        // ?123 hit on its top edge or its upper right corner in the middle
        // of a word: the finger reaching for я came down short — when the
        // word goes on with it («пр» → «пря…»; not «ок» → «окя»: «ок?»).
        // (Not its upper half: a tap a little above the middle is ?123.)
        let symbols = &self.keys[k];
        if symbols.action == Action::Symbols && self.suggest && !self.word.is_empty() {
            let (lx, ly) = symbols.local(x, y);
            if ly < -0.3 * symbols.h || (lx > 0.25 * symbols.w && ly < -0.1 * symbols.h) {
                let letter = self
                    .keys
                    .iter()
                    .enumerate()
                    .filter(|(i, b)| *i != k && matches!(b.action, Action::Char(_)))
                    .min_by(|a, b| a.1.dist2(x, y).total_cmp(&b.1.dist2(x, y)));
                if let Some((i, Action::Char(c))) = letter.map(|(i, b)| (i, b.action)) {
                    if self.engine.is_prefix(&format!("{}{c}", self.word)) {
                        return Some(Target::Key(i));
                    }
                }
            }
        }
        let enter = &self.keys[k];
        if enter.action != Action::Enter
            || !self.last_bksp.is_some_and(|b| time_ms - b < BKSP_ENTER_MS)
            || y > enter.y + enter.h * 0.5
        {
            return t;
        }
        self.keys
            .iter()
            .position(|b| {
                b.action == Action::Backspace && x >= b.x - b.w * 0.25 && x < b.x + b.w * 1.25
            })
            .map(Target::Key)
            .or(t)
    }

    /// A touch event; `time_ms` is the event time (any monotonic clock).
    pub fn touch(&mut self, action: i32, id: i32, x: f32, y: f32, time_ms: i64) -> i32 {
        self.now_ms = time_ms;
        if action != MOVE {
            let ev = ["down", "move", "up", "cancel"]
                .get(action as usize)
                .copied()
                .unwrap_or("?");
            let key = self.key_label(x, y);
            let fields = [
                ("id", id.to_string()),
                ("x", format!("{x:.0}")),
                ("y", format!("{y:.0}")),
                ("key", jstr(&key)),
            ];
            self.log_event(ev, &fields);
        }
        let height = self.height();
        let mut flags = self.touch_inner(action, id, x, y, time_ms);
        if self.height() != height {
            flags |= RELAYOUT;
        }
        self.gate(flags)
    }

    /// What a press on letter key `k` tells the engine: keys slid across, held
    /// still longer than the user's rhythm (`up_ms`: when it lifted, if it
    /// did), and where exactly it touched.
    fn press_hint(&self, p: &Pointer, k: usize, up_ms: Option<i64>) -> PressInfo {
        let slid: Vec<char> = p
            .path
            .iter()
            .filter(|&&j| j != k)
            .filter_map(|&j| match self.keys[j].action {
                Action::Char(c) => Some(c),
                _ => None,
            })
            .collect();
        let rollover = p.rolled;
        if !slid.is_empty() {
            return (
                Hint {
                    slid,
                    rollover,
                    ..Hint::default()
                },
                None,
            );
        }
        let spatial = match self.keys[k].action {
            Action::Column(col) => self.column_costs(p.start_x, col as usize),
            _ if self.settings.touch_points => self.spatial_costs(p.aim.0, p.aim.1, k),
            _ => Vec::new(),
        };
        let Some(up_ms) = up_ms else {
            return (
                Hint {
                    spatial,
                    rollover,
                    ..Hint::default()
                },
                None,
            );
        };
        let ms = (up_ms - p.down_ms).max(0) as f32;
        let held = self.rhythm.held_above().is_some_and(|limit| ms > limit);
        (
            Hint {
                held,
                slid,
                rollover,
                spatial,
                kept: 0.0,
            },
            Some(ms),
        )
    }

    /// Substitution costs of the letters around a touch at `(x, y)` that landed
    /// on key `typed`, from the Gaussian touch model on the real key centers:
    /// `base_sub + (d²(tap, letter) − d²(tap, typed)) / 2σ²`. On the border of
    /// two keys both are nearly free; dead-center, the neighbors are dear.
    fn spatial_costs(&self, x: f32, y: f32, typed: usize) -> Vec<(char, f32)> {
        let cfg = self.engine.config();
        // Distances in QWERTY key widths — the unit σ is calibrated in.
        let unit = self.width / 10.0;
        let two_sigma2 = 2.0 * cfg.sigma * cfg.sigma;
        // Near the edge of a thumb's reach, a key farther out along the line
        // from the thumb is a likelier aim than one beside it.
        let spread = if self.settings.grip_offsets && self.grips.calibrated() && self.arc.is_none()
        {
            let (u, v) = self.to_uv(x, y);
            self.grips.spread(self.sense.probs(), u, v)
        } else {
            None
        };
        let d2 = |key: &Key| {
            let (cx, cy) = key.center();
            let (dx, dy) = ((cx - x) / unit, (cy - y) / unit);
            match spread {
                Some(((ex, ey), f)) => {
                    let along = dx * ex + dy * ey;
                    let across = -dx * ey + dy * ex;
                    let along = if along > 0.0 { along / f } else { along };
                    along * along + across * across
                }
                None => dx * dx + dy * dy,
            }
        };
        let base = d2(&self.keys[typed]);
        self.keys
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != typed)
            .filter_map(|(_, key)| match key.action {
                Action::Char(c) if c.is_alphabetic() => {
                    let d = d2(key);
                    (d <= 6.25).then(|| {
                        (
                            c,
                            cfg.base_sub + ((d - base).max(0.0) / two_sigma2).min(cfg.max_sub),
                        )
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// One-row layout: costs of every letter for a tap at `x` on column key
    /// `typed` — only the horizontal distance to each column counts, and all
    /// letters of the tapped column are equally likely.
    fn column_costs(&self, x: f32, typed: usize) -> Vec<(char, f32)> {
        let cfg = self.engine.config();
        let unit = self.width / 10.0;
        let two_sigma2 = 2.0 * cfg.sigma * cfg.sigma;
        let centers: Vec<(usize, f32)> = self
            .keys
            .iter()
            .filter_map(|k| match k.action {
                Action::Column(c) => Some((c as usize, k.center().0)),
                _ => None,
            })
            .collect();
        let Some(&(_, typed_x)) = centers.iter().find(|(c, _)| *c == typed) else {
            return Vec::new();
        };
        let dt = ((x - typed_x) / unit).powi(2);
        let columns = layout::columns(self.lang);
        let mut out = Vec::new();
        for (c, cx) in centers {
            let cost = ((((x - cx) / unit).powi(2) - dt).max(0.0) / two_sigma2).min(cfg.max_sub);
            out.extend(columns[c].0.iter().map(|&l| (l, cost)));
        }
        out
    }

    /// The key under a point (tests, simulations).
    pub fn action_at(&self, x: f32, y: f32) -> Option<Action> {
        match self.hit(x, y) {
            Some(Target::Key(k)) => Some(self.keys[k].action),
            _ => None,
        }
    }

    /// The last letter of the word being typed (lowercase), if any.
    pub fn last_typed(&self) -> Option<char> {
        self.word
            .chars()
            .last()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
    }

    /// The suggestion strip: left, center (what space commits), right.
    pub fn suggestions(&self) -> &[Option<String>; 3] {
        &self.slots
    }

    /// Append a debug log line (JSON). Never in fields without suggestions
    /// (passwords, …) — not even touches, which would spell them.
    fn log_event(&mut self, ev: &str, fields: &[(&str, String)]) {
        if !self.logging() || !self.suggest {
            return;
        }
        let mut line = format!("{{\"t\":{},\"ev\":{}", self.now_ms, jstr(ev));
        for (k, v) in fields {
            line.push_str(&format!(",{}:{}", jstr(k), v));
        }
        line.push('}');
        self.log.push(line);
    }

    /// Debug log lines since the last call (the shim appends them to a file).
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    fn key_label(&self, x: f32, y: f32) -> String {
        match self.hit(x, y) {
            Some(Target::Key(k)) => match self.keys[k].action {
                Action::Char(c) => c.to_string(),
                Action::Column(c) => format!("col{c}"),
                a => format!("{a:?}").to_lowercase(),
            },
            Some(Target::Slot(i)) => format!("slot{i}"),
            Some(Target::Panel(h)) => format!("{h:?}").to_lowercase(),
            None => String::new(),
        }
    }

    /// Drop the flags the user switched off.
    fn gate(&self, flags: i32) -> i32 {
        if self.settings.haptic {
            flags
        } else {
            flags & !HAPTIC
        }
    }

    fn touch_inner(&mut self, action: i32, id: i32, x: f32, y: f32, time_ms: i64) -> i32 {
        if action == DOWN && self.maybe_calibrate() {
            return REDRAW | RELAYOUT; // this touch only started it
        }
        if self.calib.is_some() {
            return self.calibration_touch(action, x, y);
        }
        if action == DOWN && self.notice.take().is_some() {
            self.update_slots();
        }
        match action {
            DOWN => {
                let mut flags = REDRAW;
                let is_letter = |ime: &Self, t: Option<Target>| matches!(t, Some(Target::Key(k)) if matches!(ime.keys[k].action, Action::Char(_) | Action::Column(_)));
                let landed = self.hit_down(x, y, time_ms);
                let overlapping = self
                    .pointers
                    .iter()
                    .any(|p| !p.done && is_letter(self, p.target));
                let rolled = is_letter(self, landed)
                    && (overlapping
                        || self
                            .last_down
                            .is_some_and(|(t0, _)| time_ms - t0 < ROLLOVER_MS));
                // Enter while reaching for a far key: the base of the thumb or
                // the palm brushes the corner. Enter together with a letter, or
                // a moment after one far away, is not typed.
                let brushed = matches!(landed, Some(Target::Key(k)) if self.keys[k].action == Action::Enter)
                    && (overlapping
                        || self.last_down.is_some_and(|(t0, x0)| {
                            time_ms - t0 < BRUSH_MS && (x - x0).abs() > BRUSH_SPAN * self.width
                        }));
                if is_letter(self, landed)
                    && self
                        .sense
                        .on_tap(&self.grips, x / self.width, time_ms, overlapping)
                {
                    flags |= REDRAW;
                }
                // Why a grip is (not) recognized: its odds and the tilt, no text.
                if is_letter(self, landed) && self.grips.calibrated() {
                    let p = self.sense.probs();
                    let tilt = self.sense.tilt().unwrap_or_default();
                    let fields = [
                        (
                            "p",
                            format!("[{:.2},{:.2},{:.2},{:.2}]", p[0], p[1], p[2], p[3]),
                        ),
                        (
                            "tilt",
                            format!("[{:.3},{:.3},{:.3}]", tilt[0], tilt[1], tilt[2]),
                        ),
                        ("u", format!("{:.3}", x / self.width)),
                        ("overlap", overlapping.to_string()),
                    ];
                    self.log_event("grip", &fields);
                }
                if is_letter(self, landed) {
                    self.last_down = Some((time_ms, x));
                }
                // Roll-over: a new finger lands before the previous one lifted —
                // type the earlier keys now, in order.
                let slip_taps: Vec<(f32, f32)> = self
                    .pointers
                    .iter()
                    .filter(|p| !p.done)
                    .filter_map(|p| match p.target {
                        Some(Target::Key(k)) if slip_key(self.keys[k].action) => {
                            Some((p.start_x, p.start_y))
                        }
                        _ => None,
                    })
                    .collect();
                if let Some(&tap) = slip_taps.last() {
                    self.slip_tap = Some(tap);
                }
                let pending: Vec<(usize, usize, Option<PressInfo>)> = self
                    .pointers
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| !p.done && !p.gesture)
                    .filter_map(|(i, p)| match p.target {
                        Some(Target::Key(k)) => {
                            let letter =
                                matches!(self.keys[k].action, Action::Char(_) | Action::Column(_));
                            Some((i, k, letter.then(|| self.press_hint(p, k, None))))
                        }
                        _ => None,
                    })
                    .collect();
                for &(i, _, _) in &pending {
                    self.pointers[i].done = true;
                }
                let pending = pending.into_iter().map(|(_, k, hint)| (k, hint));
                for (k, hint) in pending {
                    // An Enter still held when another finger lands was a
                    // brush (see `brushed`): dropped, not typed.
                    if self.keys[k].action == Action::Enter {
                        self.log_event("enter-dropped", &[]);
                        continue;
                    }
                    self.pending = hint;
                    self.activate(Target::Key(k));
                    self.pending = None;
                    flags |= OUTPUT;
                }
                // Hit-test again: an earlier press just activated may have
                // switched the layout (language, symbols).
                let target = self.hit_down(x, y, time_ms);
                let path = match target {
                    Some(Target::Key(k)) => vec![k],
                    _ => Vec::new(),
                };
                let mut p = Pointer {
                    id,
                    target,
                    origin: target,
                    start_x: x,
                    start_y: y,
                    down_ms: time_ms,
                    path,
                    rolled,
                    done: brushed,
                    trail: vec![(x, y)],
                    trail_ms: vec![time_ms],
                    travel: 0.0,
                    gesture: false,
                    aim: self.grip_shift(x, y),
                    previewed: 0,
                };
                if brushed {
                    self.log_event("enter-dropped", &[]);
                }
                if let Some(Target::Key(k)) = target {
                    flags |= HAPTIC;
                    if self.keys[k].action == Action::Backspace {
                        self.backspace();
                        p.done = true;
                        self.timer = Timer::Repeat(id);
                        flags |= OUTPUT | timer_flag(REPEAT_START_MS);
                    } else if self.keys[k].alt.is_some() {
                        self.timer = Timer::LongPress(id);
                        flags |= timer_flag(LONG_PRESS_MS);
                    } else if self.keys[k].action == Action::Space {
                        self.timer = Timer::LongPress(id);
                        flags |= timer_flag(SPACE_MENU_MS);
                    } else if matches!(self.keys[k].action, Action::Column(_)) {
                        // One-row keys: held, their letters to pick one exactly.
                        self.timer = Timer::LongPress(id);
                        flags |= timer_flag(LONG_PRESS_MS);
                    }
                }
                // Hold the word as typed (left of the strip): into the dictionary.
                if target == Some(Target::Slot(0)) && self.left_is_typed() {
                    self.timer = Timer::LongPress(id);
                    flags |= timer_flag(LONG_PRESS_MS);
                }
                self.pointers.push(p);
                flags
            }
            MOVE if self.popup.as_ref().is_some_and(|p| p.pointer == id) => {
                let (x0, _, w, _) = self.popup_rect();
                if let Some(pop) = self.popup.as_mut() {
                    let i = ((x - x0) / w)
                        .floor()
                        .clamp(0.0, (pop.options.len() - 1) as f32);
                    pop.selected = i as usize;
                }
                REDRAW
            }
            UP if self.popup.as_ref().is_some_and(|p| p.pointer == id) => {
                self.pointers.retain(|p| p.id != id);
                if let Some(pop) = self.popup.take() {
                    match pop.options[pop.selected] {
                        // A letter picked from a one-row key: exactly that
                        // one, the decoding keeps it.
                        c if matches!(self.keys[pop.key].action, Action::Column(_)) => {
                            let exact = Hint {
                                kept: KEPT_HARD,
                                ..Hint::default()
                            };
                            self.pending = Some((exact, None));
                            self.type_char(c);
                            self.pending = None;
                        }
                        _ if self.keys[pop.key].action != Action::Space => {
                            self.type_char(pop.options[pop.selected])
                        }
                        MENU_SETTINGS => self.out.push(Op::Settings),
                        _ => self.out.push(Op::Picker),
                    }
                }
                REDRAW | OUTPUT
            }
            MOVE => {
                let target = self.hit(x, y);
                let swipe = SPACE_SWIPE_DP * self.dp;
                let slide = SLIDE_DP * self.dp;
                let lift = DRAG_ADD_DP * self.dp;
                let (key_w, row_h) = (self.letter_width(), self.row_h());
                let gestures = self.settings.gestures && !self.compact() && self.suggest;
                let space_swipe = self.settings.lang_switch != LangSwitch::Key;
                let keys = &self.keys;
                let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) else {
                    return 0;
                };
                if p.done {
                    return 0;
                }
                // On the panel: a sideways swipe turns the page.
                if let Some(Target::Panel(_)) = p.origin {
                    let (dx, dy) = (x - p.start_x, y - p.start_y);
                    if dx.abs() > swipe && dx.abs() > 2.0 * dy.abs() {
                        p.done = true;
                        self.flip_page(if dx < 0.0 { 1 } else { -1 });
                        return REDRAW | HAPTIC;
                    }
                    return 0;
                }
                // A finger that set off from a letter and keeps going draws a
                // word: record the way, don't retarget.
                let from_letter = matches!(p.origin, Some(Target::Key(k)) if matches!(keys[k].action, Action::Char(c) if c.is_alphabetic()));
                if from_letter && gestures {
                    if let Some(&(lx, ly)) = p.trail.last() {
                        p.travel += ((x - lx) / key_w).hypot((y - ly) / row_h);
                    }
                    p.trail.push((x, y));
                    p.trail_ms.push(time_ms);
                    if let Some(Target::Key(k)) = target {
                        if p.path.last() != Some(&k) {
                            p.path.push(k);
                        }
                    }
                    let far = p.travel >= GESTURE_FAR;
                    if !p.gesture && p.travel >= GESTURE_KEYS && (p.path.len() >= 3 || far) {
                        p.gesture = true;
                        p.target = p.origin;
                        if self.timer == Timer::LongPress(id) {
                            self.timer = Timer::None;
                        }
                    }
                    if p.gesture {
                        // The word it makes so far, in the strip's center —
                        // read again every few points of the way.
                        let due = p.trail.len() >= p.previewed + GESTURE_PREVIEW_POINTS;
                        let trail = due.then(|| {
                            p.previewed = p.trail.len();
                            (p.trail.clone(), p.trail_ms.clone())
                        });
                        if let Some((trail, ms)) = trail {
                            self.gesture_preview(&trail, &ms);
                        }
                        return REDRAW;
                    }
                }
                // The word as typed, dragged up out of the strip: into the
                // user's dictionary.
                if p.origin == Some(Target::Slot(0)) {
                    let typed_left = self.slots[0].as_deref().is_some_and(|s| {
                        !self.word.is_empty() && s.to_lowercase() == self.word.to_lowercase()
                    });
                    if p.start_y - y > lift && typed_left {
                        p.done = true;
                        if self.timer == Timer::LongPress(id) {
                            self.timer = Timer::None;
                        }
                        let w = self.word.clone();
                        self.add_user_word(&w);
                        self.update_slots();
                        return REDRAW | HAPTIC;
                    }
                    return 0;
                }
                // A swipe up on ?123 or Enter: the arc layout for the left or
                // right thumb (again: back to the usual layout).
                let arc_swipe = match p.origin {
                    Some(Target::Key(k)) => match keys[k].action {
                        Action::Symbols | Action::Letters => Some(Hand::Left),
                        Action::Enter => Some(Hand::Right),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(hand) = arc_swipe {
                    let (dx, up) = (x - p.start_x, p.start_y - y);
                    if up > swipe && up > 2.0 * dx.abs() {
                        p.done = true;
                        return self.toggle_arc(hand);
                    }
                }
                // The space bar doesn't retarget: a swipe up on it offers the
                // next likeliest word, a horizontal one switches the language.
                if let Some(Target::Key(k)) = p
                    .origin
                    .filter(|&o| matches!(o, Target::Key(k) if keys[k].action == Action::Space))
                {
                    let (dx, up) = (x - p.start_x, p.start_y - y);
                    if up > swipe && up > 2.0 * dx.abs() {
                        p.done = true;
                        self.next_variant();
                        return REDRAW | OUTPUT | HAPTIC;
                    }
                    // Up into the letters, aslant: a word drawn from just below
                    // its first letter — not a language swipe.
                    let letter = gestures
                        .then(|| {
                            keys.iter()
                                .enumerate()
                                .filter(|(_, key)| {
                                    matches!(key.action, Action::Char(c) if c.is_alphabetic())
                                })
                                .min_by(|a, b| {
                                    a.1.dist2(p.start_x, p.start_y)
                                        .total_cmp(&b.1.dist2(p.start_x, p.start_y))
                                })
                                .map(|(i, _)| i)
                        })
                        .flatten()
                        .filter(|_| y < keys[k].y - 0.15 * row_h);
                    if let Some(first) = letter {
                        p.origin = Some(Target::Key(first));
                        p.target = Some(Target::Key(first));
                        p.path = vec![first];
                        p.travel = ((x - p.start_x) / key_w).hypot((y - p.start_y) / row_h);
                        p.trail.push((x, y));
                        p.trail_ms.push(time_ms);
                        if self.timer == Timer::LongPress(id) {
                            self.timer = Timer::None;
                        }
                        return REDRAW;
                    }
                    // Down: the previous reading.
                    if -up > swipe && -up > 2.0 * dx.abs() {
                        p.done = true;
                        self.prev_variant();
                        return REDRAW | OUTPUT | HAPTIC;
                    }
                    if !space_swipe || dx.abs() < swipe {
                        return 0;
                    }
                    p.done = true;
                    self.switch_lang(if dx > 0.0 { 1 } else { -1 });
                    return REDRAW | HAPTIC;
                }
                if p.target == target || (x - p.start_x).hypot(y - p.start_y) < slide {
                    return 0;
                }
                // Sliding to another key retargets the press (and cancels a long-press).
                p.target = target;
                if let Some(Target::Key(k)) = target {
                    if p.path.last() != Some(&k) {
                        p.path.push(k);
                    }
                }
                if self.timer == Timer::LongPress(id) {
                    self.timer = Timer::None;
                }
                REDRAW
            }
            UP => {
                let Some(i) = self.pointers.iter().position(|p| p.id == id) else {
                    return REDRAW;
                };
                let p = self.pointers.remove(i);
                if matches!(self.timer, Timer::LongPress(t) | Timer::Repeat(t) if t == id) {
                    if matches!(self.timer, Timer::Repeat(_)) {
                        self.end_hold();
                    }
                    self.timer = Timer::None;
                }
                if p.gesture && !p.done {
                    let (mut trail, mut ms) = (p.trail, p.trail_ms);
                    trail.push((x, y));
                    ms.push(time_ms);
                    self.type_gesture(&trail, &ms);
                    return REDRAW | OUTPUT;
                }
                if !p.done {
                    if let Some(t) = p.target {
                        if let Target::Key(k) = t {
                            if matches!(self.keys[k].action, Action::Char(_) | Action::Column(_)) {
                                self.pending = Some(self.press_hint(&p, k, Some(time_ms)));
                            }
                            if slip_key(self.keys[k].action) {
                                self.slip_tap = Some((p.start_x, p.start_y));
                            }
                        }
                        self.activate(t);
                        self.pending = None;
                    }
                }
                REDRAW | OUTPUT
            }
            _ => {
                self.pointers.clear();
                self.popup = None;
                if matches!(self.timer, Timer::Repeat(_)) {
                    self.end_hold();
                }
                self.timer = Timer::None;
                REDRAW | OUTPUT
            }
        }
    }

    /// The delay requested by the last response elapsed.
    pub fn timer(&mut self) -> i32 {
        let flags = self.timer_inner();
        self.gate(flags)
    }

    fn timer_inner(&mut self) -> i32 {
        match self.timer {
            Timer::LongPress(id) => {
                self.timer = Timer::None;
                let Some(p) = self.pointers.iter_mut().find(|p| p.id == id && !p.done) else {
                    return 0;
                };
                if p.target == Some(Target::Slot(0)) {
                    p.done = true;
                    let w = self.word.clone();
                    self.add_user_word(&w);
                    self.update_slots();
                    return REDRAW | HAPTIC;
                }
                let Some(Target::Key(k)) = p.target else {
                    return 0;
                };
                if let Action::Column(col) = self.keys[k].action {
                    // A one-row key held: its letters in a row, the one it
                    // stands for lit — the finger picks one exactly.
                    p.done = true;
                    let (letters, stand) = layout::columns(self.lang)[col as usize].clone();
                    let selected = letters.iter().position(|&c| c == stand).unwrap_or(0);
                    self.popup = Some(Popup {
                        pointer: id,
                        key: k,
                        options: letters,
                        selected,
                        anchor: selected as f32 + 0.5,
                    });
                    return REDRAW | HAPTIC;
                }
                if self.keys[k].action == Action::Space {
                    // Held still: other keyboards or the settings.
                    p.done = true;
                    self.popup = Some(Popup {
                        pointer: id,
                        key: k,
                        options: vec![MENU_KEYBOARDS, MENU_SETTINGS],
                        selected: 0,
                        anchor: 1.0,
                    });
                    return REDRAW | HAPTIC;
                }
                let Some(alt) = self.keys[k].alt else {
                    return 0;
                };
                p.done = true;
                if alt == EMOJI_KEY {
                    self.open_panel();
                    return REDRAW | RELAYOUT | HAPTIC;
                }
                let more = self.keys[k].more;
                if more.is_empty() {
                    self.type_char(alt);
                    return REDRAW | OUTPUT | HAPTIC;
                }
                // Several: a row above the key, the first one lit.
                self.popup = Some(Popup {
                    pointer: id,
                    key: k,
                    options: std::iter::once(alt).chain(more.chars()).collect(),
                    selected: 0,
                    anchor: 0.5,
                });
                REDRAW | HAPTIC
            }
            Timer::Repeat(id) if self.pointers.iter().any(|p| p.id == id) => {
                self.held_bksp += 1;
                if self.held_bksp <= CHAR_REPEATS {
                    self.backspace();
                    return REDRAW | OUTPUT | timer_flag(REPEAT_MS);
                }
                self.delete_word();
                let words = (self.held_bksp - CHAR_REPEATS - 1) as i32;
                let ms = (WORD_REPEAT_MS * WORD_REPEAT_FASTER.powi(words)).max(WORD_REPEAT_MIN_MS);
                REDRAW | OUTPUT | timer_flag(ms as i32)
            }
            _ => 0,
        }
    }

    fn activate(&mut self, target: Target) {
        match target {
            Target::Slot(i) => self.pick_slot(i),
            Target::Panel(h) => self.panel_tap(h),
            Target::Key(k) => match self.keys[k].action {
                Action::Char(c) => {
                    // Punctuation from the symbols brings the letters back —
                    // not after a digit (3,5 · 1.5 · 5-6 go on in numbers).
                    let after_digit = self
                        .before
                        .chars()
                        .last()
                        .is_some_and(|d| d.is_ascii_digit())
                        && self.word.is_empty();
                    self.type_char(c);
                    if self.layer != Layer::Letters && BACK_TO_LETTERS.contains(c) && !after_digit {
                        self.layer = Layer::Letters;
                        self.relayout();
                    }
                }
                Action::Column(col) => {
                    let stand = layout::columns(self.lang)[col as usize].1;
                    self.type_char(stand);
                }
                Action::Shift => {
                    self.shift = match self.shift {
                        Shift::Off => Shift::Once,
                        Shift::Once => Shift::Caps,
                        Shift::Caps => Shift::Off,
                    }
                }
                Action::Space => self.space(),
                Action::Enter => self.enter(),
                Action::Backspace => {}
                Action::Lang => self.switch_lang(1),
                Action::Symbols => {
                    self.layer = Layer::Symbols;
                    self.relayout();
                }
                Action::Symbols2 => {
                    self.layer = Layer::Symbols2;
                    self.relayout();
                }
                Action::Letters => {
                    if self.panel.take().is_some() {
                        self.stash.settle();
                    }
                    self.layer = Layer::Letters;
                    self.relayout();
                }
                Action::Page(step) => self.flip_page(step as i32),
            },
        }
    }

    /// The panel's shape: tabs where the strip is, the page over three key
    /// rows (the fourth is its bottom row).
    fn panel_frame(&self) -> panel::Frame {
        panel::Frame {
            width: self.width,
            tabs_h: self.strip_h(),
            page_bottom: self.strip_h() + 3.0 * self.row_h(),
        }
    }

    /// Hold the comma: the emoji (the recent ones first, if any).
    fn open_panel(&mut self) {
        self.stash.settle();
        self.popup = None;
        self.panel = Some(Panel::new(&self.stash));
        self.relayout();
    }

    fn flip_page(&mut self, step: i32) {
        if let Some(p) = self.panel.as_mut() {
            let tab = p.tab;
            p.flip(step, &self.stash, self.emoji_max);
            if p.tab != tab {
                self.stash.settle();
            }
        }
    }

    fn panel_tap(&mut self, hit: Hit) {
        let Some(mut p) = self.panel else {
            return;
        };
        match hit {
            Hit::Tab(i) => {
                self.stash.settle();
                p.open_tab(Tab::from_index(i as usize));
            }
            Hit::Remove(i) if p.tab == Tab::Clips => {
                let at = p.page * panel::CLIP_ROWS + i as usize;
                if at < self.stash.clips.len() {
                    self.stash.clips.remove(at);
                }
                p.page = p
                    .page
                    .min(Panel::pages(p.tab, &self.stash, self.emoji_max) - 1);
            }
            Hit::Cell(i) | Hit::Remove(i) => {
                let item = p
                    .page_items(&self.stash, self.emoji_max)
                    .get(i as usize)
                    .map(|s| s.to_string());
                match item {
                    Some(text) if p.tab == Tab::Clips => self.paste(&text),
                    Some(emoji) => self.type_emoji(&emoji),
                    None => {}
                }
            }
        }
        if self.panel.is_some() {
            self.panel = Some(p);
        }
    }

    /// An emoji from the panel: after the word being typed (finished as
    /// space finishes it), then remembered among the recent ones.
    fn type_emoji(&mut self, emoji: &str) {
        if !self.word.is_empty() {
            self.space();
        }
        self.undo = None;
        self.prev_word = None;
        self.auto_space = false;
        self.commit(emoji.to_string());
        self.stash.used(emoji);
    }

    /// Paste a copied text: the word being typed stays as it is.
    fn paste(&mut self, text: &str) {
        if !self.word.is_empty() {
            self.commit_word(false);
        }
        self.undo = None;
        self.prev_word = None;
        self.auto_space = false;
        self.clip_offer = None;
        self.commit(text.to_string());
    }

    /// The Android version (`Build.VERSION.SDK_INT`): which emoji its font has.
    pub fn set_platform(&mut self, sdk: i32) {
        self.emoji_max = panel::emoji_max(sdk);
    }

    /// The clipboard changed (`text`: its text; None: it holds none) at
    /// `time_ms` (the touches' clock). A sensitive text (a password) is
    /// neither kept nor offered; another goes first among the copies and to
    /// the strip, to paste with a tap.
    pub fn clip(&mut self, text: Option<&str>, sensitive: bool, time_ms: i64) -> i32 {
        let text = text.filter(|t| !sensitive && !t.trim().is_empty() && self.settings.clipboard);
        let Some(text) = text else {
            self.clip_offer = None;
            return 0;
        };
        self.stash.copied(text);
        self.clip_offer = Some(time_ms);
        self.now_ms = self.now_ms.max(time_ms);
        if self.word.is_empty() && self.recorrect.is_none() && self.calib.is_none() {
            self.refresh_predictions();
        }
        REDRAW
    }

    /// A text copied lately, in the strip's left slot (see [`CLIP_OFFER_MS`]).
    fn offer_clip(&mut self) {
        let fresh = self
            .clip_offer
            .is_some_and(|t| self.now_ms - t < CLIP_OFFER_MS);
        if !fresh {
            self.clip_offer = None;
            return;
        }
        if self.settings.strip == Strip::Off {
            return;
        }
        if let Some(text) = self.stash.clips.first() {
            self.slots[0] = Some(format!("{CLIP_MARK}{}", panel::preview(text, 20)));
        }
    }

    /// On to the next (`step` 1) or previous (-1) language of the list.
    fn switch_lang(&mut self, step: i32) {
        let langs = &self.settings.languages;
        if langs.is_empty() {
            return;
        }
        let n = langs.len() as i32;
        let at = langs
            .iter()
            .position(|&l| l == self.lang)
            .map_or(-1, |i| i as i32);
        self.lang = langs[(at + step).rem_euclid(n) as usize];
        self.use_pack();
        self.layer = Layer::Letters;
        self.relayout();
    }

    fn is_word_char(&self, c: char) -> bool {
        let lower = c.to_lowercase().next().unwrap_or(c);
        match kbcore::alphabet::char_to_id(lower) {
            // Apostrophes and hyphens only inside a word (don't, кто-то).
            Some(_) if c == '\'' || c == '-' => !self.word.is_empty(),
            Some(_) => true,
            None => false,
        }
    }

    fn type_char(&mut self, c: char) {
        self.type_char_now(c);
        if matches!(c, '.' | '!' | '?') {
            self.proofread(false);
        }
    }

    fn type_char_now(&mut self, c: char) {
        self.undo = None;
        self.clip_offer = None;
        self.comma_undo = None;
        self.proof_undo = None;
        // Punctuation right after a word the strip joins with the one before
        // («кто нибудь,»): joined first, as space would (the space it leaves
        // moves after the mark, below).
        if !self.word.is_empty()
            && ",.!?;".contains(c)
            && self.settings.autocorrect
            && !self.editing
        {
            if let Some(m) = self.merge.take() {
                self.apply_merge(m);
            }
        }
        let tap = self.slip_tap.take();
        // "word ," → "word, ": the space moves after sentence punctuation,
        // and after a closing bracket that ends the word (привет) · a+b)).
        let word_end = self
            .before
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|b| b.is_alphanumeric() || CLOSING.contains(b) || ",.!?;".contains(b));
        if self.word.is_empty()
            && (",.!?;".contains(c) || CLOSING.contains(c) && word_end)
            && self.before.ends_with(' ')
            && !self.before.trim().is_empty()
        {
            self.auto_space = false;
            self.out.push(Op::Delete(1));
            self.before.pop();
            self.commit(c.to_string());
            self.commit(" ".into());
            self.auto_space = true;
            return;
        }
        self.auto_space = false;
        let c = if self.shift != Shift::Off && self.layer == Layer::Letters {
            c.to_uppercase().next().unwrap_or(c)
        } else {
            c
        };
        if self.shift == Shift::Once && c.is_alphabetic() {
            self.shift = Shift::Off;
        }
        // French elision: «l'» is a word; the next one starts right after it.
        if c == '\'' && self.lang == Lang::Fr && ELIDED.contains(&self.word.to_lowercase().as_str())
        {
            self.word.push(c);
            self.commit_word(false);
            return;
        }
        if self.suggest && self.is_word_char(c) {
            // The cursor put back at a word's end: the letters go on with it
            // (the whole word is read, not what's typed from here) — not in
            // the middle of one.
            let at_end = self.before.chars().last().is_some_and(char::is_alphabetic)
                && !self.after.chars().next().is_some_and(char::is_alphabetic);
            if self.word.is_empty() && at_end {
                self.resume_word();
            }
            if self.word.is_empty() {
                self.clean = true;
                self.restored = false;
            } else if self.editing && !self.settings.one_row {
                // Typing on after ⌫: the letters left on screen were seen and
                // kept. Erasing several, or going on with a word brought back
                // whole, is a deliberate edit — they stand; one erased letter
                // may be a quick fix of the last press — they only weigh more.
                let kept = if self.restored || self.erased >= 2 {
                    KEPT_HARD
                } else {
                    KEPT_SOFT
                };
                for h in &mut self.hints {
                    h.kept = h.kept.max(kept);
                }
            }
            let (mut hint, ms) = self.pending.take().unwrap_or_default();
            let c = if self.editing || self.erased > 0 {
                c // typed again after ⌫: meant as typed
            } else {
                self.zone_letter(c, &mut hint)
            };
            self.editing = false;
            self.erased = 0;
            self.chosen = None;
            self.drawn = None;
            self.word.push(c);
            self.hints.push(hint);
            self.durs.push(ms);
            self.update_slots();
            self.show();
            return;
        }
        self.prev_word = None;
        if self.drawn.is_some() && ",.!?;:".contains(c) {
            // A drawn word, then punctuation: the mark and a space (the next
            // word is drawn — no space bar in between).
            self.commit_word(false);
            self.commit(c.to_string());
            self.commit(" ".into());
            self.auto_space = true;
            return;
        }
        if !self.word.is_empty() {
            // Sentence punctuation after a word also triggers autocorrect.
            let correct = matches!(c, ',' | '.' | '!' | '?' | ';' | ':');
            let typed = self.word.clone();
            let input = self.composing_input();
            let settled = self.settled();
            let corrected = self.commit_word(correct);
            // ⌫ right after brings the typed word back, the mark kept.
            let undo = corrected.clone().map(|(t, f)| Undo {
                typed: t,
                fixed: f,
                sep: c,
                hints: match &input {
                    Input::Typed { hints, .. } => hints.clone(),
                    Input::Drawn { .. } => Vec::new(),
                },
                restore: None,
                pairs: Vec::new(),
            });
            // A comma or period hit instead of a letter above it: joined back
            // once the next letters show it (пр,вет → привет). Not after
            // short forms (т.е., и т.п.).
            let lower = typed.to_lowercase();
            if matches!(c, ',' | '.')
                && lower.chars().count() >= 2
                && !ABBREVIATIONS.contains(&lower.as_str())
            {
                let committed = corrected.map_or_else(|| typed.clone(), |(_, f)| f);
                self.commit(c.to_string());
                self.note_slip(typed, input, committed, tap, c, settled);
                self.undo = undo;
                return;
            }
            self.commit(c.to_string());
            self.undo = undo;
            return;
        }
        self.commit(c.to_string());
    }

    fn commit(&mut self, text: String) {
        self.leave_word();
        let joined = format!("{}{text}", self.before);
        self.set_before(&joined);
        if text == " " && self.after.starts_with([' ', '\u{a0}']) {
            // Mid-text, a space already follows: step over it.
            self.after.remove(0);
            self.out.push(Op::Skip);
        } else {
            self.out.push(Op::Commit(text));
        }
        self.update_shift();
        self.refresh_predictions();
    }

    /// Candidate `word` (lowercase) cased like `typed`, and names and
    /// abbreviations the way they are written (москва → Москва, сша → США)
    /// unless the typed capitals say otherwise.
    fn cased(&self, typed: &str, word: &str) -> String {
        let s = apply_case(typed, word);
        let rest_lower = s.chars().skip(1).all(|c| !c.is_uppercase());
        // A word the user added with a capital (a name) keeps it.
        let user_capital = self
            .user_words
            .iter()
            .any(|u| u.to_lowercase() == word && u.chars().next().is_some_and(char::is_uppercase));
        let casing = if user_capital {
            Casing::Capital
        } else {
            self.engine.casing(word)
        };
        match casing {
            Casing::Upper if rest_lower => s.to_uppercase(),
            Casing::Capital => {
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            }
            _ => s,
        }
    }

    /// The cursor leaves the word: what editing it showed no longer applies.
    fn leave_word(&mut self) {
        self.drawn = None;
        self.pair_undo = None;
        self.rejected.clear();
        self.erased = 0;
        self.restored = false;
    }

    /// Commit the composing word, autocorrected if `correct`. Returns what was
    /// committed and whether it differs from what was typed.
    fn commit_word(&mut self, correct: bool) -> Option<(String, String)> {
        if self.word.is_empty() {
            return None;
        }
        let typed = std::mem::take(&mut self.word);
        let drawn = self.drawn.take();
        // A join or split offered for this word no longer stands.
        self.merge = None;
        self.split = None;
        // While editing, what is shown is what gets committed; a trailing
        // hyphen (кто-, из-) is a word in the making.
        let lower = typed.to_lowercase();
        let insisted = self.rejected.iter().any(|(t, _)| *t == lower);
        let chosen = self.chosen.take();
        let fixed = match (
            &self.autocorrect,
            correct && !self.editing && !typed.ends_with('-') && !insisted,
        ) {
            _ if chosen.is_some() => chosen.unwrap_or_default(),
            (Some(a), true) => self.cased(&typed, a),
            // Typed right: still, names take their capital.
            (None, true) if self.settings.autocorrect && self.engine.contains(&lower) => {
                self.cased(&typed, &lower)
            }
            _ => typed.clone(),
        };
        // Written with its ё where е and ё are the same word (еще → ещё);
        // not once undone here, nor while editing.
        let yo = self.settings.yo
            && correct
            && !self.editing
            && !insisted
            && !self
                .rejected
                .iter()
                .any(|(t, _)| *t == fixed.to_lowercase());
        let fixed = match yo
            .then(|| self.engine.yo_form(&fixed.to_lowercase()))
            .flatten()
        {
            Some(y) => self.cased(&fixed, &y),
            None => fixed,
        };
        self.editing = false;
        // A clean, correct, uncorrected word shows the user's normal rhythm.
        let durs = std::mem::take(&mut self.durs);
        let hints = std::mem::take(&mut self.hints);
        if self.logging() {
            let press: Vec<String> = hints
                .iter()
                .map(|h| {
                    let f = [(h.held, 'H'), (h.rollover, 'R'), (!h.slid.is_empty(), 'S')];
                    jstr(&f.iter().filter(|x| x.0).map(|x| x.1).collect::<String>())
                })
                .collect();
            let cands: Vec<String> = self
                .last_cands
                .iter()
                .map(|(w, c)| format!("[{},{c:.2}]", jstr(w)))
                .collect();
            let fields = [
                ("typed", jstr(&typed)),
                ("out", jstr(&fixed)),
                ("auto", (fixed != typed).to_string()),
                ("press", format!("[{}]", press.join(","))),
                ("cands", format!("[{}]", cands.join(","))),
            ];
            self.log_event("word", &fields);
        }
        if self.clean && fixed == typed && self.engine.contains(&typed) {
            for ms in durs.into_iter().flatten() {
                self.rhythm.add(ms);
            }
        }
        self.autocorrect = None;
        self.slots = [None, None, None];
        self.shown.clear();
        if self.written.len() >= WRITTEN {
            self.written.remove(0);
        }
        let input = match drawn {
            Some((trail, ms)) => Input::Drawn { trail, ms },
            None => Input::Typed {
                letters: typed.to_lowercase(),
                hints,
            },
        };
        self.written.push(Written {
            text: fixed.to_lowercase(),
            input,
        });
        self.commit(fixed.clone());
        (fixed != typed).then_some((typed, fixed))
    }

    fn space(&mut self) {
        let tap = self.slip_tap.take();
        // Punctuation from the symbols, then space: back to the letters.
        let last = if self.auto_space {
            self.before.chars().rev().nth(1)
        } else {
            self.before.chars().last()
        };
        if self.layer != Layer::Letters
            && self.word.is_empty()
            && last.is_some_and(|c| ",.!?;:)".contains(c))
        {
            self.layer = Layer::Letters;
            self.relayout();
        }
        if std::mem::take(&mut self.auto_space) && self.word.is_empty() {
            return; // already inserted after the punctuation
        }
        if self.settings.autocorrect && !self.editing {
            if let Some(m) = self.merge.take() {
                self.apply_merge(m);
                return;
            }
            if let Some(sp) = self.split.take() {
                self.apply_split(sp);
                return;
            }
        }
        let typed = self.word.clone();
        let input = self.composing_input();
        let settled = self.settled();
        // A drawn word's other readings come from its way, not from typos;
        // a typed one's are what the strip offered.
        let drawn = self.drawn.is_some().then(|| self.alternatives.clone());
        let offered: Vec<String> = self.last_cands.iter().map(|(w, _)| w.clone()).collect();
        let one_row = self.settings.one_row;
        let corrected = self.commit_word(true);
        let committed = corrected.as_ref().map_or(typed.clone(), |c| c.1.clone());
        self.commit(" ".into());
        let hints = match &input {
            Input::Typed { hints, .. } => hints.clone(),
            Input::Drawn { .. } => Vec::new(),
        };
        self.undo = corrected.map(|(t, f)| Undo {
            typed: t,
            fixed: f,
            sep: ' ',
            hints,
            restore: None,
            pairs: Vec::new(),
        });
        self.note_slip(typed.clone(), input, committed.clone(), tap, ' ', settled);
        self.put_comma(&committed);
        if let Some(p) = self.prev_word.as_mut() {
            let from_way = drawn.is_some();
            let others = drawn.unwrap_or(offered);
            let typed_too = (!one_row || from_way).then(|| typed.to_lowercase());
            let mut variants = vec![committed.to_lowercase()];
            for w in others.into_iter().take(VARIANTS).chain(typed_too) {
                if !variants.contains(&w) {
                    variants.push(w);
                }
            }
            if variants.len() > 1 {
                p.variants = variants;
                p.variant = 0;
            }
        }
    }

    /// `word` just went in with a space after another word, a space between
    /// them: the comma between the two when it is almost surely due (the
    /// commas model, see [`COMMA_SURE`]) — «я думаю что» → «я думаю, что».
    fn put_comma(&mut self, word: &str) {
        self.comma_undo = None;
        if !self.settings.auto_commas || !self.suggest || word.is_empty() {
            return;
        }
        let tail = format!(" {word} ");
        let Some(head) = self.before.strip_suffix(&tail) else {
            return;
        };
        let prev: String = {
            let rev: Vec<char> = head
                .chars()
                .rev()
                .take_while(|c| c.is_alphabetic() || *c == '-')
                .collect();
            rev.into_iter().rev().collect()
        };
        if prev.is_empty() || !word.chars().all(|c| c.is_alphabetic() || c == '-') {
            return;
        }
        let (a, b) = (prev.to_lowercase(), word.to_lowercase());
        if self.commas_rejected.contains(&(a.clone(), b.clone())) {
            return;
        }
        if !self
            .engine
            .comma_odds_in(head, &a, &b)
            .is_some_and(|o| o >= COMMA_SURE)
        {
            return;
        }
        let n = tail.chars().count();
        self.out.push(Op::Delete(n));
        let keep = self.before.chars().count() - n;
        self.before = self.before.chars().take(keep).collect();
        self.commit(format!(",{tail}"));
        self.comma_undo = Some((a, b));
    }

    /// ⌫ right after a comma went in by itself: out it goes, and not again
    /// between these two words in this field.
    fn take_comma(&mut self) -> bool {
        let Some((a, b)) = self.comma_undo.take() else {
            return false;
        };
        let lower = self.before.to_lowercase();
        let tail = format!(", {b} ");
        if !self.word.is_empty() || !lower.ends_with(&tail) {
            return false;
        }
        let n = tail.chars().count();
        let word: String = self
            .before
            .chars()
            .skip(self.before.chars().count() - n + 2)
            .collect();
        self.out.push(Op::Delete(n));
        let keep = self.before.chars().count() - n;
        self.before = self.before.chars().take(keep).collect();
        self.commit(format!(" {word}"));
        self.commas_rejected.push((a, b));
        true
    }

    /// A sentence just finished — by `.`, `!` or `?` (`by_enter`: by Enter,
    /// no mark after its last word): the commas the graph of the whole
    /// sentence is sure of ([`PROOF_SURE`]) go in where no mark stands, the
    /// comma model not against them ([`PROOF_ODDS`]); ⌫ right after takes
    /// them out.
    fn proofread(&mut self, by_enter: bool) {
        self.proof_undo = None;
        if !self.settings.auto_commas || !self.suggest || self.lang != Lang::Ru {
            return;
        }
        let Some(places) = self.engine.sentence_parser().map(|p| p.places()) else {
            return;
        };
        let text = self.before.clone();
        let body = text.trim_end_matches(|c: char| c.is_whitespace() || ".!?…".contains(c));
        let rest = &text[body.len()..];
        if by_enter {
            if rest.chars().any(|c| !c.is_whitespace()) {
                return; // proofread at its mark already
            }
        } else if !sentence_start(&format!("{} ", text.trim_end())) {
            return; // a short form's period (т. е.), not the sentence's end
        }
        let toks = tokens(body);
        // Where the sentence starts: after the one before, or where the field
        // does — never where the text kept happens to be cut.
        let starts = |i: usize| {
            let pre = &body[..toks[i].start];
            if pre.chars().all(|c| !c.is_alphanumeric()) {
                self.whole || sentence_start(pre) && !pre.trim().is_empty()
            } else {
                sentence_start(pre)
            }
        };
        let Some(first) = (0..toks.len()).rev().find(|&i| starts(i)) else {
            return;
        };
        let sent = &toks[first..];
        if sent.len() < 3
            || sent.len() > places
            || sent.iter().any(|t| t.text.chars().any(|c| c.is_ascii_alphabetic()))
        {
            return;
        }
        // Before the first word only what opens the sentence (a quote, a
        // bracket), not the mark that ended the one before.
        let marks: Vec<String> = sent
            .iter()
            .enumerate()
            .map(|(k, t)| {
                if k == 0 {
                    t.before.chars().filter(|c| !".!?…".contains(*c)).collect()
                } else {
                    t.before.clone()
                }
            })
            .collect();
        let words: Vec<(&str, &str)> = sent.iter().zip(&marks).map(|(t, m)| (t.text, m.as_str())).collect();
        let Some((put, _)) = self.engine.sentence_marks(&words) else {
            return;
        };
        let mut at: Vec<(usize, (String, String))> = Vec::new();
        for p in put {
            if p.mark != Mark::Comma || p.before == 0 || p.chance < PROOF_SURE {
                continue;
            }
            let (prev, next) = (&sent[p.before - 1], &sent[p.before]);
            if !next.before.is_empty() || at.iter().any(|(pos, _)| *pos == prev.end) {
                continue; // a mark stands there already
            }
            let pair = (prev.text.to_lowercase(), next.text.to_lowercase());
            if self.commas_rejected.contains(&pair) {
                continue;
            }
            let odds = self.engine.comma_odds_in(&text[..prev.end], &pair.0, &pair.1);
            if odds.is_some_and(|o| o >= PROOF_ODDS) {
                at.push((prev.end, pair));
            }
        }
        if at.is_empty() {
            return;
        }
        at.sort_by_key(|(pos, _)| *pos);
        let from = at[0].0;
        let mut now = String::new();
        let mut last = from;
        for (pos, _) in &at {
            now.push_str(&text[last..*pos]);
            now.push(',');
            last = *pos;
        }
        now.push_str(&text[last..]);
        let was = text[from..].to_string();
        let n = was.chars().count();
        self.log_event("proof", &[("commas", at.len().to_string())]);
        self.out.push(Op::Delete(n));
        let keep = self.before.chars().count() - n;
        self.before = self.before.chars().take(keep).collect();
        let auto_space = self.auto_space;
        self.commit(now.clone());
        self.auto_space = auto_space;
        self.proof_undo = Some(ProofUndo {
            was,
            now,
            pairs: at.into_iter().map(|(_, pair)| pair).collect(),
        });
    }

    /// ⌫ right after a sentence's proofreading: its commas go out, and they
    /// don't come again between those words in this field.
    fn take_proof(&mut self) -> bool {
        let Some(u) = self.proof_undo.take() else {
            return false;
        };
        if !self.word.is_empty() || !self.before.ends_with(&u.now) {
            return false;
        }
        let n = u.now.chars().count();
        self.out.push(Op::Delete(n));
        let keep = self.before.chars().count() - n;
        self.before = self.before.chars().take(keep).collect();
        self.commit(u.was);
        self.commas_rejected.extend(u.pairs);
        true
    }

    /// The user has had their say on the current word (⌫ inside it, brought
    /// back, or typed again after an undo): it stays as it is.
    fn settled(&self) -> bool {
        let lower = self.word.to_lowercase();
        // A drawn word isn't typed clean (no rhythm to show): only editing
        // settles it.
        let touched = !self.clean && self.drawn.is_none();
        touched || self.editing || self.rejected.iter().any(|(t, _)| *t == lower)
    }

    /// `typed` went into the text as `committed`, then `sep` (a space, comma
    /// or period) tapped at `tap`: keep what it takes to join it with the
    /// next word, should that tap have been meant for a letter above — and,
    /// after a space, to read the two words together.
    fn note_slip(
        &mut self,
        typed: String,
        input: Input,
        committed: String,
        tap: Option<(f32, f32)>,
        sep: char,
        settled: bool,
    ) {
        self.prev_word = None;
        if typed.is_empty() || !self.suggest || self.settings.one_row {
            return;
        }
        // A drawn word's space isn't a tap that missed a letter.
        let letters = match input {
            Input::Typed { .. } => tap.map_or_else(Vec::new, |(x, y)| self.slip_letters(x, y)),
            Input::Drawn { .. } => Vec::new(),
        };
        let lower = typed.to_lowercase();
        let mut readings = self.decode_input(&Context::default(), &input);
        readings.retain(|c| !self.blocked(&c.word, &lower));
        let alone = readings.first().map_or(f32::INFINITY, |c| c.cost);
        let mut alts = Vec::new();
        if sep == ' ' && !settled {
            // The word before it still counts (… в | а принципе).
            let rest = self
                .before
                .strip_suffix(sep)
                .and_then(|b| b.strip_suffix(committed.as_str()))
                .unwrap_or("");
            // (Without the chooser, as the window weighs them: its pull to
            // the word typed held «в» against «во что».)
            let here = self.context_at(rest).without_chooser();
            self.engine.weigh(&here, &mut readings);
            readings.truncate(WINDOW_ALTS);
            // The words it is spelled for by mistake (к / ко, учится /
            // учиться): the next word may speak for one of them — a slip of
            // its own kind, not of the finger. (A key's slip, н / нн, comes
            // in with its own cost above.)
            let lower_committed = committed.to_lowercase();
            // (Found by the search too, it may cost a key's slip there —
            // с → со a letter put in: the pair's cost holds.)
            for w in self.engine.partners(&lower_committed) {
                let found = readings.iter().position(|c| c.word == w);
                if found.is_some_and(|i| readings[i].edit <= PAIR_SLIP)
                    || self.blocked(&w, &lower)
                    || !spelling_pair(&lower_committed, &w)
                {
                    continue;
                }
                if let Some(prior) = self.engine.word_cost(&w) {
                    let mut c = vec![Candidate {
                        word: w,
                        cost: prior + PAIR_SLIP,
                        edit: PAIR_SLIP,
                    }];
                    self.engine.weigh(&here, &mut c);
                    match found {
                        Some(i) => readings[i] = c.remove(0),
                        None => readings.extend(c),
                    }
                }
            }
            alts = readings;
        }
        // (A drawn word is kept anyway: its way gives the swipe up's variants.)
        if letters.is_empty() && alts.is_empty() && matches!(input, Input::Typed { .. }) {
            return;
        }
        self.prev_word = Some(PrevWord {
            typed,
            input,
            committed,
            alone,
            letters,
            sep,
            alts,
            variants: Vec::new(),
            variant: 0,
        });
    }

    /// Letters just above a space, comma or period tap at `(x, y)` it may
    /// have been meant for, cheapest first, with the cost of that slip.
    fn slip_letters(&self, x: f32, y: f32) -> Vec<(char, f32)> {
        let cfg = self.engine.config();
        let unit = self.width / 10.0;
        let two_sigma2 = 2.0 * cfg.sigma * cfg.sigma;
        let mut out: Vec<(char, f32)> = self
            .keys
            .iter()
            .filter_map(|k| match k.action {
                Action::Char(c) if c.is_alphabetic() => {
                    let (cx, cy) = k.center();
                    let d2 = ((x - cx) / unit).powi(2) + ((y - cy) / unit).powi(2);
                    Some((c, SPACE_SLIP + d2 / two_sigma2))
                }
                _ => None,
            })
            .filter(|&(_, cost)| cost <= SPACE_SLIP_MAX)
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out.truncate(2);
        out
    }

    /// "That space was a letter": is joining the previous word, the letter and
    /// the current word clearly likelier than two separate words?
    fn find_merge(&self, lower: &str, alone_now: f32) -> Option<Merge> {
        let prev = self.prev_here()?;
        let Input::Typed {
            letters: head,
            hints: head_hints,
        } = &prev.input
        else {
            return None;
        };
        let mut best: Option<(f32, String)> = None;
        for &(letter, slip) in &prev.letters {
            let joined = format!("{head}{letter}{lower}");
            let hints: Vec<Hint> = head_hints
                .iter()
                .cloned()
                .chain([Hint::default()])
                .chain(self.hints.iter().cloned())
                .collect();
            if let Some(c) = self
                .engine
                .read(Evidence::Taps(&joined, &hints))
                .into_iter()
                .next()
            {
                let cost = c.cost + slip;
                if best.as_ref().is_none_or(|b| cost < b.0) {
                    best = Some((cost, c.word));
                }
            }
        }
        let (cost, word) = best?;
        (cost + MERGE_MARGIN < prev.alone + alone_now).then(|| Merge {
            word: self.cased(&prev.typed, &word),
            variants: Vec::new(),
            delete: prev.committed.chars().count() + 1,
        })
    }

    /// «кто то», «по русски», «во первых», «кое что»: the previous word and
    /// this one are one word written with a hyphen — when the dictionary has
    /// it and, at the previous word's place, it is clearly likelier than
    /// the two apart (this one not a word at all: always).
    fn find_hyphen(&self, lower: &str) -> Option<Merge> {
        let prev = self.prev_here().filter(|p| p.sep == ' ')?;
        let head = prev.typed.to_lowercase();
        let joined = format!("{head}-{lower}");
        if !self.engine.contains(&joined) {
            return None;
        }
        let rest = self
            .before
            .strip_suffix(' ')
            .and_then(|b| b.strip_suffix(prev.committed.as_str()))
            .unwrap_or("");
        let at = |ctx: &Context, w: &str| -> Option<f32> {
            let mut c = vec![Candidate {
                word: w.to_string(),
                cost: self.engine.word_cost(w)?,
                edit: 0.0,
            }];
            self.engine.weigh(ctx, &mut c);
            Some(c[0].cost)
        };
        let here = self.context_at(rest);
        let one = at(&here, &joined)?;
        let apart = match (at(&here, &head), self.engine.contains(lower)) {
            (Some(first), true) => first + at(&self.context_at(&self.before), lower)?,
            _ => f32::INFINITY,
        };
        (one + HYPHEN_MARGIN < apart).then(|| Merge {
            word: self.cased(&prev.typed, &joined),
            variants: Vec::new(),
            delete: prev.committed.chars().count() + 1,
        })
    }

    /// The previous word, if it is still right before the cursor (nothing
    /// was typed or moved in between).
    fn prev_here(&self) -> Option<&PrevWord> {
        self.prev_word
            .as_ref()
            .filter(|p| self.before.ends_with(&format!("{}{}", p.committed, p.sep)))
    }

    /// Read the previous word and this one together: when this word makes
    /// another reading of the previous one clearly likelier (а принципе → в
    /// принципе), offer both — applied by space like a merge. `plain`: this
    /// word's readings on their own (no context yet).
    fn find_window(&self, plain: &[Candidate]) -> Option<Merge> {
        let prev = self.prev_here()?;
        let committed = prev.committed.to_lowercase();
        // ё put in is no correction: «ее» went in as «её», at the cost of
        // the «ее» typed — one word, whichever spelling (not все / всё).
        let same = |w: &str| {
            w == committed
                || self.engine.yo_form(w).is_some_and(|y| y == committed)
                || self.engine.yo_form(&committed).is_some_and(|y| y == w)
        };
        let base_prev = prev
            .alts
            .iter()
            .filter(|a| same(&a.word))
            .map(|a| a.cost)
            .min_by(f32::total_cmp)?;
        // The text before the previous word.
        let rest = self
            .before
            .strip_suffix(prev.sep)
            .and_then(|b| b.strip_suffix(prev.committed.as_str()))
            .unwrap_or("");
        // This word's likeliest readings after `p` there, with their costs.
        // (Without the chooser: it learned to choose among a word's readings,
        // not between pairs of words — here it cost the spelling pairs.)
        let after = |p: &str| -> Vec<(f32, String)> {
            let mut cur = plain.to_vec();
            self.engine.weigh(
                &self.context_at(&format!("{rest}{p} ")).without_chooser(),
                &mut cur,
            );
            cur.into_iter().take(3).map(|c| (c.cost, c.word)).collect()
        };
        let base = base_prev + after(&committed).first()?.0;
        // Typed as a real word: only a slip of the finger may change it —
        // a bigger one when the next word speaks up strongly for the other
        // (а принципе: «принципе» almost always follows «в»).
        // (A drawn word was read, not typed: its readings' costs decide.)
        let typed_word = matches!(prev.input, Input::Typed { .. })
            && same(&prev.typed.to_lowercase())
            && self.engine.contains(&prev.typed.to_lowercase());
        let w_rules = self.engine.config().w_rules * self.engine.config().w_lm;
        // Every reading of the pair: the previous word as it stands, or
        // re-read (only a slip of the finger for a word typed as a real word).
        let mut pairs: Vec<(f32, &str, String)> = after(&committed)
            .into_iter()
            .map(|(cost, c)| (base_prev + cost, committed.as_str(), c))
            .collect();
        for a in prev.alts.iter().filter(|a| !same(&a.word)) {
            for (cost, c) in after(&a.word) {
                let e = self.engine.next_evidence(&committed, &a.word, &c);
                let slip = self.known_slip + 0.5 * (e - 3.0).max(0.0);
                if !typed_word || a.edit <= slip {
                    pairs.push((a.cost + cost - w_rules * e, &a.word, c));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (total, alt, _) = pairs.iter().find(|(_, p, _)| *p != committed)?;
        // A spelling of the pair's other word (в что → во что): not a slip
        // the finger may or may not have made — the next word settles it.
        let margin = if spelling_pair(&committed, alt) {
            SPELLING_MARGIN
        } else {
            self.window_margin
        };
        if *total + margin >= base {
            return None;
        }
        // The best re-reading first, then the other likeliest pairs.
        let join = |p: &str, c: &str| {
            format!(
                "{} {}",
                self.cased(&prev.typed, p),
                self.cased(&self.word, c)
            )
        };
        let mut variants: Vec<String> = pairs
            .iter()
            .filter(|(_, p, _)| *p != committed)
            .take(1)
            .chain(pairs.iter())
            .map(|(_, p, c)| join(p, c))
            .collect();
        let mut seen = std::collections::HashSet::new();
        variants.retain(|v| seen.insert(v.clone()));
        variants.truncate(6);
        Some(Merge {
            word: variants[0].clone(),
            variants,
            delete: prev.committed.chars().count() + 1,
        })
    }

    /// Letters in the row right above the space bar (a space-bound tap that
    /// landed a little high types one of these).
    fn above_space(&self) -> Vec<char> {
        let Some(space) = self.keys.iter().find(|k| k.action == Action::Space) else {
            return Vec::new();
        };
        self.keys
            .iter()
            .filter(|k| (k.y + k.h - space.y).abs() < 1.0)
            .filter(|k| {
                let cx = k.center().0;
                cx > space.x - k.w / 2.0 && cx < space.x + space.w + k.w / 2.0
            })
            .filter_map(|k| match k.action {
                Action::Char(c) if c.is_alphabetic() => Some(c),
                _ => None,
            })
            .collect()
    }

    /// The typed word as two dictionary words — a space missed (`приветкак`)
    /// or hit as the letter above it (`приветькак`) — when that is clearly
    /// likelier than the best single word. Exact lookups only: cheap.
    fn find_split(&self, lower: &str, single: f32) -> Option<Split> {
        let chars: Vec<char> = lower.chars().collect();
        let m = chars.len();
        if m < 4 || self.engine.contains(lower) {
            return None;
        }
        let above = self.above_space();
        let one = one_letter_words(self.lang);
        let part_ok = |p: &[char]| p.len() >= 2 || p.first().is_some_and(|c| one.contains(*c));
        let mut best: Option<(f32, usize, usize)> = None;
        for i in 1..m {
            let mut cuts = vec![(i, i, MISSING_SPACE)];
            if i + 1 < m && above.contains(&chars[i]) {
                cuts.push((i, i + 1, SPACE_AS_LETTER));
            }
            for (left_end, right_start, penalty) in cuts {
                let (l, r) = (&chars[..left_end], &chars[right_start..]);
                if !part_ok(l) || !part_ok(r) {
                    continue;
                }
                let (l, r): (String, String) = (l.iter().collect(), r.iter().collect());
                let (Some(cl), Some(cr)) = (self.engine.word_cost(&l), self.engine.word_cost(&r))
                else {
                    continue;
                };
                let cost = cl + cr + penalty;
                if best.is_none_or(|b| cost < b.0) {
                    best = Some((cost, left_end, right_start));
                }
            }
        }
        let (cost, left_end, right_start) = best?;
        if cost + SPLIT_MARGIN >= single {
            return None;
        }
        let typed: Vec<char> = self.word.chars().collect();
        let left: String = typed[..left_end].iter().collect();
        let right: String = chars[right_start..].iter().collect();
        let left_cased = self.cased(&left, &left.to_lowercase());
        let ctx = self.context_at(&format!("{}{} ", self.before, left.to_lowercase()));
        let mut alts = self.engine.decode(&ctx, Evidence::Taps(&right, &[]));
        alts.retain(|c| !self.blocked(&c.word, &right));
        let mut variants = vec![format!("{left_cased} {}", self.cased(&right, &right))];
        for c in alts.iter().take(4) {
            let v = format!("{left_cased} {}", self.cased(&right, &c.word));
            if !variants.contains(&v) {
                variants.push(v);
            }
        }
        Some(Split {
            left: left_cased,
            right: self.cased(&right, &right),
            variants,
        })
    }

    /// Commit the current word as the two words of `sp`.
    fn apply_split(&mut self, sp: Split) {
        self.log_event(
            "split",
            &[("left", jstr(&sp.left)), ("right", jstr(&sp.right))],
        );
        let text = format!("{} {}", sp.left, sp.right);
        let typed = std::mem::take(&mut self.word);
        let hints = std::mem::take(&mut self.hints);
        self.shown.clear();
        self.durs.clear();
        self.autocorrect = None;
        self.slots = [None, None, None];
        self.prev_word = None;
        self.merge = None;
        self.split = None;
        self.commit(text.clone());
        self.commit(" ".into());
        // A swipe up goes through the pair's readings and back to the word
        // as typed.
        self.prev_word = Some(unit(&typed, &text, &sp.variants));
        // ⌫ right after puts the word back as typed (and typing it again
        // keeps it whole).
        self.undo = Some(Undo {
            typed,
            fixed: text,
            sep: ' ',
            hints,
            restore: None,
            pairs: sp.variants,
        });
    }

    /// Replace "previous word, space, current word" by the joined word.
    fn apply_merge(&mut self, m: Merge) {
        self.log_event(
            "merge",
            &[("word", jstr(&m.word)), ("typed", jstr(&self.word))],
        );
        self.out.push(Op::Composing(String::new()));
        self.out.push(Op::Delete(m.delete));
        let keep = self.before.chars().count().saturating_sub(m.delete);
        let restore: String = self.before.chars().skip(keep).collect();
        self.before = self.before.chars().take(keep).collect();
        let typed = std::mem::take(&mut self.word);
        let hints = std::mem::take(&mut self.hints);
        self.word.clear();
        self.shown.clear();
        self.hints.clear();
        self.durs.clear();
        self.autocorrect = None;
        self.slots = [None, None, None];
        self.prev_word = None;
        self.merge = None;
        self.commit(m.word.clone());
        self.commit(" ".into());
        // A swipe up goes through the readings and back to what was typed.
        let was = format!("{restore}{typed}");
        self.prev_word = Some(unit(&was, &m.word, &m.variants));
        // ⌫ right after puts both words back as they were.
        self.undo = Some(Undo {
            typed,
            fixed: m.word,
            sep: ' ',
            hints,
            restore: Some(restore),
            pairs: m.variants,
        });
    }

    fn enter(&mut self) {
        // The word goes in as the field shows it — corrected, as a space
        // would put it in (the app's own send button sends what is shown
        // too); ⌫ right after a new line brings the typed word back.
        let hints = match self.composing_input() {
            Input::Typed { hints, .. } => hints,
            Input::Drawn { .. } => Vec::new(),
        };
        let corrected = self.commit_word(true);
        self.leave_word();
        self.prev_word = None;
        // A sentence ended by Enter, no mark after it: proofread as a mark
        // would have it (before the app sends it).
        self.proofread(true);
        self.out.push(Op::Enter);
        self.before.push('\n');
        if let Some(u) = self.proof_undo.as_mut() {
            u.was.push('\n');
            u.now.push('\n');
        }
        self.update_shift();
        self.undo = corrected.map(|(typed, fixed)| Undo {
            typed,
            fixed,
            sep: '\n',
            hints,
            restore: None,
            pairs: Vec::new(),
        });
    }

    fn backspace(&mut self) {
        // Right after a drawn word went in with its space (nothing since).
        let after_drawn = std::mem::take(&mut self.auto_space);
        self.variants_shown = false;
        self.last_bksp = Some(self.now_ms);
        if self.take_proof() || self.take_comma() {
            return;
        }
        let drawn_in = after_drawn
            .then(|| self.prev_here())
            .flatten()
            .filter(|p| matches!(p.input, Input::Drawn { .. }))
            .map(|p| p.committed.chars().count() + 1);
        if let (Some(n), true) = (drawn_in, self.word.is_empty()) {
            // A word drawn and put in with its space: ⌫ takes both — it was
            // read, not typed.
            self.log_event("bksp", &[("drawn", "true".into())]);
            self.out.push(Op::Delete(n));
            for _ in 0..n {
                self.before.pop();
            }
            self.prev_word = None;
            self.update_shift();
            self.refresh_predictions();
            return;
        }
        if self.selected && self.word.is_empty() {
            // Selected text: ⌫ deletes the selection, not the char before it.
            self.selected = false;
            self.out.push(Op::Commit(String::new()));
            self.log_event("bksp", &[("selection", "true".into())]);
            return;
        }
        self.log_event(
            "bksp",
            &[
                ("word", jstr(&self.word)),
                ("undo", self.undo.is_some().to_string()),
            ],
        );
        if !self.word.is_empty() && self.drawn.take().is_some() {
            // A word drawn across the keys goes as a whole: it was read, not typed.
            self.word.clear();
            self.hints.clear();
            self.durs.clear();
            self.chosen = None;
            self.alternatives.clear();
            self.autocorrect = None;
            self.shown.clear();
            self.out.push(Op::Composing(String::new()));
            // Nothing typed here now: a sentence's start takes a capital again.
            self.update_shift();
            self.update_slots();
            return;
        }
        if !self.word.is_empty() {
            self.chosen = None;
            self.word.pop();
            self.hints.pop();
            self.durs.pop();
            self.clean = false;
            self.editing = true;
            self.erased += 1;
            if self.word.is_empty() {
                // All of it: the next letters start a new word (with a
                // capital at a sentence's start).
                self.erased = 0;
                self.restored = false;
                self.update_shift();
            }
            self.update_slots();
            self.show();
        } else if let Some(u) = self
            .undo
            .take_if(|u| u.sep != ' ' && self.before.ends_with(&format!("{}{}", u.fixed, u.sep)))
        {
            // After a mark: only the change goes — «Созван,» → «созвон,».
            self.prev_word = None;
            self.out.push(Op::Delete(u.fixed.chars().count() + 1));
            for _ in 0..=u.fixed.chars().count() {
                self.before.pop();
            }
            self.rejected
                .push((u.typed.to_lowercase(), u.fixed.to_lowercase()));
            self.commit(format!("{}{}", u.typed, u.sep));
        } else if let Some(Undo {
            typed,
            fixed,
            hints,
            restore,
            pairs,
            ..
        }) = self
            .undo
            .take()
            .filter(|u| self.before.ends_with(&format!("{} ", u.fixed)))
        {
            self.prev_word = None;
            // Revert the autocorrect: "fixed␣" becomes the typed word, composing again.
            self.out.push(Op::Delete(fixed.chars().count() + 1));
            for _ in 0..=fixed.chars().count() {
                self.before.pop();
            }
            let prefix = restore.clone().unwrap_or_default();
            if let Some(r) = restore {
                self.commit(r);
            }
            self.rejected
                .push((typed.to_lowercase(), fixed.to_lowercase()));
            let pair_undo = (!pairs.is_empty()).then(|| PairUndo {
                prefix,
                typed: typed.clone(),
                variants: pairs,
            });
            self.hints = if hints.len() == typed.chars().count() {
                hints
            } else {
                vec![Hint::default(); typed.chars().count()]
            };
            self.durs = vec![None; typed.chars().count()];
            self.clean = false;
            self.editing = true;
            self.restored = true;
            self.erased = 0;
            self.word = typed;
            self.update_slots();
            self.show();
            self.pair_undo = pair_undo;
        } else {
            self.prev_word = None;
            self.leave_word();
            // An emoji goes whole (joined ones, flags, skin tones too).
            let n = last_cluster(&self.before).max(1);
            self.out.push(Op::Delete(n));
            for _ in 0..n {
                self.before.pop();
            }
            if self.held_bksp == 0 {
                self.resume_word();
            }
            self.update_shift();
        }
    }

    /// Where the alternatives row sits: x, y, one option's width, height —
    /// above the held key (over the strip for the top row), kept on screen.
    fn popup_rect(&self) -> (f32, f32, f32, f32) {
        let Some(pop) = &self.popup else {
            return (0.0, 0.0, 1.0, 1.0);
        };
        let key = &self.keys[pop.key];
        let space = key.action == Action::Space;
        let w = if space {
            self.width / 7.0
        } else {
            key.w.max(self.width / 12.0)
        };
        let total = w * pop.options.len() as f32;
        let x0 = (key.x + key.w / 2.0 - w * pop.anchor).clamp(0.0, (self.width - total).max(0.0));
        let y = (key.y - key.h).max(0.0);
        (x0, y, w, key.h)
    }

    /// A letter key's width in px.
    fn letter_width(&self) -> f32 {
        self.keys
            .iter()
            .find(|k| matches!(k.action, Action::Char(c) if c.is_alphabetic()))
            .map_or(self.width / 11.0, |k| k.w)
    }

    /// A word drawn across the keys: decode it and put the likeliest reading
    /// in as the word being typed (the others go to the strip; space up
    /// cycles them). A word before it is finished first, with a space.
    /// The letter keys' centers, for reading a drawn word.
    fn gesture_keys(&self) -> Vec<(char, f32, f32)> {
        self.keys
            .iter()
            .filter_map(|k| match k.action {
                Action::Char(c) if c.is_alphabetic() => {
                    let (x, y) = k.center();
                    Some((c, x, y))
                }
                _ => None,
            })
            .collect()
    }

    /// The letter a tap on `c`'s key means, `hint` its touch: on the border
    /// with a neighbor, when no word goes on with `c` but some do with the
    /// neighbor, the neighbor — the likeliest at the place if several (the
    /// key's zones follow the text; nothing is learned). The hint is turned
    /// round to the letter taken. Only a dead end: a letter that some word
    /// goes on with stays — the word's own correction weighs it later with
    /// all its letters (choosing here by the best word the letters so far
    /// begin, a short frequent one — «с», «в» — would win every border).
    fn zone_letter(&self, c: char, hint: &mut Hint) -> char {
        if !self.suggest || !self.settings.autocorrect || self.settings.one_row {
            return c;
        }
        let lower = c.to_lowercase().next().unwrap_or(c);
        let base = self.engine.config().base_sub;
        let near: Vec<(char, f32)> = hint
            .spatial
            .iter()
            .copied()
            .filter(|&(x, cost)| x != lower && cost <= base + ZONE_NEAR)
            .collect();
        if near.is_empty() {
            return c;
        }
        let ctx = self.context_at(&self.before);
        let w_ch = self.engine.config().w_ch;
        let so_far = self.word.to_lowercase();
        let begun = |x: char, touch: f32| {
            let prefix = format!("{so_far}{x}");
            self.engine
                .decode(&ctx, Evidence::Begun(&prefix))
                .first()
                .map_or(f32::INFINITY, |b| b.cost + w_ch * touch)
        };
        if begun(lower, 0.0).is_finite() {
            return c;
        }
        let Some((best, cost, at)) = near
            .iter()
            .map(|&(x, t)| (x, t, begun(x, t - base)))
            .min_by(|a, b| a.2.total_cmp(&b.2))
        else {
            return c;
        };
        if !at.is_finite() {
            return c;
        }
        // The touch from the letter taken: the pressed key now its
        // neighbor on the border.
        hint.spatial.retain(|&(x, _)| x != best);
        hint.spatial.push((lower, cost));
        if c.is_uppercase() {
            best.to_uppercase().next().unwrap_or(best)
        } else {
            best
        }
    }

    /// What the finger gave for the word being composed.
    fn composing_input(&self) -> Input {
        match &self.drawn {
            Some((trail, ms)) => Input::Drawn {
                trail: trail.clone(),
                ms: ms.clone(),
            },
            None => Input::typed(&self.word, &self.hints),
        }
    }

    /// The readings of a word the finger gave `input` for, at the place
    /// `ctx` was read for, best first.
    fn decode_input(&self, ctx: &Context, input: &Input) -> Vec<Candidate> {
        match input {
            Input::Typed { letters, hints } => {
                self.engine.decode(ctx, Evidence::Taps(letters, hints))
            }
            Input::Drawn { trail, ms } => self.decode_drawn(ctx, trail, ms),
        }
    }

    /// Read a drawn word at the place `ctx` was read for: its readings, best
    /// first — with the times of the way when the pace setting is on.
    fn decode_drawn(&self, ctx: &Context, trail: &[(f32, f32)], ms: &[i64]) -> Vec<Candidate> {
        let keys = self.gesture_keys();
        let t0 = ms.first().copied().unwrap_or(0);
        let times: Vec<f32> = ms.iter().map(|&t| (t - t0) as f32).collect();
        let times =
            (self.settings.gesture_pace && times.len() == trail.len()).then_some(&times[..]);
        let drawn = Evidence::Drawn {
            points: trail,
            times,
            keys: &keys,
            key_w: self.letter_width(),
        };
        let mut cands = self.engine.decode(ctx, drawn);
        cands.retain(|c| !stretched(&c.word) && !self.blocked(&c.word, ""));
        cands
    }

    /// The text a drawn word will follow: a word being typed goes in first,
    /// and a word comes after a word or punctuation with a space (the word
    /// being typed as it stands — space may yet correct it).
    fn gesture_place(&self) -> String {
        if !self.word.is_empty() {
            format!("{}{} ", self.before, self.word)
        } else if self.needs_space_before_gesture() {
            format!("{} ", self.before)
        } else {
            self.before.clone()
        }
    }

    /// A word or punctuation right before the cursor: a drawn word goes
    /// after a space.
    fn needs_space_before_gesture(&self) -> bool {
        self.before
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || ",.!?;:".contains(c) || CLOSING.contains(c))
    }

    /// While a word is drawn: what the way so far reads as, in the strip's
    /// center (nothing reaches the text until the finger lifts).
    fn gesture_preview(&mut self, trail: &[(f32, f32)], ms: &[i64]) {
        let cands = self.decode_drawn(&self.context_at(&self.gesture_place()), trail, ms);
        let Some(best) = cands.first() else {
            return;
        };
        let word = match self.shift {
            Shift::Caps => best.word.to_uppercase(),
            Shift::Once => self.cased("Ы", &best.word),
            Shift::Off => self.cased("", &best.word),
        };
        self.slots = if self.settings.strip == Strip::Off {
            [None, None, None]
        } else {
            [None, Some(word), None]
        };
    }

    fn type_gesture(&mut self, trail: &[(f32, f32)], ms: &[i64]) {
        let place = self.gesture_place();
        let mut cands = self.decode_drawn(&self.context_at(&place), trail, ms);
        self.log_event(
            "gesture",
            &[
                ("points", trail.len().to_string()),
                (
                    "cands",
                    format!(
                        "[{}]",
                        cands
                            .iter()
                            .take(3)
                            .map(|c| format!("[{},{:.2}]", jstr(&c.word), c.cost))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                ),
            ],
        );
        if cands.is_empty() {
            return;
        }
        // Finish what came before: the word being typed, or a word right
        // before the cursor — a gesture word comes with its space.
        if !self.word.is_empty() {
            self.space();
        } else if self.needs_space_before_gesture() {
            self.commit(" ".into());
        }
        // Space corrected the word before: read the way again after it.
        if self.before != place {
            let again = self.decode_drawn(&self.context_at(&self.before), trail, ms);
            if !again.is_empty() {
                cands = again;
            }
        }
        let best = cands[0].word.clone();
        let word = match self.shift {
            Shift::Caps => best.to_uppercase(),
            Shift::Once => self.cased("Ы", &best),
            Shift::Off => self.cased("", &best),
        };
        if self.shift == Shift::Once {
            self.shift = Shift::Off;
        }
        let n = word.chars().count();
        self.undo = None;
        self.editing = false;
        self.clean = false;
        self.word = word.clone();
        self.hints = vec![Hint::default(); n];
        self.durs = vec![None; n];
        self.autocorrect = None;
        self.merge = None;
        self.split = None;
        self.chosen = None;
        self.alternatives = cands
            .iter()
            .skip(1)
            .take(5)
            .map(|c| c.word.clone())
            .collect();
        self.drawn = Some((trail.to_vec(), ms.to_vec()));
        self.shown = word.clone();
        self.out.push(Op::Composing(word.clone()));
        // It goes in with its space: the next word starts after it (a space
        // tapped next isn't doubled, punctuation takes the space's place);
        // its other readings stay in the strip.
        self.space();
        self.auto_space = true;
        self.variant_strip();
    }

    /// Right after a drawn word went in: its other readings in the strip,
    /// the word itself in the center — a tap puts one in its place.
    fn variant_strip(&mut self) {
        self.variants_shown = false;
        let Some(p) = self.prev_here() else {
            return;
        };
        if p.variants.len() < 2 || self.settings.strip == Strip::Off {
            return;
        }
        let current = p.committed.clone();
        let others: Vec<String> = p
            .variants
            .iter()
            .map(|w| self.cased(&p.typed, w))
            .filter(|w| *w != current)
            .collect();
        self.slots = match self.settings.strip {
            Strip::Two => [others.first().cloned(), Some(current), None],
            _ => [
                others.first().cloned(),
                Some(current),
                others.get(1).cloned(),
            ],
        };
        self.variants_shown = true;
    }

    /// The strip for a drawn word: the word itself in the center (as it is
    /// in the text), the next readings beside it.
    fn gesture_strip(&mut self) {
        let current = self.chosen.clone().unwrap_or_else(|| self.word.clone());
        let lower = current.to_lowercase();
        let first = self.word.to_lowercase();
        let mut others: Vec<String> = Vec::new();
        for w in self.alternatives.iter().chain(std::iter::once(&first)) {
            let w = self.cased(&self.word, w);
            if w.to_lowercase() != lower && !others.contains(&w) {
                others.push(w);
            }
        }
        self.slots = match self.settings.strip {
            Strip::Off => [None, None, None],
            Strip::Two => [others.first().cloned(), Some(current), None],
            Strip::Full => [
                others.first().cloned(),
                Some(current),
                others.get(1).cloned(),
            ],
        };
    }

    /// Space swiped up: the word being typed — or, right after a space, the
    /// one before it — becomes its next likeliest reading; round and round,
    /// back to what was typed.
    fn next_variant(&mut self) {
        self.step_variant(1);
    }

    /// Space swiped down: back to the previous reading, the same round the
    /// other way.
    fn prev_variant(&mut self) {
        self.step_variant(-1);
    }

    /// One step round the readings (`step` 1: on, -1: back).
    fn step_variant(&mut self, step: i32) {
        if let Some(pu) = self
            .pair_undo
            .take()
            .filter(|pu| self.word == pu.typed && self.before.ends_with(&pu.prefix))
        {
            // Both words as ⌫ left them: the pair's readings again, the
            // first one now — then round the rest, back to what was typed.
            let n = pu.prefix.chars().count();
            self.out.push(Op::Composing(String::new()));
            self.out.push(Op::Delete(n));
            for _ in 0..n {
                self.before.pop();
            }
            let was = format!("{}{}", pu.prefix, pu.typed);
            // On: the change that was undone; back: the last reading.
            let first = if step > 0 {
                pu.variants[0].clone()
            } else {
                pu.variants[pu.variants.len() - 1].clone()
            };
            self.word.clear();
            self.hints.clear();
            self.durs.clear();
            self.shown.clear();
            self.autocorrect = None;
            self.chosen = None;
            self.slots = [None, None, None];
            self.undo = None;
            self.commit(format!("{first} "));
            self.prev_word = Some(unit(&was, &first, &pu.variants));
            return;
        }
        if !self.word.is_empty() {
            let lower = self.word.to_lowercase();
            let mut list: Vec<String> = Vec::new();
            // What the space would put, the strip's readings and the next
            // likeliest — and what was typed, unless it is the one-row
            // layout's column letters.
            let offered: Vec<String> = if self.drawn.is_some() {
                self.alternatives.clone()
            } else {
                self.last_cands.iter().map(|(w, _)| w.clone()).collect()
            };
            let typed = (!self.settings.one_row || self.drawn.is_some()).then(|| lower.clone());
            for w in self
                .autocorrect
                .iter()
                .chain(&offered)
                .chain(typed.iter())
                .take(VARIANTS + 1)
            {
                if !list.contains(w) {
                    list.push(w.clone());
                }
            }
            if list.is_empty() {
                list.push(lower.clone());
            }
            let now = self
                .chosen
                .as_ref()
                .map(|c| c.to_lowercase())
                .or_else(|| self.autocorrect.clone())
                .unwrap_or_else(|| lower.clone());
            let n = list.len() as i32;
            let i = list
                .iter()
                .position(|w| *w == now)
                .map_or(if step > 0 { 0 } else { list.len() - 1 }, |i| {
                    (i as i32 + step).rem_euclid(n) as usize
                });
            let next = self.cased(&self.word, &list[i]);
            self.log_event("variant", &[("word", jstr(&next))]);
            self.chosen = Some(next.clone());
            if next != self.shown {
                self.shown = next.clone();
                self.out.push(Op::Composing(next));
            }
            if self.drawn.is_some() {
                self.gesture_strip();
            }
            return;
        }
        let Some(prev) = self.prev_here().filter(|p| p.sep == ' ') else {
            return;
        };
        let (typed, committed, input) = (
            prev.typed.clone(),
            prev.committed.clone(),
            prev.input.clone(),
        );
        let mut variants = prev.variants.clone();
        if variants.is_empty() {
            let lower = typed.to_lowercase();
            let rest = self
                .before
                .strip_suffix(' ')
                .and_then(|b| b.strip_suffix(committed.as_str()))
                .unwrap_or("");
            let ctx = self.context_at(rest);
            let read = |input: &Input| -> Vec<String> {
                let mut c = self.decode_input(&ctx, input);
                c.retain(|c| !self.blocked(&c.word, &lower));
                c.truncate(WINDOW_ALTS);
                c.into_iter().map(|c| c.word).collect()
            };
            let mut alts: Vec<String> = if prev.alts.is_empty() {
                read(&input)
            } else {
                prev.alts.iter().map(|c| c.word.clone()).collect()
            };
            // A way that reads as the word alone: the words its letters
            // might be, as if typed.
            if alts.iter().all(|w| *w == lower) && matches!(input, Input::Drawn { .. }) {
                alts = read(&Input::typed(&lower, &[]));
            }
            variants.push(committed.to_lowercase());
            for w in alts.into_iter().chain(std::iter::once(lower)) {
                if !variants.contains(&w) {
                    variants.push(w);
                }
            }
        }
        let variant = (prev.variant as i32 + step).rem_euclid(variants.len() as i32) as usize;
        if let Some(p) = self.prev_word.as_mut() {
            p.variants = variants;
        }
        self.put_variant(variant);
    }

    /// The previous word's reading `variant` (of its variants) in its place.
    fn put_variant(&mut self, variant: usize) {
        let Some(prev) = self.prev_here() else {
            return;
        };
        let (typed, committed, variants) = (
            prev.typed.clone(),
            prev.committed.clone(),
            prev.variants.clone(),
        );
        let Some(word) = variants.get(variant) else {
            return;
        };
        let next = self.cased(&typed, word);
        self.log_event("variant", &[("word", jstr(&next))]);
        let n = committed.chars().count() + 1;
        self.out.push(Op::Delete(n));
        for _ in 0..n {
            self.before.pop();
        }
        self.undo = None;
        let keep = self.prev_word.take();
        self.commit(format!("{next} "));
        if let Some(mut p) = keep {
            p.committed = next;
            p.variants = variants;
            p.variant = variant;
            self.prev_word = Some(p);
        }
    }

    /// ⌫ held long enough takes whole words: the composing word, or the
    /// spaces, punctuation and word before the cursor.
    fn delete_word(&mut self) {
        self.undo = None;
        self.prev_word = None;
        self.auto_space = false;
        self.selected = false;
        if !self.word.is_empty() {
            self.word.clear();
            self.hints.clear();
            self.durs.clear();
            self.shown.clear();
            self.autocorrect = None;
            self.editing = false;
            self.leave_word();
            self.out.push(Op::Composing(String::new()));
            return;
        }
        let chars: Vec<char> = self.before.chars().collect();
        let mut n = chars.iter().rev().take_while(|c| c.is_whitespace()).count();
        let rest = &chars[..chars.len() - n];
        let punct = rest
            .iter()
            .rev()
            .take_while(|&&c| !c.is_whitespace() && !self.is_resumable(c))
            .count();
        let rest = &rest[..rest.len() - punct];
        let word = rest
            .iter()
            .rev()
            .take_while(|&&c| self.is_resumable(c))
            .count();
        n = (n + punct + word).max(1);
        self.before = chars[..chars.len().saturating_sub(n)].iter().collect();
        self.out.push(Op::Delete(n));
        self.leave_word();
        self.update_shift();
    }

    /// ⌫ let go after a hold: look up where it stopped, once.
    fn end_hold(&mut self) {
        if std::mem::take(&mut self.held_bksp) == 0 {
            return;
        }
        if self.word.is_empty() {
            self.resume_word();
        }
        if self.word.is_empty() {
            self.refresh_predictions();
        } else {
            self.update_slots();
            self.show();
        }
    }

    /// Backspacing into the end of a word makes it the composing word again,
    /// so its suggestions come back (a typo noticed one word later).
    fn resume_word(&mut self) {
        if !self.suggest {
            return;
        }
        let tail: Vec<char> = self
            .before
            .chars()
            .rev()
            .take_while(|&c| self.is_resumable(c))
            .collect();
        // Only whole words: the kept tail must not have been cut by BEFORE_CHARS.
        if tail.is_empty()
            || tail.len() == self.before.chars().count()
                && self.before.chars().count() >= BEFORE_CHARS
        {
            return;
        }
        let word: String = tail.into_iter().rev().collect();
        let n = word.chars().count();
        self.before = self
            .before
            .chars()
            .take(self.before.chars().count() - n)
            .collect();
        self.out.push(Op::Delete(n));
        self.word = word;
        self.hints = vec![Hint::default(); n];
        self.durs = vec![None; n];
        self.clean = false;
        self.editing = true;
        self.restored = true;
        self.erased = 0;
        self.update_slots();
        self.show();
    }

    /// Put the composing text in the field: what was typed or — with live
    /// correction on, once the typed letters can't start any word — the
    /// correction (the original then sits in the strip).
    fn show(&mut self) {
        let display = match &self.autocorrect {
            _ if self.chosen.is_some() => self.chosen.clone().unwrap_or_default(),
            _ if self.editing && !self.settings.one_row => self.word.clone(),
            // One-row taps are column placeholders: show the decoded word.
            Some(a) if self.settings.one_row => self.cased(&self.word, a),
            Some(a)
                if self.settings.live_correction
                    && !self.engine.is_prefix(&self.word.to_lowercase()) =>
            {
                self.cased(&self.word, a)
            }
            _ => self.word.clone(),
        };
        if display != self.shown {
            self.shown = display;
            self.out.push(Op::Composing(self.shown.clone()));
        }
    }

    fn is_resumable(&self, c: char) -> bool {
        c.is_alphabetic()
            && kbcore::alphabet::char_to_id(c.to_lowercase().next().unwrap_or(c)).is_some()
            || c == '\''
            || c == '-'
    }

    fn pick_slot(&mut self, i: usize) {
        let Some(s) = self.slots[i].clone() else {
            return;
        };
        if s.starts_with(CLIP_MARK) && self.word.is_empty() {
            if let Some(text) = self.stash.clips.first().cloned() {
                self.paste(&text);
            }
            return;
        }
        if let Some((w, add)) = self.dict_actions[i].take() {
            self.dict_actions = [None, None, None];
            if add {
                self.add_user_word(&w);
            } else {
                self.remove_user_word(&w);
            }
            self.slots = [None, None, None];
            return;
        }
        self.log_event(
            "pick",
            &[
                ("slot", i.to_string()),
                ("text", jstr(&s)),
                ("typed", jstr(&self.word)),
            ],
        );
        if let Some((left, right)) = self.recorrect.take().filter(|_| self.word.is_empty()) {
            // The word the cursor is in, replaced whole.
            self.out.push(Op::Replace(left, right, s.clone()));
            let keep = self.before.chars().count().saturating_sub(left);
            let joined: String = self.before.chars().take(keep).chain(s.chars()).collect();
            self.set_before(&joined);
            self.after = self.after.chars().skip(right).collect();
            self.slots = [None, None, None];
            self.forget_spot();
            self.update_shift();
            return;
        }
        if std::mem::take(&mut self.variants_shown) && self.word.is_empty() {
            let at = self
                .prev_here()
                .and_then(|p| p.variants.iter().position(|w| self.cased(&p.typed, w) == s));
            if let Some(i) = at {
                self.put_variant(i);
                self.variant_strip();
                return;
            }
        }
        if let Some(m) = self.merge.clone().filter(|_| s.starts_with(MERGE_MARK)) {
            self.apply_merge(m);
            return;
        }
        if let Some(sp) = self
            .split
            .clone()
            .filter(|sp| s == format!("{} {}", sp.left, sp.right))
        {
            self.apply_split(sp);
            return;
        }
        self.prev_word = None;
        self.word.clear();
        self.shown.clear();
        self.hints.clear();
        self.durs.clear();
        self.autocorrect = None;
        self.slots = [None, None, None];
        self.undo = None;
        self.commit(s);
        self.commit(" ".into());
    }

    fn update_shift(&mut self) {
        if self.shift == Shift::Caps {
            return;
        }
        // Inside a word (typed on, or taken up again after ⌫): no capital,
        // though the text before it starts a sentence («Привет?» ⌫ о).
        self.shift = if self.auto_caps && self.word.is_empty() && sentence_start(&self.before) {
            Shift::Once
        } else {
            Shift::Off
        };
    }

    /// The word ending `text` before its trailing spaces, lowercase; None
    /// across punctuation.
    fn word_before(&self, text: &str) -> Option<String> {
        let t = text.trim_end_matches(' ');
        if t.len() == text.len() {
            return self.elided_before(text);
        }
        let word: String = t
            .chars()
            .rev()
            .take_while(|&c| self.is_resumable(c))
            .collect();
        (!word.is_empty()).then(|| word.chars().rev().collect::<String>().to_lowercase())
    }

    /// French: the clitic right before the cursor (`l'`), the previous word of
    /// the one typed after it.
    fn elided_before(&self, text: &str) -> Option<String> {
        let stem = text.strip_suffix('\'').filter(|_| self.lang == Lang::Fr)?;
        let letters: String = stem
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic())
            .collect();
        let letters: String = letters.chars().rev().collect::<String>().to_lowercase();
        ELIDED
            .contains(&letters.as_str())
            .then(|| format!("{letters}'"))
    }

    /// The words of `text` back to the last punctuation (at most 5),
    /// lowercase: the phrase the next word has to fit.
    fn phrase_of(text: &str) -> Vec<String> {
        let tail: Vec<char> = text
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\''))
            .collect();
        let tail: String = tail.into_iter().rev().collect();
        let mut words: Vec<String> = tail.split_whitespace().map(str::to_lowercase).collect();
        let n = words.len();
        words.drain(..n.saturating_sub(5));
        words
    }

    /// What `text` says about the word after it — the word before, the
    /// phrase, the sentence ([`kbcore::Context`]): every reading of a word,
    /// typed, completed, drawn or predicted, is weighed by it.
    fn context_at(&self, text: &str) -> Context {
        self.engine.context(
            self.word_before(text).as_deref(),
            &Self::phrase_of(text),
            &Self::sentence_of(text),
        )
    }

    /// The words of `text` back to the end of the sentence before (at most
    /// 12), lowercase: what the sentence is about.
    fn sentence_of(text: &str) -> Vec<String> {
        let tail: Vec<char> = text
            .chars()
            .rev()
            .take_while(|c| !matches!(c, '.' | '!' | '?' | '…' | '\n'))
            .collect();
        let tail: String = tail.into_iter().rev().collect();
        let mut words: Vec<String> = tail
            .split(|c: char| !(c.is_alphabetic() || c == '-'))
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        let n = words.len();
        words.drain(..n.saturating_sub(12));
        words
    }

    /// With no word being typed: the likeliest next words after the previous
    /// one (context model), center first.
    fn refresh_predictions(&mut self) {
        self.recorrect = None;
        if !self.word.is_empty() || self.held_bksp > 0 {
            return;
        }
        self.slots = [None, None, None];
        self.predicted.clear();
        self.decoded_next.clear();
        self.predict_next();
        self.offer_clip();
    }

    fn predict_next(&mut self) {
        if self.settings.strip == Strip::Off || !self.suggest || self.settings.one_row {
            return;
        }
        let ctx = self.context_at(&self.before);
        self.decoded_next = self.engine.decode(&ctx, Evidence::Nothing);
        self.predicted = self
            .decoded_next
            .iter()
            .map(|c| c.word.clone())
            .filter(|w| !self.blocked(w, ""))
            .take(3)
            .collect();
        let shown: Vec<String> = self.predicted.iter().map(|w| self.cased("", w)).collect();
        let mut next = shown.into_iter();
        let center = next.next();
        self.slots = [next.next(), center, next.next()];
        self.fit_strip();
    }

    fn update_slots(&mut self) {
        self.recorrect = None;
        self.slots = [None, None, None];
        self.autocorrect = None;
        self.merge = None;
        self.split = None;
        if self.word.is_empty() {
            // Erased back to nothing: the next-word guesses return.
            self.refresh_predictions();
            return;
        }
        if self.held_bksp > 0 {
            return;
        }
        let lower = self.word.to_lowercase();
        let mut corrections = self.engine.read(Evidence::Taps(&lower, &self.hints));
        corrections.retain(|c| !self.blocked(&c.word, &lower));
        // Stretched interjections (ааа, ммм, эээ) are words when typed on
        // purpose, but never what a slip was meant to be.
        corrections.retain(|c| c.word == lower || !stretched(&c.word));
        // A correction the user undid here isn't offered again; typing the
        // same word again means it was meant.
        let insisted = self.rejected.iter().any(|(t, _)| *t == lower);
        let rejected = |w: &str| self.rejected.iter().any(|(_, f)| f == w);
        if !self.settings.one_row && !insisted {
            let alone = corrections.first().map_or(f32::INFINITY, |c| c.cost);
            self.merge = self
                .find_hyphen(&lower)
                .or_else(|| self.find_merge(&lower, alone));
            if self.merge.is_none() {
                self.split = self.find_split(&lower, alone);
            }
        }
        let plain = corrections.clone();
        let ctx = self.context_at(&self.before);
        let mut corrections = self
            .engine
            .decode(&ctx, Evidence::Taps(&lower, &self.hints));
        corrections.retain(|c| !self.blocked(&c.word, &lower));
        corrections.retain(|c| c.word == lower || !stretched(&c.word));
        self.last_cands = corrections
            .iter()
            .take(VARIANTS)
            .map(|c| (c.word.clone(), c.cost))
            .collect();
        self.decoded = corrections.clone();
        if self.settings.one_row {
            // The typed letters are placeholders: the best decoding is the
            // word — unless every letter was picked exactly (an abbreviation).
            let exact = !self.hints.is_empty() && self.hints.iter().all(|h| h.kept >= KEPT_HARD);
            self.autocorrect = if exact {
                None
            } else {
                corrections.first().map(|c| c.word.clone())
            };
            if self.settings.strip != Strip::Off {
                let typed = self.word.clone();
                let words: Vec<String> = corrections
                    .iter()
                    .map(|c| self.cased(&typed, &c.word))
                    .collect();
                let mut words = words.into_iter();
                let center = words.next();
                self.slots = [words.next(), center, words.next()];
            }
            return;
        }
        let mut completions = self.engine.decode(&ctx, Evidence::Begun(&lower));
        completions.retain(|c| !stretched(&c.word) && !self.blocked(&c.word, &lower));
        // Abbreviations are left alone: several capitals (ГОСТ, iPhone) or a
        // short vowel-less token (тс, хз, спс, lmk).
        // Known abbreviations live in the dictionary (slang list); an unknown
        // vowel-less token one slip away from a word is a typo (`чтт` → `что`).
        let one_slip = corrections
            .iter()
            .find(|c| c.word != lower)
            .is_some_and(|c| c.edit <= ONE_SLIP);
        let vowelless = (2..=5).contains(&lower.chars().count())
            && !lower.chars().any(|c| VOWELS.contains(c))
            && !one_slip;
        // A letter drawn out on purpose (каааак) is emphasis, not a slip.
        let drawn_out = lower
            .chars()
            .collect::<Vec<_>>()
            .windows(3)
            .any(|w| w[0] == w[1] && w[1] == w[2]);
        let shaped =
            vowelless || drawn_out || self.word.chars().filter(|c| c.is_uppercase()).count() >= 2;
        let n = lower.chars().count();
        if self.settings.autocorrect && !shaped && !insisted && n >= 2 {
            let (per_letter, lead, known_margin) = self.settings.strength.limits();
            let known = self.engine.contains(&lower);
            // A real word typed is guarded: kept unless a cheap slip, or a
            // form the grammar asks for. Not at «Always»: there it gives way
            // to any reading clearly likelier (the margin still protects what
            // is itself likely).
            let always = self.settings.strength == crate::settings::Strength::Always;
            let guarded = known && !always;
            // What the typed word costs here: the search at the place may have
            // left it out, crowded by likelier words — its reading on its own,
            // weighed by the place, is the same number.
            let typed_cost = corrections
                .iter()
                .find(|c| c.word == lower)
                .map(|c| c.cost)
                .or_else(|| {
                    let mut own: Vec<Candidate> =
                        plain.iter().filter(|c| c.word == lower).cloned().collect();
                    self.engine.weigh(&ctx, &mut own);
                    own.first().map(|c| c.cost)
                });
            // An undone fix stays on offer in the strip (a tap puts it back)
            // but space never applies it again here. A word not yet finished
            // competes with its completions too, as they stand in context:
            // «такого исх» offered «исхода», «такого исхо» must not jump to
            // «исход» because that one is a letter closer.
            let mut pool: Vec<&Candidate> = corrections
                .iter()
                .filter(|c| c.word != lower && !rejected(&c.word))
                .collect();
            if !guarded && n >= COMPLETE_FROM {
                for c in &completions {
                    let extra = c.word.chars().count().saturating_sub(n);
                    if !(1..=COMPLETE_ON_SPACE).contains(&extra) || rejected(&c.word) {
                        continue;
                    }
                    // A word both a correction and a completion: at its
                    // better cost.
                    match pool.iter_mut().find(|p| p.word == c.word) {
                        Some(p) if c.cost < p.cost => *p = c,
                        Some(_) => {}
                        None => pool.push(c),
                    }
                }
                pool.sort_by(|a, b| a.cost.total_cmp(&b.cost));
            }
            let typed_fit = known.then(|| self.engine.grammar_fit(&ctx, &lower));
            let other_form = |c: &Candidate| {
                typed_fit.is_some_and(|t| {
                    c.edit <= GRAMMAR_SLIP
                        && same_stem(&lower, &c.word)
                        && self.engine.grammar_fit(&ctx, &c.word) - t >= GRAMMAR_GAP
                })
            };
            // A guarded word's completion too, when it is the form the
            // grammar asks for («эта девочка бежа» → «бежала»), space before
            // its end counted as a slip.
            let mut early: Vec<Candidate> = Vec::new();
            if guarded {
                for c in &completions {
                    let extra = c.word.chars().count().saturating_sub(n);
                    if !(1..=COMPLETE_ON_SPACE).contains(&extra)
                        || rejected(&c.word)
                        || !other_form(c)
                    {
                        continue;
                    }
                    early.push(Candidate {
                        cost: c.cost + SPACE_EARLY,
                        edit: c.edit,
                        word: c.word.clone(),
                    });
                }
            }
            for c in &early {
                match pool.iter_mut().find(|p| p.word == c.word) {
                    Some(p) if c.cost < p.cost => *p = c,
                    Some(_) => {}
                    None => pool.push(c),
                }
            }
            if !early.is_empty() {
                pool.sort_by(|a, b| a.cost.total_cmp(&b.cost));
            }
            let mut others = pool.into_iter();
            let best = others.next();
            let runner_up = others.next().map_or(f32::INFINITY, |c| c.cost);
            self.autocorrect = best
                .filter(|c| c.edit <= per_letter * n.max(3) as f32)
                .filter(|c| runner_up - c.cost >= lead)
                // A real word typed: only a cheap slip, or another form of
                // it the grammar asks for, clearly likelier (the context is
                // already in the costs — a missing word pair only means the
                // previous word doesn't change its odds).
                // (A completion the grammar asks for has paid for space
                // before the word's end: no margin on top.)
                .filter(|c| {
                    let margin = if early.iter().any(|e| std::ptr::eq(e, *c)) {
                        0.0
                    } else {
                        known_margin
                    };
                    !known
                        || (c.edit <= self.known_slip || other_form(c) || !guarded)
                            && typed_cost.is_some_and(|t| c.cost + margin < t)
                })
                .map(|c| c.word.clone());
        }
        // Once this word reads as a word, the previous one may read better.
        if self.settings.autocorrect
            && self.merge.is_none()
            && self.split.is_none()
            && !self.settings.one_row
            && !insisted
            && (self.engine.contains(&lower) || self.autocorrect.is_some())
        {
            self.merge = self.find_window(&plain);
        }
        if self.settings.strip == Strip::Off {
            return;
        }

        let mut merged: Vec<Candidate> = corrections.into_iter().chain(completions).collect();
        merged.sort_by(|a, b| a.cost.total_cmp(&b.cost));
        self.alternatives.clear();
        for c in &merged {
            if c.word != lower
                && !self.alternatives.contains(&c.word)
                && self.alternatives.len() < 5
            {
                self.alternatives.push(c.word.clone());
            }
        }
        let mut seen: Vec<String> = Vec::new();
        // A word shown as the next-word guess stays first while it matches.
        let sticky: Vec<String> = self
            .predicted
            .iter()
            .filter(|w| w.starts_with(&lower) && **w != lower)
            .cloned()
            .collect();
        let mut ranked = sticky
            .into_iter()
            .chain(merged.into_iter().map(|c| c.word))
            .filter(|w| {
                let new = !seen.contains(w);
                seen.push(w.clone());
                new
            });

        let typed = self.word.clone();
        if self.settings.live_correction {
            // Left: always exactly what was typed. Center: the fix space will
            // make — or, once the text already shows it, the next best (no
            // point repeating what is on screen); right: the one after.
            let on_screen = self
                .autocorrect
                .clone()
                .filter(|_| !self.editing && !self.engine.is_prefix(&lower));
            let mut others = ranked.filter(|w| *w != lower && Some(w) != on_screen.as_ref());
            let center = match on_screen {
                Some(_) => others.next(),
                None => self.autocorrect.clone().or_else(|| others.next()),
            };
            let right = others.find(|w| Some(w) != center.as_ref());
            self.slots = [
                Some(typed.clone()),
                center.map(|w| self.cased(&typed, &w)),
                right.map(|w| self.cased(&typed, &w)),
            ];
            self.place_merge();
            self.fit_strip();
            return;
        }
        let center = self
            .autocorrect
            .clone()
            .or_else(|| ranked.next())
            .unwrap_or_else(|| lower.clone());
        let mut rest = ranked.filter(|w| *w != center);
        // With an autocorrect pending, the left slot keeps the word exactly as typed.
        let left = if self.autocorrect.is_some() {
            Some(typed.clone())
        } else {
            rest.next().map(|w| self.cased(&typed, &w))
        };
        let right = rest.next().map(|w| self.cased(&typed, &w));
        self.slots = [left, Some(self.cased(&typed, &center)), right];
        self.place_merge();
        self.fit_strip();
    }

    /// Trim the strip to the chosen mode: "two" keeps what was typed on the
    /// left and the fix (or best alternative) in the center, nothing right.
    fn fit_strip(&mut self) {
        match self.settings.strip {
            Strip::Full => {}
            Strip::Off => self.slots = [None, None, None],
            Strip::Two if self.word.is_empty() || self.settings.one_row => {
                self.slots[0] = None;
                self.slots[2] = None;
            }
            Strip::Two => {
                let typed = self.word.clone();
                if self.slots[1].as_deref() == Some(typed.as_str()) {
                    self.slots[1] = self.slots[2]
                        .take()
                        .or_else(|| self.slots[0].take().filter(|s| *s != typed));
                }
                self.slots[0] = Some(typed);
                self.slots[2] = None;
            }
        }
    }

    /// The center suggestion is what space will put in place of the word
    /// (drawn as a filled pill, so it's clear before pressing space).
    fn center_applies(&self) -> bool {
        if self.editing || !self.settings.autocorrect || self.word.ends_with('-') {
            return false;
        }
        let Some(center) = self.slots[1].as_deref() else {
            return false;
        };
        if let Some(sp) = &self.split {
            return center == format!("{} {}", sp.left, sp.right);
        }
        if self.merge.is_some() {
            return center.starts_with(MERGE_MARK);
        }
        self.autocorrect
            .as_ref()
            .is_some_and(|a| center.to_lowercase() == *a)
    }

    /// A pending "that space was a letter" takes the center of the strip; the
    /// word it displaced moves right.
    fn place_merge(&mut self) {
        let text = match (&self.merge, &self.split) {
            (Some(m), _) => format!("{MERGE_MARK}{}", m.word),
            (None, Some(sp)) => format!("{} {}", sp.left, sp.right),
            _ => return,
        };
        let displaced = self.slots[1].replace(text);
        self.slots[2] = displaced;
    }

    /// Pending editor operations, encoded for the shim: ops separated by `\0`,
    /// each `C<text>` commit, `Z<text>` composing, `F` finish composing,
    /// `D<n>` delete before cursor, `E` enter/editor action.
    pub fn take_output(&mut self) -> String {
        let ops: Vec<String> = self
            .out
            .drain(..)
            .map(|op| match op {
                Op::Commit(t) => format!("C{t}"),
                Op::Composing(t) => format!("Z{t}"),
                Op::Finish => "F".into(),
                Op::Delete(n) => format!("D{n}"),
                Op::Enter => "E".into(),
                Op::Skip => "K".into(),
                Op::Replace(b, a, t) => format!("R{b}\t{a}\t{t}"),
                Op::Picker => "P".into(),
                Op::Settings => "S".into(),
            })
            .collect();
        ops.join("\0")
    }

    #[cfg(test)]
    fn take_ops(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.out)
    }

    fn label(&self, key: &Key) -> String {
        let upper = self.shift != Shift::Off && self.layer == Layer::Letters;
        match key.action {
            Action::Char(c) if upper => c.to_uppercase().collect(),
            Action::Char(c) => c.to_string(),
            Action::Shift => if self.shift == Shift::Caps {
                "⇪"
            } else {
                "⇧"
            }
            .into(),
            Action::Backspace => "⌫".into(),
            Action::Page(step) => if step < 0 { "‹" } else { "›" }.into(),
            Action::Space if self.panel.is_some() => self.panel.map_or_else(String::new, |p| {
                format!(
                    "{} {}/{}",
                    p.tab.icon(),
                    p.page + 1,
                    Panel::pages(p.tab, &self.stash, self.emoji_max)
                )
            }),
            Action::Space if self.settings.languages.len() > 1 => {
                format!("‹ {} ›", self.lang.name())
            }
            Action::Space => self.lang.name().into(),
            Action::Enter => "⏎".into(),
            Action::Lang => self.lang.short().into(),
            Action::Symbols => "?123".into(),
            Action::Symbols2 => "=\\<".into(),
            Action::Letters => self.lang.abc().into(),
            // Drawn as stacked letters (see `draw`).
            Action::Column(_) => String::new(),
        }
    }

    /// The draw list: `(ops, texts)`, 7 ints per op (see `OP_*`).
    pub fn draw(&self) -> (Vec<i32>, Vec<String>) {
        let pal = self.palette();
        let dp = self.dp;
        let height = self.height();
        let mut ops: Vec<i32> = Vec::with_capacity(64 * 7 * 2);
        let mut texts: Vec<String> = Vec::new();
        let rect = |ops: &mut Vec<i32>, x: f32, y: f32, w: f32, h: f32, color: i32, r: f32| {
            ops.extend([
                OP_RECT, x as i32, y as i32, w as i32, h as i32, color, r as i32,
            ]);
        };
        let text = |ops: &mut Vec<i32>,
                    texts: &mut Vec<String>,
                    s: String,
                    cx: f32,
                    cy: f32,
                    size: f32,
                    color: i32,
                    bold: bool| {
            // Baseline so the text's x-height sits on the center line.
            ops.extend([
                OP_TEXT,
                cx as i32,
                (cy + size * 0.35) as i32,
                size as i32,
                color,
                texts.len() as i32,
                bold as i32,
            ]);
            texts.push(s);
        };

        // A pixel more than the view: rounding must not leave a line uncovered.
        rect(
            &mut ops,
            0.0,
            0.0,
            self.width,
            height.ceil() + 1.0,
            pal.bg,
            0.0,
        );

        // The emoji and clipboard panel: its tabs where the strip is, a page
        // over the letter rows (the keys below are its bottom row).
        let (gx, gy) = (GAP_X_DP * dp / 2.0, GAP_Y_DP * dp / 2.0);
        if let Some(p) = &self.panel {
            let frame = self.panel_frame();
            let pressed = self.pointers.iter().find_map(|q| match q.target {
                Some(Target::Panel(h)) if !q.done => Some(h),
                _ => None,
            });
            let tw = frame.tab_w();
            for i in 0..panel::TABS {
                if i == p.selected() {
                    let (x, y) = (i as f32 * tw + 4.0 * dp, frame.tabs_h - 5.0 * dp);
                    rect(
                        &mut ops,
                        x,
                        y,
                        tw - 8.0 * dp,
                        3.0 * dp,
                        pal.accent,
                        1.5 * dp,
                    );
                }
                let icon = Tab::from_index(i).icon().to_string();
                let cx = (i as f32 + 0.5) * tw;
                let size = (19.0 * dp).min(tw * 0.55);
                text(
                    &mut ops,
                    &mut texts,
                    icon,
                    cx,
                    frame.tabs_h / 2.0,
                    size,
                    pal.text,
                    false,
                );
            }
            let items = p.page_items(&self.stash, self.emoji_max);
            if items.is_empty() {
                let key = if p.tab == Tab::Clips {
                    "panel.clips_empty"
                } else {
                    "panel.recent_empty"
                };
                let line = t(self.ui, key).to_string();
                let size =
                    (15.0 * dp).min(self.width / (line.chars().count() as f32 * 0.5).max(1.0));
                let mid = (frame.tabs_h + frame.page_bottom) / 2.0;
                text(
                    &mut ops,
                    &mut texts,
                    line,
                    self.width / 2.0,
                    mid,
                    size,
                    pal.text_dim,
                    false,
                );
            }
            if p.tab == Tab::Clips {
                for (i, clip) in items.iter().enumerate() {
                    let (x, y, w, h) = frame.clip_row(i);
                    let lit = pressed == Some(Hit::Cell(i as u8));
                    let bg = if lit { pal.key_pressed } else { pal.key };
                    rect(
                        &mut ops,
                        x + gx,
                        y + gy,
                        w - 2.0 * gx,
                        h - 2.0 * gy,
                        bg,
                        RADIUS_DP * dp,
                    );
                    let room = w - 1.2 * h - 16.0 * dp;
                    let line = panel::preview(clip, (room / (8.0 * dp)).max(4.0) as usize);
                    let cx = (w - 1.2 * h) / 2.0;
                    text(
                        &mut ops,
                        &mut texts,
                        line,
                        cx,
                        y + h / 2.0,
                        15.0 * dp,
                        pal.text,
                        false,
                    );
                    let xc = w - 0.6 * h;
                    text(
                        &mut ops,
                        &mut texts,
                        "✕".into(),
                        xc,
                        y + h / 2.0,
                        16.0 * dp,
                        pal.text_dim,
                        false,
                    );
                }
            } else {
                let (_, _, cw, ch) = frame.cell(0);
                let size = (ch * 0.6).min(cw * 0.6);
                for (i, e) in items.iter().enumerate() {
                    let (x, y, w, h) = frame.cell(i);
                    if pressed == Some(Hit::Cell(i as u8)) {
                        rect(
                            &mut ops,
                            x + gx,
                            y + gy,
                            w - 2.0 * gx,
                            h - 2.0 * gy,
                            pal.key_pressed,
                            RADIUS_DP * dp,
                        );
                    }
                    text(
                        &mut ops,
                        &mut texts,
                        e.to_string(),
                        x + w / 2.0,
                        y + h / 2.0,
                        size,
                        pal.text,
                        false,
                    );
                }
            }
        }

        // Suggestion strip — or the calibration prompt / a notice.
        let slot_w = self.width / 3.0;
        let strip_mid = self.strip_h() / 2.0;
        let prompt = self.strip_prompt().filter(|_| self.panel.is_none());
        if let Some((line, way_out)) = &prompt {
            let (cx, w) = if way_out.is_some() {
                (slot_w, 2.0 * slot_w)
            } else {
                (self.width / 2.0, self.width)
            };
            let size = (17.0 * dp).min(w / (line.chars().count() as f32 * 0.55).max(1.0));
            text(
                &mut ops,
                &mut texts,
                line.clone(),
                cx,
                strip_mid,
                size,
                pal.text,
                false,
            );
            if let Some(out) = way_out {
                let x = 2.5 * slot_w;
                text(
                    &mut ops,
                    &mut texts,
                    out.to_string(),
                    x,
                    strip_mid,
                    15.0 * dp,
                    pal.text_dim,
                    false,
                );
            }
        }
        let slots: &[Option<String>] = if prompt.is_some() || self.panel.is_some() {
            &[]
        } else {
            &self.slots
        };
        for (i, s) in slots.iter().enumerate() {
            if i > 0 {
                rect(
                    &mut ops,
                    i as f32 * slot_w,
                    strip_mid - 10.0 * dp,
                    dp.max(1.0),
                    20.0 * dp,
                    pal.key_special,
                    0.0,
                );
            }
            if let Some(s) = s {
                let applies = i == 1 && self.center_applies();
                if applies {
                    rect(
                        &mut ops,
                        slot_w + 6.0 * dp,
                        strip_mid - 15.0 * dp,
                        slot_w - 12.0 * dp,
                        30.0 * dp,
                        pal.accent,
                        15.0 * dp,
                    );
                }
                let bold = applies;
                let color = if applies {
                    pal.text_on_accent
                } else if i == 1 {
                    pal.text
                } else {
                    pal.text_dim
                };
                // A long word gets smaller to fit its third of the strip;
                // past the smallest size, it is cut short.
                let avail = slot_w - 14.0 * dp;
                let per_char = if bold { 0.62 } else { 0.58 };
                let n = s.chars().count().max(1) as f32;
                let size = (avail / (n * per_char)).min(17.0 * dp);
                let smallest = 10.0 * dp;
                let (label, size) = if size >= smallest {
                    (s.clone(), size)
                } else {
                    let keep = (avail / (smallest * per_char)).floor() as usize;
                    let cut: String = s.chars().take(keep.saturating_sub(1)).collect();
                    (format!("{cut}…"), smallest)
                };
                text(
                    &mut ops,
                    &mut texts,
                    label,
                    (i as f32 + 0.5) * slot_w,
                    strip_mid,
                    size,
                    color,
                    bold,
                );
            }
        }

        // The guessed grip, top right of the strip.
        if let (true, true, Some(g), None, None) = (
            self.settings.grip_indicator,
            self.grips.calibrated(),
            self.sense.shown(),
            &self.calib,
            &self.panel,
        ) {
            let (x, y) = (self.width - 18.0 * dp, 9.0 * dp);
            text(
                &mut ops,
                &mut texts,
                g.label().into(),
                x,
                y,
                11.0 * dp,
                pal.text_dim,
                false,
            );
        }

        // Keys.
        for (i, key) in self.keys.iter().enumerate() {
            if key.angle != 0.0 {
                let (cx, cy) = key.center();
                ops.extend([
                    OP_ROTATE,
                    cx as i32,
                    cy as i32,
                    (key.angle.to_degrees() * 1000.0) as i32,
                    0,
                    0,
                    0,
                ]);
            }
            let pressed = self
                .pointers
                .iter()
                .any(|p| p.target == Some(Target::Key(i)));
            let special = !matches!(
                key.action,
                Action::Char(_) | Action::Space | Action::Column(_)
            );
            let accent = match key.action {
                Action::Shift => self.shift != Shift::Off,
                Action::Enter => true,
                _ => false,
            };
            let bg = if pressed {
                pal.key_pressed
            } else if accent {
                pal.accent
            } else if special {
                pal.key_special
            } else {
                pal.key
            };
            rect(
                &mut ops,
                key.x + gx,
                key.y + gy,
                key.w - 2.0 * gx,
                key.h - 2.0 * gy,
                bg,
                RADIUS_DP * dp,
            );
            let (cx, cy) = key.center();
            if let Action::Column(col) = key.action {
                let letters = &layout::columns(self.lang)[col as usize].0;
                let n = letters.len() as f32;
                for (i, &l) in letters.iter().enumerate() {
                    let l = if self.shift != Shift::Off {
                        l.to_uppercase().collect()
                    } else {
                        l.to_string()
                    };
                    // On a slant, like the rows of the full keyboard.
                    let ly = key.y + gy + (key.h - 2.0 * gy) * (i as f32 + 0.5) / n;
                    let lx = cx + (key.w - 2.0 * gx) * 0.22 * (i as f32 - (n - 1.0) / 2.0);
                    text(&mut ops, &mut texts, l, lx, ly, 15.0 * dp, pal.text, false);
                }
                continue;
            }
            let label = self.label(key);
            let size = match key.action {
                Action::Char(_) => 23.0,
                // The arrows are thin glyphs: larger, and bold below.
                Action::Backspace | Action::Shift | Action::Enter => 28.0,
                Action::Space => 13.0,
                Action::Lang | Action::Symbols | Action::Symbols2 | Action::Letters => 15.0,
                _ => 21.0,
            } * dp;
            let bold = matches!(
                key.action,
                Action::Backspace | Action::Shift | Action::Enter
            );
            let fg = if accent && !pressed {
                pal.text_on_accent
            } else if key.action == Action::Space {
                pal.text_dim
            } else {
                pal.text
            };
            text(&mut ops, &mut texts, label, cx, cy, size, fg, bold);
            if let Some(alt) = key.alt {
                let hint = (key.x + key.w - gx - 7.0 * dp, key.y + gy + 9.0 * dp);
                text(
                    &mut ops,
                    &mut texts,
                    alt.to_string(),
                    hint.0,
                    hint.1,
                    10.0 * dp,
                    pal.text_dim,
                    false,
                );
            }
            if key.angle != 0.0 {
                ops.extend([OP_RESTORE, 0, 0, 0, 0, 0, 0]);
            }
        }
        // A held key's alternatives.
        if let Some(pop) = &self.popup {
            let (x0, y, w, h) = self.popup_rect();
            let n = pop.options.len() as f32;
            rect(&mut ops, x0, y, w * n, h, pal.key_pressed, RADIUS_DP * dp);
            for (i, &c) in pop.options.iter().enumerate() {
                let x = x0 + w * i as f32;
                let lit = i == pop.selected;
                if lit {
                    rect(
                        &mut ops,
                        x + 2.0 * dp,
                        y + 2.0 * dp,
                        w - 4.0 * dp,
                        h - 4.0 * dp,
                        pal.accent,
                        RADIUS_DP * dp,
                    );
                }
                let fg = if lit { pal.text_on_accent } else { pal.text };
                text(
                    &mut ops,
                    &mut texts,
                    c.to_string(),
                    x + w / 2.0,
                    y + h / 2.0,
                    22.0 * dp,
                    fg,
                    false,
                );
            }
        }

        // The arc being drawn in the calibration, or a word being drawn
        // across the keys: dots close enough to read as a line, however fast
        // the finger moved.
        let to_px = |(u, v): (f32, f32)| (u * self.width, self.strip_h() + v * self.width);
        // The area painted in this step: its strokes, as they were drawn.
        if let Some(cal) = self
            .calib
            .as_ref()
            .filter(|c| matches!(c.step(), Some(Step::Area(_))))
        {
            let r = 4.0 * dp;
            for stroke in cal.painted() {
                let pts: Vec<(f32, f32)> = stroke.into_iter().map(to_px).collect();
                for (i, &(x, y)) in pts.iter().enumerate() {
                    let (px, py) = if i > 0 { pts[i - 1] } else { (x, y) };
                    let steps = (((x - px).hypot(y - py) / r).ceil() as usize).clamp(1, 40);
                    for s in 1..=steps {
                        let f = s as f32 / steps as f32;
                        let (dx, dy) = (px + (x - px) * f, py + (y - py) * f);
                        rect(&mut ops, dx - r, dy - r, 2.0 * r, 2.0 * r, pal.accent, r);
                    }
                }
            }
        }
        let calib_trail: Option<Vec<(f32, f32)>> = self
            .calib
            .as_ref()
            .filter(|c| matches!(c.step(), Some(Step::Type(_))))
            .map(|cal| cal.trail().map(to_px).collect());
        // The phrase's taps: the last few, as dots — the letters stay visible.
        let typing = self
            .calib
            .as_ref()
            .is_some_and(|c| matches!(c.step(), Some(Step::Type(_))));
        let calib_trail = match calib_trail {
            Some(pts) if typing => {
                let r = 2.5 * dp;
                for &(x, y) in pts.iter().rev().take(6) {
                    rect(&mut ops, x - r, y - r, 2.0 * r, 2.0 * r, pal.accent, r);
                }
                None
            }
            other => other,
        };
        let gesture_trail = self
            .pointers
            .iter()
            .find(|p| p.gesture)
            .map(|p| p.trail.clone());
        if let Some(pts) = calib_trail.or(gesture_trail) {
            let r = 3.0 * dp;
            let mut dots = 0;
            for (i, &(x, y)) in pts.iter().enumerate() {
                let (px, py) = if i > 0 { pts[i - 1] } else { (x, y) };
                let steps = (((x - px).hypot(y - py) / r).ceil() as usize).clamp(1, 60);
                for s in 1..=steps {
                    let f = s as f32 / steps as f32;
                    let (dx, dy) = (px + (x - px) * f, py + (y - py) * f);
                    rect(&mut ops, dx - r, dy - r, 2.0 * r, 2.0 * r, pal.accent, r);
                    dots += 1;
                }
                if dots > 1500 {
                    break;
                }
            }
        }
        (ops, texts)
    }

    /// Center of the first key with this action (tests, diagnostics).
    /// A character as its key would give it, where this layer has no key
    /// of its own for it — a symbol from the other layer, an alternative
    /// from a held key (ё from е): for simulations.
    pub fn put_char(&mut self, c: char) -> i32 {
        self.type_char(c);
        REDRAW | OUTPUT
    }

    /// The word being typed read at its place: every candidate, best first
    /// (tools: what the keyboard chose among).
    pub fn decoded(&self) -> &[Candidate] {
        &self.decoded
    }

    /// The next word expected before any letter of it: every candidate, best
    /// first (tools).
    pub fn decoded_next(&self) -> &[Candidate] {
        &self.decoded_next
    }

    pub fn key_center(&self, action: Action) -> Option<(f32, f32)> {
        self.keys
            .iter()
            .find(|k| k.action == action)
            .map(Key::center)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grip::PHRASE;
    use fst::Map;
    use kbcore::{Config, Latin, Profile};

    /// An engine over `words`, with the Latin letters laid out as `latin`.
    fn test_engine(words: &[(&str, u64)], latin: Latin) -> Engine<Vec<u8>> {
        let mut kv: Vec<(Vec<u8>, u64)> = words
            .iter()
            .map(|(w, v)| {
                (
                    w.chars()
                        .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                        .collect(),
                    *v,
                )
            })
            .collect();
        kv.sort();
        let map = Map::from_iter(kv.iter().map(|(k, v)| (k.as_slice(), *v))).unwrap();
        let cfg = Config {
            latin,
            ..Config::balanced().with_profile(Profile::Phone)
        };
        Engine::new(map, cfg)
    }

    fn ime(words: &[(&str, u64)]) -> Ime<Vec<u8>> {
        let mut ime = Ime::new(test_engine(words, Latin::Qwerty), 2.625);
        ime.measure(1080);
        ime.set_settings(Settings {
            live_correction: false,
            strip: Strip::Full,
            ..Settings::default()
        });
        ime.start_input("", 1 | 0x4000); // text field, capitalize sentences
        ime
    }

    thread_local!(static CLOCK: std::cell::Cell<i64> = const { std::cell::Cell::new(0) });

    /// A fake event clock: each call is 100 ms later.
    fn now() -> i64 {
        CLOCK.with(|c| {
            c.set(c.get() + 100);
            c.get()
        })
    }

    /// Press and release a key, holding it `ms`.
    fn press(ime: &mut Ime<Vec<u8>>, action: Action, ms: i64) {
        let (x, y) = ime
            .key_center(action)
            .unwrap_or_else(|| panic!("no key {action:?}"));
        let t = now();
        ime.touch(DOWN, 0, x, y, t);
        ime.touch(UP, 0, x, y, t + ms);
    }

    fn tap(ime: &mut Ime<Vec<u8>>, c: char) {
        let action = match c {
            ' ' => Action::Space,
            '⌫' => Action::Backspace,
            '⏎' => Action::Enter,
            _ => Action::Char(c),
        };
        press(ime, action, 80);
    }

    fn type_str(ime: &mut Ime<Vec<u8>>, s: &str) {
        for c in s.chars() {
            tap(ime, c);
        }
    }

    const WORDS: &[(&str, u64)] = &[
        ("привет", 5000),
        ("как", 3000),
        ("дела", 4000),
        ("при", 4000),
    ];

    #[test]
    fn capitalizes_sentence_start_and_composes() {
        let mut k = ime(WORDS);
        type_str(&mut k, "при");
        let ops = k.take_ops();
        assert_eq!(ops.last(), Some(&Op::Composing("При".into())));
    }

    #[test]
    fn space_autocorrects_and_backspace_reverts() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт ");
        let ops = k.take_ops();
        assert!(ops.contains(&Op::Commit("Привет".into())), "{ops:?}");
        assert_eq!(ops.last(), Some(&Op::Commit(" ".into())));
        tap(&mut k, '⌫');
        assert_eq!(
            k.take_ops(),
            vec![Op::Delete(7), Op::Composing("Привкт".into())]
        );
    }

    #[test]
    fn backspace_after_a_comma_reverts_the_word_and_keeps_the_comma() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт,");
        let ops = k.take_ops();
        assert!(ops.contains(&Op::Commit("Привет".into())), "{ops:?}");
        tap(&mut k, '⌫');
        assert_eq!(
            k.take_ops(),
            vec![Op::Delete(7), Op::Commit("Привкт,".into())]
        );
        // The next ⌫ takes the comma, as any.
        tap(&mut k, '⌫');
        assert!(k.take_ops().contains(&Op::Delete(1)));
    }

    #[test]
    fn enter_puts_in_the_word_as_corrected_and_backspace_reverts() {
        let mut k = ime(WORDS);
        type_str(&mut k, "как дкла⏎");
        let ops = k.take_ops();
        assert!(ops.contains(&Op::Commit("дела".into())), "{ops:?}");
        assert_eq!(ops.last(), Some(&Op::Enter));
        tap(&mut k, '⌫');
        assert_eq!(
            k.take_ops(),
            vec![Op::Delete(5), Op::Commit("дкла\n".into())]
        );
    }

    #[test]
    fn an_undone_correction_is_not_applied_again() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт ⌫");
        assert!(
            k.slots.iter().flatten().any(|s| s == "Привет"),
            "a tap brings it back"
        );
        type_str(&mut k, "ы⌫");
        assert_ne!(k.autocorrect.as_deref(), Some("привет"));
        assert!(!k.center_applies());
        type_str(&mut k, ",");
        assert!(
            k.take_ops().contains(&Op::Commit("Привкт".into())),
            "keeps it before punctuation"
        );
    }

    #[test]
    fn going_on_with_a_restored_word_keeps_it() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт ⌫ы ");
        let ops = k.take_ops();
        assert!(ops.contains(&Op::Commit("Привкты".into())), "{ops:?}");
    }

    #[test]
    fn retyping_the_same_word_after_undo_keeps_it() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт ⌫⌫⌫⌫⌫⌫⌫привкт ");
        let ops = k.take_ops();
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::Commit(s) if s.to_lowercase() == "привкт")),
            "{ops:?}"
        );
        // …but only there: the next word is corrected as usual.
        type_str(&mut k, "привкт ");
        assert!(k.take_ops().contains(&Op::Commit("привет".into())));
    }

    #[test]
    fn erasing_several_letters_pins_the_rest() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт⌫⌫е");
        let kept: Vec<f32> = k.hints.iter().map(|h| h.kept).collect();
        assert_eq!(kept, vec![KEPT_HARD, KEPT_HARD, KEPT_HARD, KEPT_HARD, 0.0]);
        type_str(&mut k, "т⌫т");
        assert_eq!(k.hints[4].kept, KEPT_SOFT, "one letter erased: weighs more");
        assert_eq!(k.hints[0].kept, KEPT_HARD);
    }

    #[test]
    fn a_trailing_hyphen_is_kept() {
        let mut k = ime(&[("кто", 5000)]);
        type_str(&mut k, "кто");
        k.type_char('-'); // on the symbols layer
        tap(&mut k, ' ');
        let ops = k.take_ops();
        assert!(ops.contains(&Op::Commit("Кто-".into())), "{ops:?}");
    }

    #[test]
    fn drawn_out_words_are_emphasis() {
        let mut k = ime(&[("как", 5000), ("каппа", 1000)]);
        type_str(&mut k, "каааак ");
        let ops = k.take_ops();
        assert!(ops.contains(&Op::Commit("Каааак".into())), "{ops:?}");
    }

    #[test]
    fn periods_after_short_forms_do_not_capitalize() {
        for s in ["Привет. ", "Это я. ", "А. ", "Ну... ", "", "Да! "] {
            assert!(sentence_start(s), "{s:?}");
        }
        for s in [
            "и т. ",
            "и т.п. ",
            "ул. ",
            "т.е. ",
            "см. ",
            "слово ",
            "Привет.",
        ] {
            assert!(!sentence_start(s), "{s:?}");
        }
    }

    #[test]
    fn border_jitter_is_not_a_slide() {
        let mut k = ime(WORDS);
        let a = k
            .keys
            .iter()
            .find(|q| q.action == Action::Char('а'))
            .unwrap();
        let (x, y) = (a.x + a.w - 2.0, a.y + a.h / 2.0);
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(MOVE, 0, x + 7.0, y, t + 30);
        k.touch(UP, 0, x + 7.0, y, t + 60);
        assert_eq!(k.word, "А");
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(MOVE, 0, x + 40.0, y, t + 30);
        k.touch(UP, 0, x + 40.0, y, t + 60);
        assert_eq!(k.word, "Ап", "a real slide still moves to the next key");
    }

    #[test]
    fn enter_right_after_backspace_is_backspace() {
        let mut k = ime(WORDS);
        type_str(&mut k, "при⌫");
        let e = k.keys.iter().find(|q| q.action == Action::Enter).unwrap();
        let (x, y) = (e.x + e.w / 2.0, e.y + 4.0);
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(UP, 0, x, y, t + 60);
        assert_eq!(k.word, "П");
        assert!(!k.take_ops().contains(&Op::Enter));
    }

    #[test]
    fn punctuation_returns_to_letters_but_numbers_stay() {
        let mut k = ime(WORDS);
        type_str(&mut k, "как");
        press(&mut k, Action::Symbols, 60);
        press(&mut k, Action::Char('!'), 60);
        assert_eq!(k.layer, Layer::Letters);
        tap(&mut k, ' ');
        press(&mut k, Action::Symbols, 60);
        for c in ['3', ',', '5', '-', '6'] {
            press(&mut k, Action::Char(c), 60);
        }
        assert_eq!(k.layer, Layer::Symbols, "3,5-6");
        assert!(k.before.ends_with("3,5-6"), "{:?}", k.before);
    }

    #[test]
    fn holding_backspace_takes_words_without_lookups() {
        let mut k = ime(WORDS);
        k.start_input("один два три четыре пять шесть ", 1);
        type_str(&mut k, "семь");
        let (x, y) = k.key_center(Action::Backspace).unwrap();
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.take_ops();
        for _ in 0..CHAR_REPEATS {
            k.timer();
            assert!(k.slots.iter().all(|s| s.is_none()) && k.autocorrect.is_none());
        }
        // 12 chars: «сем» → «», « шесть » → «один два три четыре пя».
        assert!(k.before.ends_with("четыре пя"), "{:?}", k.before);
        k.timer();
        k.timer();
        let ops = k.take_ops();
        assert!(ops.ends_with(&[Op::Delete(2), Op::Delete(7)]), "{ops:?}");
        k.timer();
        k.touch(UP, 0, x, y, t + 3000);
        assert_eq!(k.before, "один два ");
        assert!(k.word.is_empty());
    }

    #[test]
    fn the_next_word_guess_stays_while_typing_it() {
        let dict = Map::from_iter(
            [
                ("бы", 900u64),
                ("был", 1000),
                ("быть", 3000),
                ("может", 2000),
            ]
            .iter()
            .map(|(w, v)| {
                (
                    w.chars()
                        .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                        .collect::<Vec<u8>>(),
                    *v,
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        let mut pair: Vec<u8> = "может"
            .chars()
            .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
            .collect();
        pair.push(0);
        pair.extend(
            "быть"
                .chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap()),
        );
        let bigrams = Map::from_iter([(pair, 300u64)]).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.start_input("может ", 1);
        assert_eq!(k.slots[1].as_deref(), Some("быть"), "{:?}", k.slots);
        type_str(&mut k, "б");
        assert_eq!(k.slots[1].as_deref(), Some("быть"), "{:?}", k.slots);
        type_str(&mut k, "ы");
        assert_eq!(k.slots[1].as_deref(), Some("быть"), "{:?}", k.slots);
    }

    #[test]
    fn a_sure_comma_goes_in_and_backspace_takes_it_out() {
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let words = [
            ("я", 9000u64),
            ("думаю", 6000),
            ("что", 4000),
            ("ты", 5000),
            ("прав", 7000),
        ];
        let dict = Map::from_iter(
            words
                .iter()
                .map(|(w, v)| (id(w), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        // A comma before «что»: almost surely (log odds 3).
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let mut kv = vec![
            (vec![0, 15, 0], byte(-2.3)),
            ([vec![0, 15, 1], id("что")].concat(), byte(3.0)),
        ];
        kv.sort();
        let bigrams = Map::from_iter(kv).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.start_input("", 1);
        type_str(&mut k, "я думаю что ");
        assert_eq!(k.before, "я думаю, что ");
        // ⌫ right after: out it goes, and it stays out for that pair.
        type_str(&mut k, "⌫");
        assert_eq!(k.before, "я думаю что ");
        type_str(&mut k, "ты прав. я думаю что ");
        assert!(k.before.ends_with("я думаю что "), "{:?}", k.before);
        // Not at the start of a sentence, nor after punctuation.
        k.start_input("", 1);
        type_str(&mut k, "что ты ");
        assert_eq!(k.before, "что ты ");
    }

    #[test]
    fn space_finishes_the_word_the_strip_offered() {
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let words = [("такого", 5000u64), ("исход", 6000), ("исхода", 5000)];
        let dict = Map::from_iter(
            words
                .iter()
                .map(|(w, v)| (id(w), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        // The grammar only: «такого» is the genitive, or the accusative with
        // an animate noun; «исход» is inanimate, «исхода» the genitive.
        let parse = kbcore::gram::parse;
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let word = |w: &str, set: u64| ([vec![0, 3], id(w)].concat(), 1 | set << 16);
        let mut kv = vec![
            word("такого", 1),
            word("исход", 2),
            word("исхода", 3),
            (vec![0, 9, 0, 1, 0], parse("ADJF,anim,masc,sing,accs")),
            (vec![0, 9, 0, 1, 1], parse("ADJF,masc,sing,gent")),
            (vec![0, 9, 0, 2, 0], parse("NOUN,inan,masc,sing,nomn")),
            (vec![0, 9, 0, 2, 1], parse("NOUN,inan,masc,sing,accs")),
            (vec![0, 9, 0, 3, 0], parse("NOUN,inan,masc,sing,gent")),
            // After adjectives: other -1.1, agree +0.5, disagree -2.7.
            (
                vec![0, 12],
                byte(-1.1)
                    | byte(0.5) << 8
                    | byte(-2.7) << 16
                    | byte(-0.8) << 24
                    | byte(-3.2) << 32,
            ),
        ];
        kv.sort();
        let bigrams = Map::from_iter(kv).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.start_input("", 1);
        type_str(&mut k, "такого исх");
        let offered = k.slots.clone();
        type_str(&mut k, "о");
        // What was offered stays offered, and space puts it in.
        assert_eq!(
            k.slots[1].as_deref(),
            Some("исхода"),
            "{offered:?} → {:?}",
            k.slots
        );
        type_str(&mut k, " ");
        assert_eq!(k.before, "такого исхода ");
    }

    #[test]
    fn the_grammar_finishes_a_real_word_it_rules_out() {
        // «девочка бежа»: a gerund right after a subject hardly ever comes;
        // space finishes it as the predicate, «бежала».
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let words = [("девочка", 5000u64), ("бежа", 12000), ("бежала", 7000)];
        let dict = Map::from_iter(
            words
                .iter()
                .map(|(w, v)| (id(w), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        let parse = kbcore::gram::parse;
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let pack = |v: &[f32]| {
            v.iter()
                .enumerate()
                .fold(0u64, |a, (i, x)| a | byte(*x) << (8 * i))
        };
        let word = |w: &str, set: u64| ([vec![0, 3], id(w)].concat(), 1 | set << 16);
        let mut kv = vec![
            word("девочка", 1),
            word("бежа", 2),
            word("бежала", 3),
            (vec![0, 9, 0, 1, 0], parse("NOUN,anim,femn,sing,nomn")),
            (vec![0, 9, 0, 2, 0], parse("GRND")),
            (vec![0, 9, 0, 3, 0], parse("VERB,femn,sing,past")),
            (vec![0, 12], pack(&[-1.1, 0.5, -2.7, -0.8, -3.2, 1.0, -4.9])),
            (
                vec![0, 18],
                pack(&[1.2, -0.7, -2.2, 0.5, -0.9, -1.1, -0.4, -0.7]),
            ),
            (
                vec![0, 21],
                pack(&[0.45, -1.5, 0.0, -0.2, 0.0, 0.0, -1.1, -1.5]),
            ),
        ];
        kv.sort();
        let bigrams = Map::from_iter(kv).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.start_input("", 1);
        type_str(&mut k, "девочка бежа ");
        assert_eq!(k.before.to_lowercase(), "девочка бежала ");
    }

    #[test]
    fn a_word_goes_in_with_its_yo() {
        // «еще» is «ещё» without its ё: the ё goes in; «все» and «всё» are
        // two words, left as typed. ⌫ right after takes it out.
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let words = [
            ("еще", 5000u64),
            ("ещё", 5500),
            ("все", 4000),
            ("всё", 4500),
            ("раз", 4000),
        ];
        let dict = Map::from_iter(
            words
                .iter()
                .map(|(w, v)| (id(w), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        // «еще» is only «ещё» spelled without it (the ё third: bit 2); the
        // text learned from writes «еще раз» without it.
        let mut kv = vec![
            ([vec![0, 24], id("еще")].concat(), 1u64 << 2),
            ([id("еще"), vec![0], id("раз")].concat(), 100),
        ];
        kv.sort();
        let bigrams = Map::from_iter(kv).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.start_input("", 1);
        type_str(&mut k, "еще ");
        assert_eq!(k.before.to_lowercase(), "ещё ");
        type_str(&mut k, "все ");
        assert_eq!(k.before.to_lowercase(), "ещё все ");
        // The next word's pair doesn't take it out again.
        k.start_input("", 1);
        type_str(&mut k, "еще раз ");
        assert_eq!(k.before.to_lowercase(), "ещё раз ");
        k.start_input("", 1);
        type_str(&mut k, "еще ⌫");
        assert_eq!(k.word.to_lowercase(), "еще");
    }

    #[test]
    fn a_real_word_the_grammar_rules_out_gives_way_to_its_form() {
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let words = [
            ("девочка", 5000u64),
            ("мальчик", 5000),
            ("быстро", 5000),
            ("побежал", 5000),
            ("побежала", 5000),
        ];
        let dict = Map::from_iter(
            words
                .iter()
                .map(|(w, v)| (id(w), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        let parse = kbcore::gram::parse;
        let byte = |x: f32| ((x / 0.05).round() + 128.0) as u64;
        let word = |w: &str, set: u64| ([vec![0, 3], id(w)].concat(), 1 | set << 16);
        let mut kv = vec![
            word("девочка", 1),
            word("мальчик", 2),
            word("быстро", 3),
            word("побежал", 4),
            word("побежала", 5),
            (vec![0, 9, 0, 1, 0], parse("NOUN,anim,femn,sing,nomn")),
            (vec![0, 9, 0, 2, 0], parse("NOUN,anim,masc,sing,nomn")),
            (vec![0, 9, 0, 3, 0], parse("ADJS,neut,sing")),
            (vec![0, 9, 0, 3, 1], parse("ADVB")),
            (vec![0, 9, 0, 4, 0], parse("ADJS,masc,sing")),
            (vec![0, 9, 0, 4, 1], parse("VERB,masc,sing,past")),
            (vec![0, 9, 0, 5, 0], parse("VERB,femn,sing,past")),
            // After a subject: a predicate that agrees +1.0, not -4.7.
            (
                vec![0, 12],
                byte(-1.1)
                    | byte(0.5) << 8
                    | byte(-2.7) << 16
                    | byte(-0.8) << 24
                    | byte(-3.2) << 32
                    | byte(1.0) << 40
                    | byte(-4.7) << 48,
            ),
        ];
        kv.sort();
        let bigrams = Map::from_iter(kv).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.start_input("", 1);
        type_str(&mut k, "девочка быстро побежал ");
        assert_eq!(k.before, "девочка быстро побежала ");
        // The form that agrees stays.
        k.start_input("", 1);
        type_str(&mut k, "мальчик быстро побежал ");
        assert_eq!(k.before, "мальчик быстро побежал ");
    }

    #[test]
    fn a_tap_on_a_border_goes_with_the_text() {
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let words = [("рыжий", 5000u64), ("кот", 9000), ("ком", 4000)];
        let dict = Map::from_iter(
            words
                .iter()
                .map(|(w, v)| (id(w), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        let kv = vec![([id("рыжий"), vec![0], id("кот")].concat(), 100u64)];
        let bigrams = Map::from_iter(kv).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        // A tap just on «е»'s side of its border with «к».
        let (kx, ky) = k.key_center(Action::Char('к')).unwrap();
        let (ex, _) = k.key_center(Action::Char('е')).unwrap();
        let x = (kx + ex) / 2.0 + (ex - kx) * 0.02;
        let tap = |k: &mut Ime<Vec<u8>>| {
            let t = now();
            k.touch(DOWN, 0, x, ky, t);
            k.touch(UP, 0, x, ky, t + 60);
        };
        // No word begins with «е»: «к».
        k.start_input("", 1);
        tap(&mut k);
        assert_eq!(k.word.to_lowercase(), "к");
        // A letter some word goes on with stays as tapped: «к|о» on the «о»
        // side of its border with «л» — «ко…» goes on.
        k.start_input("", 1);
        type_str(&mut k, "рыжий к");
        let (ox, oy) = k.key_center(Action::Char('о')).unwrap();
        let (lx, _) = k.key_center(Action::Char('л')).unwrap();
        let x = (ox + lx) / 2.0 - (lx - ox) * 0.02;
        let t = now();
        k.touch(DOWN, 0, x, oy, t);
        k.touch(UP, 0, x, oy, t + 60);
        assert_eq!(k.word, "ко");
    }

    #[test]
    fn names_take_their_capitals() {
        let mut k = ime(&[("москва", 3000), ("сша", 2000), ("вера", 2000), ("в", 9000)]);
        let casing = Map::from_iter(
            [("москва", 1u64), ("сша", 2)]
                .iter()
                .map(|(w, v)| {
                    (
                        w.chars()
                            .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                            .collect::<Vec<u8>>(),
                        *v,
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        let empty = Map::from_iter(Vec::<(Vec<u8>, u64)>::new()).unwrap();
        let engine = std::mem::replace(&mut k.engine, Engine::new(empty, Config::default()));
        k.engine = engine.with_casing(casing);
        k.start_input("", 1);
        type_str(&mut k, "в москва ");
        assert!(k.take_ops().contains(&Op::Commit("Москва".into())));
        // ⌫ right after: as typed.
        type_str(&mut k, "⌫ ");
        assert!(k.before.ends_with("в москва "), "{:?}", k.before);
        type_str(&mut k, "сша вера ");
        let ops = k.take_ops();
        for w in ["США", "вера"] {
            assert!(ops.contains(&Op::Commit(w.into())), "{w}: {ops:?}");
        }
    }

    #[test]
    fn the_strip_marks_what_space_will_put() {
        let mut k = ime(WORDS);
        type_str(&mut k, "привкт");
        assert!(k.center_applies(), "{:?}", k.slots);
        type_str(&mut k, "⌫");
        assert!(!k.center_applies(), "editing: space keeps what is typed");
    }

    #[test]
    fn a_comma_meant_as_a_letter_is_joined() {
        let k = ime(WORDS);
        let comma = k
            .keys
            .iter()
            .find(|q| q.action == Action::Char(','))
            .unwrap();
        let (cx, cy) = comma.center();
        let top = comma.y + 3.0;
        let above = k
            .keys
            .iter()
            .filter_map(|q| match q.action {
                Action::Char(c) if c.is_alphabetic() && q.y < comma.y => {
                    Some((c, (q.center().0 - cx).abs() + (q.center().1 - cy).abs()))
                }
                _ => None,
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap()
            .0;
        let word = format!("ко{above}ка");
        let mut k = ime(&[(word.as_str(), 3000)]);
        k.start_input("", 1);
        type_str(&mut k, "ко");
        let t = now();
        k.touch(DOWN, 0, cx, top, t);
        k.touch(UP, 0, cx, top, t + 60);
        type_str(&mut k, "ка");
        assert_eq!(k.merge.as_ref().map(|m| m.word.clone()), Some(word));
        // Short forms stay: «т.п».
        let mut k = ime(&[("тип", 3000)]);
        k.start_input("", 1);
        type_str(&mut k, "т");
        let t = now();
        k.touch(DOWN, 0, cx, top, t);
        k.touch(UP, 0, cx, top, t + 60);
        assert!(k.prev_word.is_none());
    }

    /// Tap `c` on its border with `toward` (an ambiguous tap).
    fn tap_between(k: &mut Ime<Vec<u8>>, c: char, toward: char) {
        let (x0, y0) = k.key_center(Action::Char(c)).unwrap();
        let (x1, y1) = k.key_center(Action::Char(toward)).unwrap();
        let (x, y) = (x0 + (x1 - x0) * 0.45, y0 + (y1 - y0) * 0.45);
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(UP, 0, x, y, t + 80);
    }

    #[test]
    fn the_next_word_can_change_the_previous_one() {
        let words = [("а", 2000), ("в", 2000), ("принципе", 8000), ("б", 9000)];
        let mut k = ime_ctx(&words, &[("в", "принципе", 300)]);
        k.start_input("", 1);
        tap_between(&mut k, 'а', 'в');
        type_str(&mut k, " принципе");
        assert_eq!(
            k.merge.as_ref().map(|m| m.word.as_str()),
            Some("в принципе")
        );
        assert!(k.center_applies());
        type_str(&mut k, " ");
        assert_eq!(k.before, "в принципе ");
        // ⌫ right after: both words back as they were typed.
        type_str(&mut k, "⌫");
        assert_eq!(k.before, "а ");
        assert_eq!(k.word, "принципе");
    }

    #[test]
    fn a_swipe_up_goes_round_a_changed_pair() {
        let words = [("а", 2000), ("в", 2000), ("принципе", 8000), ("б", 9000)];
        let mut k = ime_ctx(&words, &[("в", "принципе", 300)]);
        k.start_input("", 1);
        tap_between(&mut k, 'а', 'в');
        type_str(&mut k, " принципе ");
        assert_eq!(k.before, "в принципе ");
        // ⌫ puts back what was typed; a swipe up then brings the pair back —
        // both words, not just the one being typed.
        type_str(&mut k, "⌫");
        assert_eq!((k.before.as_str(), k.word.as_str()), ("а ", "принципе"));
        k.next_variant();
        assert_eq!(k.before, "в принципе ");
        assert!(k.word.is_empty());
        // Round the pair's readings, through what was typed, and back.
        let mut seen = Vec::new();
        for _ in 0..8 {
            k.next_variant();
            seen.push(k.before.clone());
            if k.before == "в принципе " {
                break;
            }
        }
        assert!(seen.contains(&"а принципе ".to_string()), "{seen:?}");
        assert_eq!(k.before, "в принципе ", "{seen:?}");
    }

    #[test]
    fn a_swipe_down_goes_back_round() {
        let mut k = ime(&[
            ("за", 9000),
            ("донатили", 9000),
            ("донатила", 3000),
            ("как", 3000),
        ]);
        k.start_input("", 1);
        type_str(&mut k, "задонатили ");
        assert_eq!(k.before, "за донатили ");
        k.prev_variant();
        assert_eq!(k.before, "задонатили ", "back: what was typed");
        k.prev_variant();
        assert_eq!(k.before, "за донатила ");
        k.next_variant();
        assert_eq!(k.before, "задонатили ");
        // The gesture itself: a swipe down on the space bar.
        let (x, y) = k.key_center(Action::Space).unwrap();
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(MOVE, 0, x, y + 60.0 * k.dp, t + 40);
        k.touch(UP, 0, x, y + 60.0 * k.dp, t + 80);
        assert_eq!(k.before, "за донатила ");
    }

    #[test]
    fn a_swipe_up_goes_round_a_split() {
        let mut k = ime(&[
            ("за", 9000),
            ("донатили", 9000),
            ("донатила", 3000),
            ("как", 3000),
        ]);
        k.start_input("", 1);
        type_str(&mut k, "задонатили ");
        assert_eq!(k.before, "за донатили ");
        k.next_variant();
        assert_eq!(k.before, "за донатила ");
        k.next_variant();
        assert_eq!(k.before, "задонатили ", "what was typed");
        k.next_variant();
        assert_eq!(k.before, "за донатили ");
    }

    #[test]
    fn a_clearly_typed_word_is_not_read_again() {
        let words = [("а", 2000), ("в", 2000), ("принципе", 8000), ("б", 9000)];
        let mut k = ime_ctx(&words, &[("в", "принципе", 3000)]);
        k.start_input("", 1);
        type_str(&mut k, "а принципе");
        assert!(k.merge.is_none(), "{:?}", k.merge);
        // …and never a word the user edited.
        let mut k = ime_ctx(&words, &[("в", "принципе", 300)]);
        k.start_input("", 1);
        tap_between(&mut k, 'а', 'в');
        type_str(&mut k, "б⌫ принципе");
        assert!(k.merge.is_none(), "{:?}", k.merge);
    }

    /// Stamp "now" for the calibration button.
    fn stamp_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn calibration_runs_in_the_own_field_and_saves_the_grips() {
        let mut k = ime(WORDS);
        let ask = |k: &mut Ime<Vec<u8>>, g: Grip| {
            let mut calibrate = k.settings.calibrate;
            calibrate[g.index()] = stamp_now();
            k.set_settings(Settings {
                calibrate,
                ..k.settings.clone()
            });
        };
        let tap_strip_end = |k: &mut Ime<Vec<u8>>| {
            let t = now();
            k.touch(DOWN, 0, 1000.0, 10.0, t);
            k.touch(UP, 0, 1000.0, 10.0, t + 60);
        };
        let type_phrase = |k: &mut Ime<Vec<u8>>| {
            for c in PHRASE.chars() {
                let action = if c == ' ' {
                    Action::Space
                } else {
                    Action::Char(c)
                };
                press(k, action, 60);
            }
        };
        k.set_own_field(false);
        ask(&mut k, Grip::Left);
        k.start_input("", 1);
        assert!(k.calib.is_none(), "not in another app's field");
        k.set_own_field(true);
        k.start_input("", 1);
        assert_eq!(
            k.calib.as_ref().and_then(Calibration::grip),
            Some(Grip::Left)
        );
        k.take_ops();
        // The left thumb's area: strokes over the lower left, then "done".
        for i in 0..4 {
            let y = k.height() - 60.0 - 90.0 * i as f32;
            k.tilt(-1.0, 8.0, 5.0);
            k.touch(DOWN, 0, 20.0, y, now());
            for j in 1..12 {
                k.touch(MOVE, 0, 20.0 + 60.0 * j as f32, y, now());
            }
            k.touch(UP, 0, 680.0, y, now());
        }
        tap_strip_end(&mut k);
        type_phrase(&mut k);
        assert!(k.calib.is_none());
        assert!(k.grips.has(Grip::Left) && !k.grips.calibrated(), "one grip");
        assert!(k.grips.rows(Grip::Left).is_some(), "its rows");
        // Two hands, on their own.
        ask(&mut k, Grip::Both);
        k.start_input("", 1);
        k.tilt(0.0, 6.0, 8.0);
        type_phrase(&mut k);
        assert!(k.calib.is_none() && k.grips.calibrated());
        assert!(k.take_grips().is_some(), "saved");
        assert!(k.take_grips().is_none());
        let typed = k.take_ops();
        assert!(
            !typed
                .iter()
                .any(|o| matches!(o, Op::Commit(_) | Op::Composing(_))),
            "{typed:?}"
        );
        // Undo the left one: gone (nothing before it); the rest stays.
        let mut grip_undo = k.settings.grip_undo;
        grip_undo[Grip::Left.index()] = stamp_now();
        k.set_settings(Settings {
            grip_undo,
            ..k.settings.clone()
        });
        assert!(!k.grips.has(Grip::Left) && k.grips.has(Grip::Both));
        // A reset forgets all.
        k.set_settings(Settings {
            grip_reset: stamp_now() + 1,
            ..k.settings.clone()
        });
        assert!(!k.grips.has(Grip::Both));
    }

    #[test]
    fn enter_brushed_while_reaching_is_not_typed() {
        let mut k = ime(WORDS);
        let (fx, fy) = k.key_center(Action::Char('ф')).unwrap();
        let (ex, ey) = k.key_center(Action::Enter).unwrap();
        // The palm on Enter, then the thumb on a far key.
        let t = now();
        k.touch(DOWN, 1, ex, ey, t);
        k.touch(DOWN, 0, fx, fy, t + 40);
        k.touch(UP, 1, ex, ey, t + 60);
        k.touch(UP, 0, fx, fy, t + 90);
        // The thumb on a far key, the palm on Enter while it is down.
        let t = now();
        k.touch(DOWN, 0, fx, fy, t);
        k.touch(DOWN, 1, ex, ey, t + 30);
        k.touch(UP, 1, ex, ey, t + 50);
        k.touch(UP, 0, fx, fy, t + 80);
        // …or right after it.
        let t = now();
        k.touch(DOWN, 0, fx, fy, t);
        k.touch(UP, 0, fx, fy, t + 40);
        k.touch(DOWN, 1, ex, ey, t + 70);
        k.touch(UP, 1, ex, ey, t + 90);
        let ops = k.take_ops();
        assert!(!ops.contains(&Op::Enter), "{ops:?}");
        assert_eq!(k.word, "Ффф");
        // A real Enter, after a pause.
        let t = now() + 500;
        k.touch(DOWN, 1, ex, ey, t);
        k.touch(UP, 1, ex, ey, t + 80);
        assert!(k.take_ops().contains(&Op::Enter));
    }

    #[test]
    fn moving_the_cursor_forgets_the_undo() {
        let mut k = ime(WORDS);
        type_str(&mut k, "как привкт ");
        assert!(k.undo.is_some());
        // The user taps into «как»: the text before the cursor is «Ка».
        k.selection(2, 2, -1, -1, "Ка", "", None);
        k.take_ops();
        type_str(&mut k, "⌫");
        let ops = k.take_ops();
        assert_eq!(
            ops.first(),
            Some(&Op::Delete(1)),
            "a plain ⌫ there: {ops:?}"
        );
        assert!(!ops
            .iter()
            .any(|o| matches!(o, Op::Composing(s) if s.contains("кт"))));
    }

    /// Touch the strip's slot `i`, hold for `ms` (the timer fires if long).
    fn hold_slot(k: &mut Ime<Vec<u8>>, i: usize, ms: i64) {
        let (x, y) = ((i as f32 + 0.5) * k.width / 3.0, k.strip_h() / 2.0);
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        if ms >= LONG_PRESS_MS as i64 {
            k.timer();
        }
        k.touch(UP, 0, x, y, t + ms);
    }

    #[test]
    fn holding_the_typed_word_adds_it_to_the_dictionary() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            strip: Strip::Two,
            ..k.settings.clone()
        });
        type_str(&mut k, "привед");
        hold_slot(&mut k, 0, 500);
        assert_eq!(k.take_user_words().as_deref(), Some("Привед\n"));
        type_str(&mut k, " ");
        assert!(k.take_ops().contains(&Op::Commit("Привед".into())));
        // …and a slip on it is fixed toward it.
        type_str(&mut k, "привкд ");
        assert!(k.before.ends_with("Привед "), "{:?}", k.before);
    }

    #[test]
    fn dragging_the_typed_word_up_adds_it() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            strip: Strip::Two,
            ..k.settings.clone()
        });
        type_str(&mut k, "привед");
        let (x, y) = (k.width / 6.0, k.strip_h() / 2.0);
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(MOVE, 0, x, y - 40.0 * k.dp, t + 60);
        k.touch(UP, 0, x, y - 60.0 * k.dp, t + 90);
        assert!(k.is_user_word("привед"));
    }

    #[test]
    fn a_selected_word_can_go_in_and_out_of_the_dictionary() {
        let mut k = ime(WORDS);
        k.selection(0, 5, -1, -1, "", "", Some("кринж"));
        assert_eq!(k.slots[1].as_deref(), Some("＋ в словарь: кринж"));
        k.pick_slot(1);
        assert!(k.is_user_word("кринж"));
        k.selection(0, 5, -1, -1, "", "", Some("кринж"));
        assert_eq!(k.slots[1].as_deref(), Some("− из словаря: кринж"));
        k.pick_slot(1);
        assert!(!k.is_user_word("кринж"));
        // Not a word, or a word the dictionary has: nothing offered.
        k.selection(0, 9, -1, -1, "", "", Some("два слова"));
        assert!(k.dict_actions.iter().all(Option::is_none));
        k.selection(0, 6, -1, -1, "", "", Some("Привет"));
        assert!(k.dict_actions.iter().all(Option::is_none));
    }

    #[test]
    fn a_capitalized_word_goes_in_as_a_name_or_as_a_word() {
        let mut k = ime(WORDS);
        // Mid-sentence: a name first.
        k.selection(4, 9, -1, -1, "как ", "", Some("Кринж"));
        assert_eq!(k.slots[1].as_deref(), Some("＋ Кринж (имя)"));
        assert_eq!(k.slots[0].as_deref(), Some("＋ кринж"));
        k.pick_slot(0);
        assert!(k.user_words == ["кринж"], "{:?}", k.user_words);
        k.remove_user_word("кринж");
        // At a sentence start: the ordinary word first.
        k.selection(0, 5, -1, -1, "", "", Some("Кринж"));
        assert_eq!(k.slots[1].as_deref(), Some("＋ кринж"));
        k.pick_slot(0);
        assert!(k.user_words == ["Кринж"], "{:?}", k.user_words);
        assert_eq!(k.cased("", "кринж"), "Кринж", "a name keeps its capital");
    }

    /// Swipe up on the space bar.
    fn space_up(k: &mut Ime<Vec<u8>>) {
        let (x, y) = k.key_center(Action::Space).unwrap();
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(MOVE, 0, x, y - 60.0 * k.dp, t + 60);
        k.touch(UP, 0, x, y - 60.0 * k.dp, t + 90);
    }

    #[test]
    fn space_up_offers_the_next_word() {
        let mut k = ime(&[("кот", 3000), ("кит", 3500), ("кто", 3200)]);
        k.start_input("", 1);
        type_str(&mut k, "кот");
        space_up(&mut k);
        let first = k.shown.clone();
        assert_ne!(first, "кот");
        let mut seen = vec![first.clone()];
        for _ in 0..5 {
            space_up(&mut k);
            seen.push(k.shown.clone());
        }
        assert!(
            seen.contains(&"кот".to_string()),
            "comes back round: {seen:?}"
        );
        type_str(&mut k, " ");
        assert!(k
            .before
            .ends_with(&format!("{} ", k.prev_word.as_ref().unwrap().committed)));
        // Right after the space: the previous word is swapped in place.
        let before = k.before.clone();
        space_up(&mut k);
        assert_ne!(k.before, before);
        assert!(k.before.ends_with(' '));
    }

    /// Draw `word` across the keys: straight lines between the key centers,
    /// a point every 10 px.
    fn swipe(k: &mut Ime<Vec<u8>>, word: &str) {
        let corners: Vec<(f32, f32)> = word
            .chars()
            .map(|c| k.key_center(Action::Char(c)).unwrap())
            .collect();
        let t = now();
        k.touch(DOWN, 0, corners[0].0, corners[0].1, t);
        let mut ms = t;
        for w in corners.windows(2) {
            let n = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / 10.0)
                .ceil()
                .max(1.0) as usize;
            for s in 1..=n {
                let f = s as f32 / n as f32;
                ms += 5;
                k.touch(
                    MOVE,
                    0,
                    w[0].0 + (w[1].0 - w[0].0) * f,
                    w[0].1 + (w[1].1 - w[0].1) * f,
                    ms,
                );
            }
        }
        let last = *corners.last().unwrap();
        k.touch(UP, 0, last.0, last.1, ms + 5);
    }

    #[test]
    fn a_word_drawn_across_the_keys() {
        let mut k = ime(&[
            ("привет", 3000),
            ("как", 2500),
            ("дела", 3000),
            ("приват", 9000),
        ]);
        swipe(&mut k, "привет");
        // In with its space; its other readings in the strip.
        assert_eq!(k.before, "Привет ", "sentence start: a capital");
        assert!(k.word.is_empty());
        assert!(
            k.slots.iter().flatten().any(|s| s == "Приват"),
            "{:?}",
            k.slots
        );
        // A tap on one puts it in the word's place.
        let at = k.slots.iter().position(|s| s.as_deref() == Some("Приват"));
        k.pick_slot(at.unwrap());
        assert_eq!(k.before, "Приват ");
        k.pick_slot(
            k.slots
                .iter()
                .position(|s| s.as_deref() == Some("Привет"))
                .unwrap(),
        );
        assert_eq!(k.before, "Привет ");
        // The next one goes after it; a space tapped then isn't doubled.
        swipe(&mut k, "как");
        assert_eq!(k.before, "Привет как ");
        type_str(&mut k, " ");
        assert_eq!(k.before, "Привет как ");
        // Letters typed after a drawn word begin the next one.
        type_str(&mut k, "дела ");
        assert_eq!(k.before, "Привет как дела ");
    }

    #[test]
    fn a_drawn_word_keeps_its_way() {
        let mut k = ime(&[("привет", 3000), ("как", 2500), ("приват", 9000)]);
        swipe(&mut k, "привет");
        // In the text it is read again from the way it was drawn, not as if
        // its letters were typed.
        let w = k.written.last().unwrap();
        assert_eq!(w.text, "привет");
        assert!(matches!(w.input, Input::Drawn { .. }));
        let prev = k.prev_word.as_ref().expect("the window keeps it");
        assert!(matches!(prev.input, Input::Drawn { .. }));
        assert!(prev.letters.is_empty(), "its space was no tap");
        assert!(
            prev.alts.iter().any(|c| c.word == "приват"),
            "{:?}",
            prev.alts
        );
    }

    #[test]
    fn a_word_goes_on_where_the_cursor_is_put_back() {
        let mut k = ime(&[("привет", 3000), ("как", 2500)]);
        type_str(&mut k, "прив");
        // The cursor away and back to the end of «Прив»: the letters typed
        // then go on with it — the whole word is read, not «ет».
        k.selection(0, 0, -1, -1, "", "Прив", None);
        k.selection(4, 4, -1, -1, "Прив", "", None);
        type_str(&mut k, "ет");
        assert_eq!(k.word, "Привет");
        type_str(&mut k, " ");
        assert_eq!(k.before, "Привет ");
        // Not in the middle of a word: there a letter is only inserted.
        k.start_input("", 1);
        k.selection(2, 2, -1, -1, "Пр", "вет", None);
        type_str(&mut k, "и");
        assert_eq!(k.word, "и");
    }

    #[test]
    fn a_word_erased_whole_brings_the_capital_back() {
        let mut k = ime(&[("привет", 3000), ("как", 2500)]);
        // A drawn word at the start of the field, erased: the next one starts
        // with a capital at once, not after another ⌫.
        swipe(&mut k, "привет");
        assert_eq!(k.shift, Shift::Off);
        type_str(&mut k, "⌫");
        assert!(k.word.is_empty());
        assert_eq!(k.shift, Shift::Once);
        // The same for typed letters erased one by one.
        type_str(&mut k, "как⌫⌫⌫");
        assert!(k.word.is_empty());
        assert_eq!(k.shift, Shift::Once);
    }

    #[test]
    fn backspace_takes_a_drawn_word_whole() {
        let mut k = ime(&[("привет", 3000), ("как", 2500)]);
        type_str(&mut k, "как ");
        swipe(&mut k, "привет");
        assert_eq!(k.before, "Как привет ");
        k.take_ops();
        // ⌫ right after: the word and its space.
        type_str(&mut k, "⌫");
        assert!(k.word.is_empty());
        assert_eq!(k.take_ops(), vec![Op::Delete(7)]);
        assert_eq!(k.before, "Как ");
        // Not once something else was typed: then ⌫ is a letter's.
        swipe(&mut k, "привет");
        type_str(&mut k, "ы⌫");
        assert_eq!(k.before, "Как привет ");
    }

    #[test]
    fn a_drawn_word_shows_in_the_strip_and_punctuation_brings_a_space() {
        let mut k = ime(&[("привет", 3000), ("приват", 9000), ("как", 2500)]);
        swipe(&mut k, "привет");
        assert_eq!(k.slots[1].as_deref(), Some("Привет"), "{:?}", k.slots);
        type_str(&mut k, ",");
        assert_eq!(k.before, "Привет, ");
        type_str(&mut k, " "); // no double space
        assert_eq!(k.before, "Привет, ");
        swipe(&mut k, "как");
        type_str(&mut k, " ");
        assert_eq!(k.before, "Привет, как ");
    }

    #[test]
    fn a_slide_to_a_neighbor_is_not_a_gesture() {
        let mut k = ime(WORDS);
        let (ax, ay) = k.key_center(Action::Char('а')).unwrap();
        let (px, py) = k.key_center(Action::Char('п')).unwrap();
        let t = now();
        k.touch(DOWN, 0, ax, ay, t);
        k.touch(MOVE, 0, (ax + px) / 2.0, (ay + py) / 2.0, t + 30);
        k.touch(MOVE, 0, px, py, t + 60);
        k.touch(UP, 0, px, py, t + 90);
        assert_eq!(k.word, "П");
    }

    #[test]
    fn holding_a_key_with_several_alternatives_offers_a_choice() {
        let mut k = ime(WORDS);
        let (x, y) = k.key_center(Action::Char('е')).unwrap();
        // Hold and lift: the first (ё).
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.timer();
        assert_eq!(
            k.popup.as_ref().map(|p| p.options.clone()),
            Some(vec!['ё', '5'])
        );
        k.touch(UP, 0, x, y, t + 500);
        assert_eq!(k.word, "Ё");
        // Hold, slide right, lift: the second (5).
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.timer();
        let (x0, _, w, _) = k.popup_rect();
        k.touch(MOVE, 0, x0 + 1.5 * w, y, t + 400);
        k.touch(UP, 0, x0 + 1.5 * w, y, t + 500);
        assert!(k.popup.is_none());
        assert!(
            k.before.ends_with('5') || k.word.ends_with('5'),
            "{:?} {:?}",
            k.before,
            k.word
        );
    }

    #[test]
    fn backspace_undoes_a_split() {
        let mut k = ime(&[("за", 9000), ("донатили", 9000), ("как", 3000)]);
        k.start_input("", 1);
        type_str(&mut k, "задонатили ");
        assert_eq!(k.before, "за донатили ", "read as two words");
        type_str(&mut k, "⌫");
        assert_eq!(k.before, "");
        assert_eq!(k.word, "задонатили");
        // Typed again: kept whole.
        type_str(&mut k, "⌫⌫⌫⌫⌫⌫⌫⌫⌫⌫задонатили ");
        assert_eq!(k.before, "задонатили ");
    }

    #[test]
    fn a_closing_bracket_holds_on_to_the_word() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "привет");
        k.type_char(')');
        assert_eq!(k.before, "привет)");
        // Typed after the space: the space moves after it (smileys too).
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "привет ");
        k.type_char(')');
        k.type_char(')');
        assert_eq!(k.before, "привет)) ");
        // An opening bracket keeps the digits' page: (2 + 3).
        let mut k = ime(WORDS);
        k.start_input("", 1);
        k.layer = Layer::Symbols;
        k.relayout();
        let (x, y) = k.key_center(Action::Char('(')).unwrap();
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(UP, 0, x, y, t + 60);
        assert_eq!(k.layer, Layer::Symbols);
    }

    #[test]
    fn mid_text_a_space_after_the_cursor_is_not_doubled() {
        let mut k = ime(WORDS);
        k.start_input("Привет", 1);
        k.set_after(" мир");
        type_str(&mut k, " дела ");
        let ops = k.take_ops();
        assert_eq!(ops.first(), Some(&Op::Skip), "{ops:?}");
        assert_eq!(k.before, "Привет дела ");
        assert!(
            !ops[1..].contains(&Op::Skip),
            "only one space was there: {ops:?}"
        );
    }

    #[test]
    fn holding_space_offers_keyboards_and_settings() {
        let mut k = ime(WORDS);
        let (x, y) = k.key_center(Action::Space).unwrap();
        let t = now();
        assert_eq!(k.touch(DOWN, 0, x, y, t) >> 16, SPACE_MENU_MS);
        k.timer();
        assert_eq!(
            k.popup.as_ref().map(|p| p.options.clone()),
            Some(vec![MENU_KEYBOARDS, MENU_SETTINGS])
        );
        let (x0, _, w, _) = k.popup_rect();
        k.touch(MOVE, 0, x0 + 1.5 * w, y, t + 800);
        k.touch(UP, 0, x0 + 1.5 * w, y, t + 900);
        assert_eq!(k.take_ops(), vec![Op::Settings]);
        // A short press is still a space.
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(UP, 0, x, y, t + 80);
        assert!(k.popup.is_none());
    }

    #[test]
    fn languages_go_round_in_the_settings_order_with_their_packs() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            languages: vec![Lang::Ru, Lang::De, Lang::En],
            ..k.settings.clone()
        });
        assert_eq!(k.wanted_packs(), vec!["de"]);
        let de = test_engine(
            &[("schön", 900), ("für", 3000), ("hallo", 2000)],
            Latin::Qwertz,
        );
        k.add_pack("de", Some(de));
        assert!(k.wanted_packs().is_empty());
        k.start_input("", 1);
        k.switch_lang(1);
        assert_eq!((k.lang, k.engine_pack), (Lang::De, "de"));
        assert!(k.key_center(Action::Char('ü')).is_some(), "QWERTZ");
        // A left-out umlaut comes back.
        type_str(&mut k, "fur ");
        assert_eq!(k.before, "für ");
        k.switch_lang(1);
        assert_eq!((k.lang, k.engine_pack), (Lang::En, "base"));
        k.switch_lang(1);
        assert_eq!(k.lang, Lang::Ru);
        k.switch_lang(-1);
        assert_eq!(k.lang, Lang::En);
        // Taken out of the list: on to the first.
        k.set_settings(Settings {
            languages: vec![Lang::De],
            ..k.settings.clone()
        });
        assert_eq!((k.lang, k.engine_pack), (Lang::De, "de"));
        // A pack the app lacks isn't asked for again.
        k.set_settings(Settings {
            languages: vec![Lang::De, Lang::Fr],
            ..k.settings.clone()
        });
        assert_eq!(k.wanted_packs(), vec!["fr"]);
        k.add_pack("fr", None);
        assert!(k.wanted_packs().is_empty());
    }

    #[test]
    fn private_windows_keep_suggestions_passwords_do_not() {
        const NO_SUGGESTIONS: i32 = 0x80000;
        let mut k = ime(WORDS);
        k.start_input("", 1 | NO_SUGGESTIONS);
        assert!(!k.suggest, "an ordinary field that asks for none");
        k.set_private(true);
        k.start_input("", 1 | NO_SUGGESTIONS);
        assert!(k.suggest, "a private window");
        k.start_input("", 1 | 0x90);
        assert!(k.suggest, "a private window's fake visible password");
        k.start_input("", 1 | 0x80);
        assert!(!k.suggest, "a password");
        k.start_input("", 1 | 0xE0);
        assert!(!k.suggest, "a web password");
    }

    #[test]
    fn opening_marks_keep_the_sentence_start() {
        assert!(sentence_start("¿"));
        assert!(sentence_start("Hola. ¡"));
        assert!(sentence_start("Привет. «"));
        assert!(!sentence_start("он сказал «"));
        assert!(!sentence_start("don'"));
    }

    #[test]
    fn french_elision_starts_a_new_word() {
        let mut k = ime(&[
            ("l'", 9000),
            ("homme", 3000),
            ("aujourd'hui", 2000),
            ("hommes", 900),
        ]);
        k.set_settings(Settings {
            languages: vec![Lang::Fr],
            ..k.settings.clone()
        });
        k.start_input("", 1);
        assert_eq!(k.lang, Lang::Fr);
        k.type_char('l');
        k.type_char('\'');
        assert_eq!((k.before.as_str(), k.word.as_str()), ("l'", ""));
        assert_eq!(k.context_at(&k.before).prev(), Some("l'"));
        type_str(&mut k, "homme ");
        assert_eq!(k.before, "l'homme ");
        // Inside a word the apostrophe stays: aujourd'hui.
        for c in "aujourd'hui".chars() {
            k.type_char(c);
        }
        assert_eq!(k.word, "aujourd'hui");
    }

    #[test]
    fn the_field_or_the_word_at_the_cursor_picks_the_language() {
        let mut k = ime(&[("привет", 5000), ("hello", 4000), ("world", 3000)]);
        k.set_settings(Settings {
            languages: vec![Lang::Ru, Lang::En, Lang::Es],
            ..k.settings.clone()
        });
        // The app says the field is Spanish (a Duolingo lesson).
        k.set_hint_languages("es-ES,en");
        k.start_input("", 1);
        assert_eq!(k.lang, Lang::Es);
        // A language that isn't enabled is not forced.
        k.set_hint_languages("fr");
        k.start_input("", 1);
        assert_eq!(k.lang, Lang::Es);
        // No hint: the word at the cursor.
        k.set_hint_languages("");
        k.lang = Lang::Ru;
        k.start_input("Привет hello", 1);
        assert_eq!(k.lang, Lang::En);
        // The cursor moved onto the Russian word.
        k.selection(6, 6, -1, -1, "Привет", " hello", None);
        assert_eq!(k.lang, Lang::Ru);
        // Off in the settings: stays.
        k.set_settings(Settings {
            lang_from_text: false,
            ..k.settings.clone()
        });
        k.selection(12, 12, -1, -1, "Привет hello", "", None);
        assert_eq!(k.lang, Lang::Ru);
    }

    #[test]
    fn a_swipe_up_on_symbols_or_enter_squeezes_the_rows_for_one_thumb() {
        let mut k = ime(WORDS);
        // Not calibrated: it says so and stays.
        {
            let (x, y) = k.key_center(Action::Symbols).unwrap();
            let t = now();
            k.touch(DOWN, 0, x, y, t);
            k.touch(MOVE, 0, x, y - 60.0 * k.dp, t + 40);
            k.touch(UP, 0, x, y - 60.0 * k.dp, t + 80);
            assert_eq!(k.arc, None);
            assert!(k.notice.is_some());
        }
        // Calibrated: each row within the stretch the thumb reaches.
        let mut grips = crate::grip::tests_support::shifted_grips(0.0, 0.0);
        grips.set_rows(
            Grip::Left,
            vec![(0.0, 0.6), (0.0, 0.7), (0.0, 0.8), (0.0, 0.9)],
        );
        grips.set_rows(
            Grip::Right,
            vec![(0.3, 0.95), (0.2, 0.95), (0.1, 0.95), (0.0, 1.0)],
        );
        k.grips = grips;
        let swipe_up = |k: &mut Ime<Vec<u8>>, action: Action| {
            let (x, y) = k.key_center(action).unwrap();
            let t = now();
            k.touch(DOWN, 0, x, y, t);
            let moved = k.touch(MOVE, 0, x, y - 60.0 * k.dp, t + 40);
            moved | k.touch(UP, 0, x, y - 60.0 * k.dp, t + 80)
        };
        let flat = k.height();
        let flags = swipe_up(&mut k, Action::Symbols);
        assert_eq!(k.arc, Some(Hand::Left));
        assert!(flags & RELAYOUT != 0 && k.height() == flat);
        // The top row ends short of the right edge, within the thumb's reach.
        let x_end = k.key_center(Action::Char('х')).unwrap().0;
        assert!(x_end < 0.9 * 1080.0, "{x_end}");
        assert_eq!(
            k.layer,
            Layer::Letters,
            "the swipe doesn't open the symbols"
        );
        // It types like the usual layout.
        type_str(&mut k, "привет ");
        assert!(k.before.ends_with("Привет "), "{:?}", k.before);
        swipe_up(&mut k, Action::Enter);
        assert_eq!(k.arc, Some(Hand::Right));
        swipe_up(&mut k, Action::Enter);
        assert_eq!(k.arc, None);
        assert_eq!(k.height(), flat);
    }

    #[test]
    fn symbols_hit_on_the_edge_mid_word_is_the_letter_reached_for() {
        let mut k = ime(&[("прямо", 3000), ("привет", 5000)]);
        k.start_input("", 1);
        type_str(&mut k, "пр");
        let sym = k
            .keys
            .iter()
            .find(|b| b.action == Action::Symbols)
            .unwrap()
            .clone();
        // The top of ?123 while typing: я.
        let t = now();
        k.touch(DOWN, 0, sym.x + sym.w * 0.6, sym.y + sym.h * 0.1, t);
        k.touch(UP, 0, sym.x + sym.w * 0.6, sym.y + sym.h * 0.1, t + 60);
        assert_eq!(k.word, "пря");
        assert_eq!(k.layer, Layer::Letters);
        // Its middle: the symbols, as ever.
        let t = now();
        k.touch(DOWN, 0, sym.x + sym.w * 0.4, sym.y + sym.h * 0.7, t);
        k.touch(UP, 0, sym.x + sym.w * 0.4, sym.y + sym.h * 0.7, t + 60);
        assert_eq!(k.layer, Layer::Symbols);
    }

    #[test]
    fn symbols_hit_a_little_above_its_middle_mid_word_opens_the_symbols() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "пр");
        let sym = k
            .keys
            .iter()
            .find(|b| b.action == Action::Symbols)
            .unwrap()
            .clone();
        let t = now();
        k.touch(DOWN, 0, sym.x + sym.w * 0.5, sym.y + sym.h * 0.35, t);
        k.touch(UP, 0, sym.x + sym.w * 0.5, sym.y + sym.h * 0.35, t + 60);
        assert_eq!(k.word, "пр");
        assert_eq!(k.layer, Layer::Symbols);
    }

    #[test]
    fn symbols_hit_on_the_edge_after_a_word_that_cannot_go_on_is_the_symbols() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "привет");
        let sym = k
            .keys
            .iter()
            .find(|b| b.action == Action::Symbols)
            .unwrap()
            .clone();
        // «приветя…» is no word: «привет?» it is.
        let t = now();
        k.touch(DOWN, 0, sym.x + sym.w * 0.6, sym.y + sym.h * 0.1, t);
        k.touch(UP, 0, sym.x + sym.w * 0.6, sym.y + sym.h * 0.1, t + 60);
        assert_eq!(k.word, "привет");
        assert_eq!(k.layer, Layer::Symbols);
    }

    #[test]
    fn a_word_taken_up_again_at_a_sentence_start_goes_on_in_lowercase() {
        let mut k = ime(WORDS);
        k.start_input("", 0x4001);
        type_str(&mut k, "привет.⌫");
        assert_eq!(k.word, "Привет");
        type_str(&mut k, "ы");
        assert_eq!(k.word, "Приветы");
    }

    #[test]
    fn the_calibration_never_moves_a_tap_to_another_key() {
        let mut k = ime(WORDS);
        // A calibration saying every tap lands far to the right.
        let grips = crate::grip::tests_support::shifted_grips(0.15, 0.0);
        k.grips = grips;
        k.start_input("", 1);
        for _ in 0..30 {
            k.tilt(0.0, 7.0, 7.0);
        }
        let (x, y) = k.key_center(Action::Char('я')).unwrap();
        let t = now();
        k.touch(DOWN, 0, x, y, t);
        k.touch(UP, 0, x, y, t + 60);
        assert_eq!(k.word, "я", "the key under the finger");
        assert_eq!(k.shift, Shift::Off);
    }

    #[test]
    fn a_drawn_word_shows_while_drawing_and_its_way_gives_the_variants() {
        let mut k = ime(&[("как", 3000), ("вас", 2000), ("кас", 100), ("дела", 4000)]);
        k.start_input("", 1);
        // Draw к → а → с, slowly enough for the preview to read it.
        let pts: Vec<(f32, f32)> = ['к', 'а', 'с']
            .iter()
            .map(|&c| k.key_center(Action::Char(c)).unwrap())
            .collect();
        let mut trail = Vec::new();
        for w in pts.windows(2) {
            for i in 0..12 {
                let f = i as f32 / 12.0;
                trail.push((
                    w[0].0 + (w[1].0 - w[0].0) * f,
                    w[0].1 + (w[1].1 - w[0].1) * f,
                ));
            }
        }
        trail.push(pts[2]);
        let t = now();
        k.touch(DOWN, 0, trail[0].0, trail[0].1, t);
        for (i, &(x, y)) in trail.iter().enumerate().skip(1) {
            k.touch(MOVE, 0, x, y, t + 10 * i as i64);
        }
        assert!(k.slots[1].is_some(), "the word so far, while drawing");
        assert!(k.word.is_empty(), "nothing in the text yet");
        let (x, y) = pts[2];
        k.touch(UP, 0, x, y, t + 500);
        assert!(!k.before.trim().is_empty(), "in, with its space");
        // The swipe up goes round the other readings of the way.
        type_str(&mut k, " ");
        let first = k.before.clone();
        k.next_variant();
        assert_ne!(k.before, first);
        let second = k.before.trim().to_string();
        assert!(["как", "кас", "вас"].contains(&second.as_str()), "{second}");
    }

    #[test]
    fn the_swipe_up_goes_round_what_the_strip_offered() {
        let words = [
            ("привет", 5000),
            ("приват", 900),
            ("прикет", 300),
            ("примет", 800),
            ("пресет", 200),
            ("прилет", 400),
        ];
        let mut k = ime(words.as_slice());
        k.start_input("", 1);
        type_str(&mut k, "прикет");
        let offered: Vec<String> = k.last_cands.iter().map(|(w, _)| w.clone()).collect();
        assert!(offered.len() >= 4, "{offered:?}");
        type_str(&mut k, " ");
        // Round the readings: every one the strip had comes by.
        let mut seen = Vec::new();
        for _ in 0..10 {
            k.next_variant();
            seen.push(k.before.trim().to_lowercase());
        }
        for w in &offered {
            assert!(seen.contains(w), "{w} missing from {seen:?}");
        }
    }

    #[test]
    fn obscene_words_are_never_offered_unless_typed() {
        let words = [("бля", 5000), ("бла", 300), ("блин", 4000)];
        let casing = Map::from_iter([(
            "бля"
                .chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect::<Vec<u8>>(),
            4u64,
        )])
        .unwrap();
        let mut k = ime(&words);
        k.engine = test_engine(&words, Latin::Qwerty).with_casing(casing);
        k.start_input("", 1);
        type_str(&mut k, "бдя");
        assert!(
            !k.slots.iter().flatten().any(|s| s.to_lowercase() == "бля"),
            "{:?}",
            k.slots
        );
        type_str(&mut k, " ");
        assert_ne!(k.before.trim(), "бля");
        // Typed exactly: stays.
        k.start_input("", 1);
        type_str(&mut k, "бля ");
        assert_eq!(k.before, "бля ");
        // Off in the settings: offered like any word.
        k.set_settings(Settings {
            block_offensive: false,
            ..k.settings.clone()
        });
        k.start_input("", 1);
        type_str(&mut k, "бдя");
        assert!(
            k.slots.iter().flatten().any(|s| s.to_lowercase() == "бля"),
            "{:?}",
            k.slots
        );
    }

    #[test]
    fn a_held_one_row_key_types_its_letter_exactly() {
        let mut k = ime(&[("пвх", 1), ("под", 5000), ("как", 3000)]);
        k.set_settings(Settings {
            one_row: true,
            ..k.settings.clone()
        });
        k.start_input("", 1);
        // Hold a column, slide to a letter, lift: that letter, exactly.
        let pick = |k: &mut Ime<Vec<u8>>, c: char| {
            let col = layout::columns(Lang::Ru)
                .iter()
                .position(|(letters, _)| letters.contains(&c))
                .unwrap();
            let key = k
                .keys
                .iter()
                .find(|b| b.action == Action::Column(col as u8))
                .unwrap()
                .clone();
            let (x, y) = key.center();
            let t = now();
            k.touch(DOWN, 0, x, y, t);
            k.timer();
            let pop = k.popup.as_ref().expect("the column's letters");
            let i = pop.options.iter().position(|&o| o == c).unwrap();
            let (x0, _, w, _) = k.popup_rect();
            let px = x0 + (i as f32 + 0.5) * w;
            k.touch(MOVE, 0, px, y, t + 400);
            k.touch(UP, 0, px, y, t + 450);
        };
        for c in "пвх".chars() {
            pick(&mut k, c);
        }
        assert_eq!(k.word, "пвх");
        type_str(&mut k, " ");
        assert_eq!(k.before, "пвх ", "an abbreviation stays");
    }

    #[test]
    fn a_word_the_cursor_is_put_in_is_read_again_from_how_it_was_typed() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "привкт как ");
        assert_eq!(k.before, "привет как ");
        k.take_ops();
        // The cursor put in the middle of «привет»: its other readings —
        // what was typed among them — and a pick replaces the whole word.
        k.selection(3, 3, -1, -1, "при", "вет как ", None);
        let offered: Vec<String> = k.slots.iter().flatten().cloned().collect();
        assert!(offered.contains(&"привкт".to_string()), "{offered:?}");
        let i = k
            .slots
            .iter()
            .position(|s| s.as_deref() == Some("привкт"))
            .unwrap();
        k.pick_slot(i);
        assert_eq!(k.take_ops(), vec![Op::Replace(3, 3, "привкт".into())]);
        assert_eq!(k.before, "привкт");
        assert!(k.after.starts_with(" как"));
        // Typing again: the strip is the word's own again.
        type_str(&mut k, "д");
        assert!(k.recorrect.is_none());
    }

    #[test]
    fn holding_the_comma_opens_the_emoji_and_the_copies() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "привет");
        k.take_ops();
        // Hold the comma: the panel, its keys in the bottom row only.
        let (x, y) = k.key_center(Action::Char(',')).unwrap();
        k.touch(DOWN, 0, x, y, 1_000);
        k.timer();
        k.touch(UP, 0, x, y, 1_600);
        assert!(k.panel.is_some());
        assert!(k
            .keys
            .iter()
            .all(|key| !matches!(key.action, Action::Char(_))));
        // An emoji: the word finished with a space first.
        let tab = k.panel.unwrap().tab;
        assert_eq!(tab, Tab::Group(0));
        let frame = k.panel_frame();
        let (cx, cy, w, h) = frame.cell(0);
        k.touch(DOWN, 0, cx + w / 2.0, cy + h / 2.0, 2_000);
        k.touch(UP, 0, cx + w / 2.0, cy + h / 2.0, 2_050);
        assert_eq!(k.before, "привет 😀");
        // ⌫ takes the whole emoji, even a joined one.
        type_str(&mut k, "⌫");
        assert_eq!(k.before, "привет ");
        assert_eq!(last_cluster("а👨‍👩‍👧"), 5);
        assert_eq!(last_cluster("🇷🇺"), 2);
        assert_eq!(last_cluster("1️⃣"), 3);
        assert_eq!(last_cluster("да"), 1);
        // Back to the letters; the emoji is among the recent ones now.
        let (x, y) = k.key_center(Action::Letters).unwrap();
        k.touch(DOWN, 0, x, y, 3_000);
        k.touch(UP, 0, x, y, 3_050);
        assert!(k.panel.is_none());
        assert_eq!(k.stash.recent, vec!["😀"]);
    }

    #[test]
    fn a_copied_text_is_offered_and_kept_in_memory_only() {
        let mut k = ime(WORDS);
        k.start_input("Ссылка: ", 1);
        k.take_ops();
        assert_eq!(k.clip(Some("https://example.org"), false, 5_000), REDRAW);
        let offered = k.slots[0].clone().unwrap();
        assert!(offered.starts_with(CLIP_MARK), "{offered}");
        k.pick_slot(0);
        assert_eq!(k.take_ops(), vec![Op::Commit("https://example.org".into())]);
        assert!(k.slots[0].is_none());
        // A password isn't kept or offered.
        k.clip(Some("hunter2"), true, 6_000);
        assert_eq!(k.stash.clips, vec!["https://example.org"]);
        assert!(k.slots.iter().flatten().all(|s| !s.starts_with(CLIP_MARK)));
        // A later copy is offered until typing starts.
        k.clip(Some("второй"), false, 7_000);
        assert!(k.slots[0]
            .as_deref()
            .is_some_and(|s| s.starts_with(CLIP_MARK)));
        type_str(&mut k, "а");
        type_str(&mut k, "⌫");
        assert!(k.slots.iter().flatten().all(|s| !s.starts_with(CLIP_MARK)));
        assert_eq!(k.stash.clips, vec!["второй", "https://example.org"]);
    }

    #[test]
    fn known_words_are_not_autocorrected() {
        let mut k = ime(WORDS);
        type_str(&mut k, "при ");
        assert!(k.take_ops().contains(&Op::Commit("При".into())));
    }

    #[test]
    fn roll_over_keeps_order() {
        // Second finger lands before the first lifts.
        let mut k = ime(WORDS);
        let (px, py) = k.key_center(Action::Char('п')).unwrap();
        let (rx, ry) = k.key_center(Action::Char('р')).unwrap();
        k.touch(DOWN, 0, px, py, now());
        k.touch(DOWN, 1, rx, ry, now());
        k.touch(UP, 0, px, py, now());
        k.touch(UP, 1, rx, ry, now());
        assert_eq!(k.take_ops().last(), Some(&Op::Composing("Пр".into())));
    }

    #[test]
    fn long_press_types_alternative() {
        let mut k = ime(WORDS);
        let (x, y) = k.key_center(Action::Char('е')).unwrap();
        let flags = k.touch(DOWN, 0, x, y, now());
        assert_eq!(flags >> 16, LONG_PRESS_MS);
        k.timer();
        k.touch(UP, 0, x, y, now());
        assert_eq!(k.take_ops(), vec![Op::Composing("Ё".into())]);
    }

    #[test]
    fn suggestion_tap_commits_word_and_space() {
        let mut k = ime(WORDS);
        type_str(&mut k, "прив");
        k.take_ops();
        let slot = k
            .slots
            .iter()
            .position(|s| s.as_deref() == Some("Привет"))
            .expect("привет suggested");
        let x = (slot as f32 + 0.5) * k.width / 3.0;
        k.touch(DOWN, 0, x, 10.0, now());
        k.touch(UP, 0, x, 10.0, now());
        assert_eq!(
            k.take_ops(),
            vec![Op::Commit("Привет".into()), Op::Commit(" ".into())]
        );
    }

    #[test]
    fn short_correction_beats_long_completion() {
        // `ди` is a (rare) word; the frequent neighbor `ли` is the suggestion,
        // not the completion `директор` — but space keeps the valid word.
        let mut k = ime(&[("ли", 3000), ("ди", 7000), ("директор", 4000)]);
        type_str(&mut k, "ди");
        assert_ne!(k.slots[1].as_deref(), Some("Директор"), "{:?}", k.slots);
        assert!(k.slots.iter().flatten().any(|s| s == "Ли"), "{:?}", k.slots);
        type_str(&mut k, " ");
        assert!(k.take_ops().contains(&Op::Commit("Ди".into())));
    }

    #[test]
    fn backspacing_into_a_word_brings_its_suggestions_back() {
        let mut k = ime(WORDS);
        type_str(&mut k, "при к");
        k.take_ops();
        // ⌫ removes "к" (composing), ⌫ removes the space → "При" composes again.
        type_str(&mut k, "⌫⌫");
        let ops = k.take_ops();
        assert_eq!(ops.last(), Some(&Op::Composing("При".into())), "{ops:?}");
        assert!(
            k.slots.iter().flatten().any(|s| s == "Привет"),
            "{:?}",
            k.slots
        );
    }

    #[test]
    fn swipe_on_space_switches_language() {
        let mut k = ime(WORDS);
        assert!(k.key_center(Action::Char('й')).is_some());
        let (x, y) = k.key_center(Action::Space).unwrap();
        k.touch(DOWN, 0, x, y, now());
        k.touch(MOVE, 0, x + 60.0 * k.dp, y, now());
        k.touch(UP, 0, x + 60.0 * k.dp, y, now());
        assert!(k.key_center(Action::Char('q')).is_some());
        assert!(k.take_ops().is_empty(), "no space typed");
    }

    #[test]
    fn live_correction_shows_the_fix_once_the_word_is_certainly_wrong() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            live_correction: true,
            strip: Strip::Full,
            ..Settings::default()
        });
        type_str(&mut k, "при");
        assert_eq!(
            k.take_ops().last(),
            Some(&Op::Composing("При".into())),
            "a valid prefix stays as typed"
        );
        type_str(&mut k, "вкт");
        assert_eq!(k.take_ops().last(), Some(&Op::Composing("Привет".into())));
        assert_eq!(
            k.slots[0].as_deref(),
            Some("Привкт"),
            "the original waits in the strip"
        );
    }

    #[test]
    fn live_correction_keeps_the_typed_word_on_the_left() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            live_correction: true,
            strip: Strip::Full,
            ..Settings::default()
        });
        k.start_input("", 1);
        for typed in ["при", "привкт", "ыыыы"] {
            type_str(&mut k, typed);
            assert_eq!(
                k.suggestions()[0].as_deref(),
                Some(typed),
                "{:?}",
                k.suggestions()
            );
            type_str(&mut k, &"⌫".repeat(typed.chars().count()));
        }
    }

    #[test]
    fn capitalized_abbreviations_are_not_corrected() {
        let mut k = ime(WORDS);
        let (x, y) = k.key_center(Action::Shift).unwrap();
        k.touch(DOWN, 0, x, y, now());
        k.touch(UP, 0, x, y, now()); // auto-caps Once → Caps
        type_str(&mut k, "привкт ");
        assert!(k.take_ops().contains(&Op::Commit("ПРИВКТ".into())));
    }

    #[test]
    fn vowelless_abbreviations_are_not_corrected() {
        let mut k = ime(&[("то", 1000), ("ты", 1000), ("с", 1000)]);
        k.start_input("", 1); // no auto-caps
        type_str(&mut k, "тс ");
        assert!(k.take_ops().contains(&Op::Commit("тс".into())));
    }

    #[test]
    fn a_tap_on_the_border_of_a_vowel_is_a_slip_not_an_abbreviation() {
        // Real proportions: «то» is ~3600× more frequent than «тт».
        let words = [("то", 7700), ("тт", 15900)];
        let tap_at = |k: &mut Ime<Vec<u8>>, (x, y): (f32, f32)| {
            let t = now();
            k.touch(DOWN, 0, x, y, t);
            k.touch(UP, 0, x, y, t + 80);
        };
        // Both taps dead-center on «т»: a deliberate «тт» stays.
        let mut k = ime(&words);
        k.start_input("", 1);
        type_str(&mut k, "тт ");
        assert!(k.take_ops().contains(&Op::Commit("тт".into())));
        // Second tap still on «т» but right under «о»: that's «то».
        let mut k = ime(&words);
        k.start_input("", 1);
        let (tx, ty) = k.key_center(Action::Char('т')).unwrap();
        let (ox, oy) = k.key_center(Action::Char('о')).unwrap();
        tap_at(&mut k, (tx, ty));
        let near_o = (tx + (ox - tx) * 0.45, ty + (oy - ty) * 0.45);
        assert_eq!(k.action_at(near_o.0, near_o.1), Some(Action::Char('т')));
        tap_at(&mut k, near_o);
        type_str(&mut k, " ");
        assert!(
            k.take_ops().contains(&Op::Commit("то".into())),
            "{:?}",
            k.suggestions()
        );
    }

    #[test]
    fn one_slip_from_a_word_is_a_typo_even_without_vowels() {
        let mut k = ime(&[("что", 1500), ("тс", 8000)]);
        k.start_input("", 1);
        type_str(&mut k, "чтт ");
        assert!(k.take_ops().contains(&Op::Commit("что".into())));
        type_str(&mut k, "тс ");
        assert!(
            k.take_ops().contains(&Op::Commit("тс".into())),
            "a known abbreviation stays"
        );
    }

    #[test]
    fn settings_toggle_behaviour() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            autocorrect: false,
            ..Settings::default()
        });
        assert!(!k.engine.config().one_row_input);
        type_str(&mut k, "привкт ");
        assert!(k.take_ops().contains(&Op::Commit("Привкт".into())));
    }

    #[test]
    fn held_key_hides_neighbor_letters() {
        // «мастер» typed as «атер» with the finger resting on «а».
        let mut k = ime(&[("мастер", 3000), ("тер", 3000)]);
        k.start_input("", 1);
        press(&mut k, Action::Char('а'), 300);
        assert!(!k.hints[0].held, "no rhythm yet: no guessing");
        type_str(&mut k, " тер тер тер тер "); // clean words teach the rhythm
        assert!(k.rhythm.held_above().is_some());
        press(&mut k, Action::Char('а'), 300);
        type_str(&mut k, "тер");
        assert_eq!(k.slots[1].as_deref(), Some("мастер"), "{:?}", k.slots);
    }

    #[test]
    fn taps_on_key_borders_favour_the_dictionary_word() {
        // «мастер» tapped with every touch on the border between the intended
        // key and its left neighbor: letter by letter it reads «вячиук»-ish;
        // by touch point every intended letter is still nearly free.
        let words = [("мастер", 3000), ("пастер", 3000), ("ссора", 3000)];
        let mut k = ime(&words);
        k.start_input("", 1);
        let unit = k.width / 11.0;
        for c in "мастер".chars() {
            let (x, y) = k.key_center(Action::Char(c)).unwrap();
            let t = now();
            k.touch(DOWN, 0, x - 0.49 * unit, y, t);
            k.touch(UP, 0, x - 0.49 * unit, y, t + 80);
        }
        assert_eq!(
            k.suggestions()[1].as_deref(),
            Some("мастер"),
            "{:?}",
            k.suggestions()
        );
    }

    #[test]
    fn long_rare_word_with_several_slips_is_still_fixed() {
        // Three slips in a long, rare word: a lot of total cost, but little per letter.
        let mut k = ime(&[("проверяемый", 17000), ("проверка", 9000)]);
        k.start_input("", 1);
        type_str(&mut k, "провпряпмыф ");
        assert!(k.take_ops().contains(&Op::Commit("проверяемый".into())));
    }

    #[test]
    fn garbage_is_left_alone() {
        let mut k = ime(&[("проверяемый", 17000), ("проверка", 9000)]);
        k.start_input("", 1);
        type_str(&mut k, "ывапролдж ");
        assert!(k.take_ops().contains(&Op::Commit("ывапролдж".into())));
    }

    #[test]
    fn always_strength_replaces_any_non_word() {
        let mut k = ime(&[("проверяемый", 17000), ("проверка", 9000)]);
        k.set_settings(Settings {
            strength: crate::settings::Strength::Always,
            ..Settings::default()
        });
        k.start_input("", 1);
        type_str(&mut k, "пркврчнмвф ");
        let ops = k.take_ops();
        assert!(!ops.contains(&Op::Commit("пркврчнмвф".into())), "{ops:?}");
    }

    #[test]
    fn edited_words_do_not_teach_the_rhythm() {
        let mut k = ime(&[("тер", 3000)]);
        k.start_input("", 1);
        for _ in 0..5 {
            type_str(&mut k, "тевр⌫⌫р "); // ⌫ inside the word: not clean
        }
        assert!(k.rhythm.held_above().is_none());
    }

    #[test]
    fn a_slide_records_the_keys_crossed() {
        let mut k = ime(WORDS);
        let (mx, my) = k.key_center(Action::Char('м')).unwrap();
        let (ax, ay) = k.key_center(Action::Char('а')).unwrap();
        let t = now();
        k.touch(DOWN, 0, mx, my, t);
        k.touch(MOVE, 0, ax, ay, t + 40);
        k.touch(UP, 0, ax, ay, t + 90);
        assert_eq!(k.hints[0].slid, vec!['м']);
    }

    #[test]
    fn one_row_layout_decodes_columns() {
        let mut k = ime(&[("мастер", 2000), ("пастер", 6000), ("ссора", 2000)]);
        k.set_settings(Settings {
            one_row: true,
            ..Settings::default()
        });
        k.start_input("", 1);
        assert!(
            k.keys
                .iter()
                .all(|key| !matches!(key.action, Action::Char('й'))),
            "letters squeezed into columns"
        );
        for col in [4u8, 3, 3, 6, 4, 5] {
            let (x, y) = k.key_center(Action::Column(col)).unwrap();
            let t = now();
            k.touch(DOWN, 0, x, y, t);
            k.touch(UP, 0, x, y, t + 80);
        }
        assert_eq!(
            k.take_ops().last(),
            Some(&Op::Composing("мастер".into())),
            "the field shows the decoded word"
        );
        type_str(&mut k, " ");
        assert!(k.take_ops().contains(&Op::Commit("мастер".into())));
    }

    #[test]
    fn overlapping_presses_are_marked_as_rollover() {
        let mut k = ime(WORDS);
        let (nx, ny) = k.key_center(Action::Char('н')).unwrap();
        let (ax, ay) = k.key_center(Action::Char('а')).unwrap();
        let t = now();
        k.touch(DOWN, 0, nx, ny, t);
        k.touch(DOWN, 1, ax, ay, t + 40); // second thumb lands before the first lifts
        k.touch(UP, 0, nx, ny, t + 70);
        k.touch(UP, 1, ax, ay, t + 120);
        assert!(!k.hints[0].rollover);
        assert!(k.hints[1].rollover);
    }

    #[test]
    fn space_before_punctuation_moves_after_it() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        type_str(&mut k, "при ,");
        let ops = k.take_ops();
        let tail: Vec<Op> = ops[ops.len() - 3..].to_vec();
        assert_eq!(
            tail,
            vec![
                Op::Delete(1),
                Op::Commit(",".into()),
                Op::Commit(" ".into())
            ]
        );
        type_str(&mut k, " ");
        assert!(
            k.take_ops().is_empty(),
            "no double space after the auto one"
        );
    }

    #[test]
    fn backspace_pauses_live_correction_until_the_next_letter() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            live_correction: true,
            strip: Strip::Full,
            ..Settings::default()
        });
        k.start_input("", 1);
        type_str(&mut k, "привкт");
        assert_eq!(k.take_ops().last(), Some(&Op::Composing("привет".into())));
        type_str(&mut k, "⌫");
        assert_eq!(
            k.take_ops().last(),
            Some(&Op::Composing("привк".into())),
            "shows what was typed"
        );
        type_str(&mut k, "т");
        assert_eq!(
            k.take_ops().last(),
            Some(&Op::Composing("привет".into())),
            "correction resumes"
        );
        type_str(&mut k, "⌫ ");
        assert!(
            k.take_ops().contains(&Op::Commit("привк".into())),
            "space keeps what is shown"
        );
    }

    #[test]
    fn a_space_meant_as_a_letter_is_undone_by_the_next_word() {
        // «мастер»: the «т» landed on the top edge of the space bar.
        let mut k = ime(&[("мастер", 3000), ("мост", 3000), ("ер", 20000)]);
        k.start_input("", 1);
        type_str(&mut k, "мас");
        let (tx, _) = k.key_center(Action::Char('т')).unwrap();
        let space = k
            .keys
            .iter()
            .find(|key| key.action == Action::Space)
            .unwrap()
            .clone();
        let t = now();
        k.touch(DOWN, 0, tx, space.y + 4.0, t);
        k.touch(UP, 0, tx, space.y + 4.0, t + 80);
        assert!(k.prev_word.is_some(), "the space tap sat right under «т»");
        type_str(&mut k, "ер");
        assert_eq!(
            k.suggestions()[1].as_deref(),
            Some("↶ мастер"),
            "{:?}",
            k.suggestions()
        );
        k.take_ops();
        type_str(&mut k, " ");
        assert_eq!(
            k.take_ops(),
            vec![
                Op::Composing(String::new()),
                Op::Delete(4),
                Op::Commit("мастер".into()),
                Op::Commit(" ".into())
            ]
        );
    }

    #[test]
    fn a_space_between_two_real_words_is_just_a_space() {
        let mut k = ime(&[("как", 2000), ("дела", 3000)]);
        k.start_input("", 1);
        type_str(&mut k, "как дела");
        assert!(k.merge.is_none());
    }

    #[test]
    fn a_letter_typed_as_space_mid_bar_is_joined() {
        // «напр мер»: the «и» hit the middle of the space bar right under it.
        let mut k = ime(&[("напр", 12000), ("мер", 10000), ("например", 4000)]);
        k.start_input("", 1);
        type_str(&mut k, "напр");
        let (ix, _) = k.key_center(Action::Char('и')).unwrap();
        let (_, sy) = k.key_center(Action::Space).unwrap();
        let t = now();
        k.touch(DOWN, 0, ix, sy, t);
        k.touch(UP, 0, ix, sy, t + 80);
        type_str(&mut k, "мер");
        assert_eq!(k.merge.as_ref().map(|m| m.word.as_str()), Some("например"));
    }

    #[test]
    fn two_slot_strip_keeps_the_typed_word_left_and_does_not_repeat_the_fix() {
        let mut k = ime(WORDS);
        k.set_settings(Settings::default()); // live correction, strip: typed + fix
        k.start_input("", 1);
        type_str(&mut k, "привкт");
        assert_eq!(
            k.take_ops().last(),
            Some(&Op::Composing("привет".into())),
            "the fix is in the text"
        );
        let s = k.suggestions();
        assert_eq!(s[0].as_deref(), Some("привкт"));
        assert_ne!(s[1].as_deref(), Some("привет"), "not repeated in the strip");
        assert!(s[2].is_none());
        type_str(&mut k, " как ");
        assert!(k.suggestions()[0].is_none() && k.suggestions()[2].is_none());
    }

    #[test]
    fn a_missed_space_is_restored() {
        let words = [("привет", 3000), ("как", 2000), ("дела", 3000)];
        let mut k = ime(&words);
        k.start_input("", 1);
        type_str(&mut k, "приветкак");
        assert_eq!(
            k.suggestions()[1].as_deref(),
            Some("привет как"),
            "{:?}",
            k.suggestions()
        );
        k.take_ops();
        type_str(&mut k, " ");
        assert_eq!(
            k.take_ops(),
            vec![Op::Commit("привет как".into()), Op::Commit(" ".into())]
        );
        // «ь» sits right above the space bar: a space typed a little high.
        type_str(&mut k, "какьдела ");
        assert!(k.take_ops().contains(&Op::Commit("как дела".into())));
    }

    #[test]
    fn stretched_interjections_do_not_attract_corrections() {
        let mut k = ime(&[("ааа", 500), ("дела", 3000)]);
        k.start_input("", 1);
        type_str(&mut k, "аап");
        assert!(
            k.suggestions().iter().flatten().all(|s| s != "ааа"),
            "{:?}",
            k.suggestions()
        );
        type_str(&mut k, "⌫⌫⌫ааа ");
        assert!(
            k.take_ops().contains(&Op::Commit("ааа".into())),
            "typed on purpose, it stays"
        );
    }

    fn ime_ctx(words: &[(&str, u64)], pairs: &[(&str, &str, u64)]) -> Ime<Vec<u8>> {
        let id = |w: &str| -> Vec<u8> {
            w.chars()
                .map(|c| kbcore::alphabet::char_to_id(c).unwrap())
                .collect()
        };
        let mut kv: Vec<(Vec<u8>, u64)> = words.iter().map(|(w, v)| (id(w), *v)).collect();
        kv.sort();
        let dict = Map::from_iter(kv.iter().map(|(k, v)| (k.as_slice(), *v))).unwrap();
        let mut bv: Vec<(Vec<u8>, u64)> = pairs
            .iter()
            .map(|(a, b, v)| {
                let mut key = id(a);
                key.push(0);
                key.extend(id(b));
                (key, *v)
            })
            .collect();
        bv.sort();
        let bigrams = Map::from_iter(bv.iter().map(|(k, v)| (k.as_slice(), *v))).unwrap();
        let cfg = Config::balanced().with_profile(Profile::Phone);
        let mut k = Ime::new(Engine::new(dict, cfg).with_bigrams(bigrams), 2.625);
        k.measure(1080);
        k.set_settings(Settings {
            live_correction: false,
            strip: Strip::Full,
            ..Settings::default()
        });
        k.start_input("", 1 | 0x4000);
        k
    }

    #[test]
    fn context_fixes_a_valid_but_unlikely_word() {
        // «как де»: «де» is a word, but after «как» it's surely «же».
        let mut k = ime_ctx(
            &[("как", 2000), ("де", 9000), ("же", 9200)],
            &[("как", "же", 1500)],
        );
        k.start_input("", 1);
        type_str(&mut k, "как ");
        tap_between(&mut k, 'д', 'ж'); // a slip toward «ж»
        type_str(&mut k, "е ");
        assert!(k.take_ops().contains(&Op::Commit("же".into())));
    }

    #[test]
    fn a_word_typed_squarely_stays() {
        // «я ее»: «я не» is far likelier, but «е» was hit dead-center.
        let words = [("я", 2000), ("ее", 6000), ("не", 3000)];
        let mut k = ime_ctx(&words, &[("я", "не", 300)]);
        k.start_input("", 1);
        type_str(&mut k, "я ее ");
        assert!(k.before.ends_with("я ее "), "{:?}", k.before);
        // The same with the first tap on the border with «н»: a slip.
        let mut k = ime_ctx(&words, &[("я", "не", 300)]);
        k.start_input("", 1);
        type_str(&mut k, "я ");
        tap_between(&mut k, 'е', 'н');
        type_str(&mut k, "е ");
        assert!(k.before.ends_with("я не "), "{:?}", k.before);
    }

    #[test]
    fn predictions_after_a_space() {
        let mut k = ime_ctx(
            &[("как", 2000), ("дела", 3000), ("же", 3000)],
            &[("как", "дела", 900), ("как", "же", 1200)],
        );
        k.start_input("", 1);
        type_str(&mut k, "как ");
        assert_eq!(
            k.suggestions()[1].as_deref(),
            Some("дела"),
            "{:?}",
            k.suggestions()
        );
        assert_eq!(k.suggestions()[0].as_deref(), Some("же"));
        let x = k.width / 2.0;
        k.take_ops();
        k.touch(DOWN, 0, x, 10.0, now());
        k.touch(UP, 0, x, 10.0, now());
        assert_eq!(
            k.take_ops(),
            vec![Op::Commit("дела".into()), Op::Commit(" ".into())]
        );
    }

    #[test]
    fn backspace_deletes_a_selection() {
        let mut k = ime(WORDS);
        k.start_input("", 1);
        k.selection(0, 5, -1, -1, "", "", None);
        type_str(&mut k, "⌫");
        assert_eq!(k.take_ops(), vec![Op::Commit(String::new())]);
    }

    #[test]
    fn swipe_only_removes_the_language_key() {
        let mut k = ime(WORDS);
        k.set_settings(Settings {
            lang_switch: LangSwitch::Swipe,
            ..Settings::default()
        });
        assert!(k.key_center(Action::Lang).is_none());
        k.set_settings(Settings {
            lang_switch: LangSwitch::Key,
            ..Settings::default()
        });
        assert!(k.key_center(Action::Lang).is_some());
        let (x, y) = k.key_center(Action::Space).unwrap();
        k.touch(DOWN, 0, x, y, now());
        k.touch(MOVE, 0, x + 80.0 * k.dp, y, now());
        k.touch(UP, 0, x + 80.0 * k.dp, y, now());
        assert!(
            k.key_center(Action::Char('й')).is_some(),
            "no swipe switching with key-only"
        );
    }

    #[test]
    fn debug_log_records_touches_and_words_but_not_passwords() {
        let mut k = ime(WORDS);
        // Off unless asked for in the settings.
        k.set_logging(true);
        k.start_input("", 1);
        type_str(&mut k, "привкт ");
        assert!(k.take_log().is_empty());
        k.set_settings(Settings {
            debug_log: true,
            ..k.settings.clone()
        });
        k.start_input("", 1);
        type_str(&mut k, "привкт ");
        let log = k.take_log();
        assert!(
            log.iter()
                .any(|l| l.contains("\"ev\":\"down\"") && l.contains("\"key\":\"п\"")),
            "{log:?}"
        );
        assert!(
            log.iter()
                .any(|l| l.contains("\"ev\":\"word\"") && l.contains("\"out\":\"привет\"")),
            "{log:?}"
        );
        k.start_input("", 1 | 0x80); // password
        type_str(&mut k, "секрет");
        assert!(k.take_log().is_empty());
    }

    #[test]
    fn a_real_word_is_not_split() {
        let mut k = ime(&[("привет", 3000), ("при", 2000), ("вет", 2000)]);
        k.start_input("", 1);
        type_str(&mut k, "привет ");
        assert!(k.take_ops().contains(&Op::Commit("привет".into())));
    }

    #[test]
    fn second_symbols_page() {
        let mut k = ime(WORDS);
        for action in [Action::Symbols, Action::Symbols2] {
            let (x, y) = k.key_center(action).unwrap();
            k.touch(DOWN, 0, x, y, now());
            k.touch(UP, 0, x, y, now());
        }
        assert!(k.key_center(Action::Char('€')).is_some());
    }

    #[test]
    fn password_fields_bypass_the_engine() {
        let mut k = ime(WORDS);
        k.start_input("", 1 | 0x80); // text, password variation
        type_str(&mut k, "при");
        assert_eq!(
            k.take_ops(),
            vec![
                Op::Commit("п".into()),
                Op::Commit("р".into()),
                Op::Commit("и".into())
            ]
        );
    }

    #[test]
    fn draw_list_is_well_formed() {
        let k = ime(WORDS);
        let (ops, texts) = k.draw();
        assert_eq!(ops.len() % 7, 0);
        for op in ops.chunks(7) {
            assert!(op[0] == OP_RECT || op[0] == OP_TEXT);
            if op[0] == OP_TEXT {
                assert!((op[5] as usize) < texts.len());
            }
        }
    }
}
