# rust-android-kb

An offline on-screen keyboard for **Russian (ЙЦУКЕН)**, **English (QWERTY)**,
**German (QWERTZ)**, **French (AZERTY)**, **Spanish** and **Brazilian
Portuguese**, with the whole brain in Rust: correction, completion, swipe
typing, layouts, touch handling and rendering decisions. Android is the first target; the engine is
platform-free and also meant for desktop use (launcher / file search).

No network, no telemetry; the engine's own heap is ~0.3 MB, the dictionaries are
memory-mapped files. See [`docs/DESIGN.md`](docs/DESIGN.md)
(in Russian) for the design and its measurements.

## Highlights

- **Tiny & mmap'd:** the dictionary is an FST (3.5 M words — every Russian word
  form from OpenCorpora, English from SCOWL — with each word's frequency and
  grammar, in 9.0 MB; `kbcore::dict`) memory-mapped at runtime — no heap
  copy, zero start-up parsing. The engine's own heap is
  ~0.3 MB; the rest is clean, evictable file pages.
- **Fast:** ~1 ms per correction query, ~0.5 ms per keystroke on real typing
  (desktop, single thread).
- **Accurate:** ~88–90% top-1 / ~99% top-8 on a held-out mix of 15 typo families
  (`tools/eval.py`).
- **Noisy-channel ranking:** `argmin w_lm·(−log P(word)) + w_ch·EditCost`, with a
  weighted Damerau-Levenshtein channel:
  - substitutions from **physical key distance** (Gaussian touch model) for a
    phone or a desktop keyboard profile;
  - confusions geometry can't see: phonetic pairs (о/а, е/и, д/т, з/с, …),
    е→ё (almost free; ё→е is not), ь/ъ;
  - context-aware insertions/deletions: key bounce (`приввет`), one press hitting
    two keys, a doubled letter typed once (`коректный`), a missing ь/ъ, a missing
    apostrophe/hyphen (`dont`→`don't`).
- **Hypotheses:** wrong layout on a desktop keyboard (`ghbdtn`→`привет`), and
  typing without leaving the home row (`ааоаааоаыа`→`каракатица`).
- **Touch points, not just keys:** substitution costs from where the finger
  actually landed; held keys, slides, roll-over of two thumbs; a space, comma
  or period hit instead of a letter above it is taken back (`напр мер`,
  `пр,вет`), a missing space is put in (`приветкакдела`).
- **Context:** word pairs from everyday sentences (Tatoeba) and the web
  (Leipzig), plus which ending follows which (`неведомых дорожках`): re-ranks
  corrections, predicts the next word, and reads the previous word again once
  the next one is typed (`а принципе` → `в принципе`).
- **Phrase grammar (Russian):** every reading of every word (OpenCorpora, the
  rare ones too: `такой` is also masculine) checks the case a preposition
  governs, agreement with the adjectives after it and a verb with its pronoun
  (`в большом дрме` → `доме`, `с некоторым опвтом` → `опытом`, `она сказал` →
  `сказала`); the next-word guesses keep only forms that fit (`к большим` →
  `деньгам`, not `успехом`). It only rules out, never pushes: all 3.19 M forms
  with their grammar take 3.6 MB — `kbcore::gram`, `tools/eval_phrase.py`.
- **Languages:** each its own dictionary, context model and capitals (German
  nouns), installed when the language is switched on; a left-out accent costs
  next to nothing (`nao`→`não`, `fur`→`für`), accents also sit on a long press.
  Languages and their order are a setting; the first start takes the phone's.
- **Swipe typing** without third-party libraries: the path is matched against
  the dictionary by a branch-and-bound walk (~0.4 ms). Where the finger
  stopped or flew past a key is a hint too (a setting): +2–3 points top-1 on
  simulated swipes (`examples/swipesim.rs`, `PACE=corners`).
- **Commas:** learned from text — the odds of a comma between two words by
  the word after («что», «но», «который»), the word before («например») and
  the pair; one almost surely due goes in by itself (98–99% right on held-out
  sentences), ⌫ takes it out (`tools/build_commas.py`).
- **Sense classes:** 20 k lemmas in 512 classes of words that share
  sentences (PPMI + SVD + k-means, `tools/build_topics.py`), 0.9 MB: a word
  whose class goes with the sentence's gains («рыба гниёт с головы»).
- **Emoji and clipboard:** hold the comma — emoji by group (only those the
  phone's font draws; the list is read on first open, only the page shown is
  drawn), the recent ones, and the texts copied while the keyboard runs; a
  text just copied is offered in the strip. All in memory only; texts an app
  marks sensitive (passwords) are skipped.
- **Context rules** for words one slip apart, learned offline from the next
  word (decision lists): a slipped real word is put back 71% of the time
  (68.7% without the rules), with no change to correctly typed words
  (`tools/eval_realword.py`).
