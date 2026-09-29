//! User settings: a tiny `key=value` file shared by the settings screen and the
//! keyboard, which re-reads it when it changes. The screen itself is generated
//! from [`Settings::schema`], so adding a setting only touches this file.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::i18n::t;
use crate::layout::Lang;

/// How eagerly space replaces the typed word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Careful,
    Normal,
    Bold,
    /// A non-word is always replaced by the best candidate (the original stays
    /// in the strip; ⌫ right after space brings it back).
    Always,
}

impl Strength {
    /// (max typing-error cost per letter of a correction, how far it must lead
    /// the runner-up, how much a word that is itself in the dictionary must
    /// lose by). Per letter, so long words with several slips still get fixed
    /// while garbage doesn't; rarity only picks between candidates.
    pub fn limits(self) -> (f32, f32, f32) {
        match self {
            Strength::Careful => (0.8, 1.5, 2.5),
            Strength::Normal => (1.3, 0.8, 1.5),
            Strength::Bold => (1.8, 0.4, 0.8),
            Strength::Always => (f32::INFINITY, 0.0, 0.8),
        }
    }
}

/// What the suggestion strip shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strip {
    /// Left: exactly what was typed (tap to keep); center: the correction.
    Two,
    /// Three suggestions.
    Full,
    Off,
}

/// The keyboard's colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// Light or dark as the phone is.
    System,
    Light,
    Dark,
}

/// How the language is switched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LangSwitch {
    Both,
    Key,
    Swipe,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    /// Android 12+: the keyboard takes its colors from the wallpaper.
    pub wallpaper_colors: bool,
    pub autocorrect: bool,
    pub strength: Strength,
    /// Show the correction in the text field while typing (once the typed
    /// letters can't start any word); the original goes to the strip.
    pub live_correction: bool,
    pub strip: Strip,
    pub auto_caps: bool,
    pub haptic: bool,
    pub lang_switch: LangSwitch,
    /// Compact one-row layout: the three letter rows squeezed into one row of
    /// column keys; words are decoded from the horizontal position alone.
    pub one_row: bool,
    /// Score letters by the distance from the actual touch point, not just by
    /// which key the touch landed on.
    pub touch_points: bool,
    /// Key row height, percent of the default.
    pub height: u32,
    /// Words drawn across the keys without lifting the finger.
    pub gestures: bool,
    /// A text just copied is offered in the strip, and the copies made while
    /// the keyboard runs are in the panel's 📋 tab (in memory only).
    pub clipboard: bool,
    /// A comma that is almost surely due goes in by itself (⌫ takes it out).
    pub auto_commas: bool,
    /// A word written without its ё (еще, пошел) goes in with it (ещё, пошёл)
    /// — only where е and ё are the same word.
    pub yo: bool,
    /// Where the finger slowed down or flew along a drawn word weighs its
    /// letters too.
    pub gesture_pace: bool,
    /// Show the guessed grip ([I ] / [ I] / [II]) in the strip.
    pub grip_indicator: bool,
    /// Shift taps by where the calibrated grip usually lands.
    pub grip_offsets: bool,
    /// The cursor put on a word of another script (an English word in a
    /// Russian text) switches the language to it.
    pub lang_from_text: bool,
    /// Experiment: the arc layout for the thumb the grip shows (a swipe up
    /// on ?123 or Enter forces it for the left or right thumb either way).
    pub arc_layout: bool,
    /// Obscene words are never suggested or corrected to (typed letter by
    /// letter, they stay).
    pub block_offensive: bool,
    /// Buttons: when each grip's calibration / its undo, and the reset of
    /// all, were last asked for (ms since the epoch; left, right, both, one
    /// finger) — the keyboard acts on a newer stamp.
    pub calibrate: [u64; 4],
    pub grip_undo: [u64; 4],
    pub grip_reset: u64,
    /// The languages to type in, in the order the language key and the
    /// swipe on the space bar go through them.
    pub languages: Vec<Lang>,
    /// For developers: log typing (touches, candidates) in the app's own
    /// test field only — never in other apps. Off by default.
    pub debug_log: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            wallpaper_colors: true,
            autocorrect: true,
            strength: Strength::Normal,
            live_correction: true,
            strip: Strip::Two,
            auto_caps: true,
            haptic: true,
            lang_switch: LangSwitch::Both,
            one_row: false,
            touch_points: true,
            height: 100,
            gestures: true,
            clipboard: true,
            auto_commas: true,
            yo: true,
            gesture_pace: true,
            grip_indicator: true,
            grip_offsets: true,
            lang_from_text: true,
            arc_layout: false,
            block_offensive: true,
            calibrate: [0; 4],
            grip_undo: [0; 4],
            grip_reset: 0,
            languages: vec![Lang::Ru, Lang::En],
            debug_log: false,
        }
    }
}

