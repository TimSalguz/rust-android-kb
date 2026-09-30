//! A phrase's readings and the grammar's walk over it (debugging).
//! Usage: DICT_FST=… BIGRAMS_FST=… walk_check "words of a phrase" …
use kbcore::{gram, Config, Engine};

const NAMES: [(u64, &str); 34] = {
    use gram::bit::*;
    [
        (NOUN, "NOUN"), (ADJF, "ADJF"), (ADJS, "ADJS"), (VERB, "VERB"), (INFN, "INFN"),
        (PRTF, "PRTF"), (PRTS, "PRTS"), (GRND, "GRND"), (NUMR, "NUMR"), (ADVB, "ADVB"),
        (NPRO, "NPRO"), (PREP, "PREP"), (CONJ, "CONJ"), (PRCL, "PRCL"), (MASC, "masc"),
        (FEMN, "femn"), (NEUT, "neut"), (MSF, "ms-f"), (SING, "sing"), (PLUR, "plur"),
        (NOMN, "nomn"), (GENT, "gent"), (DATV, "datv"), (ACCS, "accs"), (ABLT, "ablt"),
        (LOCT, "loct"), (PER1, "1per"), (PER2, "2per"), (PER3, "3per"), (PAST, "past"),
        (PRES, "pres"), (FUTR, "futr"), (ANIM, "anim"), (INAN, "inan"),
    ]
};

fn show(r: u64) -> String {
    NAMES.iter().filter(|(b, _)| r & b != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(",")
}

fn main() -> std::io::Result<()> {
    let dict = std::env::var("DICT_FST").unwrap();
    let b = std::env::var("BIGRAMS_FST").unwrap();
    let engine = Engine::open_with_bigrams(&dict, &b, Config::balanced())?;
    for s in std::env::args().skip(1) {
        let phrase: Vec<(String, Vec<u64>)> = s
            .split_whitespace()
            .map(|w| (w.to_string(), engine.readings(w)))
            .collect();
        println!("{s:?}");
        for (w, r) in &phrase {
            println!("  {w}: {}", r.iter().map(|&x| show(x)).collect::<Vec<_>>().join(" | "));
        }
        let walk = gram::walk(&phrase);
        println!("  walk: governor {:?} subject {:?} next {}", walk.governor, walk.subject.map(show), walk.subject_next);
    }
    Ok(())
}
