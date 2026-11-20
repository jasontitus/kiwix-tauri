//! "Look up in Kiwix": an external request to prefill a library search.
//!
//! Requests arrive as `--search TEXT` or as a `kiwix://search?q=TEXT` link
//! (`--open-url URL` on Windows, where the registered handler passes the link
//! as the only argument). Either form can only *prefill a search* in the running
//! window: it never opens files, changes settings, downloads or reaches the
//! network. The text is untrusted, so it is sanitised here before anything
//! queues it, and only the latest pending request is kept.
#![forbid(unsafe_code)]

use unicode_normalization::UnicodeNormalization;

/// Search queries never need more; Android caps platform entries at 4096, but a
/// desktop launch argument is easier to abuse, so the bound is tighter.
pub const MAX_QUERY_CHARS: usize = 512;
/// A link longer than this is rejected before parsing.
const MAX_URL_BYTES: usize = 8192;

/// Invisible, blank-rendering and reordering characters can hide part of a
/// query from the person reading the search field. One definition is shared
/// with the text the agent connector returns.
use crate::exports::html::invisible;

/// NFC-normalise, drop control and invisible characters, collapse whitespace,
/// trim, and bound the length. `None` when nothing searchable remains.
pub fn sanitize(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut count = 0;
    let mut space = false;
    for c in text.nfc() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if c.is_control() || invisible(c) {
            continue;
        }
        if space {
            if count + 1 >= MAX_QUERY_CHARS {
                break;
            }
            out.push(' ');
            count += 1;
            space = false;
        }
        if count >= MAX_QUERY_CHARS {
            break;
        }
        out.push(c);
        count += 1;
    }
    (!out.is_empty()).then_some(out)
}

/// Parse `kiwix://search?q=TEXT`. Any other host, a path, a fragment,
/// credentials, a port, or any parameter other than exactly one `q` is
/// rejected, so a link cannot reach a different action.
pub fn parse_url(raw: &str) -> Result<String, String> {
    const INVALID: &str = "Only kiwix://search?q=… links are supported";
    if raw.len() > MAX_URL_BYTES {
        return Err("The kiwix:// link is too long".into());
    }
    let url = url::Url::parse(raw).map_err(|_| INVALID)?;
    if url.scheme() != "kiwix"
        // Hosts of non-special schemes are opaque, so the url crate keeps their case.
        || !url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("search"))
        || !matches!(url.path(), "" | "/")
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(INVALID.into());
    }
    let mut pairs = url.query_pairs();
    let (Some((key, value)), None) = (pairs.next(), pairs.next()) else {
        return Err(INVALID.into());
    };
    if key != "q" {
        return Err(INVALID.into());
    }
    sanitize(&value).ok_or_else(|| "The kiwix:// link has no search text".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_hidden_text_and_bounds_length() {
        assert_eq!(sanitize("  Bahrain\n\tisland  ").as_deref(), Some("Bahrain island"));
        assert_eq!(sanitize("a\u{202E}b\u{200B}c\u{E0041}d\u{0007}").as_deref(), Some("abcd"));
        assert_eq!(sanitize("e\u{0301}").as_deref(), Some("\u{00E9}"));
        // Joiners belong to Persian and Indic words; fillers and marks do not.
        let persian = "\u{0645}\u{06CC}\u{200C}\u{062E}";
        assert_eq!(sanitize(persian).as_deref(), Some(persian));
        assert_eq!(sanitize("a\u{061C}\u{3164}b\u{FE0F}\u{E0101}").as_deref(), Some("ab"));
        assert_eq!(sanitize(" \u{200B}\u{2066} \n"), None);
        let long = "x".repeat(MAX_QUERY_CHARS * 3);
        assert_eq!(sanitize(&long).unwrap().chars().count(), MAX_QUERY_CHARS);
        let spaced = "ab ".repeat(MAX_QUERY_CHARS);
        let bounded = sanitize(&spaced).unwrap();
        assert!(bounded.chars().count() <= MAX_QUERY_CHARS);
        assert!(!bounded.ends_with(' '));
    }

    #[test]
    fn only_a_single_query_search_link_is_accepted() {
        assert_eq!(parse_url("kiwix://search?q=Manama").unwrap(), "Manama");
        assert_eq!(parse_url("kiwix://search/?q=New%20York+City").unwrap(), "New York City");
        assert_eq!(parse_url("KIWIX://SEARCH?q=x").unwrap(), "x");
        for bad in [
            "kiwix://library",
            "kiwix://speech?action=stop",
            "kiwix://search",
            "kiwix://search?q=",
            "kiwix://search?q=%E2%80%8B",
            "kiwix://search?q=a&q=b",
            "kiwix://search?q=a&open=b",
            "kiwix://search?x=a",
            "kiwix://search/path?q=a",
            "kiwix://search?q=a#frag",
            "kiwix://user@search?q=a",
            "kiwix://search:80?q=a",
            "zim://search?q=a",
            "https://search?q=a",
            "--data-dir",
            "",
        ] {
            assert!(parse_url(bad).is_err(), "{bad} should be rejected");
        }
        let huge = format!("kiwix://search?q={}", "a".repeat(MAX_URL_BYTES));
        assert!(parse_url(&huge).is_err());
    }
}
