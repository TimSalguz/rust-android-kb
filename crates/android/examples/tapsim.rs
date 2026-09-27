//! Tap-level benchmark: "type" dictionary words with simulated imprecise
//! touches on the real phone layout and see what the keyboard makes of them —
//! scoring letters by the key hit vs by the touch point.
//!
//!   cargo run --release -p kbime --example tapsim -- dict.fst data/lexicon.tsv [N]

use std::io::{BufRead, BufReader};

use kbcore::{Config, Engine, Profile};
use kbime::ime::{Ime, DOWN, UP};
use kbime::layout::Action;
use kbime::settings::Settings;

/// xorshift64* — deterministic, no dependencies.
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.unit().max(1e-7), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

#[derive(Clone, Copy)]
enum Noise {
    /// Gaussian around the key center, σ in QWERTY key widths.
    Gauss(f32),
    /// Every tap exactly on the border with a random neighboring key.
    Border,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dict, lexicon) = (&args[0], &args[1]);
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(400);

    // Frequent-but-not-trivial Russian words, typeable without long-presses.
    let mut lex: Vec<(String, u64)> = BufReader::new(std::fs::File::open(lexicon).unwrap())
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| {
            let (w, c) = l.split_once('\t')?;
            Some((w.to_string(), c.parse().ok()?))
        })
        .filter(|(w, _)| {
            (4..=12).contains(&w.chars().count())
                && w.chars().all(|c| ('а'..='я').contains(&c) && c != 'ъ')
        })
        .collect();
    lex.sort_by_key(|w| std::cmp::Reverse(w.1));
    let pool: Vec<String> = lex
        .into_iter()
        .skip(200)
        .take(20000)
        .map(|(w, _)| w)
        .collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let words: Vec<String> = (0..n)
        .map(|_| pool[(rng.unit() * pool.len() as f32) as usize % pool.len()].clone())
        .collect();

    println!("{n} words, phone layout 1080 px, density 2.75");
    println!(
        "{:<14} {:<10} {:>7} {:>7} {:>9}",
        "noise", "scoring", "top-1", "top-3", "exact-hit"
    );
    for (label, noise) in [
        ("σ=0.30", Noise::Gauss(0.30)),
        ("σ=0.45", Noise::Gauss(0.45)),
        ("borders", Noise::Border),
    ] {
        for touch_points in [false, true] {
            let engine =
                Engine::open(dict, Config::balanced().with_profile(Profile::Phone)).unwrap();
            let mut ime = Ime::new(engine, 2.75);
            ime.set_settings(Settings {
                touch_points,
                ..Settings::default()
            });
            ime.measure(1080);
            let unit = 1080.0 / 10.0;
            let mut rng = Rng(42);
            let (mut top1, mut top3, mut exact, mut t) = (0, 0, 0, 0i64);
            for word in &words {
                ime.start_input("", 1);
                let mut hit_all = true;
                for c in word.chars() {
                    let (cx, cy) = ime.key_center(Action::Char(c)).unwrap();
                    let (x, y) = match noise {
                        // A tap that would land on a non-letter key (⇧, ⌫,
                        // space…) is one the user would redo: resample it.
                        Noise::Gauss(s) => loop {
                            let (x, y) = (cx + rng.gauss() * s * unit, cy + rng.gauss() * s * unit);
                            if matches!(ime.action_at(x, y), Some(Action::Char(l)) if l.is_alphabetic())
                            {
                                break (x, y);
                            }
                        },
                        Noise::Border => {
                            // Midpoint to a random letter key within ~1.6 key widths.
                            let near: Vec<(f32, f32)> = "йцукенгшщзхфывапролджэячсмитьбю"
                                .chars()
                                .filter(|&o| o != c)
                                .filter_map(|o| ime.key_center(Action::Char(o)))
                                .filter(|&(ox, oy)| {
                                    ((ox - cx) / unit).powi(2) + ((oy - cy) / unit).powi(2) < 2.6
                                })
                                .collect();
                            let (ox, oy) =
                                near[(rng.unit() * near.len() as f32) as usize % near.len()];
                            ((cx + ox) / 2.0, (cy + oy) / 2.0)
                        }
                    };
                    t += 150;
                    ime.touch(DOWN, 0, x, y, t);
                    ime.touch(UP, 0, x, y, t + 80);
                    if ime
                        .key_center(Action::Char(c))
                        .is_some_and(|_| ime.last_typed() != Some(c))
                    {
                        hit_all = false;
                    }
                }
                let s = ime.suggestions();
                top1 += (s[1].as_deref() == Some(word.as_str())) as usize;
                top3 += s.iter().any(|x| x.as_deref() == Some(word.as_str())) as usize;
                exact += hit_all as usize;
                ime.take_output();
            }
            let pct = |k: usize| 100.0 * k as f32 / n as f32;
            let scoring = if touch_points { "touch" } else { "letters" };
            println!(
                "{label:<14} {scoring:<10} {:>6.1}% {:>6.1}% {:>8.1}%",
                pct(top1),
                pct(top3),
                pct(exact)
            );
        }
    }
}
