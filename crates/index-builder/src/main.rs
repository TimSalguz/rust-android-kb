//! Builds the mmap'able FST dictionary blob from a word list.
//!
//! Usage: `index-builder [WORDS] [OUT_FST] [FREQ] [--quantum Q] [--grammar CLASSES WORD_READINGS]`
//!   WORDS    one word per line, optionally `word<TAB>count`   (default: data/lexicon.tsv)
//!   OUT_FST  output blob                                        (default: dict.fst)
//!   FREQ     optional `word count` list; overrides inline counts
//!   --quantum  the prior in steps of Q thousandths of a nat (fewer distinct
//!              values: more word tails shared)
//!   --priors   WORD_PRIORS.tsv: `word<TAB>−ln P × 1000` given (tools/factor_priors.py),
//!              for the words it lists, instead of the counts
//!   --ranked   the automaton gives each word's rank (and grammar id), the
//!              priors go beside it, OUT_FST with `.bin` (`kbcore::store`)
//!   --grammar  CLASSES.tsv WORD_READINGS.tsv (tools/build_classes.py): each
//!              word's grammar id — its (class, reading set) pair — in its
//!              value below the prior (`kbcore::dict`); the context model
//!              then takes `--grammar-table` with the same two files
//!
//!        `index-builder --bigrams PAIRS.tsv OUT_FST [--endings ENDINGS.tsv]
//!               [--classes WORDS.tsv CLASS_TAGS.tsv TAG_PAIRS.tsv]
//!               [--readings WORD_READINGS.tsv READINGS.tsv]
//!               [--grammar-table CLASSES.tsv WORD_READINGS.tsv]
//!               [--rules CONFUSIONS.tsv RULES.tsv]`
//!   --classes    grammar (tools/build_classes.py): word → class under `0, 3, word`,
//!                class → its readings' tags (a byte each, packed) under `0, 5,
//!                class (2 bytes)`, tag pair → (PMI+8)×1000 under `0, 4, tag, tag`
//!   --readings   WORD_READINGS.tsv READINGS.tsv: each word's reading set (all
//!                its readings, for the phrase grammar) in the bits of its
//!                `0, 3, word` value above the class; each set's readings'
//!                grammemes (kbcore::gram) under `0, 9, set (2 bytes), index`
//!   --frames     FRAMES.tsv (tools/build_frames.py): each governing word's
//!                frame under `0, 11, word`, the weights after adjectives and
//!                a subject under `0, 12`, a verb's clause frame (after a noun
//!                phrase) under `0, 16, verb`, a preposition's frame by the
//!                sense class of the verb it hangs on under `0, 17, class (2
//!                bytes), preposition`, what kind of word follows a subject
//!                under `0, 18`, the case after «X и» under `0, 19`, a
//!                preposition's frame by the word right before it under `0,
//!                20, word, 0, preposition` («что за») — log
//!                ratios, a byte each (0.05 nat steps around 128)
//!   --topics     TOPIC_WORDS.tsv (tools/build_topics.py): each word's sense
//!                class under `0, 13, word`
//!   --topic-pairs  TOPIC_PAIRS.tsv (tools/build_topics.py): how much likelier
//!                two sense classes share a sentence, a byte (0.05 nat steps
//!                around 128) under `0, 14, class, class` (2 bytes each, the
//!                smaller first)
//!   --lemmas     IDS.tsv (tools/lemma_export.py): each word's lemma id under
//!                `0, 25, word` — the row of its lemma's vectors (kbcore::lemmas)
//!   --yo         YO.tsv (tools/yo_words.py): words written without their ё
//!                (еще → ещё) under `0, 24, word`, the ё's letter positions
//!                as a bit mask
//!   --commas     COMMAS.tsv (tools/build_commas.py): the log odds of a comma
//!                between two words, a byte each (0.05 nat steps around 128):
//!                anywhere `0, 15, 0`, before a word `0, 15, 1, word`, after
//!                it `0, 15, 2, word`, a pair of its own `0, 15, 3, a, 0, b`, by
//!                the words' kinds and the sentence's state `0, 15, 4, feature`
//!   --grammar-table  the dictionary holds the words' grammar ids: each id's
//!                (class, reading set) under `0, 10, id (2 bytes)`, and no
//!                word under `0, 3`
//!   --rules      confusion pairs and their rules (docs/rules-format.md), under
//!                `0, 7, word, 0, other` and `0, 6, pair id (3 bytes), feature`
//!   PAIRS    `previous<TAB>word<TAB>nats×1000` (tools/build_bigrams.py): the
//!            context model, keyed `previous\0word`
//!
//!        `index-builder --casing CASING.tsv OUT_FST`
//!   CASING   `word<TAB>1|2` (tools/proper_nouns.py): words always written
//!            Capitalized (1) or in CAPITALS (2)
//!
//! Each word is remapped to the single-byte alphabet (see `kbcore::alphabet`);
//! words containing unsupported characters are dropped. The FST value stores a
//! quantized `−log P(word)` prior (0 for every word when no counts are known).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter};
use std::process::ExitCode;

