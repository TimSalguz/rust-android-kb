//! A finished sentence read whole: the marks its graph puts
//! ([`crate::marks`]) — where the keyboard may add the commas the text
//! lacks.

use super::Engine;
use crate::marks::{place, Mark, Placed};
use crate::parser::marks_of;

/// A word of a sentence's text: where it is (bytes) and the marks typed
/// before it (since the word before).
#[derive(Clone, Debug, PartialEq)]
pub struct Token<'a> {
    pub start: usize,
    pub end: usize,
    pub text: &'a str,
    pub before: String,
}

/// The words of `text` (letters, joined by inner hyphens and apostrophes:
/// «кто-то», «во-первых»; digits as words) and the marks between them.
pub fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut marks = String::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_alphanumeric() {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if d.is_alphanumeric() {
                    end = j + d.len_utf8();
                    chars.next();
                } else if matches!(d, '-' | '\'' | '’')
                    && text[j + d.len_utf8()..].chars().next().is_some_and(char::is_alphanumeric)
                {
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(Token {
                start: i,
                end,
                text: &text[i..end],
                before: std::mem::take(&mut marks),
            });
        } else if !c.is_whitespace() {
            marks.push(c);
        }
    }
    out
}

impl<D: AsRef<[u8]>> Engine<D> {
    /// The marks the graph of a whole sentence puts — before its words (the
    /// word's index in `words`) and at its end — `words` each with the marks
    /// typed before it. None without a parser of whole sentences (and the
    /// lemma vectors), or for a sentence longer than it reads.
    pub fn sentence_marks(&self, words: &[(&str, &str)]) -> Option<(Vec<Placed>, Mark)> {
        let parser = self.sentence_parser.as_ref()?;
        let lemmas = self.lemmas.as_ref()?;
        if words.is_empty() || words.len() > parser.places() {
            return None;
        }
        let lower: Vec<String> = words.iter().map(|(w, _)| w.to_lowercase()).collect();
        let read = lower
            .iter()
            .zip(words)
            .map(|(w, (_, m))| {
                let mut x = self.parser_word(w)?;
                x.marks = marks_of(m);
                Some(x)
            })
            .collect::<Option<Vec<_>>>()?;
        let readings: Vec<Vec<u64>> = lower.iter().map(|w| self.readings(w)).collect();
        let graph = parser.parse_full(lemmas, &read);
        Some(place(&lower, &readings, &graph, parser.relations()))
    }
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn words_and_the_marks_before_them() {
        let t = tokens("Ну, кто-то придёт — «наверное» в 5 часов");
        let words: Vec<&str> = t.iter().map(|t| t.text).collect();
        assert_eq!(words, ["Ну", "кто-то", "придёт", "наверное", "в", "5", "часов"]);
        let before: Vec<&str> = t.iter().map(|t| t.before.as_str()).collect();
        assert_eq!(before, ["", ",", "", "—«", "»", "", ""]);
        assert_eq!(&"Ну, кто-то"[t[1].start..t[1].end], "кто-то");
    }
}
