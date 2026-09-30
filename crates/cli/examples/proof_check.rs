//! Proofreading a finished sentence (Engine::sentence_marks) on sentences
//! whose commas are known, the commas taken out first (typed without them):
//! the commas the graph of the whole sentence puts, by how sure it is and
//! by the comma model's log odds for the same gap (with the text typed so
//! far) — how many are right, how many of the text's are found; and the
//! comma model alone at the keyboard's sure level, and the two together.
//!
//!   proof_check DICT BIGRAMS LEMMAS SENTENCE_PARSER PARSES.tsv

use kbcore::marks::Mark;
use kbcore::{Config, Engine, Profile};

const COMMA_SURE: f32 = 2.2;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&a[0], &a[1], cfg)
        .and_then(|e| e.open_lemmas(&a[2]))
        .and_then(|e| e.open_sentence_parser(&a[3]))
        .expect("engine");
    let places = engine.sentence_parser().expect("a parser of whole sentences").places();
    let graph_levels = [0.0f32, 0.5, 0.8, 0.9, 0.95, 0.97];
    let odds_levels = [f32::NEG_INFINITY, -2.0, -1.0, 0.0, 1.0];
    let mut table = vec![vec![(0usize, 0usize); odds_levels.len()]; graph_levels.len()];
    let mut union = table.clone();
    let mut model = (0usize, 0usize);
    let (mut gold, mut sentences, mut t) = (0usize, 0usize, 0f64);
    for line in std::fs::read_to_string(&a[4]).expect("parses").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let words: Vec<&str> = f[1].split(' ').collect();
        let marks: Vec<&str> = f[6].split(' ').collect();
        if words.len() != marks.len() || words.len() > places {
            continue;
        }
        if words.iter().any(|w| w.chars().any(|c| c.is_ascii_alphabetic())) {
            continue;
        }
        let typed: Vec<String> = marks.iter().map(|m| m.replace([',', '_'], "")).collect();
        let pairs: Vec<(&str, &str)> = words.iter().zip(&typed).map(|(w, m)| (*w, m.as_str())).collect();
        let t0 = std::time::Instant::now();
        let Some((put, _)) = engine.sentence_marks(&pairs) else {
            continue;
        };
        t += t0.elapsed().as_secs_f64();
        sentences += 1;
        let has = |i: usize| marks[i].contains(',');
        gold += (1..words.len()).filter(|&i| has(i)).count();
        // The comma model's odds for each gap, with the text typed so far.
        let mut odds = vec![f32::NEG_INFINITY; words.len()];
        let mut text = String::new();
        for i in 1..words.len() {
            text.push_str(words[i - 1]);
            if let Some(o) = engine.comma_odds_in(&text, words[i - 1], words[i]) {
                odds[i] = o;
            }
            text.push(' ');
        }
        let mut graph = vec![None; words.len()];
        for p in put.iter().filter(|p| p.mark == Mark::Comma && p.before > 0) {
            graph[p.before] = Some(p.chance);
        }
        for i in 1..words.len() {
            let right = usize::from(has(i));
            let sure = odds[i] >= COMMA_SURE;
            if sure {
                model.0 += 1;
                model.1 += right;
            }
            for (k, &g) in graph_levels.iter().enumerate() {
                let by_graph = graph[i].is_some_and(|c| c >= g);
                for (j, &m) in odds_levels.iter().enumerate() {
                    let put = by_graph && odds[i] >= m;
                    if put {
                        table[k][j].0 += 1;
                        table[k][j].1 += right;
                    }
                    if put || sure {
                        union[k][j].0 += 1;
                        union[k][j].1 += right;
                    }
                }
            }
        }
    }
    let pct = |(n, ok): (usize, usize)| {
        format!(
            "{n:>6} {:>5.1}% {:>5.1}%",
            100.0 * ok as f64 / n.max(1) as f64,
            100.0 * ok as f64 / gold.max(1) as f64
        )
    };
    println!(
        "{sentences} sentences, {:.1} ms each; commas in the text {gold}",
        t * 1e3 / sentences.max(1) as f64
    );
    println!("the comma model alone (odds ≥ {COMMA_SURE}): {}", pct(model));
    for (title, table) in [
        ("the graph (sure ≥ g) and the model's odds ≥ m", &table),
        ("the same, or the model sure (as the keyboard puts them)", &union),
    ] {
        println!("{title} — put, right, found:");
        print!("  g \\ m ");
        for m in odds_levels {
            print!("  {:>20}", if m.is_finite() { format!("{m}") } else { "any".into() });
        }
        println!();
        for (k, g) in graph_levels.iter().enumerate() {
            print!("  {g:>5.2}");
            for cell in &table[k] {
                print!("  {}", pct(*cell));
            }
            println!();
        }
    }
}
