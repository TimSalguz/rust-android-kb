//! Gesture-typing benchmark: draw frequent words as a finger would — corners
//! missed by a Gaussian amount, cut by smoothing — on the real phone layout,
//! and see what the decoder reads.
//!
//!   cargo run --release -p kbime --example swipesim -- dict.fst data/lexicon.tsv [N]
//!
//! `PACE=corners` times the way too: the finger moves in strokes between the
//! turns (fast in the middle, slow at the ends: minimum jerk) and passes the
//! letters on a straight line at speed; `PACE=letters` slows down on every
//! letter. Without it, the path alone.

use std::io::{BufRead, BufReader};
use std::time::Instant;

use kbcore::{Config, Engine, Profile};
use kbime::ime::Ime;
use kbime::layout::Action;

mod common;

use common::{Pace, Rng};

const LETTERS: &str = "йцукенгшщзхфывапролджэячсмитьбю";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dict, lexicon) = (&args[0], &args[1]);
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(500);

    let mut lex: Vec<(String, u64)> = BufReader::new(std::fs::File::open(lexicon).unwrap())
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| {
            let (w, c) = l.split_once('\t')?;
            Some((w.to_string(), c.parse().ok()?))
        })
        .filter(|(w, _)| {
            (2..=12).contains(&w.chars().count()) && w.chars().all(|c| LETTERS.contains(c))
        })
        .collect();
    lex.sort_by_key(|w| std::cmp::Reverse(w.1));
    let pool: Vec<String> = lex.into_iter().take(20000).map(|(w, _)| w).collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let words: Vec<String> = (0..n)
        .map(|_| pool[(rng.unit() * pool.len() as f32) as usize % pool.len()].clone())
        .collect();

    let engine = Engine::open(dict, Config::balanced().with_profile(Profile::Phone)).unwrap();
    let mut ime = Ime::new(engine, 2.625);
    ime.measure(1080);
    let keys: Vec<(char, f32, f32)> = LETTERS
        .chars()
        .filter_map(|c| ime.key_center(Action::Char(c)).map(|(x, y)| (c, x, y)))
        .collect();
    let key_w = 1080.0 / 11.0;
    // KB_SET="gesture_location=20,…" tunes the weights.
    let mut cfg = Config::balanced().with_profile(Profile::Phone);
    for kv in std::env::var("KB_SET")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        if let Some((k, v)) = kv.split_once('=') {
            if let Ok(v) = v.trim().parse::<f32>() {
                let _ = cfg.set_param(k.trim(), v);
            }
        }
    }
    let decoder = Engine::open(dict, cfg).unwrap();

    // WORDS="мне,три" SIGMA=0.3: each drawn 200 times, what it reads as —
    // and, where it goes wrong, the leaders with their geometry and prior.
    if let Ok(list) = std::env::var("WORDS") {
        let sigma: f32 = std::env::var("SIGMA")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.3);
        let mut rng = Rng(42);
        for word in list.split(',') {
            let mut got: std::collections::BTreeMap<String, usize> = Default::default();
            let mut shown = 0;
            for _ in 0..200 {
                let Some((pts, times)) =
                    common::draw(word, &keys, key_w, sigma, Pace::Corners, &mut rng)
                else {
                    break;
                };
                let cands = decoder.gesture_timed(&pts, times.as_deref(), &keys, key_w);
                let top = cands
                    .first()
                    .map_or("(none)".to_string(), |c| c.word.clone());
                if top != word && shown < 4 {
                    shown += 1;
                    let mine = cands.iter().position(|c| c.word == word);
                    let line: Vec<String> = cands
                        .iter()
                        .take(4)
                        .map(|c| {
                            format!("{} geo {:.2} prior {:.2}", c.word, c.edit, c.cost - c.edit)
                        })
                        .collect();
                    println!("  {word}: {} | {word} at {mine:?}", line.join(" · "));
                }
                *got.entry(top).or_default() += 1;
            }
            let mut v: Vec<_> = got.into_iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            println!("{word}: {v:?}");
        }
        return;
    }
    let pace_name = std::env::var("PACE").unwrap_or_default();
    let pace = match pace_name.as_str() {
        "corners" => Pace::Corners,
        "letters" => Pace::Letters,
        _ => Pace::None,
    };
    println!("{n} words drawn on the phone layout (top 20 k by frequency), pace: {pace_name:?}");
    println!(
        "{:<10} {:>7} {:>7} {:>9}",
        "noise", "top-1", "top-3", "ms/word"
    );
    for sigma in [0.2f32, 0.35, 0.5] {
        let mut rng = Rng(42);
        let (mut top1, mut top3, mut ms) = (0, 0, 0.0f64);
        for word in &words {
            let Some((smooth, times)) = common::draw(word, &keys, key_w, sigma, pace, &mut rng)
            else {
                continue;
            };
            let t = Instant::now();
            let got = decoder.gesture_timed(&smooth, times.as_deref(), &keys, key_w);
            ms += t.elapsed().as_secs_f64() * 1000.0;
            top1 += (got.first().map(|c| c.word.as_str()) == Some(word)) as usize;
            top3 += got.iter().take(3).any(|c| c.word == *word) as usize;
        }
        let pct = |k: usize| 100.0 * k as f32 / n as f32;
        println!(
            "σ={sigma:<8} {:>6.1}% {:>6.1}% {:>9.2}",
            pct(top1),
            pct(top3),
            ms / n as f64
        );
    }
}
