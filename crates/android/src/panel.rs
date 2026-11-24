//! The emoji and clipboard panels take the letters' place: emoji tabs and a
//! page in the strip, or a separate clipboard with its own header. Emoji open
//! by holding the comma; the clipboard opens from its strip button.
//!
//! Nothing of it is kept: the recent emoji and the copied texts live in
//! memory only, while the keyboard runs. The emoji list ships inside the
//! library (`emoji.txt`, from Unicode's emoji-test.txt by tools/emoji.py) and
//! is read the first time the panel opens; only the rows visible in the
//! current scroll position are drawn.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::layout::Lang;

static LIST: &str = include_str!("emoji.txt");
/// Each emoji's name and keywords, lowercase, ё as е (Unicode CLDR
/// annotations, Unicode License v3; tools/emoji_names.py).
static NAMES: [&str; 6] = [
    include_str!("emoji_names/ru.txt"),
    include_str!("emoji_names/en.txt"),
    include_str!("emoji_names/de.txt"),
    include_str!("emoji_names/fr.txt"),
    include_str!("emoji_names/es.txt"),
    include_str!("emoji_names/pt.txt"),
];
/// Emoji found for a word, at most (a page).
const FOUND: usize = 32;

/// Emoji groups, in Unicode's order, and each one's tab.
pub const GROUPS: usize = 9;
const GROUP_ICONS: [&str; GROUPS] = ["😀", "👋", "🐻", "🍔", "🚗", "⚽", "💡", "🔣", "🏁"];

/// Emoji tabs: the recent ones, then the groups. The clipboard is a separate panel.
pub const TABS: usize = GROUPS + 1;
/// Emoji on a page, and copied texts.
pub const COLS: usize = 8;
pub const ROWS: usize = 4;
pub const CLIP_ROWS: usize = 4;
/// How many recent emoji and copied texts are remembered, and the longest
/// copied text kept (chars).
const RECENT: usize = 32;
const CLIPS: usize = 12;
pub const CLIP_MAX: usize = 50_000;

/// The newest Emoji version (×10) the font of this Android version draws.
pub fn emoji_max(sdk: i32) -> u16 {
    match sdk {
        i32::MIN..=0 => u16::MAX, // unknown (tests): all
        1..=27 => 50,
        28 => 110,
        29 => 120,
        30 => 130,
        31 | 32 => 131,
        33 => 140,
        34 => 150,
        35 => 151,
        _ => 160,
    }
}

/// Each group's emoji the phone can draw (`max`: see [`emoji_max`]), read
/// from the list the first time.
fn groups(max: u16) -> &'static [Vec<&'static str>] {
    static CELL: OnceLock<Vec<Vec<&'static str>>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut out: Vec<Vec<&'static str>> = Vec::with_capacity(GROUPS);
        for line in LIST.lines() {
            if line.starts_with('@') {
                out.push(Vec::new());
            } else if let (Some(group), Some((v, e))) = (out.last_mut(), line.split_once('\t')) {
                if v.parse::<u16>().is_ok_and(|v| v <= max) {
                    group.push(e);
                }
            }
        }
        out
    })
}

/// The emoji whose name or keywords hold `query` in `lang` (the phone can
/// draw them: `max`): a name that is just the word first, then a keyword
/// that is, then a name or keyword with that word in it, then one with a
/// word it begins; a word in another form («кота», «котом») by its first
/// letters, when nothing else is found.
pub fn search(lang: Lang, query: &str, max: u16) -> Vec<&'static str> {
    let q = query.to_lowercase().replace('ё', "е");
    if q.chars().count() < 2 {
        return Vec::new();
    }
    let drawn: HashSet<&str> = groups(max).iter().flatten().copied().collect();
    let names = NAMES[match lang {
        Lang::Ru => 0,
        Lang::En => 1,
        Lang::De => 2,
        Lang::Fr => 3,
        Lang::Es => 4,
        Lang::Pt => 5,
    }];
    let score = |terms: &str, q: &str, stem: bool| -> Option<u8> {
        let mut total = 0u8;
        for query_word in q.split_whitespace() {
            let mut best: Option<u8> = None;
            for (k, term) in terms.split('|').enumerate() {
                let s = if term == query_word {
                    Some(if k == 0 { 0 } else { 1 })
                } else if term
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|w| w == query_word)
                {
                    Some(2)
                } else if term
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|w| w.len() > query_word.len() && w.starts_with(query_word))
                    && (stem || query_word.chars().count() >= 3)
                {
                    Some(3)
                } else {
                    None
                };
                best = match (best, s) {
                    (Some(b), Some(s)) => Some(b.min(s)),
                    (b, s) => b.or(s),
                };
            }
            total = total.saturating_add(best?);
        }
        Some(total)
    };
    let find = |q: &str, stem: bool| -> Vec<&'static str> {
        let mut hits: Vec<(u8, usize, &'static str)> = names
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let (e, terms) = line.split_once('\t')?;
                drawn.contains(e).then_some(())?;
                score(terms, q, stem).map(|s| (s, i, e))
            })
            .collect();
        hits.sort_unstable();
        hits.into_iter().map(|(_, _, e)| e).take(FOUND).collect()
    };
    let found = find(&q, false);
    if !found.is_empty() {
        return found;
    }
    // Another form of the word: its first letters (at least three).
    let chars: Vec<char> = q.chars().collect();
    for cut in 1..=2 {
        if chars.len() >= cut + 3 {
            let stem: String = chars[..chars.len() - cut].iter().collect();
            let found = find(&stem, true);
            if !found.is_empty() {
                return found;
            }
        }
    }
    Vec::new()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Clips,
    Recent,
    Group(u8),
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Tab::Clips | Tab::Recent => 0,
            Tab::Group(g) => 1 + g as usize,
        }
    }

    pub fn from_index(i: usize) -> Tab {
        match i {
            0 => Tab::Recent,
            i => Tab::Group((i - 1).min(GROUPS - 1) as u8),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tab::Clips => "📋",
            Tab::Recent => "🕘",
            Tab::Group(g) => GROUP_ICONS[g as usize],
        }
    }
}

