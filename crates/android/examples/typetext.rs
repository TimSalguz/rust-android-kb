//! Type text through the keyboard, with the real dictionary, and print what
//! ends up in the field — the whole pipeline (autocorrect, merges, the
//! two-word window, capitals) on real sentences.
//!
//!   cargo run --release -p kbime --example typetext -- dict.fst bigrams.fst [casing.fst] < lines.txt
//!
//! Each input line is typed tap by tap at the key centers and ended with a
//! space; one output line per input line. `JITTER=0.3` adds Gaussian noise
//! (σ in key widths, seeded, reproducible; `SEED=n` another run); `LIVE=1`
//! turns live correction on;
//! `WINDOW=<margin>` sets the two-word window's margin (`inf`: off);
//! `KNOWN=<cost>` how cheap a slip must be to replace a typed real word;
//! `INSIDE=1` keeps every noisy tap inside the key meant (careful typing:
//! off-center, never a wrong key — every change is then a false correction).
//! `[x/y]` in a line: the finger meant y and caught x, just past the border
//! between them (a real-word slip, see tools/eval_realword.py). `⌫` taps
//! backspace, `⇡` swipes up on the space bar (the next variant), `|` prints
//! the field and the suggestion strip there; `NOSPACE=1`: no space at the
//! end of a line.
//!
//! `DUMP=file` writes, before each space, what the keyboard read the word
//! as: `text before<TAB>word meant<TAB>typed<TAB>word:cost:edit|…` (every
//! candidate, best first) — data for tools/chooser.py.
//!
//! `COMMAS=1` lets the keyboard put in the commas it is sure of.
//! `SWIPE=σ` draws the words instead (see `common::draw`: corners missed by
//! σ key widths, the finger's pace too), each followed by a tap on space;
//! one-letter words, and words whose way is too short to read as a drawn
//! word, are tapped.

use std::io::{BufRead, Write};

use kbcore::{Config, Engine, Profile};
use kbime::ime::{Ime, DOWN, MOVE, UP};
use kbime::layout::Action;
use kbime::settings::{Settings, Strip};

mod common;

use common::{Pace, Rng};

const LETTERS: &str = "йцукенгшщзхфывапролджэячсмитьбю";
/// A word whose way between its keys is shorter than this (key widths) is
/// tapped: the keyboard wouldn't read it as drawn.
const SHORTEST_SWIPE: f32 = 2.6;

/// The field's text, from the IME's output ops (see `Ime::take_output`).
#[derive(Default)]
struct Field {
    text: String,
    composing: String,
}

impl Field {
    fn apply(&mut self, out: &str) {
        for op in out.split('\0').filter(|s| !s.is_empty()) {
            let (kind, arg) = op.split_at(1);
            match kind {
                "C" => {
                    self.composing.clear();
                    self.text.push_str(arg);
                }
                "Z" => self.composing = arg.to_string(),
                "F" => self.text.push_str(&std::mem::take(&mut self.composing)),
                "D" => {
                    let n: usize = arg.parse().unwrap_or(0);
                    let keep = self.text.chars().count().saturating_sub(n);
                    self.text = self.text.chars().take(keep).collect();
                }
                "E" => self.text.push('\n'),
                _ => {}
            }
        }
    }
}