enum Kind {
    Bool,
    /// Two buttons side by side: this one and the one of the key given.
    ActionPair(&'static str),
    /// One of the values (labels: `set.<key>.<value>` in [`crate::i18n`]).
    Choice(&'static [&'static str]),
    /// A button: stores when it was pressed.
    Action,
    /// Several of the options, in an order (value: `a,b,c`), with their own
    /// names.
    Order(&'static [(&'static str, &'static str)]),
}

/// A setting on the screen; its label is `set.<key>` in [`crate::i18n`].
struct Field {
    key: &'static str,
    kind: Kind,
}

const FIELDS: &[Field] = &[
    Field {
        key: "languages",
        kind: Kind::Order(&[
            ("ru", "Русский"),
            ("en", "English"),
            ("de", "Deutsch"),
            ("fr", "Français"),
            ("es", "Español"),
            ("pt", "Português (Brasil)"),
        ]),
    },
    Field {
        key: "theme",
        kind: Kind::Choice(&["system", "light", "dark"]),
    },
    Field {
        key: "wallpaper_colors",
        kind: Kind::Bool,
    },
    Field {
        key: "autocorrect",
        kind: Kind::Bool,
    },
    Field {
        key: "strength",
        kind: Kind::Choice(&["careful", "normal", "bold", "always"]),
    },
    Field {
        key: "block_offensive",
        kind: Kind::Bool,
    },
    Field {
        key: "live_correction",
        kind: Kind::Bool,
    },
    Field {
        key: "strip",
        kind: Kind::Choice(&["two", "full", "off"]),
    },
    Field {
        key: "clipboard",
        kind: Kind::Bool,
    },
    Field {
        key: "auto_commas",
        kind: Kind::Bool,
    },
    Field {
        key: "yo",
        kind: Kind::Bool,
    },
    Field {
        key: "auto_caps",
        kind: Kind::Bool,
    },
    Field {
        key: "haptic",
        kind: Kind::Bool,
    },
    Field {
        key: "lang_switch",
        kind: Kind::Choice(&["both", "key", "swipe"]),
    },
    Field {
        key: "lang_from_text",
        kind: Kind::Bool,
    },
    Field {
        key: "one_row",
        kind: Kind::Bool,
    },
    Field {
        key: "touch_points",
        kind: Kind::Bool,
    },
    Field {
        key: "height",
        kind: Kind::Choice(&["90", "100", "115"]),
    },
    Field {
        key: "gestures",
        kind: Kind::Bool,
    },
    Field {
        key: "gesture_pace",
        kind: Kind::Bool,
    },
    Field {
        key: "calibrate_left",
        kind: Kind::ActionPair("grip_undo_left"),
    },
    Field {
        key: "calibrate_right",
        kind: Kind::ActionPair("grip_undo_right"),
    },
    Field {
        key: "calibrate_both",
        kind: Kind::ActionPair("grip_undo_both"),
    },
    Field {
        key: "calibrate_finger",
        kind: Kind::ActionPair("grip_undo_finger"),
    },
    Field {
        key: "arc_layout",
        kind: Kind::Bool,
    },
    Field {
        key: "grip_indicator",
        kind: Kind::Bool,
    },
    Field {
        key: "grip_offsets",
        kind: Kind::Bool,
    },
    Field {
        key: "grip_reset",
        kind: Kind::Action,
    },
    Field {
        key: "debug_log",
        kind: Kind::Bool,
    },
];

fn flag(b: bool) -> String {
    if b { "1" } else { "0" }.into()
}

impl Settings {
    pub fn get(&self, key: &str) -> Option<String> {
        Some(match key {
            "theme" => match self.theme {
                Theme::System => "system",
                Theme::Light => "light",
                Theme::Dark => "dark",
            }
            .into(),
            "wallpaper_colors" => flag(self.wallpaper_colors),
            "autocorrect" => flag(self.autocorrect),
            "strength" => match self.strength {
                Strength::Careful => "careful",
                Strength::Normal => "normal",
                Strength::Bold => "bold",
                Strength::Always => "always",
            }
            .into(),
            "live_correction" => flag(self.live_correction),
            "strip" => match self.strip {
                Strip::Two => "two",
                Strip::Full => "full",
                Strip::Off => "off",
            }
            .into(),
            "auto_caps" => flag(self.auto_caps),
            "haptic" => flag(self.haptic),
            "lang_switch" => match self.lang_switch {
                LangSwitch::Both => "both",
                LangSwitch::Key => "key",
                LangSwitch::Swipe => "swipe",
            }
            .into(),

            "one_row" => flag(self.one_row),
            "touch_points" => flag(self.touch_points),
            "height" => self.height.to_string(),
            "gestures" => flag(self.gestures),
            "clipboard" => flag(self.clipboard),
            "auto_commas" => flag(self.auto_commas),
            "yo" => flag(self.yo),
            "gesture_pace" => flag(self.gesture_pace),
            "grip_indicator" => flag(self.grip_indicator),
            "grip_offsets" => flag(self.grip_offsets),
            "lang_from_text" => flag(self.lang_from_text),
            "arc_layout" => flag(self.arc_layout),
            "block_offensive" => flag(self.block_offensive),
            k if GRIP_KEYS.iter().any(|g| k == format!("calibrate_{g}")) => {
                self.calibrate[grip_index(&k["calibrate_".len()..])].to_string()
            }
            k if GRIP_KEYS.iter().any(|g| k == format!("grip_undo_{g}")) => {
                self.grip_undo[grip_index(&k["grip_undo_".len()..])].to_string()
            }
            "grip_reset" => self.grip_reset.to_string(),
            "debug_log" => flag(self.debug_log),
            "languages" => self
                .languages
                .iter()
                .map(|l| l.code())
                .collect::<Vec<_>>()
                .join(","),
            _ => return None,
        })
    }

