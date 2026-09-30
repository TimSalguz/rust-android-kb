//! Terminal demo for the kbcore autocorrect engine.
//!
//! Type a (possibly misspelled / wrong-layout) word and watch the ranked
//! candidates update live, with the cost of each and the query latency.
//!
//!   Tab   accept the top candidate      Enter clear (new word)
//!   Esc   quit                          Backspace delete a char
//!
//! Usage: `kbdemo [DICT_FST]`  (default: dict.fst)

use std::io::{self, Write};
use std::time::Instant;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::{cursor, execute, terminal};

use kbcore::{Config, Engine, Evidence, Profile};

/// Restores the terminal on drop, even if we panic mid-session.
struct TermGuard;

impl TermGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(TermGuard)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Non-interactive modes (dict from DICT_FST, context model from BIGRAMS_FST):
    //   kbdemo --query word1 prev|word2 ...   corrections (+ completions); `prev|` adds context
    //   kbdemo --predict word1 "w1 w2" ...    likeliest next words (after a phrase: its grammar)
    //   kbdemo --swipes FILE                  drawn words read back (tools/real_swipes.py)
    let open = || -> io::Result<Engine<kbcore::Mmap>> {
        let dict = std::env::var("DICT_FST").unwrap_or_else(|_| "dict.fst".into());
        match std::env::var("BIGRAMS_FST") {
            Ok(b) => {
                let engine = Engine::open_with_bigrams(&dict, &b, config_from_env())?;
                // The lemma vectors, when they lie beside the context model.
                let lemmas = std::path::Path::new(&b).with_file_name("lemmas.bin");
                let chooser = std::path::Path::new(&b).with_file_name("chooser.bin");
                match (lemmas.exists(), chooser.exists()) {
                    (true, true) => engine.open_lemmas(lemmas)?.open_chooser(chooser),
                    (true, false) => engine.open_lemmas(lemmas),
                    _ => Ok(engine),
                }
            }
            Err(_) => Engine::open(&dict, config_from_env()),
        }
    };
    if args.first().map(String::as_str) == Some("--predict") {
        let engine = open()?;
        for w in &args[1..] {
            let phrase: Vec<String> = w.split_whitespace().map(str::to_lowercase).collect();
            let ctx = engine.context(phrase.last().map(String::as_str), &phrase, &phrase);
            let next: Vec<String> = engine
                .decode(&ctx, Evidence::Nothing)
                .into_iter()
                .take(8)
                .map(|c| format!("{} {:.2}", c.word, c.cost))
                .collect();
            println!("{w:?} → {}", next.join(", "));
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--graph") {
        // The sentence's graph as the phone builds it: each word's likeliest
        // heads (the student parser, parser.bin beside the context model).
        let engine = open()?;
        let b = std::env::var("BIGRAMS_FST").unwrap_or_default();
        let path = std::path::Path::new(&b).with_file_name("parser.bin");
        let engine = engine.open_parser(path)?;
        let (parser, lemmas) = (engine.parser().unwrap(), engine.lemma_vectors().unwrap());
        for s in &args[1..] {
            let words: Vec<String> = s.split_whitespace().map(str::to_lowercase).collect();
            let read: Vec<_> = words.iter().filter_map(|w| engine.parser_word(w)).collect();
            let t = std::time::Instant::now();
            let chances = parser.parse(lemmas, &read);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            println!("{s:?}  ({ms:.2} ms)");
            for (i, row) in chances.iter().enumerate() {
                let mut ranked: Vec<(usize, f32)> = row.iter().copied().enumerate().collect();
                ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
                let shown: Vec<String> = ranked
                    .iter()
                    .take(3)
                    .filter(|x| x.1 >= 0.01)
                    .map(|&(j, p)| {
                        let head = match j {
                            0 => "корень".to_string(),
                            j if j <= words.len() => words[j - 1].clone(),
                            _ => "(впереди)".to_string(),
                        };
                        format!("{head} {p:.2}")
                    })
                    .collect();
                println!("  {:<12} → {}", words[i], shown.join(" | "));
            }
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--swipes") {
        let engine = open()?;
        let path = args
            .get(1)
            .ok_or_else(|| io::Error::other("--swipes FILE"))?;
        return swipes(&engine, path);
    }
    if args.first().map(String::as_str) == Some("--query") {
        let engine = open()?;
        for q in &args[1..] {
            // "words before|typo": the last one for the word pairs, all of
            // them for the phrase grammar.
            let (before, q) = match q.split_once('|') {
                Some((p, w)) => (p.split_whitespace().map(str::to_lowercase).collect(), w),
                None => (Vec::new(), q.as_str()),
            };
            let t0 = Instant::now();
            let ctx = engine.context(before.last().map(String::as_str), &before, &before);
            let cands = engine.decode(&ctx, Evidence::Taps(q, &[]));
            let t1 = Instant::now();
            let comps = engine.decode(&ctx, Evidence::Begun(q));
            let t2 = Instant::now();
            let (sug, com) = (
                t1.duration_since(t0).as_micros(),
                t2.duration_since(t1).as_micros(),
            );
            println!(
                "\n{q:?}  (suggest {:.2} ms + complete {:.2} ms, in context)",
                sug as f64 / 1000.0,
                com as f64 / 1000.0
            );
            println!("  corrections:");
            for (i, c) in cands.iter().enumerate() {
                let mark = if i == 0 { "→" } else { " " };
                println!("    {mark} {:<24} {:6.2}", c.word, c.cost);
            }
            if !comps.is_empty() {
                println!("  completions:");
                for c in &comps {
                    println!("      {:<24} {:6.2}", c.word, c.cost);
                }
            }
        }
        return Ok(());
    }

    let dict = args.first().cloned().unwrap_or_else(|| "dict.fst".into());
    let engine = match Engine::open(&dict, config_from_env()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("failed to open {dict}: {e}\nbuild it first: cargo run -p index-builder --release -- words.txt dict.fst");
            std::process::exit(1);
        }
    };

    let _guard = TermGuard::enter()?;
    let mut input = String::new();

    // Compute suggestions once per keystroke and pass them to the renderer.
    let compute = |input: &str| {
        let t = Instant::now();
        let candidates = engine.suggest(input);
        let completions = engine.complete(input);
        (candidates, completions, t.elapsed().as_micros())
    };

    let (mut cands, mut comps, mut last_us) = compute(&input);
    render(&input, &cands, &comps, last_us)?;
    loop {
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Esc => break,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Char(c) => {
                for lc in c.to_lowercase() {
                    input.push(lc);
                }
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => input.clear(),
            KeyCode::Tab => {
                if let Some(top) = cands.first() {
                    input = top.word.clone();
                }
            }
            _ => continue,
        }

        (cands, comps, last_us) = compute(&input);
        render(&input, &cands, &comps, last_us)?;
    }
    Ok(())
}

