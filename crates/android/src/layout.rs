//! Key layouts and their pixel geometry.

use kbcore::Latin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
    De,
    Fr,
    Es,
    /// Brazilian Portuguese.
    Pt,
}

impl Lang {
    pub const ALL: [Lang; 6] = [Lang::Ru, Lang::En, Lang::De, Lang::Fr, Lang::Es, Lang::Pt];

    /// Code in the settings file and the data packs.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::Pt => "pt",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The language's own name (space bar, settings).
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::Pt => "Português",
        }
    }

    /// The language key's label.
    pub fn short(self) -> &'static str {
        match self {
            Lang::Ru => "RU",
            Lang::En => "EN",
            Lang::De => "DE",
            Lang::Fr => "FR",
            Lang::Es => "ES",
            Lang::Pt => "PT",
        }
    }

    /// The data pack with the dictionary: Russian and English share one.
    pub fn pack(self) -> &'static str {
        match self {
            Lang::Ru | Lang::En => "base",
            other => other.code(),
        }
    }

    /// Where the Latin letters sit.
    pub fn latin(self) -> Latin {
        match self {
            Lang::Ru | Lang::En => Latin::Qwerty,
            Lang::De => Latin::Qwertz,
            Lang::Fr => Latin::Azerty,
            Lang::Es => Latin::Spanish,
            Lang::Pt => Latin::Portuguese,
        }
    }

    /// The letters page key's label on the symbols pages.
    pub fn abc(self) -> &'static str {
        match self {
            Lang::Ru => "АБВ",
            _ => "ABC",
        }
    }

    /// The currency on the first symbols page.
    fn currency(self) -> char {
        match self {
            Lang::Ru => '₽',
            Lang::En => '$',
            _ => '€',
        }
    }
}

/// Which thumb the one-handed (arc) layout is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Letters,
    Symbols,
    /// Second symbols page (`=\<`).
    Symbols2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Char(char),
    Shift,
    Backspace,
    Space,
    Enter,
    /// Switch RU ↔ EN.
    Lang,
    Symbols,
    Symbols2,
    Letters,
    /// A key of the one-row (compact) layout: stands for a whole column.
    Column(u8),
    /// The emoji panel's page back (-1) or on (1).
    Page(i8),
    /// The emoji panel: search the emoji by name (the letters type the
    /// query, the strip shows what it finds).
    EmojiSearch,
}

/// The comma's long-press: the emoji panel (drawn as its hint).
pub const EMOJI_KEY: char = '😊';

#[derive(Clone, Debug)]
pub struct Key {
    pub action: Action,
    /// Long-press alternative (ё on е, digits on the top row, …).
    pub alt: Option<char>,
    /// More long-press alternatives after `alt` (е: ё, then 5): with any,
    /// holding the key opens a row to choose from.
    pub more: &'static str,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Turned this much (radians, clockwise) about its center: the arc
    /// layout's keys lie along the arc. 0 elsewhere.
    pub angle: f32,
}

impl Key {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if self.angle != 0.0 {
            let (lx, ly) = self.local(x, y);
            return 2.0 * lx.abs() <= self.w && 2.0 * ly.abs() <= self.h;
        }
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// `(x, y)` in the key's own frame: from its center, turned back by its
    /// angle (x along the key's width, y down its height).
    pub fn local(&self, x: f32, y: f32) -> (f32, f32) {
        let (cx, cy) = self.center();
        let (dx, dy) = (x - cx, y - cy);
        if self.angle == 0.0 {
            return (dx, dy);
        }
        let (s, c) = self.angle.sin_cos();
        (dx * c + dy * s, -dx * s + dy * c)
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Squared distance from a point to the key's rectangle (0 inside).
    pub fn dist2(&self, x: f32, y: f32) -> f32 {
        if self.angle != 0.0 {
            let (lx, ly) = self.local(x, y);
            let dx = (lx.abs() - self.w / 2.0).max(0.0);
            let dy = (ly.abs() - self.h / 2.0).max(0.0);
            return dx * dx + dy * dy;
        }
        let dx = (self.x - x).max(0.0).max(x - (self.x + self.w));
        let dy = (self.y - y).max(0.0).max(y - (self.y + self.h));
        dx * dx + dy * dy
    }
}

type Row = Vec<(f32, Action, Option<char>)>;

/// Long-press alternatives beyond a key's first one.
fn more_alts(lang: Lang, action: Action) -> &'static str {
    match action {
        Action::Char('е') => "5",
        Action::Char('.') if lang == Lang::Es => "?:;-¿¡",
        Action::Char('.') => "?:;-",
        _ => "",
    }
}