/// Where a touch on the panel landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Close,
    Tab(u8),
    /// An emoji or a copied text on the page (its place on the page).
    Cell(u8),
    /// The ✕ that forgets a copied text.
    Remove(u8),
}

/// What the panel remembers — in memory only.
#[derive(Default)]
pub struct Stash {
    /// Emoji typed from the panel, the newest first…
    pub recent: Vec<String>,
    /// …and those typed since it opened: into `recent` when it closes or
    /// the tab changes (the page doesn't shift under the finger).
    fresh: Vec<String>,
    /// Texts copied while the keyboard ran, the newest first.
    pub clips: Vec<String>,
    /// Emoji found for the word typed when the panel opened: shown in the
    /// recent ones' tab (🔍) while there are any.
    pub found: Vec<String>,
}

impl Stash {
    pub fn used(&mut self, emoji: &str) {
        self.fresh.retain(|e| e != emoji);
        self.fresh.push(emoji.to_string());
    }

    /// The emoji typed lately go first among the recent ones.
    pub fn settle(&mut self) {
        for e in std::mem::take(&mut self.fresh) {
            self.recent.retain(|r| *r != e);
            self.recent.insert(0, e);
        }
        self.recent.truncate(RECENT);
    }

    /// A text was copied: first in the list (once).
    pub fn copied(&mut self, text: &str) {
        if text.trim().is_empty() || text.chars().count() > CLIP_MAX {
            return;
        }
        self.clips.retain(|c| c != text);
        self.clips.insert(0, text.to_string());
        self.clips.truncate(CLIPS);
    }
}

/// The open panel: which tab, which page of it, and how many emoji rows are
/// scrolled above the visible grid (a fraction while a finger drags).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub tab: Tab,
    pub page: usize,
    pub scroll: f32,
}

impl Panel {
    pub fn new(stash: &Stash) -> Panel {
        Panel {
            tab: if stash.recent.is_empty() {
                Tab::Group(0)
            } else {
                Tab::Recent
            },
            page: 0,
            scroll: 0.0,
        }
    }

    pub fn clipboard() -> Panel {
        Panel {
            tab: Tab::Clips,
            page: 0,
            scroll: 0.0,
        }
    }

    /// The tab's items: copied texts or emoji.
    pub fn items(tab: Tab, stash: &Stash, max: u16) -> Vec<&str> {
        match tab {
            Tab::Clips => stash.clips.iter().map(String::as_str).collect(),
            Tab::Recent if !stash.found.is_empty() => {
                stash.found.iter().map(String::as_str).collect()
            }
            Tab::Recent => stash.recent.iter().map(String::as_str).collect(),
            Tab::Group(g) => groups(max)
                .get(g as usize)
                .map_or_else(Vec::new, |v| v.clone()),
        }
    }

    pub fn per_page(tab: Tab) -> usize {
        match tab {
            Tab::Clips => CLIP_ROWS,
            _ => COLS * ROWS,
        }
    }

    pub fn pages(tab: Tab, stash: &Stash, max: u16) -> usize {
        Self::items(tab, stash, max)
            .len()
            .div_ceil(Self::per_page(tab))
            .max(1)
    }

