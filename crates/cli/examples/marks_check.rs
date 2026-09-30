//! The marks the graph puts (kbcore::marks) on sentences whose marks are
//! known, the commas taken out first (as typed on a phone): how many of the
//! commas it puts stand in the text, how many of the text's it finds, by how
//! sure the graph is; and the sentences' ends. Set against
//! tools/graph_marks.py on the same parses.
//!
//!   marks_check DICT BIGRAMS LEMMAS PARSER PARSES.tsv

use kbcore::marks::{place, Mark};
use kbcore::parser::marks_of;
use kbcore::{Config, Engine, Profile};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&a[0], &a[1], cfg)
        .and_then(|e| e.open_lemmas(&a[2]))
        .and_then(|e| e.open_parser(&a[3]))
        .expect("engine");
    let (parser, lemmas) = (engine.parser().unwrap(), engine.lemma_vectors().unwrap());
    let levels = [0.0, 0.5, 0.8, 0.9, 0.95];
    let mut at = vec![(0usize, 0usize); levels.len()];
    let (mut gold, mut ends, mut ends_right, mut t) = (0usize, 0usize, 0usize, 0f64);
    for line in std::fs::read_to_string(&a[4]).expect("parses").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 {
            continue;
        }
        let words: Vec<String> = f[1].split(' ').map(String::from).collect();
        let marks: Vec<&str> = f[6].split(' ').collect();
        if words.len() != marks.len() || words.len() > parser.places() {
            continue;
        }
        if words.iter().any(|w| w.chars().any(|c| c.is_ascii_alphabetic())) {
            continue;
        }
        // The commas out, the other marks as written.
        let read: Vec<_> = words
            .iter()
            .zip(&marks)
            .map(|(w, m)| {
                let mut x = engine.parser_word(w).unwrap();
                x.marks = marks_of(&m.replace(',', ""));
                x
            })
            .collect();
        let readings: Vec<Vec<u64>> = words.iter().map(|w| engine.readings(w)).collect();
        let t0 = std::time::Instant::now();
        let graph = parser.parse_full(lemmas, &read);
        let (put, end) = place(&words, &readings, &graph, parser.relations());
        t += t0.elapsed().as_secs_f64();
        gold += marks[1..].iter().filter(|m| m.contains(',')).count();
        for p in put.iter().filter(|p| p.mark == Mark::Comma) {
            let right = marks[p.before].contains(',');
            for (k, &l) in levels.iter().enumerate() {
                if p.chance >= l {
                    at[k].0 += 1;
                    at[k].1 += usize::from(right);
                }
            }
        }
        let want = f[7].chars().find(|c| ".?!".contains(*c)).unwrap_or('.');
        let got = if end == Mark::Question { '?' } else { '.' };
        ends += 1;
        ends_right += usize::from(want == got);
    }
    println!("{ends} sentences, {:.1} ms each; commas in the text {gold}", t * 1e3 / ends as f64);
    for (k, l) in levels.iter().enumerate() {
        let (n, ok) = at[k];
        println!(
            "  sure ≥ {l:.2}: put {n}, right {:.1}%, found {:.1}%",
            100.0 * ok as f64 / n.max(1) as f64,
            100.0 * ok as f64 / gold.max(1) as f64
        );
    }
    println!("sentence ends right {:.1}%", 100.0 * ends_right as f64 / ends as f64);
}
