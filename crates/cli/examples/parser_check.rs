//! Check the student parser's Rust inference against PyTorch's: each line
//! of FILE is `words<TAB>i<TAB>chances…` (tools/graph_parser_export.py
//! --check); prints the largest difference.
//!
//!   cargo run --release -p cli --example parser_check -- dict.fst bigrams.fst lemmas.bin parser.bin FILE

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
}
