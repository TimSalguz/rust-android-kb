//! `kbcore` — offline, mmap-backed, keyboard-geometry-weighted autocorrect
//! engine for Russian (ЙЦУКЕН), English (QWERTY) and the Latin-script
//! languages with accents (German, French, Spanish, Portuguese).
//!
//! See `docs/DESIGN.md` for the full rationale. The runtime holds no heap copy
//! of the dictionary: it memory-maps a prebuilt FST blob (built by the
//! `index-builder` tool) and runs a weighted Damerau-Levenshtein search over it.

pub mod alphabet;
pub mod config;
pub mod dict;
pub mod engine;
pub mod gesture;
pub mod gram;
pub mod keyboard;
pub mod lemmas;

pub use config::{Config, Latin, Profile};
pub use dict::DictFormat;
pub use engine::{Candidate, Casing, Context, Engine, Evidence, Hint, Stats};
pub use keyboard::{Keyboard, Layout};
pub use lemmas::Lemmas;

/// Re-exported so downstream crates can name the mmap-backed engine type
/// (`Engine<Mmap>`) without depending on `memmap2` directly.
pub use memmap2::Mmap;
