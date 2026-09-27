//! Single-byte alphabet remapping.
//!
//! Words are stored in the FST as sequences of 1-byte ids instead of raw UTF-8.
//! This keeps the weighted edit-distance search *character*-aligned (one FST
//! transition == one logical character), which is what the cost model needs —
//! Cyrillic letters are 2 bytes in UTF-8 and would otherwise split across
//! several transitions.
//!
//! Ids are assigned in ascending Unicode codepoint order, so a codepoint-sorted
//! word list (our `words.txt`, `LC_ALL=C`) stays sorted after remapping and can
//! be fed straight into the FST builder.

/// Canonical, ordered set of supported characters (ascending codepoint).
pub const CHARSET: &[char] = &[
    '\'', '-', // 0x27, 0x2D
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', // 0x61..0x7A
    // The Latin-1 letters (German, French, Spanish, Portuguese, …).
    'ß', 'à', 'á', 'â', 'ã', 'ä', 'å', 'æ', 'ç', 'è', 'é', 'ê', 'ë', 'ì', 'í', 'î', 'ï', 'ð', 'ñ',
    'ò', 'ó', 'ô', 'õ', 'ö', // 0xDF..0xF6
    'ø', 'ù', 'ú', 'û', 'ü', 'ý', 'þ', 'ÿ', // 0xF8..0xFF (÷ is not a letter)
    'œ', // 0x153
    'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'й', 'к', 'л', 'м', 'н', 'о', 'п', 'р', 'с', 'т',
    'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ъ', 'ы', 'ь', 'э', 'ю', 'я', // 0x430..0x44F
    'ё', // 0x451 (sorts AFTER я, matching byte order)
];

/// Map a character to its 1-byte id (`1..=CHARSET.len()`), or `None` if unsupported.
#[inline]
pub fn char_to_id(c: char) -> Option<u8> {
    let id = match c {
        '\'' => 1,
        '-' => 2,
        'a'..='z' => 3 + (c as u32 - 'a' as u32),
        'ß'..='ö' => 29 + (c as u32 - 'ß' as u32),
        'ø'..='ÿ' => 53 + (c as u32 - 'ø' as u32),
        'œ' => 61,
        'а'..='я' => 62 + (c as u32 - 'а' as u32),
        'ё' => 94,
        _ => return None,
    };
    Some(id as u8)
}

/// Inverse of [`char_to_id`]. Panics on an out-of-range id (a corrupt index).
#[inline]
pub fn id_to_char(id: u8) -> char {
    CHARSET[id as usize - 1]
}

/// Decode a slice of ids back into a word.
pub fn ids_to_string(ids: &[u8]) -> String {
    ids.iter().map(|&id| id_to_char(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all() {
        for (i, &c) in CHARSET.iter().enumerate() {
            let id = char_to_id(c).unwrap();
            assert_eq!(id as usize, i + 1, "id order mismatch for {c:?}");
            assert_eq!(id_to_char(id), c);
        }
    }

    #[test]
    fn ids_are_codepoint_monotonic() {
        // Guarantees a codepoint-sorted word list stays sorted after remapping.
        let mut prev = 0u8;
        for &c in CHARSET {
            let id = char_to_id(c).unwrap();
            assert!(
                id > prev,
                "ids must be strictly increasing in codepoint order"
            );
            prev = id;
        }
    }

    #[test]
    fn unsupported_chars() {
        assert_eq!(char_to_id('1'), None);
        assert_eq!(char_to_id(' '), None);
        assert_eq!(char_to_id('÷'), None);
        assert_eq!(char_to_id('ł'), None);
        assert_eq!(char_to_id('É'), None, "lowercase only");
    }
}
