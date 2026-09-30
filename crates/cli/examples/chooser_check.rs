//! Check the chooser's Rust inference against tools/chooser.py's: each line
//! of FILE is `lemma:class …<TAB>lemma<TAB>class<TAB>features<TAB>f` as
//! PyTorch computed it; prints the largest difference.
//!
//!   cargo run --release -p cli --example chooser_check -- chooser.bin lemmas.bin FILE

use kbcore::{Chooser, Lemmas};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let chooser = Chooser::open(&args[0]).expect("chooser");
    let lemmas = Lemmas::open(&args[1]).expect("lemmas");
    let text = std::fs::read_to_string(&args[2]).expect("file");
    let mut worst = 0f32;
    let mut n = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let words: Vec<(u32, u32)> = f[0]
            .split_whitespace()
            .map(|t| {
                let (l, c) = t.split_once(':').unwrap();
                (l.parse().unwrap(), c.parse().unwrap())
            })
            .collect();
        // A lone START is how the script writes an empty sentence.
        let words = if words == [(lemmas.start(), 0)] {
            Vec::new()
        } else {
            words
        };
        let r = chooser.prepare(&lemmas, &chooser.read(&lemmas, &words));
        let feats: Vec<f32> = f[3].split(',').map(|x| x.parse().unwrap()).collect();
        let got = chooser.score(
            &lemmas,
            &r,
            f[1].parse().unwrap(),
            f[2].parse().unwrap(),
            &feats,
        );
        let want: f32 = f[4].parse().unwrap();
        worst = worst.max((got - want).abs());
        n += 1;
    }
    println!(
        "{n} scores, largest difference {worst:.6}, τ {:.4}",
        chooser.tau()
    );
}