/// Select a preset from `KB_PRESET` (fast|balanced|accurate) and a keyboard
/// from `KB_PROFILE` (desktop|phone), then apply any fine-grained `KB_*`
/// overrides. Used by both interactive and `--query` modes.
/// Swipes drawn by people, read back one by one (no context: each word on its
/// own): how often the word drawn comes first, among the first three, at all
/// — by its length — and how long a reading takes. FILE (tools/real_swipes.py):
/// `#key GRID CHAR X Y` (key centers), `#width GRID W` (a key's width), then
/// `WORD GRID x,y,t x,y,t …`. `PACE=0` leaves the times out.
fn swipes(engine: &Engine<kbcore::Mmap>, path: &str) -> io::Result<()> {
    use std::collections::HashMap;
    let text = std::fs::read_to_string(path)?;
    let mut keys: HashMap<String, Vec<(char, f32, f32)>> = HashMap::new();
    let mut width: HashMap<String, f32> = HashMap::new();
    let pace = std::env::var("PACE").as_deref() != Ok("0");
    let fold = |w: &str| w.to_lowercase().replace('ё', "е");
    // By length: 1–2, 3–4, 5–7, 8+ letters — (seen, first, top 3, found).
    let bucket = |n: usize| match n {
        0..=2 => 0,
        3..=4 => 1,
        5..=7 => 2,
        _ => 3,
    };
    let mut stats = [[0usize; 4]; 4];
    let mut ms = 0f64;
    let mut misses: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        match f.next() {
            Some("#key") => {
                let (Some(g), Some(c), Some(x), Some(y)) = (f.next(), f.next(), f.next(), f.next())
                else {
                    continue;
                };
                let (Some(c), Ok(x), Ok(y)) = (c.chars().next(), x.parse(), y.parse()) else {
                    continue;
                };
                keys.entry(g.to_string()).or_default().push((c, x, y));
            }
            Some("#width") => {
                if let (Some(g), Some(Ok(w))) = (f.next(), f.next().map(str::parse::<f32>)) {
                    width.insert(g.to_string(), w);
                }
            }
            Some(word) => {
                let Some(g) = f.next() else { continue };
                let (Some(k), Some(&w)) = (keys.get(g), width.get(g)) else {
                    continue;
                };
                let mut pts = Vec::new();
                let mut times = Vec::new();
                for p in f {
                    let v: Vec<f32> = p.split(',').filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 3 {
                        pts.push((v[0], v[1]));
                        times.push(v[2]);
                    }
                }
                let t0 = times.first().copied().unwrap_or(0.0);
                let times: Vec<f32> = times.iter().map(|t| t - t0).collect();
                let start = Instant::now();
                let got = engine.gesture_timed(&pts, pace.then_some(&times[..]), k, w);
                ms += start.elapsed().as_secs_f64() * 1000.0;
                let want = fold(word);
                let rank = got.iter().position(|c| fold(&c.word) == want);
                let s = &mut stats[bucket(want.chars().count())];
                s[0] += 1;
                s[1] += (rank == Some(0)) as usize;
                s[2] += rank.is_some_and(|r| r < 3) as usize;
                s[3] += rank.is_some() as usize;
                if rank != Some(0) && misses.len() < 30 {
                    let top: Vec<&str> = got.iter().take(3).map(|c| c.word.as_str()).collect();
                    misses.push(format!("{want} → {}", top.join(", ")));
                }
            }
            None => {}
        }
    }
    let total: [usize; 4] = std::array::from_fn(|i| stats.iter().map(|s| s[i]).sum());
    let pct = |a: usize, n: usize| 100.0 * a as f64 / n.max(1) as f64;
    println!(
        "{} swipes: first {:.2}%, top-3 {:.2}%, found {:.2}% ({:.2} ms each)",
        total[0],
        pct(total[1], total[0]),
        pct(total[2], total[0]),
        pct(total[3], total[0]),
        ms / total[0].max(1) as f64
    );
    for (s, name) in stats.iter().zip(["1–2", "3–4", "5–7", "8+"]) {
        println!(
            "  {name:>4} letters: {:>6} swipes, first {:.2}%, top-3 {:.2}%, found {:.2}%",
            s[0],
            pct(s[1], s[0]),
            pct(s[2], s[0]),
            pct(s[3], s[0])
        );
    }
    if std::env::var("MISSES").is_ok() {
        for m in &misses {
            println!("  miss: {m}");
        }
    }
    Ok(())
}

