//! Drive the keyboard by hand, a step at a time, as a person does: type a
//! word or two without looking, then look at what came out and decide what
//! to do. The session file holds the steps; each run replays them all on a
//! fresh keyboard (the same taps: the noise is seeded by the step) and shows
//! the frame after the last one — the field, the strip — and writes the
//! frame's draw list beside the session (`SESSION.frame`, for
//! tools/drive_frame.py to draw).
//!
//!   cargo run --release -p kbime --example drive -- dict.fst bigrams.fst casing.fst SESSION
//!
//! Steps, one per line (`#` a comment):
//!   type σ text   tap the text's keys, each tap off the key's center by
//!                 Gaussian noise of σ key widths (0.25 careful, 0.4 fast,
//!                 0.6 sloppy), 140–280 ms apart; capitals with shift, other
//!                 characters from the symbol layers (and back)
//!   bksp n        tap backspace n times
//!   hold ms       hold backspace down for ms
//!   cursor where  tap into the field: `end`, `start`, a char index,
//!                 `before:w`, `after:w`, `in:w` (the middle of w; its last
//!                 occurrence)
//!   pick k        tap the strip's slot k (1 left, 2 center, 3 right)
//!   swipeup       swipe up on the space bar (the next variant)
//!   at x y        a tap at that point of the keyboard (px)
//!   wait ms       nothing for ms
//!
//! The field acts as Android's editor does for an input connection: commit,
//! composing text, deletes around the cursor; after each batch of edits
//! that moves the cursor or the composing span the keyboard hears of it
//! (`Ime::selection`), as from onUpdateSelection.

use kbcore::{Config, Engine, Profile};
use kbime::ime::{Ime, DOWN, MOVE, OUTPUT, UP};
use kbime::layout::Action;
use kbime::settings::Settings;

#[allow(dead_code)]
mod common;

use common::Rng;

/// The editor: its text, the selection and the composing span (in chars).
#[derive(Default)]
struct Field {
    text: Vec<char>,
    sel: (usize, usize),
    composing: Option<(usize, usize)>,
}

impl Field {
    fn replace(&mut self, (a, b): (usize, usize), with: &str) -> usize {
        let t: Vec<char> = with.chars().collect();
        let n = t.len();
        self.text.splice(a..b, t);
        a + n
    }

    /// Where new text goes: the composing span, else the selection.
    fn target(&self) -> (usize, usize) {
        self.composing.unwrap_or(self.sel)
    }

    fn apply(&mut self, ops: &str) {
        for op in ops.split('\0').filter(|s| !s.is_empty()) {
            let (kind, arg) = op.split_at(1);
            match kind {
                "C" => {
                    let end = self.replace(self.target(), arg);
                    self.composing = None;
                    self.sel = (end, end);
                }
                "Z" => {
                    let (a, _) = self.target();
                    let end = self.replace(self.target(), arg);
                    self.composing = (end > a).then_some((a, end));
                    self.sel = (end, end);
                }
                "F" => self.composing = None,
                "D" => self.delete(arg.parse().unwrap_or(0), 0),
                "E" => {
                    let end = self.replace(self.target(), "\n");
                    self.composing = None;
                    self.sel = (end, end);
                }
                "K" => {
                    let c = (self.sel.1 + 1).min(self.text.len());
                    self.composing = None;
                    self.sel = (c, c);
                }
                "R" => {
                    let r: Vec<&str> = arg.splitn(3, '\t').collect();
                    let before = r[0].parse().unwrap_or(0);
                    let after = r.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    self.delete(before, after);
                    let end = self.replace(self.sel, r.get(2).copied().unwrap_or(""));
                    self.composing = None;
                    self.sel = (end, end);
                }
                _ => {}
            }
        }
    }

    /// Delete `before` chars before the selection and `after` after it; the
    /// composing span shrinks with what went.
    fn delete(&mut self, before: usize, after: usize) {
        let (a, b) = self.sel;
        let (a0, b1) = (a.saturating_sub(before), (b + after).min(self.text.len()));
        let gone = |p: usize| {
            if p <= a0 {
                p
            } else if p <= a {
                a0
            } else if p <= b {
                p - (a - a0)
            } else if p <= b1 {
                b - (a - a0)
            } else {
                p - (a - a0) - (b1 - b)
            }
        };
        self.composing = self
            .composing
            .map(|(s, e)| (gone(s), gone(e)))
            .filter(|(s, e)| e > s);
        self.text.drain(b..b1);
        self.text.drain(a0..a);
        self.sel = (a0, a0 + (b - a));
    }