/// A letter's accented forms on a long press, the likeliest first (the
/// engine also restores a left-out accent by itself: nao → não).
fn accents(lang: Lang, c: char) -> &'static str {
    match (lang, c) {
        (Lang::De, 's') => "ß",
        (Lang::De, 'e') => "é",
        (Lang::Fr, 'a') => "àâæ",
        (Lang::Fr, 'c') => "ç",
        (Lang::Fr, 'e') => "éèêë",
        (Lang::Fr, 'i') => "îï",
        (Lang::Fr, 'o') => "ôœ",
        (Lang::Fr, 'u') => "ùûü",
        (Lang::Fr, 'y') => "ÿ",
        (Lang::Es, 'a') => "á",
        (Lang::Es, 'e') => "é",
        (Lang::Es, 'i') => "í",
        (Lang::Es, 'o') => "ó",
        (Lang::Es, 'u') => "úü",
        (Lang::Pt, 'a') => "ãáâàª",
        (Lang::Pt, 'e') => "éê",
        (Lang::Pt, 'i') => "í",
        (Lang::Pt, 'o') => "õóôº",
        (Lang::Pt, 'u') => "úü",
        _ => "",
    }
}

/// A key's long-press choices: the first (a digit on the top row, else the
/// likeliest accent) and the rest.
fn key_alts(lang: Lang, action: Action, alt: Option<char>) -> (Option<char>, &'static str) {
    let acc = match action {
        Action::Char(c) => accents(lang, c),
        _ => "",
    };
    match (alt, acc.chars().next()) {
        (_, None) => (alt, more_alts(lang, action)),
        (Some(digit), Some(_)) => (Some(digit), acc),
        (None, Some(first)) => (Some(first), &acc[first.len_utf8()..]),
    }
}

fn chars(s: &str, alts: &str) -> Row {
    let mut alts = alts.chars();
    s.chars()
        .map(|c| (1.0, Action::Char(c), alts.next().filter(|&a| a != ' ')))
        .collect()
}

fn scaled(row: Row, k: f32) -> Row {
    row.into_iter().map(|(w, a, alt)| (w * k, a, alt)).collect()
}

fn with(mut row: Row, left: (f32, Action), right: (f32, Action)) -> Row {
    row.insert(0, (left.0, left.1, None));
    row.push((right.0, right.1, None));
    row
}

/// Bottom row, in units of a `grid`-wide row. Without the language key the
/// space bar takes its place.
fn bottom(grid: f32, layer_key: Action, lang_key: bool) -> Row {
    let side = if grid > 10.0 { 1.25 } else { 1.0 };
    let edge = (grid - 4.0 - 3.0 * side) / 2.0;
    let mut row = vec![(edge, layer_key, None)];
    if lang_key {
        row.push((side, Action::Lang, None));
    }
    row.extend([
        (side, Action::Char(','), Some(EMOJI_KEY)),
        (if lang_key { 4.0 } else { 4.0 + side }, Action::Space, None),
        (side, Action::Char('.'), Some('!')),
        (edge, Action::Enter, None),
    ]);
    row
}

/// Rows (in key-width units) and the grid width they're laid out on.
fn rows(lang: Lang, layer: Layer, lang_key: bool) -> (f32, Vec<Row>) {
    match (layer, lang) {
        (Layer::Letters, Lang::Ru) => (
            11.0,
            vec![
                chars("йцукенгшщзх", "1234ё67890 "),
                chars("фывапролджэ", ""),
                // Narrower bottom letters leave room for wider ⇧ and ⌫ (as on Gboard).
                with(
                    scaled(chars("ячсмитьбю", "      ъ  "), 0.9),
                    (1.45, Action::Shift),
                    (1.45, Action::Backspace),
                ),
                bottom(11.0, Action::Symbols, lang_key),
            ],
        ),
        (Layer::Letters, _) => {
            // QWERTY and its kin: 10 keys a row (11 for QWERTZ), ⇧ and ⌫
            // share what the bottom row leaves.
            let [top, home, low] = lang.latin().rows();
            let grid = top.chars().count().max(home.chars().count()) as f32;
            let side = (grid - low.chars().count() as f32) / 2.0;
            (
                grid,
                vec![
                    chars(top, "1234567890 "),
                    chars(home, ""),
                    with(
                        chars(low, ""),
                        (side, Action::Shift),
                        (side, Action::Backspace),
                    ),
                    bottom(grid, Action::Symbols, lang_key),
                ],
            )
        }
        (Layer::Symbols, _) => (
            10.0,
            vec![
                chars("1234567890", ""),
                chars(&format!("@#{}_&-+()/", lang.currency()), ""),
                with(
                    chars("*\"':;!?", ""),
                    (1.5, Action::Symbols2),
                    (1.5, Action::Backspace),
                ),
                bottom(10.0, Action::Letters, lang_key),
            ],
        ),
        (Layer::Symbols2, _) => (
            10.0,
            vec![
                chars("~`|•√π÷×¶∆", ""),
                // The other currencies (the language's own is on page one).
                chars(&"£€¥$^°={}\\".replace(lang.currency(), "₽"), ""),
                with(
                    chars("%[]<>©™", ""),
                    (1.5, Action::Symbols),
                    (1.5, Action::Backspace),
                ),
                bottom(10.0, Action::Letters, lang_key),
            ],
        ),
    }
}