    /// The items of the page shown, with their places on it.
    pub fn page_items<'a>(&self, stash: &'a Stash, max: u16) -> Vec<&'a str> {
        let per = Self::per_page(self.tab);
        Self::items(self.tab, stash, max)
            .into_iter()
            .skip(self.page * per)
            .take(per)
            .collect()
    }

    /// The copied texts on the page, or the emoji rows that can touch the
    /// visible grid at the current fractional scroll.
    pub fn visible_items<'a>(&self, stash: &'a Stash, max: u16) -> Vec<&'a str> {
        if self.tab == Tab::Clips {
            return self.page_items(stash, max);
        }
        Self::items(self.tab, stash, max)
            .into_iter()
            .skip(self.scroll.floor() as usize * COLS)
            .take((ROWS + 1) * COLS)
            .collect()
    }

    /// Drag the emoji grid vertically; `rows` is positive when content moves up.
    pub fn scroll_by(&mut self, rows: f32, stash: &Stash, max: u16) {
        if self.tab == Tab::Clips {
            return;
        }
        let total = Self::items(self.tab, stash, max).len().div_ceil(COLS);
        self.scroll = (self.scroll + rows).clamp(0.0, total.saturating_sub(ROWS) as f32);
        self.page = self.scroll as usize / ROWS;
    }

    /// On to the next page (`step` 1) or back (-1): past a tab's last page
    /// into the next tab, as if they were one long strip.
    pub fn flip(&mut self, step: i32, stash: &Stash, max: u16) {
        let pages = Self::pages(self.tab, stash, max);
        if self.tab == Tab::Clips {
            self.page = (self.page as i32 + step.signum()).clamp(0, pages as i32 - 1) as usize;
            self.scroll = 0.0;
            return;
        }
        if step > 0 {
            if self.page + 1 < pages {
                self.page += 1;
            } else if self.tab.index() + 1 < TABS {
                self.tab = Tab::from_index(self.tab.index() + 1);
                self.page = 0;
            }
        } else if self.page > 0 {
            self.page -= 1;
        } else if self.tab.index() > 0 {
            self.tab = Tab::from_index(self.tab.index() - 1);
            self.page = Self::pages(self.tab, stash, max) - 1;
        }
        self.scroll = (self.page * ROWS) as f32;
    }

    pub fn open_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.page = 0;
        self.scroll = 0.0;
    }

    pub fn selected(&self) -> usize {
        self.tab.index()
    }
}

/// The panel's shape on screen: the tabs over `tabs_h`, the page from there
/// down to `page_bottom`, `width` wide.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub width: f32,
    pub tabs_h: f32,
    pub page_bottom: f32,
}

impl Frame {
    pub fn tab_w(&self) -> f32 {
        self.width / TABS as f32
    }

    /// An emoji's cell: (x, y, w, h). `scroll` is the fractional first row.
    pub fn cell(&self, i: usize, scroll: f32) -> (f32, f32, f32, f32) {
        let (w, h) = (
            self.width / COLS as f32,
            (self.page_bottom - self.tabs_h) / ROWS as f32,
        );
        let (c, r) = (i % COLS, i / COLS);
        (c as f32 * w, self.tabs_h + (r as f32 - scroll) * h, w, h)
    }

    /// A copied text's row: (x, y, w, h); its ✕ is the last `h` of it.
    pub fn clip_row(&self, i: usize) -> (f32, f32, f32, f32) {
        let h = (self.page_bottom - self.tabs_h) / CLIP_ROWS as f32;
        (0.0, self.tabs_h + i as f32 * h, self.width, h)
    }

    pub fn hit(&self, tab: Tab, x: f32, y: f32, scroll: f32) -> Option<Hit> {
        if y < 0.0 || y >= self.page_bottom {
            return None;
        }
        if y < self.tabs_h {
            if tab == Tab::Clips {
                return Some(Hit::Close);
            }
            let i = (x / self.tab_w()).floor().clamp(0.0, (TABS - 1) as f32);
            return Some(Hit::Tab(i as u8));
        }
        if tab == Tab::Clips {
            let (_, _, _, h) = self.clip_row(0);
            let r = (((y - self.tabs_h) / h).floor() as usize).min(CLIP_ROWS - 1) as u8;
            return Some(if x > self.width - h * 1.2 {
                Hit::Remove(r)
            } else {
                Hit::Cell(r)
            });
        }
        let (_, _, w, h) = self.cell(0, 0.0);
        let c = ((x / w).floor() as usize).min(COLS - 1);
        let first = scroll.floor();
        let r = ((y - self.tabs_h) / h + scroll).floor().max(first) as usize;
        Some(Hit::Cell((r * COLS + c - first as usize * COLS) as u8))
    }
}