    /// Set one field from its stored text form; unknown keys/values are ignored.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        let on = value == "1";
        match key {
            "theme" => {
                self.theme = match value {
                    "system" => Theme::System,
                    "light" => Theme::Light,
                    "dark" => Theme::Dark,
                    _ => return false,
                }
            }
            "wallpaper_colors" => self.wallpaper_colors = on,
            "autocorrect" => self.autocorrect = on,
            "strength" => {
                self.strength = match value {
                    "careful" => Strength::Careful,
                    "bold" => Strength::Bold,
                    "always" => Strength::Always,
                    "normal" => Strength::Normal,
                    _ => return false,
                }
            }
            "live_correction" => self.live_correction = on,
            "strip" => {
                self.strip = match value {
                    "two" => Strip::Two,
                    "full" => Strip::Full,
                    "off" => Strip::Off,
                    _ => return false,
                }
            }
            // Older files: suggestions=0 meant no strip.
            "suggestions" if !on => self.strip = Strip::Off,
            "suggestions" => {}
            "auto_caps" => self.auto_caps = on,
            "haptic" => self.haptic = on,
            "lang_switch" => {
                self.lang_switch = match value {
                    "both" => LangSwitch::Both,
                    "key" => LangSwitch::Key,
                    "swipe" => LangSwitch::Swipe,
                    _ => return false,
                }
            }

            "one_row" => self.one_row = on,
            "touch_points" => self.touch_points = on,
            "height" => match value.parse::<u32>() {
                Ok(h) if (70..=140).contains(&h) => self.height = h,
                _ => return false,
            },
            "gestures" => self.gestures = on,
            "clipboard" => self.clipboard = on,
            "auto_commas" => self.auto_commas = on,
            "yo" => self.yo = on,
            "gesture_pace" => self.gesture_pace = on,
            "grip_indicator" => self.grip_indicator = on,
            "grip_offsets" => self.grip_offsets = on,
            "lang_from_text" => self.lang_from_text = on,
            "arc_layout" => self.arc_layout = on,
            "block_offensive" => self.block_offensive = on,
            "debug_log" => self.debug_log = on,
            "languages" => {
                let mut langs: Vec<Lang> = Vec::new();
                for l in value.split(',').filter_map(|c| Lang::from_code(c.trim())) {
                    if !langs.contains(&l) {
                        langs.push(l);
                    }
                }
                if langs.is_empty() {
                    return false;
                }
                self.languages = langs;
            }
            "grip_reset" => {
                let Ok(stamp) = value.parse::<u64>() else {
                    return false;
                };
                self.grip_reset = stamp;
            }
            k if k.starts_with("calibrate_") || k.starts_with("grip_undo_") => {
                let (undo, grip) = match k.strip_prefix("calibrate_") {
                    Some(g) => (false, g),
                    None => (true, &k["grip_undo_".len()..]),
                };
                let (Some(i), Ok(stamp)) = (
                    GRIP_KEYS.iter().position(|g| *g == grip),
                    value.parse::<u64>(),
                ) else {
                    return false;
                };
                if undo {
                    self.grip_undo[i] = stamp;
                } else {
                    self.calibrate[i] = stamp;
                }
            }
            _ => return false,
        }
        true
    }

