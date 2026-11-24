//! Conservative recognition of words a finite dictionary cannot list: productive
//! Russian derivation, compounds and Cyrillic spellings of English words.

const PREFIXES: &[&str] = &[
    "недо",
    "пере",
    "предо",
    "псевдо",
    "мульти",
    "контр",
    "анти",
    "архи",
    "гипер",
    "макро",
    "микро",
    "пост",
    "супер",
    "ультра",
    "интер",
    "мини",
    "полу",
    "экс",
    "наи",
    "над",
    "низ",
    "нис",
    "обо",
    "ото",
    "подо",
    "пред",
    "при",
    "раз",
    "рас",
    "воз",
    "вос",
    "без",
    "бес",
    "вы",
    "до",
    "за",
    "из",
    "ис",
    "на",
    "не",
    "об",
    "от",
    "под",
    "по",
    "про",
    "со",
    "во",
    "вз",
    "вс",
    "о",
    "с",
    "у",
    "в",
    "к",
];

const SUFFIXES: &[&str] = &[
    "изировать",
    "фицировать",
    "ировать",
    "ывать",
    "ивать",
    "овать",
    "евать",
    "ование",
    "ирование",
    "ения",
    "ания",
    "ение",
    "ание",
    "овый",
    "евый",
    "ивый",
    "ический",
    "ически",
    "ическое",
    "ические",
    "ического",
    "ическому",
    "ическим",
    "ических",
    "ский",
    "ская",
    "ское",
    "ские",
    "ского",
    "скому",
    "ским",
    "ских",
    "ный",
    "ная",
    "ное",
    "ные",
    "ного",
    "ному",
    "ным",
    "ных",
    "ость",
    "есть",
    "тель",
    "щик",
    "чик",
    "ник",
    "ист",
    "изм",
    "онок",
    "енок",
    "инка",
    "овка",
    "евка",
    "ика",
    "ок",
    "ек",
    "ик",
    "ка",
    "ун",
    "ух",
    "уш",
    "ыш",
    "арь",
    "яр",
    "ач",
    "ить",
    "ать",
    "ять",
    "нуть",
    "нуться",
    "иться",
    "аться",
    "ов",
    "ев",
    "ам",
    "ям",
    "ами",
    "ями",
    "ах",
    "ях",
    "ом",
    "ем",
    "ой",
    "ей",
    "а",
    "я",
    "и",
    "ы",
    "е",
    "у",
    "ю",
];

const COMBINING: &[&str] = &[
    "авто",
    "аэро",
    "аудио",
    "био",
    "бизнес",
    "видео",
    "гидро",
    "гео",
    "дев",
    "инфо",
    "интернет",
    "кибер",
    "макро",
    "медиа",
    "микро",
    "мульти",
    "нейро",
    "поли",
    "псевдо",
    "радио",
    "социо",
    "софт",
    "техно",
    "теле",
    "термо",
    "фото",
    "электро",
    "эконом",
];

fn variants(c: char, last: bool) -> &'static [&'static str] {
    match c {
        'а' => &["a"],
        'б' => &["b"],
        'в' => &["v"],
        'г' => &["g"],
        'д' => &["d"],
        'е' => &["e"],
        'ё' => &["yo", "e"],
        'ж' => &["zh"],
        'з' => &["z"],
        'и' => &["i", "e"],
        'й' => &["y", "i"],
        'к' => &["k", "c", "ck"],
        'л' => &["l"],
        'м' => &["m"],
        'н' => &["n"],
        'о' => &["o"],
        'п' => &["p"],
        'р' => &["r"],
        'с' if last => &["s", "c", "ce"],
        'с' => &["s", "c"],
        'т' => &["t"],
        'у' => &["u"],
        'ф' => &["f"],
        'х' => &["h", "kh"],
        'ц' => &["c", "ts"],
        'ч' => &["ch"],
        'ш' => &["sh"],
        'щ' => &["sch"],
        'ъ' => &[""],
        'ы' => &["y", "i"],
        'ь' => &[""],
        'э' => &["e"],
        'ю' => &["yu"],
        'я' => &["ya"],
        _ => &[],
    }
}

