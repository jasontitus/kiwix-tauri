// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Strict intake for external `zim://<archive-uuid>/<article>` links.
//!
//! The link names a library archive by its ZIM UUID (the same identifier the
//! reader origin and upstream Kiwix use) and an archive-relative article path.
//! Parsing here never resolves a filesystem path, contacts a host, or imports
//! anything: callers must map the UUID to an existing library book and enqueue
//! an ordinary launch location. Unknown hosts, ports, credentials and traversal
//! segments are rejected instead of being repaired.
// Linux/Windows desktop builds register no zim:// handler; the parser serves
// macOS, mobile and tests.
#![cfg_attr(all(desktop, not(target_os = "macos"), not(test)), allow(dead_code))]
use url::Url;
use uuid::Uuid;

/// A whole link is bounded before the URL parser allocates for it.
pub const MAX_LINK_LENGTH: usize = 65_536;
/// Archive-relative article paths and location suffixes stay well below the
/// request limits used by the reader origin.
pub const MAX_PATH_LENGTH: usize = 8_192;
pub const MAX_SUFFIX_LENGTH: usize = 8_192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZimLink {
    /// Canonical lowercase hyphenated ZIM UUID from the URL host.
    pub book_id: String,
    /// Decoded libzim path, or `None` for the archive's main document.
    pub path: Option<String>,
    /// Raw `?query` and/or `#fragment` suffix, including its delimiters.
    pub suffix: String,
}

/// Parse one external `zim://` link.
///
/// Accepted form: `zim://<uuid>/<archive path>[?query][#fragment]`.
/// The archive path is percent-decoded exactly once. The caller re-encodes it
/// with [`crate::server::encode_path`] before it becomes an application
/// location, so a literal `?`/`#` in an archive path cannot split the location.
///
/// Apple delivers `RunEvent::Opened` URLs through `url::Url`, which removes
/// literal and percent-encoded dot segments before this function sees them.
/// The normalized remainder is still only an archive-relative libzim entry
/// key, so it cannot escape the archive. Callers that hold the original raw
/// string (for example a future native URL bridge) get the stricter refusal
/// path below.
pub fn parse(raw: &str) -> Result<ZimLink, String> {
    if raw.is_empty() {
        return Err("Empty zim:// link".into());
    }
    if raw.len() > MAX_LINK_LENGTH {
        return Err("zim:// link is too long".into());
    }
    let url = Url::parse(raw).map_err(|_| "Malformed zim:// link".to_string())?;
    if !url.scheme().eq_ignore_ascii_case("zim") {
        return Err("Only zim:// links are accepted".into());
    }
    if url.cannot_be_a_base() {
        return Err("zim:// link must name an archive and path".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("zim:// links cannot carry credentials".into());
    }
    if url.port().is_some() {
        return Err("zim:// links cannot carry a port".into());
    }
    let host = url
        .host_str()
        .ok_or("This link is missing its archive. Open the article from your library")?;
    // Accept the UUID forms upstream Apple stores (including uppercase) but
    // normalize to the canonical lowercase form used by the library.
    let book_id = Uuid::parse_str(host)
        .map_err(|_| "This link is not valid. Open the article from your library")?
        .to_string();
    // The URL parser removes literal and percent-encoded dot segments before
    // `path()` is observable. Validate the raw path first so traversal attempts
    // are refused rather than silently repaired into a different article.
    let raw_path = raw_path(raw)?;
    validate_archive_path(&crate::server::decode_location_path(raw_path)?)?;
    // A URL path is percent-encoded; decode once at this boundary. The helper
    // rejects malformed escapes and non-UTF-8 bytes rather than resolving a
    // different (prefix) entry.
    let decoded = crate::server::decode_location_path(url.path())?;
    if decoded.len() > MAX_PATH_LENGTH {
        return Err("zim:// article path is too long".into());
    }
    validate_archive_path(&decoded)?;
    let mut suffix = String::new();
    if let Some(query) = url.query().filter(|query| !query.is_empty()) {
        if query.len() > MAX_SUFFIX_LENGTH {
            return Err("zim:// article query is too long".into());
        }
        validate_suffix(query)?;
        suffix.push('?');
        suffix.push_str(query);
    }
    if let Some(fragment) = url.fragment().filter(|fragment| !fragment.is_empty()) {
        if fragment.len() > MAX_SUFFIX_LENGTH {
            return Err("zim:// article fragment is too long".into());
        }
        validate_suffix(fragment)?;
        suffix.push('#');
        suffix.push_str(fragment);
    }
    Ok(ZimLink {
        book_id,
        path: (!decoded.is_empty()).then_some(decoded),
        suffix,
    })
}

fn raw_path(raw: &str) -> Result<&str, String> {
    let (_, after_scheme) = raw
        .split_once(':')
        .ok_or("Malformed zim:// link")?;
    let after_authority = after_scheme
        .strip_prefix("//")
        .ok_or("zim:// link must name an archive and path")?;
    let authority_end = after_authority
        .find(['/', '?', '#'])
        .unwrap_or(after_authority.len());
    let tail = after_authority[authority_end..].strip_prefix('/').unwrap_or(&after_authority[authority_end..]);
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    Ok(&tail[..path_end])
}

fn validate_archive_path(path: &str) -> Result<(), String> {
    if path.chars().any(char::is_control) {
        return Err("zim:// article path contains control characters".into());
    }
    // Archive paths are exact libzim entry names. A backslash is never a path
    // separator here, but rejecting it avoids platform-dependent reinterpretation.
    if path.contains('\\') {
        return Err("zim:// article path contains a backslash".into());
    }
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("zim:// article path contains a traversal segment".into());
    }
    Ok(())
}