/// Letters of each column (top to bottom) for the one-row layout, and the one
/// typed to stand for the column (its home-row letter, else the top one).
/// Rows are assigned to columns by x on the standard phone grid.
pub fn columns(lang: Lang) -> Vec<(Vec<char>, char)> {
    let (grid, offsets, rows): (usize, [f32; 3], [&str; 3]) = match lang {
        Lang::Ru => (
            11,
            [0.0, 0.0, 1.0],
            ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"],
        ),
        _ => {
            let rows = lang.latin().rows();
            let grid = rows.iter().map(|r| r.chars().count()).max().unwrap_or(10);
            (grid, lang.latin().phone_offsets(), rows)
        }
    };
    let mut cols: Vec<Vec<char>> = vec![Vec::new(); grid];
    let mut stand: Vec<Option<char>> = vec![None; grid];
    for (r, row) in rows.iter().enumerate() {
        for (i, c) in row.chars().enumerate() {
            // Key center x in key widths; ties go to the left column.
            let x = offsets[r] + i as f32 + 0.5;
            let col = ((x - 0.5).ceil() as usize).min(grid - 1);
            let col = if (x - (col as f32 + 0.5)).abs() >= 0.5 && col > 0 {
                col - 1
            } else {
                col
            };
            cols[col].push(c);
            if r == 1 || stand[col].is_none() {
                stand[col] = Some(c);
            }
        }
    }
    cols.into_iter()
        .zip(stand)
        .map(|(l, s)| (l, s.unwrap_or(' ')))
        .collect()
}

/// The one-row layout: a row of column keys over a compact bottom row.
pub fn build_compact(lang: Lang, width: f32, top: f32, row_h: f32, lang_key: bool) -> Vec<Key> {
    let cols = columns(lang);
    let grid = cols.len() as f32;
    let unit = width / grid;
    let mut keys: Vec<Key> = (0..cols.len())
        .map(|i| Key {
            action: Action::Column(i as u8),
            alt: None,
            more: "",
            angle: 0.0,
            x: i as f32 * unit,
            y: top,
            w: unit,
            h: row_h,
        })
        .collect();
    let bottom: Row = if grid > 10.0 {
        vec![
            (1.5, Action::Symbols, None),
            (1.25, Action::Lang, None),
            (1.0, Action::Char(','), Some(EMOJI_KEY)),
            (3.5, Action::Space, None),
            (1.0, Action::Char('.'), Some('!')),
            (1.5, Action::Backspace, None),
            (1.25, Action::Enter, None),
        ]
    } else {
        vec![
            (1.25, Action::Symbols, None),
            (1.0, Action::Lang, None),
            (1.0, Action::Char(','), Some(EMOJI_KEY)),
            (3.0, Action::Space, None),
            (1.0, Action::Char('.'), Some('!')),
            (1.5, Action::Backspace, None),
            (1.25, Action::Enter, None),
        ]
    };
    let bottom: Row = if lang_key {
        bottom
    } else {
        // The language key's width goes to the space bar.
        let lang_w: f32 = bottom
            .iter()
            .filter(|k| k.1 == Action::Lang)
            .map(|k| k.0)
            .sum();
        bottom
            .into_iter()
            .filter(|k| k.1 != Action::Lang)
            .map(|(w, a, alt)| {
                if a == Action::Space {
                    (w + lang_w, a, alt)
                } else {
                    (w, a, alt)
                }
            })
            .collect()
    };
    let mut x = 0.0;
    for (units, action, alt) in bottom {
        let (alt, more) = key_alts(lang, action, alt);
        keys.push(Key {
            action,
            alt,
            more,
            x,
            y: top + row_h,
            w: units * unit,
            h: row_h,
            angle: 0.0,
        });
        x += units * unit;
    }
    keys
}