fn transliterations(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut out = vec![String::new()];
    for (i, c) in chars.iter().copied().enumerate() {
        let endings = variants(c, i + 1 == chars.len());
        if endings.is_empty() {
            return Vec::new();
        }
        let mut next = Vec::new();
        for prefix in &out {
            for ending in endings {
                next.push(format!("{prefix}{ending}"));
            }
        }
        next.sort_unstable();
        next.dedup();
        next.truncate(128);
        out = next;
    }
    out
}

fn transliterated(word: &str, known: impl Fn(&str) -> bool) -> bool {
    word.chars().count() >= 3 && transliterations(word).into_iter().any(|w| known(&w))
}

fn simple(word: &str, known: impl Fn(&str) -> bool + Copy) -> bool {
    known(word) || COMBINING.contains(&word) || transliterated(word, known)
}

fn direct(word: &str, known: impl Fn(&str) -> bool + Copy) -> bool {
    known(word)
        || COMBINING.contains(&word)
        || word.chars().count() >= 4 && transliterated(word, known)
}

fn compound(word: &str, known: impl Fn(&str) -> bool + Copy) -> bool {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 6 {
        return false;
    }
    for i in 3..=chars.len() - 3 {
        let left: String = chars[..i].iter().collect();
        let right: String = chars[i..].iter().collect();
        let left_ok = simple(&left, known)
            || ["о", "е"].iter().any(|vowel| {
                left.strip_suffix(vowel)
                    .is_some_and(|stem| simple(stem, known))
            });
        if !left_ok {
            continue;
        }
        if simple(&right, known)
            || SUFFIXES.iter().any(|suffix| {
                right
                    .strip_suffix(suffix)
                    .is_some_and(|stem| stem.chars().count() >= 3 && simple(stem, known))
            })
        {
            return true;
        }
    }
    false
}

fn base(word: &str, known: impl Fn(&str) -> bool + Copy, depth: usize) -> bool {
    if if depth == 0 {
        direct(word, known)
    } else {
        simple(word, known)
    } {
        return true;
    }
    if depth >= 2 || word.chars().count() < 4 {
        return false;
    }
    if PREFIXES.iter().any(|prefix| {
        word.strip_prefix(prefix)
            .is_some_and(|rest| rest.chars().count() >= 3 && base(rest, known, depth + 1))
    }) {
        return true;
    }
    if SUFFIXES.iter().any(|suffix| {
        word.strip_suffix(suffix)
            .is_some_and(|stem| stem.chars().count() >= 3 && base(stem, known, depth + 1))
    }) {
        return true;
    }
    compound(word, known)
}

/// Whether an unknown word has a supported derivation rather than being an
/// arbitrary string. Used to guard automatic replacement, not to manufacture
/// dictionary candidates or frequencies.
pub fn plausible(word: &str, known: impl Fn(&str) -> bool + Copy) -> bool {
    let lower = word.to_lowercase();
    if known(&lower) {
        return true;
    }
    if !lower
        .chars()
        .all(|c| "абвгдеёжзийклмнопрстуфхцчшщъыьэюя-".contains(c))
    {
        return false;
    }
    if lower.contains('-') {
        return lower
            .split('-')
            .all(|part| part.chars().count() >= 3 && simple(part, known));
    }
    base(&lower, known, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn productive_words_and_transliterations_survive() {
        let words = [
            "пар",
            "ход",
            "сеть",
            "кризис",
            "дом",
            "работница",
            "нейрон",
            "prompt",
            "docker",
            "chat",
            "cat",
            "kubernetes",
            "service",
        ];
        let known = |w: &str| words.contains(&w);
        assert_eq!(transliterations("докер"), vec!["docer", "docker", "doker"]);
        for word in [
            "пароход",
            "нейросеть",
            "антикризисный",
            "домработница",
            "промпт",
            "докеров",
            "чатика",
            "кубернетис",
            "сервис",
        ] {
            assert!(plausible(word, known), "{word}");
        }
        for word in ["фывапрол", "привет", "несуществующий", "кат", "blah"]
        {
            assert!(!plausible(word, known), "{word}");
        }
    }
}
