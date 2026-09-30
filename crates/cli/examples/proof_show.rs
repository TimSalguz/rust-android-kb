//! What proofreading sees in a sentence: each mark the graph of the whole
//! sentence puts (before which word, how sure, by which rule), the comma
//! model's log odds for that gap, and each word's link (its head and
//! relation) — to see why a comma goes in or not. Sentences from stdin,
//! one a line.
//!
//!   echo "угу да конечно" | proof_show DICT BIGRAMS LEMMAS SENTENCE_PARSER

use std::io::BufRead;

use kbcore::engine::proof::tokens;
use kbcore::parser::marks_of;
use kbcore::{Config, Engine, Profile};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&a[0], &a[1], cfg)
        .and_then(|e| e.open_lemmas(&a[2]))
        .and_then(|e| e.open_sentence_parser(&a[3]))
        .expect("engine");
    let parser = engine.sentence_parser().expect("a parser of whole sentences");
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("stdin");
        let toks = tokens(&line);
        let words: Vec<(&str, &str)> = toks.iter().map(|t| (t.text, t.before.as_str())).collect();
        println!("{line}");
        let lower: Vec<String> = words.iter().map(|(w, _)| w.to_lowercase()).collect();
        let read: Vec<_> = lower
            .iter()
            .zip(&words)
            .map(|(w, (_, m))| {
                let mut x = engine.parser_word(w).unwrap();
                x.marks = marks_of(m);
                x
            })
            .collect();
        let graph = parser.parse_full(engine.lemma_vectors().unwrap(), &read);
        for (i, (row, &(rel, p))) in graph.heads.iter().zip(&graph.relations).enumerate() {
            let h = (0..row.len()).max_by(|&x, &y| row[x].total_cmp(&row[y])).unwrap_or(0);
            let head = if h == 0 { "ROOT".to_string() } else { lower[h - 1].clone() };
            let kinds: Vec<String> = engine.readings(&lower[i]).iter().map(|r| format!("{r:x}")).collect();
            println!(
                "  {:<12} → {:<12} {:<10} head {:.2} rel {:.2}  readings {}",
                lower[i],
                head,
                parser.relations()[rel],
                row[h],
                p,
                kinds.len()
            );
        }
        match engine.sentence_marks(&words) {
            Some((put, end)) => {
                for m in put {
                    let odds = (m.before > 0)
                        .then(|| {
                            let text = lower[..m.before].join(" ");
                            engine.comma_odds_in(&text, &lower[m.before - 1], &lower[m.before])
                        })
                        .flatten();
                    println!(
                        "  {:?} before «{}»: sure {:.2} ({}), comma odds {}",
                        m.mark,
                        lower.get(m.before).map_or("", String::as_str),
                        m.chance,
                        m.rule,
                        odds.map_or("—".into(), |o| format!("{o:.2}"))
                    );
                }
                println!("  end: {end:?}");
            }
            None => println!("  (not read)"),
        }
        // The comma model on each gap by itself.
        let mut text = String::new();
        let gaps: Vec<String> = (1..lower.len())
            .map(|i| {
                text.push_str(&lower[i - 1]);
                let o = engine.comma_odds_in(&text, &lower[i - 1], &lower[i]);
                text.push(' ');
                format!("{}|{} {}", lower[i - 1], lower[i], o.map_or("—".into(), |o| format!("{o:.1}")))
            })
            .collect();
        println!("  comma odds: {}", gaps.join(", "));
    }
}