/// The emoji panel's bottom row at `top`: letters, page back, space, page
/// on, ⌫, Enter.
pub fn build_panel_row(width: f32, top: f32, row_h: f32) -> Vec<Key> {
    let row = [
        (1.5, Action::Letters),
        (1.0, Action::EmojiSearch),
        (1.0, Action::Page(-1)),
        (3.0, Action::Space),
        (1.0, Action::Page(1)),
        (1.5, Action::Backspace),
        (1.0, Action::Enter),
    ];
    let unit = width / row.iter().map(|k| k.0).sum::<f32>();
    let mut x = 0.0;
    row.iter()
        .map(|&(units, action)| {
            let key = Key {
                action,
                alt: None,
                more: "",
                x,
                y: top,
                w: units * unit,
                h: row_h,
                angle: 0.0,
            };
            x += units * unit;
            key
        })
        .collect()
}

/// Lay the keys out on a `width`-wide area, rows of `row_h` starting at `top`.
/// Rows narrower than the grid are centered (QWERTY's home row).
pub fn build(
    lang: Lang,
    layer: Layer,
    width: f32,
    top: f32,
    row_h: f32,
    lang_key: bool,
) -> Vec<Key> {
    let (grid, rows) = rows(lang, layer, lang_key);
    let unit = width / grid;
    let mut keys = Vec::new();
    for (r, row) in rows.into_iter().enumerate() {
        let used: f32 = row.iter().map(|k| k.0).sum();
        let mut x = (grid - used) / 2.0 * unit;
        let y = top + r as f32 * row_h;
        for (units, action, alt) in row {
            let w = units * unit;
            let (alt, more) = key_alts(lang, action, alt);
            keys.push(Key {
                action,
                alt,
                more,
                x,
                y,
                w,
                h: row_h,
                angle: 0.0,
            });
            x += w;
        }
    }
    keys
}

/// The one-handed layout: the usual rows, each fitted into the stretch the
/// thumb reaches at its height — `span(row)`, px from the left (the painted
/// area of the calibration). A row never gets narrower than
/// [`MIN_REACH_ROW`] of the width.
#[allow(clippy::too_many_arguments)]
pub fn build_reach(
    lang: Lang,
    layer: Layer,
    width: f32,
    top: f32,
    row_h: f32,
    lang_key: bool,
    span: impl Fn(usize) -> (f32, f32),
) -> Vec<Key> {
    let mut keys = build(lang, layer, width, top, row_h, lang_key);
    for k in &mut keys {
        let row = ((k.y - top) / row_h).round().max(0.0) as usize;
        let (mut from, mut to) = span(row);
        let min = MIN_REACH_ROW * width;
        if to - from < min {
            let mid = ((from + to) / 2.0).clamp(min / 2.0, width - min / 2.0);
            (from, to) = (mid - min / 2.0, mid + min / 2.0);
        }
        let (from, to) = (from.max(0.0), to.min(width));
        let f = (to - from) / width;
        k.x = from + k.x * f;
        k.w *= f;
    }
    keys
}