    fn before(&self, n: usize) -> String {
        let a = self.sel.0;
        self.text[a.saturating_sub(n)..a].iter().collect()
    }

    fn after(&self, n: usize) -> String {
        let b = self.sel.1;
        self.text[b..(b + n).min(self.text.len())].iter().collect()
    }

    /// The field as a line: the cursor `│`, the composing span `[…]`.
    fn show(&self) -> String {
        let mut s = String::new();
        for (i, c) in self.text.iter().enumerate() {
            if self.composing.is_some_and(|(a, _)| a == i) {
                s.push('[');
            }
            if self.sel.0 == i {
                s.push('│');
            }
            if self.composing.is_some_and(|(_, b)| b == i) {
                s.push(']');
            }
            s.push(*c);
        }
        let n = self.text.len();
        if self.composing.is_some_and(|(a, _)| a == n) {
            s.push('[');
        }
        if self.composing.is_some_and(|(_, b)| b == n) {
            s.push(']');
        }
        if self.sel.0 == n {
            s.push('│');
        }
        s.replace('\n', "⏎")
    }
}

struct Hands {
    ime: Ime<kbcore::Mmap>,
    field: Field,
    t: i64,
    /// When the keyboard asked to be woken (its timer).
    wake: Option<i64>,
    key_w: f32,
    /// What the taps of the step hit (the key under the finger).
    hit: String,
}

impl Hands {
    /// Hear the keyboard's answer: its edits (and the editor's word back
    /// about the cursor), its timer.
    fn answer(&mut self, flags: i32) {
        let delay = (flags as u32 >> 16) as i64;
        if delay > 0 {
            self.wake = Some(self.t + delay);
        }
        if flags & OUTPUT == 0 {
            return;
        }
        let mut ops = self.ime.take_output();
        if std::env::var("OPS").is_ok() {
            eprintln!("    ops {:?}", ops.replace('\0', " | "));
        }
        for _ in 0..4 {
            if ops.is_empty() {
                break;
            }
            let was = (self.field.sel, self.field.composing);
            self.field.apply(&ops);
            ops.clear();
            if (self.field.sel, self.field.composing) == was {
                break;
            }
            let (cs, ce) = self
                .field
                .composing
                .map_or((-1, -1), |(a, b)| (a as i32, b as i32));
            let selected: Option<String> = (self.field.sel.0 != self.field.sel.1)
                .then(|| self.field.text[self.field.sel.0..self.field.sel.1].iter().collect());
            let f = self.ime.selection(
                self.field.sel.0 as i32,
                self.field.sel.1 as i32,
                cs,
                ce,
                &self.field.before(48),
                &self.field.after(32),
                selected.as_deref(),
            );
            let delay = (f as u32 >> 16) as i64;
            if delay > 0 {
                self.wake = Some(self.t + delay);
            }
            if f & OUTPUT != 0 {
                ops = self.ime.take_output();
            }
        }
    }

    /// Let time pass to `until`, the keyboard's timers going off.
    fn pass(&mut self, until: i64) {
        while let Some(w) = self.wake.filter(|&w| w <= until) {
            self.wake = None;
            self.t = self.t.max(w);
            let f = self.ime.timer();
            self.answer(f);
        }
        self.t = self.t.max(until);
    }

    fn touch(&mut self, action: i32, x: f32, y: f32) {
        let f = self.ime.touch(action, 0, x, y, self.t);
        self.answer(f);
    }

    /// A tap near (x, y): off by the noise, down for a finger's while.
    fn tap_at(&mut self, x: f32, y: f32, gap: i64) {
        if std::env::var("OPS").is_ok() {
            eprintln!("  tap {x:.0},{y:.0} {:?}", self.ime.action_at(x, y));
        }
        self.pass(self.t + gap);
        self.touch(DOWN, x, y);
        self.pass(self.t + 70);
        self.touch(UP, x, y);
    }

    fn key(&mut self, action: Action) -> Option<(f32, f32)> {
        self.ime.key_center(action)
    }

