//! The keyboard's comma model on sentences whose commas are known: for each
//! gap between words, its log odds of a comma with the text typed so far
//! (no commas typed) — to set against the commas the graph's rules put
//! (tools/graph_marks.py).
//!
//!   comma_check DICT BIGRAMS PARSES.tsv > odds.tsv
//!
//! PARSES.tsv: data/parse_m's format (words in column 2, the marks before
//! each word in column 7). Out: id<TAB>word index (1-based, the word after
//! the gap)<TAB>log odds<TAB>whether the text has a comma there.

use kbcore::{Config, Engine, Profile};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&a[0], &a[1], cfg).expect("engine");
    for line in std::fs::read_to_string(&a[2]).expect("parses").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let words: Vec<&str> = f[1].split(' ').collect();
        let marks: Vec<&str> = f[6].split(' ').collect();
        if words.len() != marks.len() {
            continue;
        }
        let mut text = String::new();
        for i in 1..words.len() {
            text.push_str(words[i - 1]);
            if let Some(odds) = engine.comma_odds_in(&text, words[i - 1], words[i]) {
                println!("{}\t{}\t{odds:.3}\t{}", f[0], i + 1, u8::from(marks[i].contains(',')));
            }
            text.push(' ');
        }
    }
}
