// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Explicit interchange only: never scans/deletes old profiles or overwrites source files.
//! libkiwix XML book descriptors use child elements (src/bookmark.cpp); attribute variants
//! are accepted as well. XML exports retain notes/tags/creation time and text highlights in
//! ignored extension elements; JSON and these XML extensions preserve annotations losslessly.
//! Library XML/OPDS paths are returned literally, relative to the ORIGINAL library directory.
//! No path is opened here. The service must ask for that base directory, validate archives,
//! and retain missing `legacy_books` records for later relinking. Remote links are not paths.
//! Apple Core Data databases, security-scoped bookmarks, opaque WebKit interaction state,
//! URLSession resume tokens, Qt geometry/INI sessions and aria2 sessions are not imported.

use crate::archive::ArchiveMetadata;
use crate::store::{
    new_id, new_installation_id, now, validate_bookmark, Book, Bookmark, LegacyBookmarkBook, State,
};
use roxmltree::{Document, Node, ParsingOptions};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use uuid::Uuid;

const MAX_IMPORT_BYTES: usize = 16 * 1024 * 1024;
// Bookmark interchange is independent of the native profile schema version.
const BOOKMARK_EXPORT_VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BookmarkExport {
    version: u32,
    bookmarks: Vec<Bookmark>,
}

#[derive(Serialize)]
struct BorrowedBookmarkExport<'a> {
    version: u32,
    bookmarks: &'a [Bookmark],
}

fn check_size(text: &str) -> Result<&str, String> {
    if text.len() > MAX_IMPORT_BYTES {
        return Err("Import exceeds the 16 MiB limit; split the export into smaller files".into());
    }
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err("Import is empty".into());
    }
    Ok(text)
}

fn xml(text: &str) -> Result<Document<'_>, String> {
    // roxmltree never fetches external entities. Explicitly forbid DTDs rather than allowing
    // even internal entity expansion; cap parsed nodes independently from input bytes.
    Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 200_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|e| format!("Invalid XML import (DTDs/external entities are unsupported): {e}"))
}