use fst::MapBuilder;
use kbcore::{alphabet, DictFormat};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("--bigrams") {
        args.next();
        let (Some(pairs), Some(out)) = (args.next(), args.next()) else {
            eprintln!(
                "usage: index-builder --bigrams PAIRS.tsv OUT_FST [--endings F] \
                 [--classes WORDS TAGS TAG_PAIRS] [--readings WORDS SETS] [--rules CONFUSIONS RULES]"
            );
            return ExitCode::FAILURE;
        };
        let (mut endings, mut classes, mut rules, mut readings) = (None, None, None, None);
        let mut table = None;
        let mut frames = None;
        let mut topic_pairs = None;
        let mut topics = None;
        let mut lemmas = None;
        let mut commas = None;
        let mut yo = None;
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--endings" => endings = args.next(),
                "--classes" => {
                    classes = match (args.next(), args.next(), args.next()) {
                        (Some(a), Some(b), Some(c)) => Some((a, b, c)),
                        _ => None,
                    }
                }
                "--rules" => rules = args.next().zip(args.next()),
                "--readings" => readings = args.next().zip(args.next()),
                "--grammar-table" => table = args.next().zip(args.next()),
                "--frames" => frames = args.next(),
                "--topic-pairs" => topic_pairs = args.next(),
                "--topics" => topics = args.next(),
                "--lemmas" => lemmas = args.next(),
                "--commas" => commas = args.next(),
                "--yo" => yo = args.next(),
                other => {
                    eprintln!("unknown option {other}");
                    return ExitCode::FAILURE;
                }
            }
        }
        return match build_bigrams(
            &pairs,
            &out,
            endings.as_deref(),
            classes,
            readings,
            table,
            frames,
            (topics, topic_pairs, lemmas),
            (commas, yo),
            rules,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.peek().map(String::as_str) == Some("--casing") {
        args.next();
        let (Some(list), Some(out)) = (args.next(), args.next()) else {
            eprintln!("usage: index-builder --casing CASING.tsv OUT_FST [OFFENSIVE.txt]");
            return ExitCode::FAILURE;
        };
        let offensive = args.next();
        return match build_casing(&list, &out, offensive.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let mut positional: Vec<String> = Vec::new();
    let mut format = DictFormat::PLAIN;
    let mut grammar: Option<(String, String)> = None;
    let mut priors: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--quantum" => match args.next().and_then(|q| q.parse::<u32>().ok()) {
                Some(q) if q > 0 && q < 0x10000 => format.quantum = q,
                _ => {
                    eprintln!("--quantum wants 1…65535");
                    return ExitCode::FAILURE;
                }
            },
            "--grammar" => grammar = args.next().zip(args.next()),
            "--priors" => priors = args.next(),
            "--ranked" => format.ranked = true,
            _ => positional.push(a),
        }
    }
    let mut positional = positional.into_iter();
    let words_path = positional
        .next()
        .unwrap_or_else(|| "data/lexicon.tsv".into());
    let out_path = positional.next().unwrap_or_else(|| "dict.fst".into());
    let freq_path = positional.next();

    match run(
        priors.as_deref(),
        &words_path,
        &out_path,
        freq_path.as_deref(),
        format,
        grammar,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Each word's grammar id — its (class, reading set) pair, numbered from the
/// commonest (then by the pair) — and each id's pair as the context model
/// keeps it (`class | set << 16`, at id − 1).
fn grammar_ids(classes: &str, readings: &str) -> io::Result<(HashMap<String, u64>, Vec<u64>)> {
    let class = numbered(classes)?;
    let set = numbered(readings)?;
    let mut words: HashMap<String, u64> = HashMap::new();
    for (w, &c) in &class {
        words.insert(w.clone(), c | set.get(w).copied().unwrap_or(0) << 16);
    }
    for (w, &s) in &set {
        words.entry(w.clone()).or_insert(s << 16);
    }
    let mut count: HashMap<u64, u64> = HashMap::new();
    for &p in words.values() {
        *count.entry(p).or_default() += 1;
    }
    let mut pairs: Vec<(u64, u64)> = count.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let id: HashMap<u64, u64> = pairs
        .iter()
        .enumerate()
        .map(|(i, &(p, _))| (p, i as u64 + 1))
        .collect();
    let words = words.into_iter().map(|(w, p)| (w, id[&p])).collect();
    Ok((words, pairs.into_iter().map(|(p, _)| p).collect()))
}

/// `word<TAB>number` lines.
fn numbered(path: &str) -> io::Result<HashMap<String, u64>> {
    let mut map = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if let Some((w, n)) = line.split_once('\t') {
            if let Ok(n) = n.trim().parse::<u64>() {
                map.insert(w.to_string(), n);
            }
        }
    }
    Ok(map)
}

