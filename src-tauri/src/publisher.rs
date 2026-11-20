// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Display hints only. Publisher IDs, sessions and transfer authority are separate.
use crate::store::Settings;

pub(crate) const MAX_NAME_BYTES: usize = 160;

pub(crate) fn valid_name(value: &str) -> bool {
    value.len() <= MAX_NAME_BYTES && value.trim() == value
        && !value.chars().any(|c| c.is_control()
            || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}

pub(crate) fn name(settings: &Settings) -> &str {
    if !settings.sharing_name.is_empty() { return &settings.sharing_name; }
    // Do not populate this display hint from account, host or OS device nicknames.
    if cfg!(target_os = "ios") { "iPhone / iPad" }
    else if cfg!(target_os = "android") { "Android" }
    else if cfg!(target_os = "macos") { "Mac" }
    else if cfg!(target_os = "windows") { "Windows" }
    else if cfg!(target_os = "linux") { "Linux" }
    else { "Kiwix" }
}

pub(crate) fn advertised_name(value: Option<&str>) -> Result<Option<String>, ()> {
    match value {
        None => Ok(None),
        Some(value) if !value.is_empty() && valid_name(value) => Ok(Some(value.into())),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_preserve_unicode_without_controls_or_unbounded_txt() {
        for name in ["Jason’s iPhone", "図書館", "مكتبة", "é"] {
            assert_eq!(advertised_name(Some(name)), Ok(Some(name.into())));
        }
        assert!(valid_name("")); // Empty preference requests the platform default.
        assert_eq!(advertised_name(None), Ok(None)); // Older publishers have no hint.
        for name in ["", " padded", "trailing ", "two\nlines", "spoof\u{202e}", "\u{2066}name"] {
            assert!(advertised_name(Some(name)).is_err());
        }
        assert!(valid_name(&"é".repeat(80)));
        assert!(!valid_name(&"é".repeat(81)));
    }
}