- **Grip calibration (opt-in):** the farthest arc of each thumb and a phrase
  typed with each grip (and, optionally, one finger on a phone lying flat),
  with the phone's tilt; then the keyboard tells the grip from the tilt and the
  taps, shows it (`[I ]`, `[ I]`, `[II]`, `[•]`) and shifts taps by where that
  grip lands — outward past a thumb's reach, where long reaches fall short.
  Tilts and offsets only, never text.
- **Private by construction:** nothing is learned or sent; the user dictionary
  changes only by explicit actions. So a private window keeps its suggestions —
  only passwords, addresses and e-mail fields go without.
- **Android IME:** a thin Java shim (the platform requires an
  `InputMethodService`) forwards touches and paints a draw list produced in Rust.
  The APK is built without Gradle.

## Layout

| path | what |
|------|------|
| `crates/core` (`kbcore`)   | the engine: alphabet, keyboard geometry, config, FST search |
| `crates/android` (`kbime`) | the keyboard: layouts, touches, composing, autocorrect, draw list + JNI |
| `crates/index-builder`     | `data/lexicon.tsv` → `dict.fst` |
| `crates/cli` (`kbdemo`)    | terminal demo / query harness |
| `android/`                 | Java shim, manifest, `build.sh` (APK without Gradle) |
| `tools/`                   | typo benchmark, dictionary pipeline (`fetch_sources.sh`, `build_lexicon.sh`) |
| `data/`                    | dictionary sources and licenses (see `data/README.md`); `slang.tsv` |

## Quick start

```sh
nix develop                     # Rust (+ aarch64-linux-android), Android SDK/NDK, JDK
pip install pymorphy3 pymorphy3-dicts-ru   # once: OpenCorpora word forms
tools/fetch_sources.sh && tools/build_lexicon.sh   # → data/lexicon.tsv (~3.5 M words)
cargo run --release -p index-builder -- data/lexicon.tsv dict.fst
cargo run --release -p cli                       # interactive TUI
cargo run --release -p cli -- --query превед ghbdtn прог
cargo test --workspace
```

`kbdemo` knobs: `KB_PROFILE=desktop|phone`, `KB_PRESET=fast|balanced|accurate`,
`KB_FLIP=0|1`, `KB_HOMEROW=0|1`, `KB_SET="c_del=3.5,sigma=0.6"` (any weight).

Benchmark (tune on one `--seed`, confirm on another):

```sh
tools/eval.py dict.fst
KB_PROFILE=phone tools/eval.py dict.fst --types touch_sub,touch_sub2,short
tools/eval.py dict.fst --types del --dump del      # show the failures
```

## Android

```sh
nix develop -c android/build.sh          # → target/apk/rust-kb.apk (arm64)
adb install -r target/apk/rust-kb.apk
```

CI builds the APK on every push to `main` (artifact `rust-kb-apk`) with the
Russian/English dictionary; the other languages' packs are built locally with
`tools/lang/<code>.sh` (they need large downloads) and packaged when present.
Builds are signed with the committed **debug** key (`android/debug.keystore`,
password `android`) so each build installs over the previous one — it is not a
release key.

On the phone: open the **Rust KB** app — a short guide on the first start,
buttons to enable the keyboard in the system and to switch to it, the
keyboard's options (languages and their order, theme — light, dark, as the
system, wallpaper colors on Android 12+ — autocorrect strength, height, …) and
a field to try it in. The options are defined in Rust
(`crates/android/src/settings.rs`) and the screen is generated from them; the
keyboard's own words follow the phone's language (`crates/android/src/i18n.rs`).

On the keyboard: swipe the space bar to change the language, swipe up on it
for the next likeliest word, hold it for another keyboard or the settings;
⌫ right after a correction brings back what was typed; hold the comma for
emoji and what was copied lately.

## Roadmap

Next: richer context rules (docs/rules-format.md), a one-handed "pseudo-cursor" pad
that draws swipes, emoji search by name and skin tones, trigrams for prediction,
more languages (Italian, Ukrainian), mapping the dictionaries straight from the
APK instead of copying them, dropping dictionary pages while the keyboard is
hidden (`MADV_DONTNEED`), a desktop build (launcher / file search).

## License

Code: MIT OR Apache-2.0.

The data built into the app comes from open sources, each under its own
license (details and attributions: [`data/README.md`](data/README.md) and the
`data/<language>/README.md` of each language pack):

- word lists and counts — OpenCorpora, hermitdave/FrequencyWords, LanguageTool
  and Wiktionary (via kaikki.org): CC BY-SA; LibreOffice Hunspell dictionaries:
  BSD-style, MPL 2.0 or LGPL; SCOWL: permissive. The derived word lists are
  distributed under CC BY-SA 4.0;
- context models — Tatoeba (CC BY 2.0 FR) and the Leipzig Corpora Collection
  (CC BY).