    /// The first settings on a phone in `locale` (a language code, `de`):
    /// its language (if the keyboard has it) and English.
    pub fn for_locale(locale: &str) -> Settings {
        let languages = match Lang::from_code(locale) {
            Some(Lang::En) | None if !matches!(locale, "uk" | "be" | "kk") => vec![Lang::En],
            Some(l) if l != Lang::En => vec![l, Lang::En],
            _ => vec![Lang::Ru, Lang::En],
        };
        Settings {
            languages,
            ..Settings::default()
        }
    }

    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                s.set(k.trim(), v.trim());
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        FIELDS
            .iter()
            .flat_map(|f| match f.kind {
                Kind::ActionPair(second) => vec![f.key, second],
                _ => vec![f.key],
            })
            .map(|k| format!("{k}={}\n", self.get(k).unwrap_or_default()))
            .collect()
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, path)
    }

    /// One entry per setting for the settings screen, in `ui`'s language:
    /// `key \t bool|choice|action|order \t label \t value \t value:label|…`.
    pub fn schema(&self, ui: Lang) -> Vec<String> {
        FIELDS
            .iter()
            .map(|f| {
                let value = self.get(f.key).unwrap_or_default();
                let label = t(ui, &format!("set.{}", f.key));
                let (kind, opts) = match f.kind {
                    Kind::Bool => ("bool", String::new()),
                    Kind::Action => ("action", String::new()),
                    Kind::ActionPair(second) => (
                        "actionpair",
                        format!(
                            "{second}:{}:{}",
                            t(ui, "set.grip_undo"),
                            self.get(second).unwrap_or_default()
                        ),
                    ),
                    Kind::Choice(values) => (
                        "choice",
                        values
                            .iter()
                            .map(|v| format!("{v}:{}", t(ui, &format!("set.{}.{v}", f.key))))
                            .collect::<Vec<_>>()
                            .join("|"),
                    ),
                    Kind::Order(opts) => (
                        "order",
                        opts.iter()
                            .map(|(v, l)| format!("{v}:{l}"))
                            .collect::<Vec<_>>()
                            .join("|"),
                    ),
                };
                format!("{}\t{kind}\t{label}\t{value}\t{opts}", f.key)
            })
            .collect()
    }
}

/// The grips' names in the settings keys, in [`crate::grip::Grip`] order.
pub const GRIP_KEYS: [&str; 4] = ["left", "right", "both", "finger"];

fn grip_index(name: &str) -> usize {
    GRIP_KEYS.iter().position(|g| *g == name).unwrap_or(0)
}

/// The settings file plus the modification time it was last read at.
pub struct SettingsFile {
    path: PathBuf,
    stamp: Option<SystemTime>,
}

impl SettingsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SettingsFile {
            path: path.into(),
            stamp: None,
        }
    }

    /// The settings, if the file changed since the last call (or on the first).
    pub fn changed(&mut self) -> Option<Settings> {
        let stamp = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if self.stamp.is_some() && stamp == self.stamp {
            return None;
        }
        self.stamp = stamp.or(Some(SystemTime::UNIX_EPOCH));
        Some(Settings::load(&self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut s = Settings::default();
        s.set("strength", "bold");
        s.set("live_correction", "1");
        s.set("height", "115");
        assert_eq!(Settings::parse(&s.to_text()), s);
    }

    #[test]
    fn languages_keep_their_order() {
        let mut s = Settings::default();
        assert!(s.set("languages", "de,ru,de,xx"));
        assert_eq!(s.languages, vec![Lang::De, Lang::Ru]);
        assert!(!s.set("languages", "xx"), "never none");
        assert_eq!(Settings::parse(&s.to_text()), s);
        assert_eq!(
            Settings::for_locale("pt").languages,
            vec![Lang::Pt, Lang::En]
        );
        assert_eq!(
            Settings::for_locale("uk").languages,
            vec![Lang::Ru, Lang::En]
        );
        assert_eq!(Settings::for_locale("ja").languages, vec![Lang::En]);
        assert_eq!(Settings::for_locale("en").languages, vec![Lang::En]);
    }

    #[test]
    fn bad_values_are_ignored() {
        let s = Settings::parse("height=5000\nstrength=wild\nnope=1\n");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn schema_covers_every_field() {
        let s = Settings::default();
        for entry in s.schema(Lang::Ru) {
            let f: Vec<&str> = entry.split('\t').collect();
            assert_eq!(f.len(), 5, "{entry}");
            assert_eq!(s.get(f[0]).as_deref(), Some(f[3]));
        }
    }
}