fn run(
    priors: Option<&str>,
    words_path: &str,
    out_path: &str,
    freq_path: Option<&str>,
    mut format: DictFormat,
    grammar: Option<(String, String)>,
) -> io::Result<()> {
    let mut words: Vec<String> = Vec::new();
    let mut inline: HashMap<String, u64> = HashMap::new();
    for line in BufReader::new(File::open(words_path)?).lines() {
        let line = line?;
        let mut it = line.split('\t');
        let word = it.next().unwrap_or("").trim();
        if word.is_empty() {
            continue;
        }
        if let Some(count) = it.next().and_then(|c| c.trim().parse::<u64>().ok()) {
            inline.insert(word.to_string(), count);
        }
        words.push(word.to_string());
    }

    let freqs = match freq_path {
        Some(p) => load_freqs(p)?,
        None => inline,
    };
    let total: f64 = freqs.values().map(|&c| c as f64).sum::<f64>().max(1.0);
    if !freqs.is_empty() {
        println!("{} frequencies (total count {total:.0})", freqs.len());
    }

    let given = match priors {
        Some(p) => numbered(p)?,
        None => HashMap::new(),
    };
    // The prior of a word the counts never saw: the floor the rarest stand on.
    if !freqs.is_empty() {
        format.floor = (((total.ln() - 0.5f64.ln()) * 10.0).round() as u32).min(0xFFFF);
    }
    let ids = match &grammar {
        Some((c, s)) => {
            let (ids, table) = grammar_ids(c, s)?;
            format.shift = 64 - (table.len() as u64).leading_zeros();
            ids
        }
        None => HashMap::new(),
    };
    // (key, prior, grammar id)
    let mut raw: Vec<(Vec<u8>, u64, u64)> = Vec::with_capacity(words.len());
    let mut dropped = 0u64;
    for word in &words {
        let g = ids.get(word).copied().unwrap_or(0);
        match remap(word) {
            Some(key) => {
                let prior = match given.get(word) {
                    Some(&p) => p,
                    None => prior_value(word, &freqs, total),
                };
                raw.push((key, prior, g))
            }
            None => dropped += 1,
        }
    }

    // FST requires strictly increasing, unique keys.
    raw.sort_by(|a, b| a.0.cmp(&b.0));
    raw.dedup_by(|a, b| a.0 == b.0);
    let q = format.quantum as u64;
    let entries: Vec<(Vec<u8>, u64)> = if format.ranked {
        // The priors by rank, beside the automaton.
        let steps: Vec<u16> = raw
            .iter()
            .map(|(_, p, _)| ((p + q / 2) / q).min(u16::MAX as u64) as u16)
            .collect();
        // The step most words share (the rarest: seen once or never) is not
        // stored.
        let mut count: HashMap<u16, usize> = HashMap::new();
        for &s in &steps {
            *count.entry(s).or_default() += 1;
        }
        let floor = count
            .into_iter()
            .max_by_key(|&(s, c)| (c, s))
            .map_or(0, |(s, _)| s);
        let bin = std::path::Path::new(out_path).with_extension("bin");
        let bin = bin.to_string_lossy().into_owned();
        std::fs::write(staged(&bin), kbcore::store::build(&steps, q as u32, floor))?;
        std::fs::rename(staged(&bin), &bin)?;
        println!(
            "wrote {bin}: priors of {} words by rank, {:.2} MB",
            steps.len(),
            std::fs::metadata(&bin)?.len() as f64 / 1_048_576.0
        );
        raw.into_iter()
            .enumerate()
            .map(|(r, (k, _, g))| (k, format.ranked_value(r as u64, g)))
            .collect()
    } else {
        raw.into_iter()
            .map(|(k, p, g)| (k, format.value(p, g)))
            .collect()
    };

    let kept = entries.len();
    let out = BufWriter::new(File::create(staged(out_path))?);
    let mut builder =
        fst::raw::Builder::new_type(out, format.fst_type()).map_err(io::Error::other)?;
    for (key, val) in &entries {
        builder.insert(key, *val).map_err(io::Error::other)?;
    }
    builder.finish().map_err(io::Error::other)?;
    std::fs::rename(staged(out_path), out_path)?;

    let size = std::fs::metadata(out_path)?.len();
    println!(
        "wrote {out_path}: {kept} words{}, {dropped} dropped, {:.1} MB",
        if grammar.is_some() {
            format!(
                " ({} with a grammar id of {} bits)",
                ids.len(),
                format.shift
            )
        } else {
            String::new()
        },
        size as f64 / 1_048_576.0
    );
    Ok(())
}