    /// Tap the key of `action`, off its center by σ key widths.
    fn tap(&mut self, action: Action, sigma: f32, rng: &mut Rng) -> bool {
        let Some((x, y)) = self.key(action) else {
            return false;
        };
        let (dx, dy) = (rng.gauss() * sigma * self.key_w, rng.gauss() * sigma * self.key_w * 1.3);
        let gap = 140 + (rng.unit() * 140.0) as i64;
        match self.ime.action_at(x + dx, y + dy) {
            Some(Action::Char(c)) => self.hit.push(c),
            Some(Action::Space) => self.hit.push('␣'),
            Some(Action::Backspace) => self.hit.push('⌫'),
            Some(Action::Shift) => self.hit.push('⇧'),
            Some(a) => self.hit.push_str(&format!("‹{a:?}›")),
            None => self.hit.push('∅'),
        }
        self.tap_at(x + dx, y + dy, gap);
        true
    }

    /// Type a character as a person does: shift for a capital, the symbol
    /// layers for what the letters don't have (and back).
    fn type_char(&mut self, c: char, sigma: f32, rng: &mut Rng) {
        let action = match c {
            ' ' => Action::Space,
            '\n' => Action::Enter,
            c => Action::Char(c.to_lowercase().next().unwrap_or(c)),
        };
        // A letter on a symbol layer: back to the letters first (digits and
        // marks in a row stay on the symbols, as a person types «18:30»).
        if c.is_alphabetic() && self.key(action).is_none() {
            self.tap(Action::Letters, sigma, rng);
        }
        if c.is_uppercase() {
            self.tap(Action::Shift, sigma, rng);
        }
        if self.tap(action, sigma, rng) {
            return;
        }
        // Another layer: the symbols, then the second page of them.
        let found = (self.tap(Action::Symbols, sigma, rng) && self.tap(action, sigma, rng))
            || (self.tap(Action::Symbols2, sigma, rng) && self.tap(action, sigma, rng));
        if !found {
            eprintln!("no key for {c:?}");
        }
    }

    /// Where in the field a tap puts the cursor.
    fn place(&self, spec: &str) -> Option<usize> {
        let text: String = self.field.text.iter().collect();
        let chars = |byte: usize| text[..byte].chars().count();
        match spec.split_once(':') {
            None if spec == "end" => Some(self.field.text.len()),
            None if spec == "start" => Some(0),
            None => spec.parse().ok(),
            Some((how, w)) => {
                let at = text.rfind(w)?;
                let (a, n) = (chars(at), w.chars().count());
                match how {
                    "before" => Some(a),
                    "after" => Some(a + n),
                    "in" => Some(a + n / 2),
                    _ => None,
                }
            }
        }
    }

    /// Tap into the field: the editor moves the cursor (the composing span
    /// stays where it was), the keyboard hears of it.
    fn cursor(&mut self, at: usize) {
        self.pass(self.t + 400);
        let at = at.min(self.field.text.len());
        self.field.sel = (at, at);
        let (cs, ce) = self
            .field
            .composing
            .map_or((-1, -1), |(a, b)| (a as i32, b as i32));
        let f = self.ime.selection(
            at as i32,
            at as i32,
            cs,
            ce,
            &self.field.before(48),
            &self.field.after(32),
            None,
        );
        self.answer(f);
    }