/// The narrowest a row of the one-handed layout gets, part of the width.
pub const MIN_REACH_ROW: f32 = 0.55;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_handed_rows_fit_the_stretch_reached() {
        let (w, top, h) = (1080.0, 100.0, 140.0);
        let spans = [
            (300.0, 1000.0),
            (200.0, 1030.0),
            (100.0, 1060.0),
            (0.0, 1080.0),
        ];
        let keys = build_reach(Lang::Ru, Layer::Letters, w, top, h, true, |r| spans[r]);
        for (r, &(from, to)) in spans.iter().enumerate() {
            let row: Vec<&Key> = keys
                .iter()
                .filter(|k| ((k.y - top) / h).round() as usize == r)
                .collect();
            let lo = row.iter().map(|k| k.x).fold(f32::MAX, f32::min);
            let hi = row.iter().map(|k| k.x + k.w).fold(0.0f32, f32::max);
            assert!(lo >= from - 0.5 && hi <= to + 0.5, "row {r}: {lo}..{hi}");
        }
        // Too narrow a stretch still leaves usable keys.
        let keys = build_reach(Lang::Ru, Layer::Letters, w, top, h, true, |_| {
            (500.0, 600.0)
        });
        let hi = keys.iter().map(|k| k.x + k.w).fold(0.0f32, f32::max);
        let lo = keys.iter().map(|k| k.x).fold(f32::MAX, f32::min);
        assert!(hi - lo >= MIN_REACH_ROW * w - 1.0);
    }

    #[test]
    fn each_language_has_its_letters_and_accents() {
        let keys = build(Lang::De, Layer::Letters, 1100.0, 0.0, 100.0, true);
        let has = |c| keys.iter().any(|k| k.action == Action::Char(c));
        assert!(has('ü') && has('ö') && has('ä') && has('z'));
        let s = keys.iter().find(|k| k.action == Action::Char('s')).unwrap();
        assert_eq!(s.alt, Some('ß'));
        let fr = build(Lang::Fr, Layer::Letters, 1000.0, 0.0, 100.0, true);
        let e = fr.iter().find(|k| k.action == Action::Char('e')).unwrap();
        assert_eq!((e.alt, e.more), (Some('3'), "éèêë"), "the digit first");
        assert!(fr.iter().any(|k| k.action == Action::Char('\'')));
        let pt = build(Lang::Pt, Layer::Letters, 1000.0, 0.0, 100.0, true);
        let a = pt.iter().find(|k| k.action == Action::Char('a')).unwrap();
        assert_eq!((a.alt, a.more), (Some('ã'), "áâàª"));
        assert!(pt.iter().any(|k| k.action == Action::Char('ç')));
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
    }

    #[test]
    fn rows_fill_the_width() {
        for lang in Lang::ALL {
            for layer in [Layer::Letters, Layer::Symbols, Layer::Symbols2] {
                let keys = build(lang, layer, 1100.0, 0.0, 100.0, true);
                for r in 0..4 {
                    let row: Vec<&Key> = keys.iter().filter(|k| k.y == r as f32 * 100.0).collect();
                    let right = row.iter().map(|k| k.x + k.w).fold(0.0, f32::max);
                    let left = row.iter().map(|k| k.x).fold(f32::INFINITY, f32::min);
                    // Full rows span the width; the QWERTY home row is centered.
                    assert!(
                        (left + right - 1100.0).abs() < 0.5,
                        "{lang:?} {layer:?} row {r}"
                    );
                }
            }
        }
    }

    #[test]
    fn one_row_columns() {
        let ru = columns(Lang::Ru);
        assert_eq!(ru.len(), 11);
        assert_eq!(ru[0], (vec!['й', 'ф'], 'ф'));
        assert_eq!(ru[3], (vec!['к', 'а', 'с'], 'а'));
        assert_eq!(ru.iter().map(|c| c.0.len()).sum::<usize>(), 31);
        let en = columns(Lang::En);
        assert_eq!(en[0], (vec!['q', 'a'], 'a'));
        assert_eq!(en[9], (vec!['p'], 'p'));
        assert_eq!(en.iter().map(|c| c.0.len()).sum::<usize>(), 26);
        let keys = build_compact(Lang::Ru, 1100.0, 0.0, 100.0, true);
        let right = keys
            .iter()
            .filter(|k| k.y == 100.0)
            .map(|k| k.x + k.w)
            .fold(0.0, f32::max);
        assert!((right - 1100.0).abs() < 0.5);
    }

    #[test]
    fn without_the_language_key_space_fills_the_row() {
        for compact in [false, true] {
            let keys = if compact {
                build_compact(Lang::Ru, 1100.0, 0.0, 100.0, false)
            } else {
                build(Lang::Ru, Layer::Letters, 1100.0, 0.0, 100.0, false)
            };
            assert!(keys.iter().all(|k| k.action != Action::Lang));
            let y = keys.iter().find(|k| k.action == Action::Space).unwrap().y;
            let right = keys
                .iter()
                .filter(|k| k.y == y)
                .map(|k| k.x + k.w)
                .fold(0.0, f32::max);
            assert!((right - 1100.0).abs() < 0.5);
        }
    }

    #[test]
    fn long_press_alternatives() {
        let keys = build(Lang::Ru, Layer::Letters, 1100.0, 0.0, 100.0, true);
        let alt = |c| {
            keys.iter()
                .find(|k| k.action == Action::Char(c))
                .unwrap()
                .alt
        };
        assert_eq!(alt('е'), Some('ё'));
        assert_eq!(alt('ь'), Some('ъ'));
        assert_eq!(alt('й'), Some('1'));
        assert_eq!(alt('х'), None);
    }
}