fn validate_suffix(suffix: &str) -> Result<(), String> {
    if suffix.chars().any(char::is_control) {
        return Err("zim:// article query or fragment contains control characters".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "861c031f-dafb-9688-4db4-8f1199fe2926";

    #[test]
    fn accepts_upstream_article_links_and_normalizes_the_uuid() {
        let link = parse(&format!(
            "zim://{}/mesquartierschinois.wordpress.com/",
            ID.to_ascii_uppercase()
        ))
        .unwrap();
        assert_eq!(link.book_id, ID);
        assert_eq!(
            link.path.as_deref(),
            Some("mesquartierschinois.wordpress.com/")
        );
        assert_eq!(link.suffix, "");

        let link = parse(&format!(
            "zim://{ID}/widgets.wp.com/likes/master.html%3Fver%3D20240530#ver=20240530&lang=fr"
        ))
        .unwrap();
        assert_eq!(
            link.path.as_deref(),
            Some("widgets.wp.com/likes/master.html?ver=20240530")
        );
        assert_eq!(link.suffix, "#ver=20240530&lang=fr");
    }

    #[test]
    fn accepts_main_document_links_and_scheme_case() {
        let link = parse(&format!("ZIM://{ID}")).unwrap();
        assert_eq!(link.book_id, ID);
        assert_eq!(link.path, None);
        assert_eq!(parse(&format!("zim://{ID}/")).unwrap().path, None);
        let query = parse(&format!("zim://{ID}/A/Page?query=x%20y")).unwrap();
        assert_eq!(query.path.as_deref(), Some("A/Page"));
        assert_eq!(query.suffix, "?query=x%20y");
    }

    #[test]
    fn preserves_percent_decoded_paths_exactly_once() {
        let link = parse(&format!("zim://{ID}/A%20B/%C3%A9t%C3%A9")).unwrap();
        assert_eq!(link.path.as_deref(), Some("A B/été"));
    }

    #[test]
    fn rejects_other_schemes_and_remote_hosts() {
        for raw in [
            "https://example.com/A",
            "kiwix://861c031f-dafb-9688-4db4-8f1199fe2926/A",
            "file:///etc/passwd",
            "zim://example.com/A",
            "zim://localhost/A",
            "zim://127.0.0.1/A",
            "zim:861c031f-dafb-9688-4db4-8f1199fe2926/A",
            "",
        ] {
            assert!(parse(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn rejects_credentials_ports_and_non_uuid_authorities() {
        for raw in [
            format!("zim://user@{ID}/A"),
            format!("zim://user:pass@{ID}/A"),
            format!("zim://{ID}:8080/A"),
            format!("zim://{ID}-suffix/A"),
            "zim://../A".to_string(),
        ] {
            assert!(parse(&raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn rejects_traversal_control_and_malformed_escapes() {
        for raw in [
            format!("zim://{ID}/../secret"),
            format!("zim://{ID}/A/../../secret"),
            format!("zim://{ID}/%2e%2e/secret"),
            format!("zim://{ID}/A/%2E%2E/secret"),
            format!("zim://{ID}/A%00B"),
            format!("zim://{ID}/A%0AB"),
            format!("zim://{ID}/A\\B"),
            format!("zim://{ID}/A%5CB"),
            format!("zim://{ID}/A%zz"),
            format!("zim://{ID}/A%2"),
        ] {
            assert!(parse(&raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn rejects_oversized_inputs() {
        assert!(parse(&format!("zim://{ID}/{}", "a".repeat(MAX_PATH_LENGTH + 1))).is_err());
        assert!(parse(&format!("zim://{ID}/A?{}", "a".repeat(MAX_SUFFIX_LENGTH + 1))).is_err());
        assert!(parse(&format!("zim://{ID}/A#{}", "a".repeat(MAX_SUFFIX_LENGTH + 1))).is_err());
        assert!(parse(&"a".repeat(MAX_LINK_LENGTH + 1)).is_err());
    }
}