/// A tap at the key's center.
fn tap(ime: &mut Ime<kbcore::Mmap>, field: &mut Field, action: Action, t: &mut i64) {
    if let Some((x, y)) = ime.key_center(action) {
        *t += 180;
        ime.touch(DOWN, 0, x, y, *t);
        ime.touch(UP, 0, x, y, *t + 70);
        field.apply(&ime.take_output());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config {
        home_row: false,
        ..Config::balanced().with_profile(Profile::Phone)
    };
    // KB_SET="w_classes=1.5,…" tunes the weights.
    for kv in std::env::var("KB_SET").unwrap_or_default().split(',') {
        if let Some((k, v)) = kv.split_once('=') {
            if let Ok(v) = v.trim().parse::<f32>() {
                let _ = cfg.set_param(k.trim(), v);
            }
        }
    }
    let mut engine = Engine::open_with_bigrams(&args[0], &args[1], cfg).expect("dictionary");
    if let Some(casing) = args.get(2) {
        engine = engine.open_casing(casing).expect("casing");
    }
    // The lemma vectors, when they lie beside the context model.
    let lemmas = std::path::Path::new(&args[1]).with_file_name("lemmas.bin");
    if lemmas.exists() {
        engine = engine.open_lemmas(lemmas).expect("lemmas");
        // And the chooser, which reads them.
        let chooser = std::path::Path::new(&args[1]).with_file_name("chooser.bin");
        if chooser.exists() {
            engine = engine.open_chooser(chooser).expect("chooser");
        }
    }
    let jitter: f32 = std::env::var("JITTER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let mut ime = Ime::new(engine, 2.625);
    ime.measure(1080);
    ime.set_settings(Settings {
        live_correction: std::env::var("LIVE").is_ok_and(|v| v == "1"),
        // The sentences are compared word for word: commas only on request.
        auto_commas: std::env::var("COMMAS").is_ok_and(|v| v == "1"),
        strip: Strip::Full,
        ..Settings::default()
    });
    if let Some(m) = std::env::var("WINDOW").ok().and_then(|s| s.parse().ok()) {
        ime.set_window_margin(m);
    }
    if let Some(m) = std::env::var("KNOWN").ok().and_then(|s| s.parse().ok()) {
        ime.set_known_slip(m);
    }
    let inside = std::env::var("INSIDE").is_ok_and(|v| v == "1");
    let key_w = 1080.0 / 11.0;
    // SEED=n: another run of the noise.
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
    let mut t: i64 = 1_000;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut dump = std::env::var("DUMP")
        .ok()
        .map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("DUMP file")));
    let swipe: Option<f32> = std::env::var("SWIPE").ok().and_then(|s| s.parse().ok());
    let letter_keys: Vec<(char, f32, f32)> = LETTERS
        .chars()
        .filter_map(|c| ime.key_center(Action::Char(c)).map(|(x, y)| (c, x, y)))
        .collect();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        ime.start_input("", 1);
        ime.take_output();
        let mut field = Field::default();
        if let Some(sigma) = swipe {
            for word in line.split_whitespace() {
                let centers: Vec<(f32, f32)> = word
                    .chars()
                    .filter_map(|c| letter_keys.iter().find(|k| k.0 == c).map(|k| (k.1, k.2)))
                    .collect();
                let way: f32 = centers
                    .windows(2)
                    .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                    .sum();
                // The finger lands on the first letter's key (as a tap would:
                // drawn again when it didn't); the corners after it wander.
                let long = centers.len() == word.chars().count() && way >= SHORTEST_SWIPE * key_w;
                let first = word.chars().next().map(Action::Char);
                let drawn = long
                    .then(|| {
                        (0..30)
                            .filter_map(|_| {
                                common::draw(
                                    word,
                                    &letter_keys,
                                    key_w,
                                    sigma,
                                    Pace::Corners,
                                    &mut rng,
                                )
                            })
                            .find(|(pts, _)| ime.action_at(pts[0].0, pts[0].1) == first)
                    })
                    .flatten();
                match drawn {
                    Some((pts, times)) => {
                        let times = times.unwrap_or_default();
                        let at = |i: usize| t + 300 + times.get(i).copied().unwrap_or(0.0) as i64;
                        let last = pts.len() - 1;
                        ime.touch(DOWN, 0, pts[0].0, pts[0].1, at(0));
                        for (i, &(x, y)) in pts.iter().enumerate().take(last).skip(1) {
                            ime.touch(MOVE, 0, x, y, at(i));
                        }
                        ime.touch(UP, 0, pts[last].0, pts[last].1, at(last));
                        t = at(last);
                        field.apply(&ime.take_output());
                    }
                    None => {
                        for c in word.chars() {
                            tap(&mut ime, &mut field, Action::Char(c), &mut t);
                        }
                    }
                }
                tap(&mut ime, &mut field, Action::Space, &mut t);
            }
            writeln!(out, "{}{}", field.text, field.composing).ok();
            continue;
        }
        let mut taps: Vec<(char, Option<char>)> = Vec::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c != '[' {
                taps.push((c, None));
                continue;
            }
            let (hit, meant) = (chars.next(), chars.nth(1));
            chars.next();
            if let (Some(hit), Some(meant)) = (hit, meant) {
                taps.push((hit, Some(meant)));
            }
        }
        if !std::env::var("NOSPACE").is_ok_and(|v| v == "1") {
            taps.push((' ', None));
        }
        let mut meant_words = line.split(' ').filter(|w| !w.is_empty());
        for (c, meant) in taps {
            if c == ' ' {
                if let (Some(d), Some(word)) = (dump.as_mut(), meant_words.next()) {
                    let cands: Vec<String> = ime
                        .decoded()
                        .iter()
                        .map(|c| format!("{}:{:.3}:{:.3}", c.word, c.cost, c.edit))
                        .collect();
                    writeln!(
                        d,
                        "{}\t{}\t{}\t{}",
                        field.text,
                        word.to_lowercase(),
                        field.composing,
                        cands.join("|")
                    )
                    .ok();
                }
            }
            if c == '|' {
                writeln!(
                    out,
                    "  [{}{}] {:?}",
                    field.text,
                    field.composing,
                    ime.suggestions()
                )
                .ok();
                continue;
            }
            if c == '⇡' {
                let Some((x, y)) = ime.key_center(Action::Space) else {
                    continue;
                };
                t += 180;
                ime.touch(DOWN, 0, x, y, t);
                ime.touch(kbime::ime::MOVE, 0, x, y - 150.0, t + 40);
                ime.touch(UP, 0, x, y - 150.0, t + 80);
                field.apply(&ime.take_output());
                continue;
            }
            let action = match c {
                ' ' => Action::Space,
                '⌫' => Action::Backspace,
                _ => Action::Char(c.to_lowercase().next().unwrap_or(c)),
            };
            let Some((x, y)) = ime.key_center(action) else {
                // No key of its own here (ё, a hyphen): as its key gives it.
                if let Action::Char(c) = action {
                    ime.put_char(c);
                    field.apply(&ime.take_output());
                }
                continue;
            };
            // Noise that lands on another kind of key (⌫, Enter, the strip)
            // is drawn again: only letter-for-letter slips are simulated.
            let (mut dx, mut dy) = (0.0, 0.0);
            let border = meant
                .and_then(|m| ime.key_center(Action::Char(m)))
                .map(|(mx, my)| (mx + 0.6 * (x - mx), my + 0.6 * (y - my)))
                .filter(|&(bx, by)| ime.action_at(bx, by) == Some(action));
            if let Some((bx, by)) = border {
                (dx, dy) = (bx - x, by - y);
            }
            for _ in 0..if border.is_some() { 0 } else { 30 } {
                let (ex, ey) = (rng.gauss() * jitter * key_w, rng.gauss() * jitter * key_w);
                let hit = ime.action_at(x + ex, y + ey);
                let same_kind = match (action, hit) {
                    _ if inside => hit == Some(action),
                    (Action::Space, Some(Action::Space)) => true,
                    (Action::Char(_), Some(Action::Char(c))) => c.is_alphabetic(),
                    _ => false,
                };
                if same_kind {
                    (dx, dy) = (ex, ey);
                    break;
                }
            }
            t += 180;
            ime.touch(DOWN, 0, x + dx, y + dy, t);
            ime.touch(UP, 0, x + dx, y + dy, t + 70);
            field.apply(&ime.take_output());
        }
        writeln!(out, "{}{}", field.text, field.composing).ok();
    }
}
