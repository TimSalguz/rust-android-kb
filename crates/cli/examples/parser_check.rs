//! Check the student parser's Rust inference against PyTorch's: each line
//! of FILE is `words<TAB>i<TAB>chances…` (tools/graph_parser_export.py
//! --check); prints the largest difference.
//!
//!   cargo run --release -p cli --example parser_check -- dict.fst bigrams.fst lemmas.bin parser.bin FILE [OTHER.bin]
//!
//! With OTHER.bin (the same parser stored otherwise — in i8): how often the
//! two agree on each word's likeliest head, and their largest difference.

use kbcore::{Config, Engine, Profile};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&a[0], &a[1], cfg)
        .and_then(|e| e.open_lemmas(&a[2]))
        .and_then(|e| e.open_parser(&a[3]))
        .expect("engine");
    let (parser, lemmas) = (engine.parser().unwrap(), engine.lemma_vectors().unwrap());
    let mut worst = 0f32;
    let mut n = 0;
    let mut last = (String::new(), Vec::new());
    for line in std::fs::read_to_string(&a[4]).expect("file").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] != last.0 {
            let words: Vec<_> = f[0]
                .split(' ')
                .map(|w| engine.parser_word(w).unwrap())
                .collect();
            last = (f[0].to_string(), parser.parse(lemmas, &words));
        }
        let i: usize = f[1].parse().unwrap();
        let want: Vec<f32> = f[2].split(' ').map(|x| x.parse().unwrap()).collect();
        for (g, w) in last.1[i].iter().zip(&want) {
            worst = worst.max((g - w).abs());
            n += 1;
        }
    }
    println!("{n} chances, largest difference {worst:.5}");
    // The sentences read word by word, as the keyboard does (each start
    // read once, the rest from the cache): the same graphs.
    let mut worst = 0f32;
    let mut seen = std::collections::HashSet::new();
    let mut timing = (0f64, 0f64, 0usize);
    for line in std::fs::read_to_string(&a[4]).expect("file").lines() {
        let s = line.split('\t').next().unwrap();
        if !seen.insert(s.to_string()) {
            continue;
        }
        let words: Vec<_> = s.split(' ').map(|w| engine.parser_word(w).unwrap()).collect();
        for k in 1..=words.len() {
            let t = std::time::Instant::now();
            let whole = parser.parse(lemmas, &words[..k]);
            let t1 = std::time::Instant::now();
            let cached = parser.parse_cached(lemmas, &words[..k]);
            timing.0 += t1.duration_since(t).as_secs_f64();
            timing.1 += t1.elapsed().as_secs_f64();
            timing.2 += 1;
            for (a, b) in whole.iter().flatten().zip(cached.heads.iter().flatten()) {
                worst = worst.max((a - b).abs());
            }
        }
    }
    if let Some(other) = a.get(5) {
        let other = kbcore::Parser::open(other).expect("other parser");
        let (mut same, mut rows, mut worst) = (0, 0, 0f32);
        let argmax = |r: &[f32]| (0..r.len()).max_by(|&i, &j| r[i].total_cmp(&r[j])).unwrap();
        for s in &seen {
            let words: Vec<_> = s.split(' ').map(|w| engine.parser_word(w).unwrap()).collect();
            for k in 1..=words.len() {
                let x = parser.parse(lemmas, &words[..k]);
                let y = other.parse_cached(lemmas, &words[..k]);
                let (x, y) = (x.last().unwrap(), y.heads.last().unwrap());
                same += usize::from(argmax(x) == argmax(y));
                rows += 1;
                for (a, b) in x.iter().zip(y) {
                    worst = worst.max((a - b).abs());
                }
            }
        }
        println!(
            "other: the likeliest head the same for {same} of {rows} words as typed ({:.2}%), largest difference {worst:.4}",
            100.0 * same as f64 / rows as f64
        );
    }
    println!(
        "word by word: largest difference {worst:.5}; a word {:.2} ms whole, {:.2} ms read on",
        timing.0 * 1e3 / timing.2 as f64,
        timing.1 * 1e3 / timing.2 as f64
    );
}