    /// The strip's slot k (1..=3): its text's place in the draw list.
    fn slot(&self, k: usize) -> Option<(f32, f32)> {
        let word = self.ime.suggestions().get(k.checked_sub(1)?)?.clone()?;
        let (ops, texts) = self.ime.draw();
        ops.chunks(7)
            .find(|op| op[0] == kbime::ime::OP_TEXT && texts.get(op[5] as usize) == Some(&word))
            .map(|op| (op[1] as f32, op[2] as f32 - op[3] as f32 / 3.0))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config::balanced().with_profile(Profile::Phone);
    for kv in std::env::var("KB_SET").unwrap_or_default().split(',') {
        if let Some((k, v)) = kv.split_once('=') {
            if let Ok(v) = v.trim().parse::<f32>() {
                let _ = cfg.set_param(k.trim(), v);
            }
        }
    }
    let mut engine = Engine::open_with_bigrams(&args[0], &args[1], cfg).expect("dictionary");
    engine = engine.open_casing(&args[2]).expect("casing");
    let beside = |name: &str| std::path::Path::new(&args[1]).with_file_name(name);
    if beside("lemmas.bin").exists() {
        engine = engine.open_lemmas(beside("lemmas.bin")).expect("lemmas");
        if beside("chooser.bin").exists() {
            engine = engine.open_chooser(beside("chooser.bin")).expect("chooser");
        }
        if beside("parser.bin").exists() {
            engine = engine.open_parser(beside("parser.bin")).expect("parser");
        }
    }
    let session = &args[3];
    let steps = std::fs::read_to_string(session).expect("session");
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut ime = Ime::new(engine, 2.625);
    let height = ime.measure(1080);
    ime.set_settings(Settings::default());
    let mut hands = Hands {
        ime,
        field: Field::default(),
        t: 1_000,
        wake: None,
        key_w: 1080.0 / 11.0,
        hit: String::new(),
    };
    // A messenger's field: text, capitals at the sentences' starts.
    let f = hands.ime.start_input("", 0x4001);
    hands.answer(f);
    for (n, line) in steps.lines().enumerate() {
        // Trailing spaces are typed.
        let line = line.trim_start().trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        hands.hit.clear();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15
            ^ (seed.wrapping_mul(31) + n as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
        let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
        match cmd {
            "type" => {
                let (sigma, text) = rest.split_once(' ').unwrap_or((rest, ""));
                let sigma: f32 = sigma.parse().unwrap_or(0.3);
                for c in text.replace("\\n", "\n").chars() {
                    hands.type_char(c, sigma, &mut rng);
                }
            }
            "bksp" => {
                for _ in 0..rest.parse().unwrap_or(1) {
                    hands.tap(Action::Backspace, 0.15, &mut rng);
                }
            }
            "hold" => {
                if let Some((x, y)) = hands.key(Action::Backspace) {
                    hands.pass(hands.t + 200);
                    hands.touch(DOWN, x, y);
                    hands.pass(hands.t + rest.parse::<i64>().unwrap_or(600));
                    hands.touch(UP, x, y);
                }
            }
            "cursor" => match hands.place(rest) {
                Some(at) => hands.cursor(at),
                None => eprintln!("no place {rest:?}"),
            },
            "pick" => match hands.slot(rest.parse().unwrap_or(2)) {
                Some((x, y)) => hands.tap_at(x, y, 500),
                None => eprintln!("no slot {rest}"),
            },
            "swipeup" => {
                if let Some((x, y)) = hands.key(Action::Space) {
                    hands.pass(hands.t + 300);
                    hands.touch(DOWN, x, y);
                    for i in 1..=6 {
                        hands.pass(hands.t + 12);
                        hands.touch(MOVE, x, y - i as f32 * 30.0);
                    }
                    hands.touch(UP, x, y - 180.0);
                }
            }
            "at" => {
                let xy: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                if let [x, y] = xy[..] {
                    hands.tap_at(x, y, 300);
                }
            }
            "wait" => hands.pass(hands.t + rest.parse::<i64>().unwrap_or(500)),
            _ => eprintln!("unknown step {line:?}"),
        }
        let hit = if hands.hit.is_empty() {
            String::new()
        } else {
            format!("   (нажато: {})", hands.hit)
        };
        println!("{:<40} {}{hit}", line, hands.field.show());
    }
    hands.pass(hands.t + 1);
    let strip: Vec<String> = hands
        .ime
        .suggestions()
        .iter()
        .map(|s| s.clone().unwrap_or_default())
        .collect();
    println!("\nполе:     {}", hands.field.show());
    println!("подсказки: [{}] [{}] [{}]", strip[0], strip[1], strip[2]);
    // The frame: the field, then the keyboard's draw list.
    let (ops, texts) = hands.ime.draw();
    let mut frame = format!(
        "field\t{}\nheight\t{height}\nbackground\t{}\n",
        hands.field.show(),
        hands.ime.background()
    );
    for op in ops.chunks(7) {
        let line: Vec<String> = op.iter().map(i32::to_string).collect();
        frame.push_str(&format!("op\t{}\n", line.join(" ")));
    }
    for t in texts {
        frame.push_str(&format!("text\t{}\n", t.replace('\n', "⏎")));
    }
    std::fs::write(format!("{session}.frame"), frame).expect("frame");
}