fn config_from_env() -> Config {
    let mut cfg = match std::env::var("KB_PRESET").as_deref() {
        Ok("fast") => Config::fast(),
        Ok("accurate") => Config::accurate(),
        _ => Config::balanced(),
    };
    if std::env::var("KB_PROFILE").as_deref() == Ok("phone") {
        cfg = cfg.with_profile(Profile::Phone);
    }
    let flag = |k: &str| std::env::var(k).ok().map(|v| v != "0");
    if let Some(v) = flag("KB_FLIP") {
        cfg.layout_flip = v;
    }
    if let Some(v) = flag("KB_HOMEROW") {
        cfg.home_row = v;
    }
    // KB_SET="c_ins=3.5,c_phonetic=3" sets any numeric weight by name.
    if let Ok(list) = std::env::var("KB_SET") {
        for kv in list.split(',').filter(|kv| !kv.is_empty()) {
            let parsed = kv
                .split_once('=')
                .and_then(|(k, v)| Some((k.trim(), v.trim().parse::<f32>().ok()?)));
            match parsed.map(|(k, v)| cfg.set_param(k, v)) {
                Some(Ok(())) => {}
                Some(Err(e)) => eprintln!("KB_SET: {e}"),
                None => eprintln!("KB_SET: bad entry `{kv}` (want name=number)"),
            }
        }
    }
    let num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse().ok());
    if let Some(v) = num("KB_MAXCOST") {
        cfg.max_cost = v;
    }
    if let Some(v) = num("KB_MAXNODES") {
        cfg.max_nodes = v as usize;
    }
    if let Some(v) = num("KB_MINRESULTS") {
        cfg.min_results = v as usize;
    }
    if let Some(v) = num("KB_CEILING") {
        cfg.max_cost_ceiling = v;
    }
    cfg
}

fn render(
    input: &str,
    candidates: &[kbcore::Candidate],
    completions: &[kbcore::Candidate],
    last_us: u128,
) -> io::Result<()> {
    let mut out = io::stdout();
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    execute!(out, SetForegroundColor(Color::Grey))?;
    execute!(
        out,
        Print("kbcore demo — Tab accept · Enter clear · Esc quit\r\n\r\n")
    )?;
    execute!(out, ResetColor, SetForegroundColor(Color::White))?;
    execute!(out, Print(format!("  > {input}\r\n\r\n")))?;
    if input.is_empty() {
        execute!(
            out,
            SetForegroundColor(Color::DarkGrey),
            Print("  (type something)\r\n")
        )?;
    } else {
        execute!(
            out,
            SetForegroundColor(Color::Grey),
            Print("  corrections\r\n")
        )?;
        if candidates.is_empty() {
            execute!(
                out,
                SetForegroundColor(Color::DarkGrey),
                Print("    (none within cost threshold)\r\n")
            )?;
        }
        for (i, c) in candidates.iter().enumerate() {
            let exact = c.cost == 0.0;
            let color = if i == 0 {
                if exact {
                    Color::Magenta
                } else {
                    Color::Green
                }
            } else {
                Color::DarkGrey
            };
            let marker = if i == 0 { "→" } else { " " };
            execute!(
                out,
                SetForegroundColor(color),
                Print(format!("    {marker} {:<24} {:6.2}\r\n", c.word, c.cost))
            )?;
        }
        if !completions.is_empty() {
            execute!(
                out,
                SetForegroundColor(Color::Grey),
                Print("\r\n  completions\r\n")
            )?;
            for c in completions {
                execute!(
                    out,
                    SetForegroundColor(Color::DarkCyan),
                    Print(format!("      {:<24} {:6.2}\r\n", c.word, c.cost))
                )?;
            }
        }
    }

    execute!(
        out,
        ResetColor,
        SetForegroundColor(Color::DarkGrey),
        Print(format!(
            "\r\n  query: {:.2} ms\r\n",
            last_us as f64 / 1000.0
        )),
        ResetColor
    )?;
    out.flush()
}