/// Casing flags per word: 1 always Capitalized, 2 ALL CAPS (the first
/// line of a word wins), and 4 obscene (a word per line of `offensive`).
fn build_casing(list_path: &str, out_path: &str, offensive: Option<&str>) -> io::Result<()> {
    let mut flags: std::collections::BTreeMap<Vec<u8>, u64> = std::collections::BTreeMap::new();
    for line in BufReader::new(File::open(list_path)?).lines() {
        let line = line?;
        let mut it = line.split('\t');
        let (Some(w), Some(v)) = (it.next(), it.next()) else {
            continue;
        };
        if let (Some(key), Ok(v)) = (remap(w), v.trim().parse::<u64>()) {
            flags.entry(key).or_insert(v & 3);
        }
    }
    let mut obscene = 0;
    if let Some(path) = offensive {
        for line in BufReader::new(File::open(path)?).lines() {
            if let Some(key) = remap(line?.trim()) {
                *flags.entry(key).or_insert(0) |= 4;
                obscene += 1;
            }
        }
    }
    let entries: Vec<(Vec<u8>, u64)> = flags.into_iter().collect();
    let mut builder = MapBuilder::new(BufWriter::new(File::create(staged(out_path))?))
        .map_err(io::Error::other)?;
    for (key, v) in &entries {
        builder.insert(key, *v).map_err(io::Error::other)?;
    }
    builder.finish().map_err(io::Error::other)?;
    std::fs::rename(staged(out_path), out_path)?;
    let size = std::fs::metadata(out_path)?.len();
    println!(
        "wrote {out_path}: {} words ({obscene} obscene), {:.1} MB",
        entries.len(),
        size as f64 / 1_048_576.0
    );
    Ok(())
}

/// A class of the ending model, `=word` or `*ending` → key bytes (see
/// `kbcore::engine::ending_key`).
fn ending_class(c: &str) -> Option<Vec<u8>> {
    let (kind, rest) = match c.chars().next()? {
        '=' => (1u8, &c[1..]),
        '*' => (2u8, &c['*'.len_utf8()..]),
        _ => return None,
    };
    let mut key = vec![kind];
    key.extend(remap(rest)?);
    Some(key)
}

