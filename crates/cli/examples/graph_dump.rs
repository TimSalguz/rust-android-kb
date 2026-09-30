//! The graph the keyboard reads a sentence with, typed with no mark at
//! all, beside the teacher's parse — for tools that study it
//! (tools/parts.py). Out, a line a sentence:
//! id, words, our heads, our relations, our tags (part of speech/form),
//! the teacher's heads, the teacher's relations, the marks before each word
//! (tab-separated; lists space-separated, tags `|`-separated).
//!
//!   graph_dump DICT BIGRAMS LEMMAS SENTENCE_PARSER PARSES.tsv > graph.tsv

use kbcore::marks::describe;
use kbcore::parser::Word;
use kbcore::{Config, Engine, Profile};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::balanced().with_profile(Profile::Phone);
    let engine = Engine::open_with_bigrams(&a[0], &a[1], cfg)
        .and_then(|e| e.open_lemmas(&a[2]))
        .and_then(|e| e.open_sentence_parser(&a[3]))
        .expect("engine");
    let parser = engine.sentence_parser().expect("a parser of whole sentences");
    let lemmas = engine.lemma_vectors().expect("lemma vectors");
    for line in std::fs::read_to_string(&a[4]).expect("parses").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let words: Vec<&str> = f[1].split(' ').collect();
        if words.len() > parser.places() || words.iter().any(|w| w.chars().any(|c| c.is_ascii_alphabetic())) {
            continue;
        }
        let read: Option<Vec<Word>> = words.iter().map(|w| engine.parser_word(w)).collect();
        let Some(read) = read else { continue };
        let g = parser.parse_full(lemmas, &read);
        let n = words.len();
        let heads: Vec<usize> = g
            .heads
            .iter()
            .map(|row| (0..row.len()).max_by(|&x, &y| row[x].total_cmp(&row[y])).unwrap_or(0).min(n))
            .collect();
        let rels: Vec<&str> = g.relations.iter().map(|&(k, _)| parser.relations()[k].as_str()).collect();
        let tags: Vec<String> = words
            .iter()
            .zip(&rels)
            .map(|(w, r)| describe(w, &engine.readings(w), r))
            .collect();
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            f[0],
            f[1],
            heads.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(" "),
            rels.join(" "),
            tags.join("|"),
            f[3],
            f[4],
            f[6]
        );
    }
}