/// Parses and validates the entire input before changing state. Duplicate identity is
/// canonical archive ID + the COMPLETE path, including its query and fragment, case-sensitive.
/// Existing entries win; no title/note/tag is overwritten by an accidental reimport.
pub fn import_bookmarks(text: &str, state: &mut State) -> Result<usize, String> {
    let text = check_size(text)?;
    let mut incoming = if text.starts_with('<') {
        bookmarks_from_xml(&xml(text)?)?
    } else if text.starts_with('[') {
        serde_json::from_str::<Vec<Bookmark>>(text)
            .map_err(|e| format!("Invalid bookmark array: {e}"))?
    } else {
        let export: BookmarkExport =
            serde_json::from_str(text).map_err(|e| format!("Invalid bookmark export: {e}"))?;
        if export.version != BOOKMARK_EXPORT_VERSION {
            return Err(format!(
                "Unsupported bookmark export version {} (expected {BOOKMARK_EXPORT_VERSION})",
                export.version
            ));
        }
        export.bookmarks
    };
    for bookmark in &mut incoming {
        normalize_bookmark(bookmark)?;
        validate_bookmark(bookmark)?;
    }
    let mut identities: HashSet<(String, String)> = state
        .bookmarks
        .iter()
        .map(|bookmark| {
            Ok((
                canonical_book_id(&bookmark.book_id),
                crate::server::normalize_location(&bookmark.path)?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let mut ids: HashSet<String> = state
        .bookmarks
        .iter()
        .map(|bookmark| bookmark.id.clone())
        .collect();
    let mut additions = Vec::new();
    for mut bookmark in incoming {
        if !identities.insert((bookmark.book_id.clone(), bookmark.path.clone())) {
            continue;
        }
        // A foreign file can reuse an ID for a different article; retain both articles safely.
        if !ids.insert(bookmark.id.clone()) {
            loop {
                bookmark.id = new_id();
                if ids.insert(bookmark.id.clone()) {
                    break;
                }
            }
        }
        additions.push(bookmark);
    }
    let count = additions.len();
    state.bookmarks.extend(additions);
    Ok(count)
}

pub fn export_bookmarks(state: &State) -> Result<String, String> {
    for bookmark in &state.bookmarks {
        validate_bookmark(bookmark)?;
    }
    serde_json::to_string_pretty(&BorrowedBookmarkExport {
        version: BOOKMARK_EXPORT_VERSION,
        bookmarks: &state.bookmarks,
    })
    .map_err(|e| format!("Cannot export bookmarks: {e}"))
}

fn canonical_book_id(value: &str) -> String {
    let value = value.trim();
    let raw = value.strip_prefix("urn:uuid:").unwrap_or(value);
    // Qt hosts append .zim; do not strip a real non-UUID ID's filename extension.
    let candidate = raw.strip_suffix(".zim").unwrap_or(raw);
    Uuid::parse_str(candidate)
        .map(|id| id.to_string())
        .unwrap_or_else(|_| raw.to_owned())
}

/// The leading URL scheme and its remainder, when `value` starts with
/// `scheme://` and `scheme` is syntactically valid (RFC 3986:
/// ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )). An archive-local path may
/// contain a URL in its query (`A/Test?source=https://example.org`); that is
/// not a bookmark URL.
fn leading_scheme(value: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = value.split_once("://")?;
    let mut characters = scheme.chars();
    let valid = characters.next().is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some((scheme, rest))
}

fn normalize_bookmark(bookmark: &mut Bookmark) -> Result<(), String> {
    bookmark.book_id = canonical_book_id(&bookmark.book_id);
    // Deliberately do NOT use Url::path(): URL libraries may normalize dot segments or
    // percent encoding, and path() alone loses query/hash routes. Split the authority only.
    if let Some((scheme, rest)) = leading_scheme(&bookmark.path) {
        if !matches!(scheme, "zim" | "kiwix") {
            return Err(format!(
                "Bookmark URLs must be archive-local, not {scheme:?}"
            ));
        }
        let boundary = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..boundary];
        if authority.is_empty() || authority.contains(['@', ':', '\\']) {
            return Err("A saved bookmark has an invalid link and was skipped".into());
        }
        let url_book_id = canonical_book_id(authority);
        if bookmark.book_id.is_empty() {
            bookmark.book_id = url_book_id;
        } else if bookmark.book_id != url_book_id {
            return Err("Bookmark book ID does not match its archive URL".into());
        }
        let suffix = &rest[boundary..];
        bookmark.path = suffix.strip_prefix('/').unwrap_or(suffix).to_owned();
    }
    bookmark.path = crate::server::normalize_location(&bookmark.path)?;
    if let Some(book) = &mut bookmark.legacy_book {
        book.id = canonical_book_id(&book.id);
    }
    Ok(())
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}
fn value(node: Node<'_, '_>, name: &str) -> String {
    child(node, name)
        .and_then(|child| child.text())
        .or_else(|| node.attribute(name))
        .unwrap_or_default()
        .to_owned()
}

fn bookmarks_from_xml(document: &Document<'_>) -> Result<Vec<Bookmark>, String> {
    let root = document.root_element();
    if !matches!(root.tag_name().name(), "bookmarks" | "library" | "feed") {
        return Err("Expected a libkiwix <bookmarks>, <library>, or OPDS <feed> document".into());
    }
    let nodes: Vec<_> = root
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "bookmark")
        .collect();
    // An explicitly empty bookmark document is valid. A library with no bookmark section
    // is not a mistaken success importing zero bookmarks; use legacy_paths for that file.
    if nodes.is_empty()
        && root.tag_name().name() != "bookmarks"
        && !root
            .descendants()
            .any(|node| node.is_element() && node.tag_name().name() == "bookmarks")
    {
        return Err("This library has no embedded bookmarks; import its separate library.bookmarks.xml file".into());
    }
    let mut bookmarks = Vec::with_capacity(nodes.len());
    for node in nodes {
        let book_node = child(node, "book").ok_or("Bookmark is missing its <book> descriptor")?;
        let book = LegacyBookmarkBook {
            id: value(book_node, "id"),
            title: value(book_node, "title"),
            name: value(book_node, "name"),
            flavour: value(book_node, "flavour"),
            language: value(book_node, "language"),
            date: value(book_node, "date"),
        };
        let url = child(node, "url").ok_or("Bookmark is missing its <url>")?;
        if child(node, "title").is_none() {
            return Err("Bookmark is missing its <title>".into());
        }
        let tags = child(node, "tags")
            .map(|tags| {
                tags.children()
                    .filter(|tag| tag.is_element() && tag.tag_name().name() == "tag")
                    .map(|tag| tag.text().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let id = value(node, "id");
        let created_at = value(node, "createdAt");
        let mut extensions = node
            .children()
            .filter(|child| child.is_element() && child.tag_name().name() == "highlights");
        let highlights = if let Some(extension) = extensions.next() {
            if extensions.next().is_some() || extension.children().any(|child| child.is_element()) {
                return Err("Malformed or duplicate highlights extension".into());
            }
            let content: String = extension
                .children()
                .filter(|child| child.is_text())
                .filter_map(|child| child.text())
                .collect();
            serde_json::from_str(&content)
                .map_err(|error| format!("Invalid highlights extension: {error}"))?
        } else {
            Vec::new()
        };
        bookmarks.push(Bookmark {
            id: if id.is_empty() { new_id() } else { id },
            book_id: book.id.clone(),
            path: url.text().unwrap_or_default().to_owned(),
            title: value(node, "title"),
            note: value(node, "note"),
            tags,
            highlights,
            created_at: if created_at.is_empty() {
                now()
            } else {
                created_at
            },
            legacy_book: Some(book),
        });
    }
    Ok(bookmarks)
}

fn element(output: &mut String, name: &str, text: &str) -> Result<(), String> {
    let _ = write!(output, "    <{name}>");
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            '\r' => output.push_str("&#13;"), // Avoid XML end-of-line normalization on roundtrip.
            c if !matches!(c, '\t' | '\n') && (c < '\u{20}' || matches!(c, '\u{fffe}' | '\u{ffff}')) =>
                return Err("Bookmark contains characters XML 1.0 cannot represent; use JSON export instead".into()),
            c => output.push(c),
        }
    }
    let _ = writeln!(output, "</{name}>");
    Ok(())
}

/// Compatible with libkiwix's nested book descriptor. Unrecognized extension elements
/// preserve our notes/tags/IDs and highlights; older clients may discard those extensions.
pub fn export_bookmarks_xml(state: &State) -> Result<String, String> {
    let mut output = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<bookmarks>\n");
    let books: HashMap<&str, &ArchiveMetadata> = state
        .books
        .iter()
        .map(|book| (book.metadata.id.as_str(), &book.metadata))
        .collect();
    let empty_legacy = LegacyBookmarkBook::default();
    for bookmark in &state.bookmarks {
        validate_bookmark(bookmark)?;
        output.push_str("  <bookmark>\n    <book>\n");
        let legacy = bookmark.legacy_book.as_ref().unwrap_or(&empty_legacy);
        let metadata = books.get(bookmark.book_id.as_str()).copied();
        element(&mut output, "id", &bookmark.book_id)?;
        element(
            &mut output,
            "title",
            metadata.map(|m| m.title.as_str()).unwrap_or(&legacy.title),
        )?;
        element(
            &mut output,
            "name",
            metadata.map(|m| m.name.as_str()).unwrap_or(&legacy.name),
        )?;
        element(&mut output, "flavour", &legacy.flavour)?;
        element(
            &mut output,
            "language",
            metadata
                .map(|m| m.language.as_str())
                .unwrap_or(&legacy.language),
        )?;
        element(
            &mut output,
            "date",
            metadata.map(|m| m.date.as_str()).unwrap_or(&legacy.date),
        )?;
        output.push_str("    </book>\n");
        element(&mut output, "title", &bookmark.title)?;
        element(&mut output, "url", &bookmark.path)?;
        element(&mut output, "id", &bookmark.id)?;
        element(&mut output, "createdAt", &bookmark.created_at)?;
        element(&mut output, "note", &bookmark.note)?;
        element(
            &mut output,
            "highlights",
            &serde_json::to_string(&bookmark.highlights)
                .map_err(|error| format!("Cannot export highlights: {error}"))?,
        )?;
        output.push_str("    <tags>\n");
        for tag in &bookmark.tags {
            element(&mut output, "tag", tag)?;
        }
        output.push_str("    </tags>\n  </bookmark>\n");
    }
    output.push_str("</bookmarks>\n");
    Ok(output)
}

/// Extracts local paths from legacy library.xml or local libkiwix OPDS acquisition links.
/// Preserves ordering, literal XML paths and missing files; removes exact duplicate paths.
/// file:// links are decoded once; remote/opaque URLs are never turned into filesystem paths.
pub fn legacy_paths(text: &str) -> Result<Vec<String>, String> {
    let document = xml(check_size(text)?)?;
    let entries = library_entries(&document)?;
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for entry in entries {
        for path in entry_paths(entry)? {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

/// Metadata-only fallback records for explicit migration, including unavailable archives.
/// Availability is intentionally false until the archive service has opened/validated a path.
/// Native metadata replaces this fallback only after a successful archive open.
pub fn legacy_books(text: &str) -> Result<Vec<Book>, String> {
    let document = xml(check_size(text)?)?;
    let entries = library_entries(&document)?;
    let mut books = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let paths = entry_paths(entry)?;
        let Some(path) = paths.into_iter().next() else {
            continue;
        };
        let id = canonical_book_id(&value(entry, "id"));
        if id.is_empty() {
            return Err(format!("Library entry for {path:?} has no archive ID"));
        }
        if !seen.insert(id.clone()) {
            return Err("Two library entries describe the same archive. Remove one before importing bookmarks".into());
        }
        let opds = entry.tag_name().name() == "entry";
        let size = if opds {
            entry
                .children()
                .filter(|node| {
                    node.is_element()
                        && node.tag_name().name() == "link"
                        && node
                            .attribute("rel")
                            .is_some_and(|rel| rel.contains("/acquisition"))
                        && node
                            .attribute("type")
                            .is_none_or(|kind| kind.split(';').next() == Some("application/x-zim"))
                })
                .find_map(|link| link.attribute("length"))
                .unwrap_or("0")
                .parse::<u64>()
                .map_err(|e| format!("Invalid OPDS acquisition length: {e}"))?
        } else {
            number(entry, "size")?
                .checked_mul(1024)
                .ok_or("Legacy library size overflows bytes")?
        };
        let tags: Vec<String> = value(entry, "tags")
            .split(';')
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect();
        let mut date = value(entry, "date");
        if date.is_empty() {
            date = value(entry, "updated").chars().take(10).collect();
        }
        let creator = if opds {
            child(entry, "author")
                .map(|author| value(author, "name"))
                .unwrap_or_default()
        } else {
            value(entry, "creator")
        };
        let publisher = if opds {
            child(entry, "publisher")
                .map(|publisher| value(publisher, "name"))
                .unwrap_or_default()
        } else {
            value(entry, "publisher")
        };
        let description = if opds {
            value(entry, "summary")
        } else {
            value(entry, "description")
        };
        books.push(Book {
            metadata: ArchiveMetadata {
                id,
                title: value(entry, "title"),
                description,
                language: value(entry, "language"),
                creator,
                publisher,
                date,
                name: value(entry, "name"),
                has_fulltext_index: tags.iter().any(|tag| tag == "_ftindex:yes"),
                has_title_index: false,
                tags,
                size,
                article_count: number(entry, "articleCount")?,
                media_count: number(entry, "mediaCount")?,
                main_path: String::new(),
            },
            path,
            installation_id: new_installation_id(),
            added_at: now(),
            available: false,
            managed: false,
            source_reference: None,
            integrity: None,
        });
    }
    Ok(books)
}

fn number(node: Node<'_, '_>, name: &str) -> Result<u64, String> {
    let value = value(node, name);
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse()
        .map_err(|e| format!("Invalid legacy {name}: {e}"))
}
fn library_entries<'a, 'input>(
    document: &'a Document<'input>,
) -> Result<Vec<Node<'a, 'input>>, String> {
    let root = document.root_element();
    let name = match root.tag_name().name() {
        "library" => "book",
        "feed" => "entry",
        _ => return Err("Expected a legacy <library> XML document or an OPDS <feed>".into()),
    };
    Ok(root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name)
        .collect())
}
fn entry_paths(entry: Node<'_, '_>) -> Result<Vec<String>, String> {
    if entry.tag_name().name() == "book" {
        return match entry.attribute("path") {
            Some(path) => Ok(local_path(path)?.into_iter().collect()),
            None => Ok(Vec::new()),
        };
    }
    let mut paths = Vec::new();
    for link in entry
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "link")
    {
        let rel = link.attribute("rel").unwrap_or_default();
        if !(rel == "http://opds-spec.org/acquisition"
            || rel.starts_with("http://opds-spec.org/acquisition/")
            || rel == "https://opds-spec.org/acquisition"
            || rel.starts_with("https://opds-spec.org/acquisition/"))
        {
            continue;
        }
        // A relative torrent/metalink link is not an installed archive path.
        if let Some(kind) = link.attribute("type") {
            if !matches!(
                kind.split(';').next().unwrap_or_default(),
                "application/x-zim" | "application/octet-stream"
            ) {
                continue;
            }
        }
        if let Some(href) = link.attribute("href") {
            if let Some(path) = local_path(href)? {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}
fn local_path(value: &str) -> Result<Option<String>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.contains('\0') {
        return Err("Legacy archive path contains a NUL byte".into());
    }
    // Windows drive paths are local even when importing their metadata on another OS.
    let windows_drive = value.as_bytes().get(1) == Some(&b':')
        && value.as_bytes()[0].is_ascii_alphabetic()
        && matches!(value.as_bytes().get(2), Some(b'/' | b'\\'));
    if windows_drive || value.starts_with("\\\\") {
        return Ok(Some(value.to_owned()));
    }
    if value.starts_with("//") {
        return Ok(None);
    } // protocol-relative web link
    if let Ok(url) = url::Url::parse(value) {
        if url.scheme() != "file" {
            return Ok(None);
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("Legacy file URL must not contain a query or fragment".into());
        }
        let path = url.to_file_path().map_err(|_| {
            "Legacy file URL cannot be represented as a local path on this platform"
        })?;
        return path
            .into_os_string()
            .into_string()
            .map(Some)
            .map_err(|_| "Legacy file path is not UTF-8".into());
    }
    Ok(Some(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highlights_roundtrip_and_invalid_import_is_atomic() {
        let mut state = State::default();
        let legacy = r#"[{"bookId":"book","path":"A/Test","title":"Test"}]"#;
        import_bookmarks(legacy, &mut state).unwrap();
        assert!(state.bookmarks[0].highlights.is_empty());
        let highlight = crate::store::Highlight {
            id: new_id(),
            document_path: "A/Test%3Fpart?q=1#/route".into(),
            exact: "😀<&\r".into(),
            prefix: "before".into(),
            suffix: "after".into(),
            start: 2,
            end: 7,
            comment: "Comment <&>\r\n😀".into(),
        };
        state.bookmarks[0].highlights.push(highlight.clone());
        for export in [
            export_bookmarks(&state).unwrap(),
            export_bookmarks_xml(&state).unwrap(),
        ] {
            let mut restored = State::default();
            import_bookmarks(&export, &mut restored).unwrap();
            assert_eq!(restored.bookmarks[0].highlights, vec![highlight.clone()]);
        }
        let original = state.bookmarks.clone();
        let mut invalid = state.bookmarks[0].clone();
        invalid.path = "other".into();
        invalid.highlights[0].end -= 1;
        assert!(
            import_bookmarks(&serde_json::to_string(&vec![invalid]).unwrap(), &mut state).is_err()
        );
        assert_eq!(state.bookmarks, original);
        let malformed = r#"<bookmarks><bookmark><book id="other"/><title>Bad</title><url>B</url><highlights>{}</highlights></bookmark></bookmarks>"#;
        assert!(import_bookmarks(malformed, &mut state).is_err());
        assert_eq!(state.bookmarks, original);
        let exported_xml = export_bookmarks_xml(&state).unwrap();
        let split_text =
            exported_xml.replacen("<highlights>", "<highlights><![CDATA[ ]]><!--split-->", 1);
        let mut restored = State::default();
        import_bookmarks(&split_text, &mut restored).unwrap();
        assert_eq!(restored.bookmarks[0].highlights, vec![highlight]);
        let trailing_payload =
            exported_xml.replacen("</highlights>", "<!--split-->[]</highlights>", 1);
        assert!(import_bookmarks(&trailing_payload, &mut state).is_err());
        assert_eq!(state.bookmarks, original);
    }

    #[test]
    fn highlight_bounds_and_uuid_alias_duplicates_are_rejected() {
        let valid = crate::store::Highlight {
            id: new_id(),
            document_path: "A/Test".into(),
            exact: "😀".into(),
            prefix: String::new(),
            suffix: String::new(),
            start: 0,
            end: 2,
            comment: String::new(),
        };
        let validate = crate::store::validate_highlights;
        assert!(validate(std::slice::from_ref(&valid)).is_ok());
        let mut alias = valid.clone();
        alias.id = Uuid::parse_str(&valid.id).unwrap().simple().to_string();
        assert!(validate(&[valid.clone(), alias]).is_err());
        for id in [
            Uuid::parse_str(&valid.id).unwrap().simple().to_string(),
            format!("{{{}}}", valid.id),
            format!("urn:uuid:{}", valid.id),
        ] {
            let mut alias = valid.clone();
            alias.id = id;
            assert!(validate(&[alias]).is_err());
        }
        let mut routed = valid.clone();
        routed.document_path = "A/Test?source=https://example.org#/https://route.example".into();
        assert!(validate(&[routed.clone()]).is_ok());
        for path in [
            "https://example.org/A/Test",
            "javascript:alert(1)",
            "/absolute",
            "?query",
        ] {
            routed.document_path = path.into();
            assert!(validate(&[routed.clone()]).is_err(), "{path}");
        }
        assert!(validate(&vec![valid.clone(); 201]).is_err());
        for field in ["exact", "prefix", "suffix", "comment", "id", "offset"] {
            let mut invalid = valid.clone();
            match field {
                "exact" => invalid.exact = "x".repeat(16_385),
                "prefix" => invalid.prefix = "😀".repeat(65),
                "suffix" => invalid.suffix = "x".repeat(129),
                "comment" => invalid.comment = "😀".repeat(8_193),
                "id" => invalid.id = "not-a-uuid".into(),
                _ => {
                    invalid.start = 100_000_000;
                    invalid.end = 100_000_002;
                }
            }
            assert!(validate(&[invalid]).is_err(), "{field}");
        }
    }

    #[test]
    fn legacy_nested_book_and_full_url_identity() {
        let mut state = State::default();
        let input = r#"<bookmarks><bookmark><book><id>bf060622-d3fd-4fb0-9cf6-9fd32ff3132f</id><name>Example</name></book><title>A &amp; B</title><url>zim://bf060622-d3fd-4fb0-9cf6-9fd32ff3132f.zim/A/Test?q=x&amp;b=y#part</url></bookmark></bookmarks>"#;
        assert_eq!(import_bookmarks(input, &mut state).unwrap(), 1);
        assert_eq!(state.bookmarks[0].path, "A/Test?q=x&b=y#part");
        assert_eq!(
            state.bookmarks[0].legacy_book.as_ref().unwrap().name,
            "Example"
        );
        assert_eq!(import_bookmarks(input, &mut state).unwrap(), 0);
        let json = export_bookmarks(&state).unwrap();
        let mut restored = State::default();
        assert_eq!(import_bookmarks(&json, &mut restored).unwrap(), 1);
        assert_eq!(restored.bookmarks[0].path, state.bookmarks[0].path);
        let xml = export_bookmarks_xml(&state).unwrap();
        assert_eq!(import_bookmarks(&xml, &mut restored).unwrap(), 0);
    }

    #[test]
    fn archive_local_paths_may_contain_urls_but_non_zim_schemes_are_rejected() {
        let book_id = "bf060622-d3fd-4fb0-9cf6-9fd32ff3132f";
        let bookmark = |path: &str| format!(
            r#"{{"version":1,"bookmarks":[{{"bookId":"{book_id}","path":"{path}","title":"T"}}]}}"#
        );
        let mut state = State::default();
        let routed = "A/Test?source=https://example.org#/https://route.example";
        assert_eq!(import_bookmarks(&bookmark(routed), &mut state).unwrap(), 1);
        assert_eq!(state.bookmarks[0].path, routed);
        assert_eq!(state.bookmarks[0].book_id, book_id);
        // A leading non-archive scheme is still a URL and still rejected.
        for path in [
            "https://example.org/A/Test",
            "javascript://alert",
            "file:///etc/passwd",
        ] {
            let mut rejected = State::default();
            assert!(import_bookmarks(&bookmark(path), &mut rejected).is_err(), "{path}");
            assert!(rejected.bookmarks.is_empty(), "{path}");
        }
    }

    #[test]
    fn all_or_nothing_and_no_entities() {
        let mut state = State::default();
        let malformed = r#"<bookmarks><bookmark><book id="a"/><title>OK</title><url>A</url></bookmark><bookmark><title>Bad</title><url>B</url></bookmark></bookmarks>"#;
        assert!(import_bookmarks(malformed, &mut state).is_err());
        assert!(state.bookmarks.is_empty());
        assert!(legacy_paths(r#"<!DOCTYPE library [<!ENTITY x SYSTEM "file:///etc/passwd">]><library><book path="&x;"/></library>"#).is_err());
    }

    #[test]
    fn local_opds_paths_do_not_fetch_remote_or_drop_missing() {
        let opds = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>urn:uuid:missing</id><link rel="http://opds-spec.org/acquisition/open-access" href="./missing&amp;name.zim"/><link rel="http://opds-spec.org/acquisition/open-access" href="https://example.org/book.zim"/></entry></feed>"#;
        assert_eq!(legacy_paths(opds).unwrap(), vec!["./missing&name.zim"]);
        assert!(!legacy_books(opds).unwrap()[0].available);
        let first = legacy_books(opds).unwrap().remove(0);
        let second = legacy_books(opds).unwrap().remove(0);
        assert_ne!(first.installation_id, second.installation_id);
        assert_ne!(first.installation_id, first.metadata.id);
        assert_eq!(
            Uuid::parse_str(&first.installation_id)
                .unwrap()
                .get_version_num(),
            4
        );
    }

    #[test]
    fn legacy_descriptors_cannot_choose_installation_principals() {
        let chosen = "d1ef8892-f2ee-4cab-87a9-9df6e4bcac6f";
        let input = format!(
            r#"<library><book id="{chosen}" path="/unavailable.zim" installationId="{chosen}"><installationId>{chosen}</installationId></book></library>"#
        );
        let book = legacy_books(&input).unwrap().remove(0);
        assert_eq!(book.metadata.id, chosen);
        assert_ne!(book.installation_id, chosen);
    }
}