#[allow(clippy::too_many_arguments)]
fn build_bigrams(
    pairs_path: &str,
    out_path: &str,
    endings: Option<&str>,
    classes: Option<(String, String, String)>,
    readings: Option<(String, String)>,
    table: Option<(String, String)>,
    frames: Option<String>,
    (topics, topic_pairs, lemmas): (Option<String>, Option<String>, Option<String>),
    (commas, yo): (Option<String>, Option<String>),
    rules: Option<(String, String)>,
) -> io::Result<()> {
    let mut entries: Vec<(Vec<u8>, u64)> = Vec::new();
    let mut dropped = 0u64;
    for line in BufReader::new(File::open(pairs_path)?).lines() {
        let line = line?;
        let mut it = line.split('\t');
        let (Some(a), Some(b), Some(v)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let (Some(a), Some(b), Ok(v)) = (remap(a), remap(b), v.trim().parse::<u64>()) else {
            dropped += 1;
            continue;
        };
        let mut key = a;
        key.push(0);
        key.extend(b);
        entries.push((key, v));
    }
    let pairs = entries.len();
    if let Some(path) = endings {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let mut it = line.split('\t');
            let (Some(a), Some(b), Some(v)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let (Some(a), Some(b), Ok(v)) = (ending_class(a), ending_class(b), v.trim().parse())
            else {
                dropped += 1;
                continue;
            };
            let mut key = vec![0];
            key.extend(a);
            key.push(0);
            key.extend(b);
            entries.push((key, v));
        }
    }
    let with_endings = entries.len();
    // Each word's reading set, and each set's readings.
    // Words written without their ё (tools/yo_words.py): which of their е
    // are ё, as a mask of letter positions.
    if let Some(path) = &yo {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let Some((e, y)) = line.split_once('\t') else {
                continue;
            };
            let mask = y
                .chars()
                .enumerate()
                .filter(|(i, c)| *c == 'ё' && *i < 64)
                .fold(0u64, |m, (i, _)| m | 1 << i);
            if let (Some(k), true) = (remap(e), mask != 0) {
                entries.push(([vec![0, 24], k].concat(), mask));
            }
        }
    }
    // Where commas go.
    if let Some(path) = &commas {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let Some((k, v)) = line.split_once('\t') else {
                continue;
            };
            let Ok(v) = v.trim().parse::<f32>() else {
                continue;
            };
            let key = if k == "@" {
                Some(vec![0, 15, 0])
            } else if let Some(feature) = k.strip_prefix('#') {
                // By the words' kinds (a gerund, a finite verb…), and the
                // sentence's state (an open phrase, an «и» before).
                Some([vec![0, 15, 4], feature.as_bytes().to_vec()].concat())
            } else if let Some(w) = k.strip_prefix('>') {
                remap(w).map(|w| [vec![0, 15, 1], w].concat())
            } else if let Some(w) = k.strip_prefix('<') {
                remap(w).map(|w| [vec![0, 15, 2], w].concat())
            } else if let Some((a, b)) = k.split_once(' ') {
                remap(a)
                    .zip(remap(b))
                    .map(|(a, b)| [vec![0, 15, 3], a, vec![0], b].concat())
            } else {
                None
            };
            if let Some(key) = key {
                let byte = ((v / 0.05).round() + 128.0).clamp(0.0, 255.0) as u64;
                entries.push((key, byte));
            }
        }
    }
    // Each word's sense class, and which classes go together.
    if let Some(path) = &topics {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let Some((w, c)) = line.split_once('\t') else {
                continue;
            };
            if let (Some(ids), Ok(c)) = (remap(w), c.trim().parse::<u64>()) {
                entries.push(([vec![0, 13], ids].concat(), c));
            }
        }
    }
    // Each word's lemma: the row of its lemma's vectors.
    if let Some(path) = &lemmas {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let Some((w, id)) = line.split_once('\t') else {
                continue;
            };
            if let (Some(k), Ok(id)) = (remap(w), id.trim().parse::<u64>()) {
                entries.push(([vec![0, 25], k].concat(), id));
            }
        }
    }
    if let Some(path) = &topic_pairs {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let f: Vec<&str> = line.split('\t').collect();
            let (Some(a), Some(b), Some(v)) = (
                f.first().and_then(|s| s.parse::<u16>().ok()),
                f.get(1).and_then(|s| s.parse::<u16>().ok()),
                f.get(2).and_then(|s| s.trim().parse::<f32>().ok()),
            ) else {
                continue;
            };
            let (a, b) = (a.min(b).to_be_bytes(), a.max(b).to_be_bytes());
            let byte = ((v / 0.05).round() + 128.0).clamp(0.0, 255.0) as u64;
            entries.push((vec![0, 14, a[0], a[1], b[0], b[1]], byte));
        }
    }
    // Learned frames and the weights after adjectives.
    if let Some(path) = &frames {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let Some((w, v)) = line.split_once('\t') else {
                continue;
            };
            let packed = v
                .split(',')
                .filter_map(|x| x.trim().parse::<f32>().ok())
                .take(8)
                .enumerate()
                .fold(0u64, |acc, (i, x)| {
                    let b = ((x / 0.05).round() + 128.0).clamp(0.0, 255.0) as u64;
                    acc | b << (8 * i)
                });
            let key = if w == "@attr" {
                Some(vec![0, 12])
            } else if w == "@subj" {
                Some(vec![0, 18])
            } else if w == "@subjx" {
                Some(vec![0, 21])
            } else if w == "@coord" {
                Some(vec![0, 19])
            } else if w == "@impers" {
                Some(vec![0, 22])
            } else if let Some((class, prep)) = w.strip_prefix('&').and_then(|r| r.split_once(' '))
            {
                let class = class.parse::<u16>().ok();
                class.zip(remap(prep)).map(|(c, p)| {
                    let c = c.to_be_bytes();
                    [vec![0, 17, c[0], c[1]], p].concat()
                })
            } else if let Some((word, prep)) = w.strip_prefix('^').and_then(|r| r.split_once(' ')) {
                remap(word)
                    .zip(remap(prep))
                    .map(|(a, b)| [vec![0, 20], a, vec![0], b].concat())
            } else if let Some(adj) = w.strip_prefix('*') {
                remap(adj).map(|k| [vec![0, 23], k].concat())
            } else if let Some(verb) = w.strip_prefix('%') {
                remap(verb).map(|k| [vec![0, 16], k].concat())
            } else {
                remap(w).map(|k| [vec![0, 11], k].concat())
            };
            if let Some(key) = key {
                entries.push((key, packed));
            }
        }
    }
    // The dictionary holds the words' grammar ids: here, what each id is.
    if let Some((c, s)) = &table {
        for (i, pair) in grammar_ids(c, s)?.1.into_iter().enumerate() {
            let id = (i as u16 + 1).to_be_bytes();
            entries.push((vec![0, 10, id[0], id[1]], pair));
        }
    }
    let mut word_set: HashMap<String, u64> = HashMap::new();
    if let Some((words, sets)) = readings.as_ref() {
        let lines = if table.is_some() {
            Vec::new()
        } else {
            BufReader::new(File::open(words)?)
                .lines()
                .collect::<io::Result<Vec<_>>>()?
        };
        for line in lines {
            if let Some((w, s)) = line.split_once('\t') {
                if let Ok(s) = s.trim().parse::<u16>() {
                    word_set.insert(w.to_string(), s as u64);
                }
            }
        }
        for line in BufReader::new(File::open(sets)?).lines() {
            let line = line?;
            let Some((s, tags)) = line.split_once('\t') else {
                continue;
            };
            let Ok(s) = s.parse::<u16>() else { continue };
            let s = s.to_be_bytes();
            let bits = tags.split('|').map(kbcore::gram::parse).filter(|&b| b != 0);
            for (i, b) in bits.take(256).enumerate() {
                entries.push((vec![0, 9, s[0], s[1], i as u8], b));
            }
        }
    }
    if let Some((words, class_tags, tag_pairs)) = classes {
        let lines = if table.is_some() {
            Vec::new()
        } else {
            BufReader::new(File::open(&words)?)
                .lines()
                .collect::<io::Result<Vec<_>>>()?
        };
        for line in lines {
            let Some((w, c)) = line.split_once('\t') else {
                continue;
            };
            if let (Some(ids), Ok(c)) = (remap(w), c.trim().parse::<u64>()) {
                let set = word_set.get(w).copied().unwrap_or(0);
                entries.push(([vec![0, 3], ids].concat(), c | set << 16));
            }
        }
        // A class's readings: up to 8 tag ids, a byte each, packed low first.
        for line in BufReader::new(File::open(class_tags)?).lines() {
            let line = line?;
            let Some((c, tags)) = line.split_once('\t') else {
                continue;
            };
            let Ok(c) = c.parse::<u16>() else { continue };
            let packed = tags
                .split(',')
                .filter_map(|t| t.trim().parse::<u8>().ok())
                .take(8)
                .enumerate()
                .fold(0u64, |v, (i, t)| v | (t as u64) << (8 * i));
            let c = c.to_be_bytes();
            entries.push((vec![0, 5, c[0], c[1]], packed));
        }
        for line in BufReader::new(File::open(tag_pairs)?).lines() {
            let line = line?;
            let f: Vec<&str> = line.split('\t').collect();
            let (Some(a), Some(b), Some(v)) = (
                f.first().and_then(|s| s.parse::<u8>().ok()),
                f.get(1).and_then(|s| s.parse::<u8>().ok()),
                f.get(2).and_then(|s| s.trim().parse::<u64>().ok()),
            ) else {
                continue;
            };
            entries.push((vec![0, 4, a, b], v));
        }
    }
    let classes = entries.len() - with_endings;
    if let Some((confusions, rule_rows)) = rules {
        let mut members: Vec<Vec<String>> = Vec::new();
        for line in BufReader::new(File::open(confusions)?).lines() {
            members.push(line?.split('\t').map(String::from).collect());
        }
        for (id, ws) in members.iter().enumerate() {
            let [a, b] = ws.as_slice() else { continue };
            let (Some(ka), Some(kb)) = (remap(a), remap(b)) else {
                continue;
            };
            let v = (id as u64 + 1) * 2;
            entries.push(([vec![0, 7], ka.clone(), vec![0], kb.clone()].concat(), v));
            entries.push(([vec![0, 7], kb, vec![0], ka].concat(), v + 1));
        }
        for line in BufReader::new(File::open(rule_rows)?).lines() {
            let line = line?;
            let f: Vec<&str> = line.split('\t').collect();
            let (Some(id), Some(feature), Some(word), Some(s)) = (
                f.first().and_then(|s| s.parse::<usize>().ok()),
                f.get(1),
                f.get(2),
                f.get(3).and_then(|s| s.trim().parse::<f32>().ok()),
            ) else {
                continue;
            };
            let Some(member) = members
                .get(id)
                .and_then(|ws| ws.iter().position(|w| w == word))
            else {
                continue;
            };
            let key = [
                vec![0, 6, (id >> 16) as u8, (id >> 8) as u8, id as u8],
                feature.as_bytes().to_vec(),
            ]
            .concat();
            let strength = (s.clamp(0.0, 8.0) * 1000.0).round() as u64;
            entries.push((key, member as u64 * 100_000 + strength));
        }
    }
    let rules = entries.len() - with_endings - classes;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by(|a, b| a.0 == b.0);
    let mut builder = MapBuilder::new(BufWriter::new(File::create(staged(out_path))?))
        .map_err(io::Error::other)?;
    for (key, val) in &entries {
        builder.insert(key, *val).map_err(io::Error::other)?;
    }
    builder.finish().map_err(io::Error::other)?;
    std::fs::rename(staged(out_path), out_path)?;
    let size = std::fs::metadata(out_path)?.len();
    println!(
        "wrote {out_path}: {pairs} pairs + {} endings + {classes} grammar + {rules} rules, {dropped} dropped, {:.1} MB",
        with_endings - pairs,
        size as f64 / 1_048_576.0
    );
    Ok(())
}

