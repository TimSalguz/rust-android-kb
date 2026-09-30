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
/// A comma the model is against no more than this goes with the other of its pair.
const RESCUE: f32 = -2.5;

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
    let mut paired = table.clone();
    // The comma head: by itself at each level, and with the rules' policy.
    let head_levels = [0.5f32, 0.6, 0.7, 0.8, 0.9, 0.95];
    let mut head_alone = vec![(0usize, 0usize); head_levels.len()];
    let mut head_with = vec![(0usize, 0usize); head_levels.len()];
    // The rules and the head both for it: a rule's comma (≥ 0.8) the head
    // gives at least t, whatever the model says.
    let mut both_say = vec![(0usize, 0usize); head_levels.len()];
    // The head's say over the rules instead of the comma model's: a comma
    // when the head is sure (≥ h), or a rule is (≥ 0.8) and the head not
    // against it (≥ v).
    let (hs, vs) = ([0.6f32, 0.7, 0.8], [0.1f32, 0.2, 0.3, 0.4]);
    let mut headed = vec![vec![(0usize, 0usize); vs.len()]; hs.len()];
    // h 0.7, v 0.3, and a phrase's other comma along (the head ≥ 0.1 for it).
    let mut headed_pairs = (0usize, 0usize);
    let mut set_phrases = (0usize, 0usize);
    let mut model = (0usize, 0usize);
    // Where the graph is sure (≥ 0.95) and the model mildly against (odds in
    // [-3, 0)): how right each rule is.
    let mut band: std::collections::BTreeMap<(&'static str, &'static str), (usize, usize)> = Default::default();
    // Where the graph is less sure (0.6…0.8, or 0.9…0.95), by rule and by
    // whether the model is for (odds ≥ 0) or a little against (-1…0).
    let mut less: std::collections::BTreeMap<(&'static str, &'static str), (usize, usize)> = Default::default();
    // Each comma of the text the keyboard doesn't put: why (no rule put
    // one there; the graph under 0.8; the model against it) and how the word
    // after it hangs in the teacher's parse.
    let mut lost: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let mut put_n = 0usize;
    let (mut gold, mut sentences, mut t) = (0usize, 0usize, 0f64);
    for line in std::fs::read_to_string(&a[4]).expect("parses").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let words: Vec<&str> = f[1].split(' ').collect();
        let marks: Vec<&str> = f[6].split(' ').collect();
        let gold_rel: Vec<&str> = f[4].split(' ').map(|r| r.split(':').next().unwrap_or(r)).collect();
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
        let mut rule = vec![""; words.len()];
        let mut learned = vec![-1f32; words.len()];
        let mut graph = vec![None; words.len()];
        let mut by = vec![None; words.len()];
        for p in put.iter().filter(|p| p.mark == Mark::Comma && p.before > 0) {
            if p.rule == "learned" {
                // The comma head's own: only in its tables.
                learned[p.before] = p.learned;
                continue;
            }
            graph[p.before] = Some(p.chance);
            by[p.before] = Some(p.by);
            rule[p.before] = p.rule;
            learned[p.before] = p.learned;
        }
        // The keyboard's policy (h 0.7, v 0.3) on each gap, for the pairs.
        let accepted: Vec<bool> = (0..words.len())
            .map(|x| {
                let veto = if rule[x] == "set aside" { 0.05 } else { 0.3 };
                learned[x] >= 0.7 || graph[x].is_some_and(|c| c >= 0.8) && learned[x] >= veto
            })
            .collect();
        // The set phrases' commas the keyboard puts: how right.
        for x in 1..words.len() {
            if rule[x] == "set aside" && accepted[x] {
                set_phrases.0 += 1;
                set_phrases.1 += usize::from(has(x));
            }
        }
        for i in 1..words.len() {
            let right = usize::from(has(i));
            let along = graph[i].is_some_and(|c| c >= 0.8)
                && ["gerund", "participle after", "relative", "said"].contains(&rule[i])
                && learned[i] >= 0.1
                && (1..words.len()).any(|x| x != i && by[x].is_some() && by[x] == by[i] && accepted[x]);
            if accepted[i] || along {
                headed_pairs.0 += 1;
                headed_pairs.1 += right;
            }
            if let Some(c) = graph[i] {
                let key = if (0.6..0.8).contains(&c) && (0.0..1.0).contains(&odds[i]) {
                    Some("0.6…0.8, odds 0…1")
                } else if (0.6..0.8).contains(&c) && odds[i] >= 1.0 {
                    Some("0.6…0.8, odds ≥ 1")
                } else if (0.9..0.95).contains(&c) && (-1.0..0.0).contains(&odds[i]) {
                    Some("0.9…0.95, odds -1…0")
                } else {
                    None
                };
                if let Some(key) = key {
                    let t = less.entry((rule[i], key)).or_default();
                    t.0 += 1;
                    t.1 += right;
                }
            }
            if graph[i].is_some_and(|c| c >= 0.95) && odds[i] < 0.0 && odds[i] >= -3.0 {
                // By rule and by how much the model is against: -1…0, -2…-1, -3…-2.
                let bin = ["-1…0", "-2…-1", "-3…-2"][((-odds[i]).floor() as usize).min(2)];
                let t = band.entry((rule[i], bin)).or_default();
                t.0 += 1;
                t.1 += right;
            }
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
                    // The two commas around a phrase go together: one the
                    // model lets through takes the other along.
                    let partner = by_graph
                        && odds[i] >= RESCUE
                        && (1..words.len()).any(|x| {
                            x != i
                                && by[x].is_some()
                                && by[x] == by[i]
                                && graph[x].is_some_and(|c| c >= g)
                                && odds[x] >= m
                        });
                    let floors = [("subordinate first", -3.0), ("relative", -2.0), ("said", -2.0),
                                  ("gerund", -2.0), ("participle after", -1.0), ("ccomp", -1.0),
                                  ("parenthetical", -1.0)];
                    let less = [("subordinate", 0.6, 1.0), ("subordinate first", 0.6, 1.0),
                                ("address", 0.6, 0.0), ("address", 0.9, -1.0)];
                    let trusted = graph[i].is_some_and(|c| c >= 0.8) && learned[i] >= 0.5
                        || graph[i].is_some_and(|c| c >= 0.95)
                        && floors.iter().any(|&(r, f)| r == rule[i] && odds[i] >= f)
                        || graph[i].is_some_and(|c| {
                            less.iter().any(|&(r, gg, o)| r == rule[i] && c >= gg && odds[i] >= o)
                        });
                    if put || partner || trusted || sure {
                        paired[k][j].0 += 1;
                        paired[k][j].1 += right;
                    }
                    if g == 0.8 && m == 0.0 {
                        for (a, &h) in hs.iter().enumerate() {
                            for (b, &v) in vs.iter().enumerate() {
                                let by_rule = graph[i].is_some_and(|c| c >= 0.8) && learned[i] >= v;
                                if learned[i] >= h || by_rule {
                                    headed[a][b].0 += 1;
                                    headed[a][b].1 += right;
                                }
                            }
                        }
                    }
                    if g == 0.8 && m == 0.0 {
                        let policy = (put || partner || trusted || sure) && rule[i] != "learned";
                        for (h, &t) in [0.3f32, 0.4, 0.5, 0.6, 0.7, 0.8].iter().enumerate() {
                            if graph[i].is_some_and(|c| c >= 0.8) && learned[i] >= t {
                                both_say[h].0 += 1;
                                both_say[h].1 += right;
                            }
                        }
                        for (h, &t) in head_levels.iter().enumerate() {
                            let by_head = learned[i] >= t;
                            if by_head {
                                head_alone[h].0 += 1;
                                head_alone[h].1 += right;
                            }
                            if by_head || policy {
                                head_with[h].0 += 1;
                                head_with[h].1 += right;
                            }
                        }
                    }
                    if g == 0.8 && m == 0.0 {
                        if put || partner || trusted || sure {
                            put_n += right;
                        } else if right == 1 {
                            let why = match graph[i] {
                                None => "no rule",
                                Some(c) if c < 0.8 => "graph < 0.8",
                                Some(_) => "model against",
                            };
                            let rel = gold_rel.get(i).copied().unwrap_or("?");
                            *lost.entry((why.to_string(), rel.to_string())).or_default() += 1;
                        }
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
    if head_alone.iter().any(|c| c.0 > 0) {
        println!("the comma head (≥ t) — by itself, and with the rules' policy — put, right, found:");
        for (h, t) in head_levels.iter().enumerate() {
            println!("  {t:>5.2}  {}   {}", pct(head_alone[h]), pct(head_with[h]));
        }
    }
    println!("the head sure (≥ h), or a rule (≥ 0.8) the head isn't against (≥ v) — put, right, found:");
    for (a, h) in hs.iter().enumerate() {
        print!("  h {h:.1}");
        for (b, v) in vs.iter().enumerate() {
            print!("   v {v:.1}: {}", pct(headed[a][b]));
        }
        println!();
    }
    println!("h 0.7, v 0.3 and a phrase's other comma along (head ≥ 0.1): {}", pct(headed_pairs));
    println!("the set asides' commas put (veto 0.05): {}", pct(set_phrases));
    println!("a rule's comma (≥ 0.8) the head gives at least t (0.3, 0.4, …, 0.8):");
    for (h, t) in [0.3f32, 0.4, 0.5, 0.6, 0.7, 0.8].iter().enumerate() {
        println!("  {t:>5.2}  {}", pct(both_say[h]));
    }
    println!("commas of the text not put ({} put), by why and the next word's relation:", put_n);
    let mut by_why: std::collections::BTreeMap<&str, usize> = Default::default();
    for ((w, _), n) in &lost {
        *by_why.entry(w.as_str()).or_default() += n;
    }
    for (w, n) in &by_why {
        let mut rels: Vec<(&String, &usize)> = lost.iter().filter(|((ww, _), _)| ww == w).map(|((_, r), n)| (r, n)).collect();
        rels.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = rels.iter().take(8).map(|(r, n)| format!("{r} {n}")).collect();
        println!("  {w:<14} {n:>5}: {}", top.join(", "));
    }
    println!("the graph less sure, by rule:");
    for ((r, k), (n, ok)) in &less {
        println!("  {r:<22} {k:<20} {n:>5} {:>5.1}%", 100.0 * *ok as f64 / (*n).max(1) as f64);
    }
    println!("the graph sure (≥ 0.95), the model against (odds -3…0), by rule and odds:");
    for ((r, bin), (n, ok)) in &band {
        println!("  {r:<22} {bin:<6} {n:>5} {:>5.1}%", 100.0 * *ok as f64 / (*n).max(1) as f64);
    }
    for (title, table) in [
        ("the graph (sure ≥ g) and the model's odds ≥ m", &table),
        ("the same, or the model sure (as the keyboard puts them)", &union),
        ("the same, or the other comma of its pair let through (odds ≥ -2.5)", &paired),
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
