// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Full default (non-Turkic) Unicode 18 case folding for spelling and path keys.
//! Callers keep their own normalization form. The static tables store only changed
//! non-ASCII scalars, grouped by the high bytes of the Unicode scalar value.

use std::borrow::Cow;

mod data;

pub(crate) fn fold(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return if text.bytes().any(|byte| byte.is_ascii_uppercase()) {
            Cow::Owned(text.to_ascii_lowercase())
        } else {
            Cow::Borrowed(text)
        };
    }

    let mut output = String::new();
    let mut unchanged_start = 0;
    for (offset, ch) in text.char_indices() {
        if ch.is_ascii_uppercase() {
            output.push_str(&text[unchanged_start..offset]);
            output.push(ch.to_ascii_lowercase());
        } else {
            let scalar = u32::from(ch);
            let page = data::page(scalar >> 8);
            let Ok(index) = page.binary_search_by_key(&(scalar as u8), |entry| entry.0) else {
                continue;
            };
            output.push_str(&text[unchanged_start..offset]);
            output.push_str(page[index].1);
        }
        unchanged_start = offset + ch.len_utf8();
    }
    if unchanged_start == 0 {
        Cow::Borrowed(text)
    } else {
        output.push_str(&text[unchanged_start..]);
        Cow::Owned(output)
    }
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn matches_reference_for_every_scalar_and_adjacent_mappings() {
        let folder = icu_casemap::CaseMapper::new();
        let mut page = String::new();
        for ch in (0..=0x10ffff).filter_map(char::from_u32) {
            let mut bytes = [0; 4];
            let value = ch.encode_utf8(&mut bytes);
            assert_eq!(
                fold(value),
                folder.fold_string(value),
                "U+{:04X}",
                u32::from(ch)
            );
            page.push(ch);
            if page.len() >= 4096 {
                assert_eq!(fold(&page), folder.fold_string(&page));
                page.clear();
            }
        }
        assert_eq!(fold(&page), folder.fold_string(&page));
    }

    #[test]
    fn full_case_folding_is_pinned_for_representative_characters() {
        for (input, folded) in [
            ("Kiwix ZIM", "kiwix zim"),
            // Multi-character (F) mappings.
            ("Straße", "strasse"),
            ("\u{1E9E}", "ss"),
            ("\u{130}", "i\u{307}"),
            ("\u{149}", "\u{2BC}n"),
            ("\u{1F0}", "j\u{30C}"),
            ("\u{390}", "\u{3B9}\u{308}\u{301}"),
            ("\u{FB00}\u{FB06}", "ffst"),
            // Greek iota subscript and capital prosgegrammeni.
            ("\u{1FB3}\u{1FBC}", "\u{3B1}\u{3B9}\u{3B1}\u{3B9}"),
            ("\u{1F88}", "\u{1F00}\u{3B9}"),
            ("\u{1FB7}", "\u{3B1}\u{342}\u{3B9}"),
            ("\u{345}", "\u{3B9}"),
            // Sigma folds without final-form context; dotless i is unchanged.
            ("ΣΑΣ ς", "σασ σ"),
            ("\u{131}", "\u{131}"),
            // Compatibility capitals and Cherokee (folds to uppercase).
            ("\u{212A}\u{212B}", "k\u{E5}"),
            ("\u{AB70}", "\u{13A0}"),
            // Added after Unicode 9.0, unfolded by the replaced crate.
            ("\u{1C90}\u{1CBF}", "\u{10D0}\u{10FF}"), // Georgian Mtavruli (11.0)
            ("\u{A7C5}", "\u{282}"),                  // Latin S with hook (12.0)
            ("\u{10570}", "\u{10597}"),               // Vithkuqi (14.0)
            ("\u{1C89}", "\u{1C8A}"),                 // Cyrillic Tje (16.0)
            ("\u{1DF95}", "ss"),                      // long s ligature (18.0)
        ] {
            assert_eq!(fold(input), folded, "{input:?}");
        }
        assert!(matches!(
            fold("already folded"),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}