/// A copied text on one line of about `chars` chars: its line breaks and
/// runs of spaces as single spaces, the end cut with «…».
pub fn preview(text: &str, chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= chars {
        return flat;
    }
    let cut: String = flat.chars().take(chars.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_reads_in_groups_and_by_version() {
        let all = groups(u16::MAX);
        assert_eq!(all.len(), GROUPS);
        assert_eq!(all[0][0], "😀");
        assert!(all.iter().all(|g| !g.is_empty()));
        // No skin tones: the base forms only.
        assert!(!all[1].iter().any(|e| e.contains('\u{1F3FB}')));
        let n: usize = all.iter().map(Vec::len).sum();
        assert!((1800..2100).contains(&n), "{n}");
    }

    #[test]
    fn pages_run_on_into_the_next_tab() {
        let mut stash = Stash::default();
        stash.used("🙂");
        stash.used("🙃");
        assert!(stash.recent.is_empty());
        stash.settle();
        assert_eq!(stash.recent, vec!["🙃", "🙂"]);
        let mut p = Panel::new(&stash);
        assert_eq!(p.tab, Tab::Recent);
        p.flip(1, &stash, u16::MAX);
        assert_eq!((p.tab, p.page), (Tab::Group(0), 0));
        p.flip(-1, &stash, u16::MAX);
        assert_eq!((p.tab, p.page), (Tab::Recent, 0));
        p.flip(-1, &stash, u16::MAX);
        assert_eq!((p.tab, p.page), (Tab::Recent, 0));
        let mut clips = Panel::clipboard();
        clips.flip(1, &stash, u16::MAX);
        assert_eq!((clips.tab, clips.page), (Tab::Clips, 0));
        // Back from a group: the last page of the one before.
        p.open_tab(Tab::Group(1));
        p.flip(-1, &stash, u16::MAX);
        let last = Panel::pages(Tab::Group(0), &stash, u16::MAX) - 1;
        assert_eq!((p.tab, p.page), (Tab::Group(0), last));
        assert!(last >= 2);
    }

    #[test]
    fn emoji_are_found_by_their_names() {
        let ru = search(Lang::Ru, "кот", u16::MAX);
        assert!(ru.contains(&"😺"), "{ru:?}");
        assert!(search(Lang::Ru, "огонь", u16::MAX).contains(&"🔥"));
        assert!(search(Lang::Ru, "россия", u16::MAX).contains(&"🇷🇺"));
        // Another form of the word, by its first letters.
        assert!(search(Lang::Ru, "котом", u16::MAX).contains(&"😺"));
        assert!(search(Lang::Ru, "черный кот", u16::MAX).contains(&"🐈‍⬛"));
        assert!(search(Lang::En, "cat", u16::MAX).contains(&"🐈"));
        assert!(search(Lang::Ru, "к", u16::MAX).is_empty());
        assert!(search(Lang::Ru, "фывапр", u16::MAX).is_empty());
    }

    #[test]
    fn emoji_scroll_by_fractional_rows_and_keep_their_hit_places() {
        let stash = Stash::default();
        let mut p = Panel::new(&stash);
        p.open_tab(Tab::Group(0));
        p.scroll_by(1.25, &stash, u16::MAX);
        assert_eq!(p.scroll, 1.25);
        assert_eq!(
            p.visible_items(&stash, u16::MAX)[0],
            Panel::items(p.tab, &stash, u16::MAX)[COLS]
        );
        let frame = Frame {
            width: 800.0,
            tabs_h: 40.0,
            page_bottom: 400.0,
        };
        let (x, y, w, h) = frame.cell(0, p.scroll.fract());
        assert_eq!(
            frame.hit(p.tab, x + w / 2.0, y + h / 2.0, p.scroll),
            Some(Hit::Cell(0))
        );
        p.scroll_by(-100.0, &stash, u16::MAX);
        assert_eq!(p.scroll, 0.0);
        p.scroll_by(1000.0, &stash, u16::MAX);
        assert!(p.scroll > 1.0);
        assert!(!p.visible_items(&stash, u16::MAX).is_empty());
        p.flip(1, &stash, u16::MAX);
        assert_eq!(p.scroll, (p.page * ROWS) as f32);
    }

    #[test]
    fn copies_are_remembered_newest_first_once() {
        let mut s = Stash::default();
        s.copied("один");
        s.copied("два");
        s.copied("один");
        s.copied("   ");
        assert_eq!(s.clips, vec!["один", "два"]);
        assert_eq!(preview("очень\nдлинный   текст", 12), "очень длинн…");
    }
}
