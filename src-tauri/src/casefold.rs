// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Unicode full case folding (default, non-Turkic: CaseFolding.txt C + F).
//!
//! Backed by ICU4X compiled data, which tracks the current Unicode version.
//! This replaces `unicode-casefold` 0.2.0 (Unicode 9.0 tables, last released
//! 2016). The only differences are 205 characters added since Unicode 9 that
//! the old tables left unfolded: 204 capital letters (Georgian Mtavruli,
//! Medefaidrin, Vithkuqi, Garay, Beria Erfe, Latin and Cyrillic additions)
//! and the small ligature U+1DF95, which now folds to `ss`. Callers keep their
//! own NFKC/NFD wrapping. Folded keys are computed locally and never exchanged
//! with peers; LocalSwarm and acquisition-path keys are also never stored, but
//! the spelling dictionary cache persists its folded words (`spelling.rs`,
//! `add_titles`). See docs/DEPENDENCIES.md for the compatibility analysis.

use icu_casemap::{CaseMapper, CaseMapperBorrowed};
use std::borrow::Cow;

/// Borrowed view of the baked data: built at compile time, no per-call setup.
const FOLDER: CaseMapperBorrowed<'static> = CaseMapper::new();

/// Full, context-insensitive case folding. Borrows when nothing changes.
pub(crate) fn fold(text: &str) -> Cow<'_, str> {
    FOLDER.fold_string(text)
}

#[cfg(test)]
mod tests {
    use super::fold;

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
            ("\u{A7C5}", "\u{282}"),                   // Latin S with hook (12.0)
            ("\u{10570}", "\u{10597}"),                 // Vithkuqi (14.0)
            ("\u{1C89}", "\u{1C8A}"),                   // Cyrillic Tje (16.0)
            ("\u{1DF95}", "ss"),                        // long s ligature (18.0)
        ] {
            assert_eq!(fold(input), folded, "{input:?}");
        }
        assert!(matches!(fold("already folded"), std::borrow::Cow::Borrowed(_)));
    }
}
