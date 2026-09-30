//! The grammar's roles of the words before each word of tools/chooser_data.py's
//! data (`Engine::roles`), for tools/chooser.py `--graph`: one line per
//! line of FILE, a role per word of the sentence so far (at most 12, 0:
//! outside the phrase).
//!
//!   cargo run --release -p cli --example roles -- dict.fst bigrams.fst FILE > roles.txt

use std::io::{BufRead, Write};

use kbcore::{Config, Engine, Profile};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&args[0], &args[1], cfg).expect("dictionary");
    let file = std::fs::File::open(&args[2]).expect("file");
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let before = line.split('\t').nth(1).unwrap_or("");
        let tail = before.rsplit(['.', '!', '?']).next().unwrap_or("");
        let words: Vec<String> = tail.split_whitespace().map(str::to_lowercase).collect();
        let sentence = &words[words.len().saturating_sub(12)..];
        let phrase = &words[words.len().saturating_sub(5)..];
        let ctx = engine.context(words.last().map(String::as_str), phrase, sentence);
        let roles = engine.roles(&ctx);
        let outside = sentence.len() - phrase.len();
        let codes: Vec<String> = (0..sentence.len())
            .map(|k| k.checked_sub(outside).map_or(0, |i| roles[i]).to_string())
            .collect();
        writeln!(out, "{}", codes.join(" ")).ok();
    }
}
