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
    let open = || -> io::Result<Engine<kbcore::Mmap>> {
        let dict = std::env::var("DICT_FST").unwrap_or_else(|_| "dict.fst".into());
        match std::env::var("BIGRAMS_FST") {
            Ok(b) => Engine::open_with_bigrams(&dict, b, config_from_env()),
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