/// Where a file is written before it takes `path`'s place, once complete: a
/// process with the old one mapped keeps reading that one (written in place,
/// its pages would be cut from under it — SIGBUS).
fn staged(path: &str) -> String {
    format!("{path}.part")
}

/// Remap a word to alphabet ids, or `None` if it holds an unsupported char.
fn remap(word: &str) -> Option<Vec<u8>> {
    word.chars().map(alphabet::char_to_id).collect()
}

/// `−log P(word)` in nats, scaled by [`PRIOR_INT_SCALE`] and stored as an int.
/// The engine divides by the same scale (`Config::prior_scale`) at query time,
/// recovering a value directly comparable to the edit costs (also in nats).
/// Returns 0 for every word when no frequency data is supplied (uniform prior).
const PRIOR_INT_SCALE: f64 = 1000.0;

fn prior_value(word: &str, freqs: &HashMap<String, u64>, total: f64) -> u64 {
    if freqs.is_empty() {
        return 0;
    }
    // Unseen dictionary words: treat as slightly rarer than a count of 1.
    let count = freqs.get(word).copied().unwrap_or(0) as f64;
    let neg_log_p = total.ln() - (count + 0.5).ln();
    (neg_log_p.max(0.0) * PRIOR_INT_SCALE) as u64
}

fn load_freqs(path: &str) -> io::Result<HashMap<String, u64>> {
    let mut map = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let mut it = line.split([' ', '\t']);
        if let (Some(w), Some(c)) = (it.next(), it.next()) {
            if let Ok(count) = c.trim().parse::<u64>() {
                map.insert(w.trim().to_lowercase(), count);
            }
        }
    }
    Ok(map)
}
