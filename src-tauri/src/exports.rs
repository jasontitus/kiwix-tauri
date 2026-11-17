// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Original-article MHTML (not a snapshot of a running DOM), and byte-exact
//! resource export. All reads are offline libzim ranges; archive names never
//! become filesystem paths. Pre-publication failures leave the destination untouched;
//! post-publication directory-sync failures retain the completed file and report
//! that power-loss durability is unconfirmed.
//!
//! Export entry paths are decoded canonical libzim identities, not reader
//! locations: callers resolve URL query/fragment suffixes before entering here.
//! Literal `?`, `#`, and `%` in entry names remain part of that identity.
//! HTML, SVG, and CSS dispatch ignores media-type case and surrounding whitespace;
//! parameters remain intact in exported MIME headers.
use crate::archive::{is_html_mime, Archive, EntryInfo};
use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use percent_encoding::percent_decode_str;
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};
use url::Url;
use uuid::Uuid;

const CHUNK: u64 = 64 * 1024;
const MAX_TEXT: u64 = 16 * 1024 * 1024;
const MAX_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PARTS: usize = 4096;
const MAX_REFERENCES: usize = 32768;
const MAX_URL: usize = 16384;

/// `overwrite` must reflect explicit native Save-dialog consent, never merely
/// an existence check. False uses an atomic no-clobber rename on every platform.
pub fn save_resource(
    archive: &Archive,
    entry_path: &str,
    destination: &Path,
    overwrite: bool,
) -> Result<(), String> {
    let entry = archive.entry(resource_path(entry_path)?)?;
    atomic_output(destination, overwrite, |output| {
        stream(archive, &entry, |bytes| {
            output.write_all(bytes).map_err(display)
        })
    })
}

/// Bundle the original article and recursively referenced static assets. Links
/// to other articles and dynamically generated script requests are not crawled.
/// Explicit bounds reject oversized packages instead of silently losing assets.
/// The one omission is a reference to an entry the archive does not contain:
/// it is equally broken in the reader, so it stays unresolved in the package.
pub fn save_mhtml(
    archive: &Archive,
    entry_path: &str,
    destination: &Path,
    overwrite: bool,
) -> Result<(), String> {
    let root = archive.entry(resource_path(entry_path)?)?;
    if !is_html_mime(&root.mime) {
        return Err("MHTML export requires an HTML article".into());
    }
    let archive_id =
        Uuid::parse_str(&archive.metadata()?.id).map_err(|_| "This archive is damaged. Remove it from your library and add it again")?;
    let origin =
        Url::parse(&format!("http://{}.localhost/", archive_id.hyphenated())).map_err(display)?;
    let root_url = entry_url(&origin, &root.path)?;
    let boundary = format!("----kiwix-{}", Uuid::new_v4().simple());
    atomic_output(destination, overwrite, |output| {
        write!(output, "MIME-Version: 1.0\r\nContent-Type: multipart/related; type=\"{}\"; boundary=\"{boundary}\"; start=\"<article@kiwix>\"\r\n\r\n", media_type(&root.mime)).map_err(display)?;
        let mut pending = VecDeque::from([(root_url.clone(), root_url.clone())]);
        let mut visited =
            HashMap::from([(root_url.as_str().to_owned(), root_url.as_str().to_owned())]);
        let mut total = 0u64;
        let mut references = 0usize;
        let mut first = true;
        let mut spent = html::Spent::default();
        while let Some((location, lookup)) = pending.pop_front() {
            let path = percent_decode_str(lookup.path().strip_prefix('/').unwrap_or(lookup.path()))
                .decode_utf8()
                .map_err(|_| format!("Asset URL is not UTF-8: {location}"))?;
            let entry = match archive.entry(&path) {
                Ok(entry) => entry,
                // A reference the archive does not contain is just as broken in
                // the reader (DevDocs styles name sprites that were never
                // packed). Leave it unresolved, as the article shows it, instead
                // of refusing the export. Any other read error still fails.
                Err(error) if !first && crate::archive::is_entry_not_found(&error) => continue,
                Err(error) => {
                    return Err(format!("Cannot package referenced asset {location}: {error}"))
                }
            };
            checked_mime(&entry.mime)?;
            total = total
                .checked_add(entry.size)
                .filter(|n| *n <= MAX_PACKAGE)
                .ok_or("MHTML package exceeds the 2 GiB source-data limit")?;
            let canonical = entry_url(&origin, &entry.path)?;
            let text = if is_markup(&entry.mime) || is_css(&entry.mime) {
                if entry.size > MAX_TEXT {
                    return Err(format!(
                        "Text asset {} exceeds the 16 MiB parsing limit",
                        entry.path
                    ));
                }
                let mut bytes = Vec::with_capacity(entry.size as usize);
                stream(archive, &entry, |part| {
                    bytes.extend_from_slice(part);
                    Ok(())
                })?;
                let source = std::str::from_utf8(&bytes).map_err(|_| {
                    format!(
                        "Text asset {} is not UTF-8; cannot safely discover its static assets",
                        entry.path
                    )
                })?;
                let dependencies = if is_css(&entry.mime) {
                    References {
                        base: None,
                        values: css_references(source, Malformed::Ignore)?,
                    }
                } else {
                    markup_references(source, &mut spent)?
                };
                let lookup_base = dependencies
                    .base
                    .as_deref()
                    .map(|base| canonical.join(base))
                    .transpose()
                    .map_err(display)?
                    .unwrap_or(canonical);
                let output_base = dependencies
                    .base
                    .as_deref()
                    .map(|base| location.join(base))
                    .transpose()
                    .map_err(display)?
                    .unwrap_or_else(|| location.clone());
                for reference in dependencies.values {
                    references += 1;
                    if references > MAX_REFERENCES {
                        return Err("Article exceeds the 32768 static-reference limit".into());
                    }
                    if let Some(target) = local_reference(&lookup_base, &reference, &origin)? {
                        if let Some(url) = local_reference(&output_base, &reference, &origin)? {
                            if let Some(existing) = visited.get(url.as_str()) {
                                if existing != target.as_str() {
                                    return Err(format!(
                                        "Conflicting redirected asset locations: {url}"
                                    ));
                                }
                            } else {
                                if visited.len() >= MAX_PARTS {
                                    return Err(
                                        "Article exceeds the 4096 packaged-asset limit".into()
                                    );
                                }
                                visited.insert(url.as_str().to_owned(), target.as_str().to_owned());
                                pending.push_back((url, target));
                            }
                        }
                    }
                }
                Some(bytes)
            } else {
                None
            };
            write!(output, "--{boundary}\r\nContent-Type: {}", entry.mime).map_err(display)?;
            if text.is_some() && !entry.mime.to_ascii_lowercase().contains("charset=") {
                output.write_all(b"; charset=utf-8").map_err(display)?;
            }
            output
                .write_all(b"\r\nContent-Transfer-Encoding: base64\r\n")
                .map_err(display)?;
            location_header(output, &location)?;
            if first {
                output
                    .write_all(b"Content-ID: <article@kiwix>\r\n")
                    .map_err(display)?;
                first = false;
            }
            output.write_all(b"\r\n").map_err(display)?;
            let mut lines = MimeLines {
                output: &mut *output,
                column: 0,
            };
            {
                let mut encoder = EncoderWriter::new(&mut lines, &STANDARD);
                if let Some(bytes) = text {
                    encoder.write_all(&bytes).map_err(display)?;
                } else {
                    stream(archive, &entry, |bytes| {
                        encoder.write_all(bytes).map_err(display)
                    })?;
                }
                encoder.finish().map_err(display)?;
            }
            lines.finish().map_err(display)?;
        }
        write!(output, "--{boundary}--\r\n").map_err(display)
    })
}


/// Prepare the completed native PDF next to its final destination. Potentially
/// slow copying/fsync happens before the short authority-checked rename commit.
#[cfg(any(desktop, test))]
pub(crate) fn prepare_pdf(source: &Path, destination: &Path) -> Result<PreparedOutput, String> {
    prepare_output(destination, |output| {
        let mut input = File::open(source).map_err(display)?;
        io::copy(&mut input, output).map_err(display)?;
        Ok(())
    })
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn resource_path(path: &str) -> Result<&str, String> {
    if path.len() > MAX_URL || path.contains('\0') {
        return Err("Invalid or oversized archive resource path".into());
    }
    // This API takes a decoded canonical libzim path, not a URL or location.
    // Delimiters and percent signs are literal archive filename characters.
    if path.is_empty() {
        return Err("Missing archive resource path".into());
    }
    Ok(path)
}
fn entry_url(origin: &Url, path: &str) -> Result<Url, String> {
    let url = Url::parse(&format!(
        "{}{}",
        origin.as_str(),
        crate::server::encode_path(path)
    ))
    .map_err(display)?;
    let decoded = percent_decode_str(url.path().strip_prefix('/').unwrap_or(url.path()))
        .decode_utf8()
        .map_err(display)?;
    if decoded != path {
        return Err(
            "Archive path contains dot segments that cannot be represented as an MHTML URL".into(),
        );
    }
    Ok(url)
}
fn location_header(output: &mut impl Write, location: &Url) -> Result<(), String> {
    // Keep URI bytes intact and headers inside MIME line-length limits.
    // Explicitly reject an oversized location instead of emitting broken MIME.
    if location.as_str().len() > 900 {
        return Err(format!(
            "Asset URL exceeds the interoperable MHTML header limit: {location}"
        ));
    }
    write!(output, "Content-Location: {location}\r\n").map_err(display)
}
fn checked_mime(mime: &str) -> Result<(), String> {
    if mime.is_empty()
        || mime.len() > 900
        || !mime.is_ascii()
        || mime.bytes().any(|b| b < 32 || b == 127)
    {
        return Err("Archive resource has an invalid MIME header".into());
    }
    Ok(())
}
fn media_type(mime: &str) -> &str {
    mime.split(';').next().unwrap_or("").trim()
}
fn is_markup(mime: &str) -> bool {
    is_html_mime(mime) || media_type(mime).eq_ignore_ascii_case("image/svg+xml")
}
fn is_css(mime: &str) -> bool {
    media_type(mime).eq_ignore_ascii_case("text/css")
}

fn stream(
    archive: &Archive,
    entry: &EntryInfo,
    mut consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < entry.size {
        let bytes = archive.read(&entry.path, offset, CHUNK.min(entry.size - offset))?;
        if bytes.is_empty() {
            return Err(format!(
                "Unexpected end of archive resource {} at byte {offset}",
                entry.path
            ));
        }
        if bytes.len() as u64 > entry.size - offset {
            return Err("Archive returned bytes beyond the resource size".into());
        }
        consume(&bytes)?;
        offset += bytes.len() as u64;
    }
    Ok(())
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

pub(crate) struct PreparedOutput {
    temporary: Temporary,
    destination: PathBuf,
    /// A private same-volume directory holding the temporary file in the macOS
    /// App Sandbox. Declared after `temporary` so the file is removed first.
    _scratch: Option<ScratchDirectory>,
}

struct ScratchDirectory(PathBuf);
impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        // Only ever empty by now; never remove contents.
        let _ = fs::remove_dir(&self.0);
    }
}
impl PreparedOutput {
    pub(crate) fn publish(mut self, overwrite: bool) -> Result<(), String> {
        if overwrite {
            fs::rename(&self.temporary.0, &self.destination)
        } else {
            crate::publication::publish_noreplace(&self.temporary.0, &self.destination)
        }.map_err(|error| format!("Publishing export (existing files require explicit overwrite consent): {error}"))?;
        // Rename relinquishes the temporary name. Clear it to disarm cleanup
        // before any fallible post-commit work or reuse of that name.
        self.temporary.0.clear();
        #[cfg(unix)]
        if !crate::sandbox::is_sandboxed() {
            // A sandboxed app holds a grant for the saved file, not for its
            // folder, so the folder cannot be opened for synchronization.
            let parent = self.destination
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            // Keep filenames and OS details out of this user-facing warning:
            // their text can trigger the consumer's internal-error sanitizer.
            sync_export_directory(parent).map_err(|_| {
                "The export completed and the file was kept, but Kiwix could not confirm that it will survive a sudden power loss.".to_owned()
            })?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
std::thread_local! {
    static FAIL_NEXT_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RECREATE_TEMPORARY_ON_DIRECTORY_SYNC: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(unix)]
fn sync_export_directory(parent: &Path) -> io::Result<()> {
    #[cfg(test)]
    if FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.replace(false)) {
        if let Some(path) = RECREATE_TEMPORARY_ON_DIRECTORY_SYNC.with(|path| path.borrow_mut().take()) {
            OpenOptions::new().write(true).create_new(true).open(path)?
                .write_all(b"unrelated replacement")?;
        }
        return Err(io::Error::other("Injected export-directory synchronization failure"));
    }
    File::open(parent)?.sync_all()
}
fn atomic_output(
    destination: &Path,
    overwrite: bool,
    produce: impl FnOnce(&mut BufWriter<File>) -> Result<(), String>,
) -> Result<(), String> {
    prepare_output(destination, produce)?.publish(overwrite)
}
fn prepare_output(
    destination: &Path,
    produce: impl FnOnce(&mut BufWriter<File>) -> Result<(), String>,
) -> Result<PreparedOutput, String> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if destination.file_name().is_none() {
        return Err("Export destination has no filename".into());
    }
    let scratch = crate::sandbox::replacement_directory(destination).map(ScratchDirectory);
    let temporary_path = scratch.as_ref().map_or(parent, |scratch| scratch.0.as_path())
        .join(format!(".kiwix-export-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&temporary_path)
        .map_err(|e| format!("Creating export: {e}"))?;
    // Arm cleanup only after create_new proves that this export owns the name.
    let temporary = Temporary(temporary_path);
    let mut output = BufWriter::with_capacity(CHUNK as usize, file);
    produce(&mut output)?;
    output.flush().map_err(display)?;
    output
        .get_ref()
        .sync_all()
        .map_err(|e| format!("Flushing export: {e}"))?;
    drop(output);
    Ok(PreparedOutput { temporary, destination: destination.to_path_buf(), _scratch: scratch })
}

struct MimeLines<'a, W: Write> {
    output: &'a mut W,
    column: usize,
}
impl<W: Write> Write for MimeLines<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for part in bytes.chunks(76) {
            let mut rest = part;
            while !rest.is_empty() {
                let n = rest.len().min(76 - self.column);
                self.output.write_all(&rest[..n])?;
                self.column += n;
                rest = &rest[n..];
                if self.column == 76 {
                    self.output.write_all(b"\r\n")?;
                    self.column = 0;
                }
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}
impl<W: Write> MimeLines<'_, W> {
    fn finish(&mut self) -> io::Result<()> {
        // Even an empty MIME body needs the CRLF preceding the next boundary.
        self.output.write_all(b"\r\n")?;
        self.column = 0;
        Ok(())
    }
}

fn inline_reference(value: &str) -> bool {
    value.is_empty()
        || value.starts_with('#')
        || value
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}
fn local_reference(base: &Url, reference: &str, origin: &Url) -> Result<Option<Url>, String> {
    let reference = reference.trim();
    if inline_reference(reference) {
        return Ok(None);
    }
    if reference.len() > MAX_URL {
        return Err("Static asset URL exceeds 16 KiB".into());
    }
    let mut url = base
        .join(reference)
        .map_err(|e| format!("Invalid static asset URL {reference:?}: {e}"))?;
    // Absolutely no web/file/data fetch: inline data already lives in its parent.
    if url.origin() != origin.origin() || !url.username().is_empty() || url.password().is_some() {
        return Ok(None);
    }
    url.set_fragment(None);
    Ok(Some(url))
}

struct References {
    base: Option<String>,
    values: Vec<String>,
}
fn add_reference(references: &mut Vec<String>, value: &str) -> Result<(), String> {
    let value = value.trim();
    if inline_reference(value) {
        return Ok(());
    }
    if references.len() >= MAX_REFERENCES {
        return Err("Too many static references in one text asset".into());
    }
    if value.len() > MAX_URL {
        return Err("Static asset URL exceeds 16 KiB".into());
    }
    references.push(value.to_owned());
    Ok(())
}

/// `spent` is shared by every document of one export, so many small parts
/// cannot each claim the fixed allowances.
fn markup_references(source: &str, spent: &mut html::Spent) -> Result<References, String> {
    let document = html::Html::parse_document_sharing(source, &html::ARTICLE_BOUNDS, None, spent)
        .map_err(|_| "This article's markup is too deeply nested or too complex to export".to_owned())?;
    let base = document.elements().find_map(|element| {
        let element = html::element(&element);
        if html::name(&element) == "base" {
            html::attr(&element, "href").map(str::to_owned)
        } else {
            None
        }
    });
    let mut references = Vec::new();
    for element in document.elements() {
        let node = html::element(&element);
        let name = html::name(&node);
        let attribute = match name {
            "img" | "iframe" | "frame" | "audio" | "video" | "source" | "track" | "embed"
            | "input" => Some("src"),
            "script" => Some(if html::attr(&node, "src").is_some() {
                "src"
            } else if html::attr(&node, "href").is_some() {
                "href"
            } else {
                "xlink:href"
            }),
            "object" => Some("data"),
            "image" | "use" | "feImage" | "feimage" => {
                Some(if html::attr(&node, "href").is_some() {
                    "href"
                } else {
                    "xlink:href"
                })
            }
            "link" => {
                let rel = html::attr(&node, "rel").unwrap_or("");
                if rel.split_ascii_whitespace().any(|r| {
                    matches!(
                        r.to_ascii_lowercase().as_str(),
                        "stylesheet"
                            | "icon"
                            | "preload"
                            | "modulepreload"
                            | "apple-touch-icon"
                            | "mask-icon"
                    )
                }) {
                    Some("href")
                } else {
                    None
                }
            }
            _ => None,
        };
        let value = attribute.and_then(|key| {
            if key == "xlink:href" {
                node.attrs
                    .iter()
                    .find(|attribute| {
                        attribute.name.local.as_ref() == "href"
                            && attribute.name.ns.as_ref() == "http://www.w3.org/1999/xlink"
                    })
                    .map(|attribute| attribute.value.as_ref())
            } else {
                html::attr(&node, key)
            }
        });
        if let Some(value) = value {
            add_reference(&mut references, value)?;
        }
        if name == "video" {
            if let Some(value) = html::attr(&node, "poster") {
                add_reference(&mut references, value)?;
            }
        }
        if let Some(value) = html::attr(&node, "background") {
            add_reference(&mut references, value)?;
        }
        for key in ["srcset", "imagesrcset"] {
            if let Some(value) = html::attr(&node, key) {
                for url in srcset_references(value) {
                    add_reference(&mut references, url)?;
                }
            }
        }
        for key in [
            "style",
            "fill",
            "stroke",
            "filter",
            "clip-path",
            "mask",
            "cursor",
            "marker-start",
            "marker-mid",
            "marker-end",
        ] {
            if let Some(style) = html::attr(&node, key) {
                for url in css_references(style, Malformed::Ignore)? {
                    add_reference(&mut references, &url)?;
                }
            }
        }
        if name == "style" {
            for url in css_references(&html::text(&element), Malformed::Ignore)? {
                add_reference(&mut references, &url)?;
            }
        }
    }
    Ok(References {
        base,
        values: references,
    })
}

// The HTML srcset algorithm consumes a URL through whitespace (not through
// commas: data URLs contain commas), then skips descriptors through a comma.
fn srcset_references(mut source: &str) -> Vec<&str> {
    let mut urls = Vec::new();
    while !source.is_empty() {
        source = source.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if source.is_empty() {
            break;
        }
        let end = source
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(source.len());
        let token = &source[..end];
        source = &source[end..];
        let url = token.trim_end_matches(',');
        if !url.is_empty() {
            urls.push(url);
        }
        if token.ends_with(',') {
            continue;
        }
        let mut parentheses = 0usize;
        let mut next = source.len();
        for (index, ch) in source.char_indices() {
            match ch {
                '(' => parentheses += 1,
                ')' => parentheses = parentheses.saturating_sub(1),
                ',' if parentheses == 0 => {
                    next = index + 1;
                    break;
                }
                _ => {}
            }
        }
        source = &source[next..];
    }
    urls
}

/// Calls `each` with every CSS token in `source`, comments and whitespace
/// included, with its byte offset and source text, in one flat stream: a
/// block's contents follow its opening token, and its closing token comes
/// through as a token too. The tokens are cssparser's, which follow the CSS
/// Syntax tokenizer browsers use (escapes in names decoded, `url(` with no
/// quote as one URL token). cssparser hands out nested blocks recursively;
/// tokenization does not depend on context, so a fresh parser resumes after
/// each opening token instead, and nesting depth costs no stack.
pub(crate) fn css_tokens(
    source: &str,
    mut each: impl FnMut(usize, &str, &cssparser::Token<'_>) -> Result<(), String>,
) -> Result<(), String> {
    use cssparser::{Parser, ParserInput, Token};
    let mut offset = 0;
    'resume: loop {
        let mut input = ParserInput::new(&source[offset..]);
        let mut parser = Parser::new(&mut input);
        loop {
            let start = parser.position().byte_index();
            let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
                return Ok(());
            };
            let end = parser.position().byte_index();
            each(
                offset + start,
                &source[offset + start..offset + end],
                &token,
            )?;
            if matches!(
                token,
                Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock
            ) {
                offset += end;
                continue 'resume;
            }
        }
    }
}

/// What [`css_references`] does with CSS the browser treats as malformed.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Malformed {
    /// As the browser does, for archive exports. A bad URL (`url(a(b))`,
    /// `url(a"b)`), a `url()` with more than one string, or a string broken by
    /// a newline is no reference: the browser fetches nothing for it, so there
    /// is nothing to package. A URL or string that the end of the sheet cuts
    /// short is still a reference, since the browser closes it and fetches it.
    Ignore,
    /// Fail on all of them, for the PDF sanitizer, which refuses anything it
    /// cannot vouch for.
    Refuse,
}

// Resource references in CSS, as the browser finds them: url tokens, url() and
// src() with a string, @import strings, and image-set() string candidates.
// Function context excludes descriptors such as type("image/png") while allowing
// nested image-set() and the supported -webkit-image-set() spelling.
pub(crate) fn css_references(source: &str, malformed: Malformed) -> Result<Vec<String>, String> {
    use cssparser::Token;
    // Two bytes each: a 16 MiB sheet can open millions of blocks.
    #[derive(Clone, Copy)]
    enum Block {
        Other(u8),
        ImageSet(bool),
        // url() or src() with a string, as URL_* flags: url() (strict),
        // whether its string (kept in `values`) was seen, and whether
        // anything else was in it.
        Url(u8),
    }
    const URL_STRICT: u8 = 1;
    const URL_VALUE: u8 = 2;
    const URL_EXTRA: u8 = 4;
    const MALFORMED_URL: &str = "Malformed CSS url() in article assets";
    // A string or url token is unterminated when EOF ended it.
    let closed = |text: &str, close: char| {
        text.len() > 1
            && text.ends_with(close)
            && text[..text.len() - 1]
                .bytes()
                .rev()
                .take_while(|&b| b == b'\\')
                .count()
                % 2
                == 0
    };
    let refuse = malformed == Malformed::Refuse;
    let mut references = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    let mut import = false;
    css_tokens(source, |_, text, token| {
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            return Ok(());
        }
        let image_candidate = matches!(blocks.last(), Some(Block::ImageSet(true)));
        match blocks.last_mut() {
            Some(Block::ImageSet(start)) => *start = false,
            Some(Block::Url(flags)) if !matches!(token, Token::CloseParenthesis) => match token {
                Token::QuotedString(string) if *flags & URL_VALUE == 0 => {
                    *flags |= URL_VALUE;
                    values.push(string.to_string());
                }
                _ => *flags |= URL_EXTRA,
            },
            _ => {}
        }
        let was_import = std::mem::take(&mut import);
        match token {
            Token::QuotedString(value) => {
                if refuse && !closed(text, text.chars().next().unwrap_or('"')) {
                    return Err("Unterminated CSS string in article assets".into());
                }
                if was_import || image_candidate {
                    push_css(&mut references, value.to_string())?;
                }
            }
            Token::BadString(_) if refuse => return Err("Unescaped newline in CSS string".into()),
            Token::UnquotedUrl(value) => {
                if refuse && !closed(text, ')') {
                    return Err(MALFORMED_URL.into());
                }
                push_css(&mut references, value.to_string())?;
            }
            Token::BadUrl(_) if refuse => return Err(MALFORMED_URL.into()),
            Token::AtKeyword(name) => import = name.eq_ignore_ascii_case("import"),
            Token::Function(name) => blocks.push(
                if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") {
                    Block::Url(if name.eq_ignore_ascii_case("url") {
                        URL_STRICT
                    } else {
                        0
                    })
                } else if name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set")
                {
                    Block::ImageSet(true)
                } else {
                    Block::Other(b')')
                },
            ),
            Token::ParenthesisBlock => blocks.push(Block::Other(b')')),
            Token::SquareBracketBlock => blocks.push(Block::Other(b']')),
            Token::CurlyBracketBlock => blocks.push(Block::Other(b'}')),
            Token::Comma => {
                if let Some(Block::ImageSet(start)) = blocks.last_mut() {
                    *start = true;
                }
            }
            Token::CloseParenthesis | Token::CloseSquareBracket | Token::CloseCurlyBracket => {
                let close = match token {
                    Token::CloseParenthesis => b')',
                    Token::CloseSquareBracket => b']',
                    _ => b'}',
                };
                // Only the matching token closes a block; others are content.
                let closes = match blocks.last() {
                    Some(Block::Other(expected)) => *expected == close,
                    Some(_) => close == b')',
                    None => false,
                };
                if closes {
                    if let Some(Block::Url(flags)) = blocks.pop() {
                        let value = (flags & URL_VALUE != 0).then(|| values.pop()).flatten();
                        if flags & URL_STRICT != 0 && (flags & URL_EXTRA != 0 || value.is_none()) {
                            if refuse {
                                return Err(MALFORMED_URL.into());
                            }
                        } else if let Some(value) = value {
                            push_css(&mut references, value)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    // EOF closes any open blocks, and the browser fetches a url() or src() it
    // closed that way (unless the url() was malformed).
    for block in blocks.into_iter().rev() {
        if let Block::Url(flags) = block {
            let value = (flags & URL_VALUE != 0).then(|| values.pop()).flatten();
            let strict = flags & URL_STRICT != 0;
            if strict && refuse {
                return Err(MALFORMED_URL.into());
            }
            if let Some(value) = value.filter(|_| !(strict && flags & URL_EXTRA != 0)) {
                push_css(&mut references, value)?;
            }
        }
    }
    Ok(references)
}
fn push_css(references: &mut Vec<String>, value: String) -> Result<(), String> {
    if inline_reference(&value) {
        return Ok(());
    }
    if value.len() > MAX_URL || references.len() >= MAX_REFERENCES {
        return Err("CSS static-reference limit exceeded".into());
    }
    if !value.is_empty() {
        references.push(value);
    }
    Ok(())
}

/// A Wikipedia-shaped article for parser tests: skin wrappers, a nested
/// infobox, reference superscripts, lists and navboxes of tables within tables.
/// Its only image is a data URL, so the live PDF sanitizer accepts it.
#[cfg(test)]
pub(crate) fn test_article(paragraphs: usize) -> String {
    let mut source = String::from("<!doctype html><html><head><title>Article</title><link rel=stylesheet href=s.css></head><body class=mediawiki>");
    source.push_str(&"<div class=wrapper>".repeat(12));
    source.push_str("<table class=infobox><tbody>");
    for row in 0..40 {
        source.push_str(&format!("<tr><th scope=row>Field {row}</th><td><div class=plainlist><ul><li><a href=\"A{row}\">Value</a><sup class=reference><a href=\"#cite-{row}\">[{row}]</a></sup></li><li><span class=nowrap><b>Bold</b> <i>italic</i></span></li></ul></div></td></tr>"));
    }
    source.push_str("</tbody></table>");
    for index in 0..paragraphs {
        source.push_str(&format!("<p>The <b>article</b> body <a href=\"Link_{index}\" title=\"Link\">links</a> to <i>another <a href=\"x\">page</a></i>, with a citation<sup id=\"r{index}\" class=reference><a href=\"#n{index}\"><span>[</span>{index}<span>]</span></a></sup>.<br>Next line &amp; more text &mdash; &#x2014;.</p>"));
        if index % 20 == 0 {
            source.push_str("<h2><span class=mw-headline id=Section>Section</span><span class=mw-editsection><span>[</span><a href=edit>edit</a><span>]</span></span></h2><ul><li>One<ul><li>Two<ol><li>Three</li></ol></li></ul></li></ul><figure><a href=f><img src=\"data:image/png;base64,AAAA\" width=220></a><figcaption>Caption <i>x</i></figcaption></figure>");
        }
    }
    // Navboxes nest tables about eight levels deep.
    let navbox = "<div class=navbox><table><tbody><tr><td><div><table><tbody><tr><td><div><ul><li><a href=n>Nav</a></li></ul></div></td></tr></tbody></table></div></td></tr></tbody></table></div>";
    source.push_str(&navbox.repeat(4).replace("<li><a href=n>Nav</a></li>", &format!("<li>{navbox}</li>")));
    source.push_str(&"</div>".repeat(12));
    source.push_str("</body></html>");
    source
}

/// HTML parsing for exports, previews, search snippets and the live PDF
/// sanitizer: html5ever building a dom_query tree, read with the semantics
/// these callers have always had (from scraper 0.24):
/// - html5ever's default options, so scripting is enabled and `<noscript>`
///   content is raw text;
/// - [`Html::elements`] yields attached elements in the order the parser
///   created them, not tree order (the two differ after table foster
///   parenting and misnested formatting), including `<template>` contents;
/// - [`attr`] matches only un-namespaced attributes (`href`, not `xlink:href`)
///   and [`attrs`] is ordered by qualified name, not source order;
/// - [`text`] includes `<template>` contents.
///
/// Untrusted documents go through [`Html::parse_document_bounded`], which
/// refuses pathological markup while parsing and can be cancelled.
pub(crate) mod html {
    use dom_query::{Document, Element, NodeData, NodeId, NodeRef};
    use html5ever::{
        local_name, ns,
        tendril::{StrTendril, TendrilSink},
        tokenizer::TokenizerOpts,
        tree_builder::{ElementFlags, NodeOrText, QuirksMode, Tracer, TreeSink},
        Attribute, ParseOpts, QualName,
    };
    use std::{
        borrow::Cow,
        cell::{Cell, Ref, RefCell},
        collections::HashMap,
        time::{Duration, Instant},
    };
    use tokio_util::sync::CancellationToken;

    pub(crate) struct Html {
        document: Document,
        elements: Vec<NodeId>,
        work: u64,
    }

    /// Why [`Html::parse_document_bounded`] stopped before finishing.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum Refused {
        Cancelled,
        /// The parser's own state passed a ceiling in [`Bounds`].
        TooComplex,
    }

    /// Ceilings on html5ever's work for untrusted documents.
    ///
    /// html5ever does linear scans of its open-element stack and its active
    /// formatting list for most tokens, so nesting makes the parse quadratic
    /// (40,000 nested `<div>` take seconds), stale formatting elements are
    /// re-created in every later paragraph, and some steps are quadratic in a
    /// tag's attributes. These ceilings read the parser's real state rather
    /// than estimating it from the markup:
    /// - `open`: open elements plus active formatting entries. The live PDF
    ///   renderer refuses render depth over 512, which is at most ~515 open
    ///   elements plus as many formatting entries, so 1,040 refuses no
    ///   document that the PDF renderer would accept (other callers had no
    ///   depth limit before);
    /// - `nodes`, `attributes`: a fixed allowance plus one per `nodes.1` /
    ///   `attributes.1` input bytes, bounding the tree's memory. Nodes are
    ///   elements, comments, processing instructions and text nodes (adjacent
    ///   text merges into one); re-created formatting elements copy their
    ///   attributes;
    /// - `work`: a fixed allowance plus `work.1` units per input byte. A unit
    ///   is about one stack or list entry the tree builder examines;
    /// - `cpu`: a fixed allowance plus `cpu.1` nanoseconds per input byte of
    ///   this thread's CPU time (wall-clock time only if the platform's
    ///   thread clock fails), for the tokenizer's per-tag attribute checks,
    ///   which no tree-builder hook observes. Debug builds allow 25 times as
    ///   much.
    ///
    /// The per-byte allowances count only the input fed so far, so expensive
    /// markup cannot borrow the allowance of a long, cheap tail; parses that
    /// share a [`Spent`] (one MHTML export) share one fixed allowance.
    ///
    /// Measured (2026-11, release build) on 13 real pages of 57 KB-3.9 MB
    /// (English Wikipedia articles, link-dense lists and indexes, Parsoid HTML,
    /// Wiktionary, a Project Gutenberg book): peak `open` 10-45, nodes 31-89
    /// bytes each (elements 63-260), attributes 0.02-0.03 per byte, work
    /// 0.16-0.50 units per byte. A node costs about 150 bytes of memory, so
    /// the node allowance permits about 650 MB for 16 MiB of input; data
    /// tables of tiny cells reach 4-6 bytes per node. Known limits on
    /// legitimate-looking markup:
    /// - unclosed legacy `<font>` in every short paragraph re-creates the
    ///   fonts each time (7.1 units and 0.5 attributes per byte), and passes
    ///   the node allowance after a few MB of such paragraphs;
    /// - work grows with nesting depth: sustained content 200 or more
    ///   elements deep (real pages peak at 45) passes the work allowance
    ///   after about 2 MB.
    ///
    /// Deep nesting is refused within milliseconds; markup only the CPU
    /// ceiling observes costs up to that ceiling (release: 1 s plus 0.5 s per
    /// MB fed, shared by one MHTML export, which cannot be cancelled). The
    /// `html_corpus` test reports these figures for any set of pages.
    #[derive(Clone, Copy)]
    pub(crate) struct Bounds {
        pub(crate) open: usize,
        pub(crate) nodes: (usize, usize),
        pub(crate) attributes: (u64, u64),
        pub(crate) work: (u64, u64),
        pub(crate) cpu: (Duration, u64),
    }

    pub(crate) const ARTICLE_BOUNDS: Bounds = Bounds {
        open: 1040,
        nodes: (1 << 18, 4),
        attributes: (1 << 20, 2),
        work: (1 << 20, 12),
        cpu: (Duration::from_secs(1), 500),
    };

    /// Ceilings for one archive-supplied search snippet: a highlight-marked
    /// text fragment of a few hundred bytes. Far below [`ARTICLE_BOUNDS`]
    /// because a page of results parses many of them, and a snippet is never
    /// a document. Deep nesting and pathological markup are refused within
    /// milliseconds, so one hostile result cannot hold a search request.
    pub(crate) const SNIPPET_BOUNDS: Bounds = Bounds {
        open: 256,
        nodes: (1 << 12, 2),
        attributes: (1 << 12, 2),
        work: (1 << 14, 16),
        cpu: (Duration::from_millis(50), 100),
    };

    /// Work and CPU time already used by earlier parses sharing one budget.
    #[derive(Default)]
    pub(crate) struct Spent {
        bytes: u64,
        work: u64,
        cpu: Duration,
    }

    /// Input is fed in chunks of at most this many bytes; ceilings and
    /// cancellation are checked between chunks, since html5ever cannot be
    /// interrupted inside one.
    pub(super) const CHUNK: usize = 256;

    /// CPU time is read after at least this much more input.
    const CPU_CHECK: usize = 256;

    const FORMATTING: [&str; 14] = [
        "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong",
        "tt", "u",
    ];
    /// Every formatting start tag begins with `<` and one of these letters,
    /// in either case.
    const FORMATTING_INITIALS: &[u8] = b"abcefinstu";

    fn formatting(name: &QualName) -> Option<usize> {
        if name.ns != ns!(html) {
            return None;
        }
        FORMATTING.iter().position(|tag| *tag == &*name.local)
    }

    /// Work to clone and sort a formatting tag's attributes, as html5ever
    /// does for every same-named active formatting entry ("Noah's Ark"):
    /// sorting compares names, equal lists then compare values.
    fn attribute_units(attrs: &[Attribute]) -> u64 {
        let count = attrs.len() as u64;
        let log = u64::from(usize::BITS - attrs.len().leading_zeros());
        let names = attrs.iter().map(|attribute| attribute.name.local.len() as u64).sum::<u64>();
        let values = attrs.iter().map(|attribute| attribute.value.len() as u64).sum::<u64>();
        count
            .saturating_mul(log + 2)
            .saturating_add(names.saturating_mul(log).saturating_add(values) / 32)
    }

    /// Formatting elements by name: (count, attribute units).
    type FormattingTally = [(u64, u64); FORMATTING.len()];

    /// Counts the tree builder's open elements and active formatting entries,
    /// and collects the distinct formatting elements among them.
    struct Count<'a> {
        formatting_elements: &'a HashMap<NodeId, (usize, u64)>,
        all: Cell<usize>,
        formatting: RefCell<Vec<NodeId>>,
    }
    impl Tracer for Count<'_> {
        type Handle = NodeId;
        fn trace_handle(&self, node: &NodeId) {
            self.all.set(self.all.get() + 1);
            if self.formatting_elements.contains_key(node) {
                self.formatting.borrow_mut().push(*node);
            }
        }
    }
    impl Count<'_> {
        /// Distinct formatting elements, open or in the active formatting list:
        /// a superset of that list, whose entries are all formatting elements.
        fn tally(self) -> FormattingTally {
            let mut nodes = self.formatting.into_inner();
            nodes.sort_unstable();
            nodes.dedup();
            let mut tally = FormattingTally::default();
            for node in nodes {
                let (index, units) = self.formatting_elements[&node];
                let entry = &mut tally[index];
                *entry = (entry.0 + 1, entry.1.saturating_add(units));
            }
            tally
        }
    }

    /// This thread's CPU time since creation, so a suspended app is not
    /// charged: `cpu_time::thread_cpu_time_micros` reads
    /// `CLOCK_THREAD_CPUTIME_ID` on unix and `GetThreadTimes` on Windows;
    /// wall-clock time only if the platform call fails.
    struct CpuClock {
        thread: Option<u128>,
        wall: Instant,
    }
    impl CpuClock {
        fn start() -> Self {
            Self { thread: Self::thread(), wall: Instant::now() }
        }
        #[cfg(any(unix, windows))]
        fn thread() -> Option<u128> {
            crate::cpu_time::thread_cpu_time_micros()
        }
        #[cfg(not(any(unix, windows)))]
        fn thread() -> Option<u128> {
            None
        }
        fn elapsed(&self) -> Duration {
            match (self.thread, Self::thread()) {
                (Some(start), Some(now)) => {
                    // A sub-microsecond remainder is below the resolution the
                    // diagnostic reports, and `u64::MAX` microseconds is far
                    // beyond any process lifetime.
                    Duration::from_micros(u64::try_from(now.saturating_sub(start)).unwrap_or(u64::MAX))
                }
                _ => self.wall.elapsed(),
            }
        }
    }

    /// html5ever's defaults, except that only a leading byte order mark is
    /// dropped: html5ever drops U+FEFF at the start of every fed chunk.
    fn options() -> ParseOpts {
        ParseOpts {
            tokenizer: TokenizerOpts { discard_bom: false, ..TokenizerOpts::default() },
            ..ParseOpts::default()
        }
    }

    /// The end of the next chunk: at most `chunk` bytes, and before a `<`
    /// that may start a formatting tag, so that each chunk holds at most one
    /// formatting start tag and its "Noah's Ark" charge sees the list as it is.
    fn chunk_end(rest: &str, chunk: usize) -> usize {
        let bytes = rest.as_bytes();
        let mut end = chunk.max(1).min(bytes.len());
        if let Some(split) = (1..end).find(|&index| {
            bytes[index] == b'<'
                && bytes
                    .get(index + 1)
                    .is_none_or(|next| FORMATTING_INITIALS.contains(&next.to_ascii_lowercase()))
        }) {
            end = split;
        }
        while !rest.is_char_boundary(end) {
            end += 1;
        }
        end
    }

    impl Html {
        /// Unbounded; only for markup this crate wrote or test literals.
        #[cfg(test)]
        pub(crate) fn parse_document(source: &str) -> Self {
            let source = source.strip_prefix('\u{feff}').unwrap_or(source);
            html5ever::parse_document(Sink::new(Document::default()), options()).one(source)
        }

        /// Parses an untrusted document, refusing it as soon as the parser's
        /// work passes `bounds` or `cancelled` fires. The tree is identical to
        /// an unbounded parse whenever this succeeds.
        pub(crate) fn parse_document_bounded(
            source: &str,
            bounds: &Bounds,
            cancelled: Option<&CancellationToken>,
        ) -> Result<Self, Refused> {
            Self::parse_document_sharing(source, bounds, cancelled, &mut Spent::default())
        }

        /// As [`Self::parse_document_bounded`], with the work and CPU
        /// allowances shared by every parse that uses `spent`.
        pub(crate) fn parse_document_sharing(
            source: &str,
            bounds: &Bounds,
            cancelled: Option<&CancellationToken>,
            spent: &mut Spent,
        ) -> Result<Self, Refused> {
            Self::parse_chunked(source, bounds, cancelled, CHUNK, spent).map(|(html, _)| html)
        }

        /// Also returns the peak `open` count seen at a chunk boundary, the
        /// element count and the work units used by this parse.
        pub(super) fn parse_chunked(
            source: &str,
            bounds: &Bounds,
            cancelled: Option<&CancellationToken>,
            chunk: usize,
            spent: &mut Spent,
        ) -> Result<(Self, (usize, usize, u64)), Refused> {
            Self::parse_chunked_as(source, bounds, cancelled, chunk, spent, false)
        }

        /// Parses an untrusted fragment in a `<body>` context, refusing it as
        /// soon as the parser's work passes `bounds` or `cancelled` fires. The
        /// tree is identical to an unbounded parse whenever this succeeds.
        pub(crate) fn parse_fragment_bounded(
            source: &str,
            bounds: &Bounds,
            cancelled: Option<&CancellationToken>,
        ) -> Result<Self, Refused> {
            Self::parse_chunked_as(source, bounds, cancelled, CHUNK, &mut Spent::default(), true)
                .map(|(html, _)| html)
        }

        fn parse_chunked_as(
            source: &str,
            bounds: &Bounds,
            cancelled: Option<&CancellationToken>,
            chunk: usize,
            spent: &mut Spent,
            fragment: bool,
        ) -> Result<(Self, (usize, usize, u64)), Refused> {
            let clock = CpuClock::start();
            // Unoptimized builds parse 25-40x slower than release builds.
            let slowdown = if cfg!(debug_assertions) { 25 } else { 1 };
            let source = source.strip_prefix('\u{feff}').unwrap_or(source);
            let limits = Limits {
                bounds, cancelled, clock: &clock, slowdown,
                prior_bytes: spent.bytes, prior_work: spent.work, prior_cpu: spent.cpu,
                fed: Cell::new(0), next_check: Cell::new(CHUNK as u64),
            };
            let mut sink = Sink::new(if fragment { Document::fragment_sink() } else { Document::default() });
            sink.limits = Some(&limits);
            let mut parser = if fragment {
                html5ever::parse_fragment(
                    sink,
                    options(),
                    QualName::new(None, ns!(html), local_name!("body")),
                    Vec::new(),
                    false,
                )
            } else {
                html5ever::parse_document(sink, options())
            };
            let mut rest = source;
            let mut fed = 0u64;
            let mut peak = 0;
            let mut unclocked = 0;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                if cancelled.is_some_and(CancellationToken::is_cancelled) {
                    break Err(Refused::Cancelled);
                }
                if rest.is_empty() {
                    assert!(parser.input_buffer.is_empty(), "HTML parser retained input before EOF");
                    #[cfg(test)]
                    BEFORE_EOF.with(|hook| {
                        if let Some(hook) = hook.borrow_mut().take() { hook(); }
                    });
                    // EOF can drain buffered table text through the tree sink.
                    parser.tokenizer.end();
                    parser.tokenizer.sink.sink.check_limits(true);
                    break Ok(());
                }
                let (head, tail) = rest.split_at(chunk_end(rest, chunk));
                rest = tail;
                fed += head.len() as u64;
                limits.fed.set(fed);
                parser.process(StrTendril::from_slice(head));
                let builder = &parser.tokenizer.sink;
                let sink = &builder.sink;
                let count = {
                    let formatting_elements = sink.formatting_elements.borrow();
                    let count = Count {
                        formatting_elements: &formatting_elements,
                        all: Cell::new(0),
                        formatting: RefCell::default(),
                    };
                    builder.trace_handles(&count);
                    let open = count.all.get();
                    sink.formatting.replace(count.tally());
                    open
                };
                peak = peak.max(count);
                sink.charge(count as u64);
                sink.ark.set(0);
                // Allowances grow with the input fed so far.
                let shared = spent.bytes.saturating_add(fed);
                let nodes = bounds.nodes.0 as u64 + fed / bounds.nodes.1.max(1) as u64;
                let attributes = bounds.attributes.0 + fed / bounds.attributes.1.max(1);
                let work = bounds.work.0.saturating_add(shared.saturating_mul(bounds.work.1));
                unclocked += head.len();
                let slow = unclocked >= CPU_CHECK && {
                    unclocked = 0;
                    let cpu = bounds
                        .cpu
                        .0
                        .saturating_add(Duration::from_nanos(shared.saturating_mul(bounds.cpu.1)))
                        .saturating_mul(slowdown);
                    spent.cpu.saturating_add(clock.elapsed()) > cpu
                };
                if count > bounds.open
                    || sink.nodes.get() > nodes
                    || sink.attributes.get() > attributes
                    || spent.work.saturating_add(sink.work.get()) > work
                    || slow
                {
                    break Err(Refused::TooComplex);
                }
            }));
            let work = parser.tokenizer.sink.sink.work.get();
            spent.bytes = spent.bytes.saturating_add(fed);
            spent.work = spent.work.saturating_add(work);
            spent.cpu = spent.cpu.saturating_add(clock.elapsed());
            match outcome {
                Ok(result) => result?,
                Err(payload) => match payload.downcast::<ParserAbort>() {
                    Ok(refusal) => return Err(refusal.0),
                    Err(payload) => std::panic::resume_unwind(payload),
                },
            }
            // Match Parser::finish after its one tokenizer.end(), not a second EOF.
            let html = parser.tokenizer.sink.sink.finish();
            let stats = (peak, html.elements.len(), html.work);
            Ok((html, stats))
        }

        /// Unbounded; only for markup this crate wrote or test literals.
        #[cfg(test)]
        pub(crate) fn parse_fragment(source: &str) -> Self {
            html5ever::parse_fragment(
                Sink::new(Document::fragment_sink()),
                ParseOpts::default(),
                QualName::new(None, ns!(html), local_name!("body")),
                Vec::new(),
                false,
            )
            .one(source)
        }

        /// The `<html>` element, which the parser always creates.
        pub(crate) fn root_element(&self) -> NodeRef<'_> {
            self.document
                .root()
                .first_element_child()
                .expect("the HTML parser always creates a root element")
        }

        /// Elements that have a parent, in parser creation order.
        pub(crate) fn elements(&self) -> impl Iterator<Item = NodeRef<'_>> {
            self.elements
                .iter()
                .map(|id| NodeRef::new(*id, &self.document.tree))
                .filter(|node| node.parent().is_some())
        }

        /// The whole tree serialized, for comparing parses.
        #[cfg(test)]
        pub(crate) fn serialized(&self) -> String {
            self.document.html().to_string()
        }

        /// Elements matching a CSS selector, in [`Self::elements`] order.
        #[cfg(test)]
        pub(crate) fn select(&self, selector: &str) -> Vec<NodeRef<'_>> {
            let matcher = dom_query::Matcher::new(selector).unwrap();
            self.elements()
                .filter(|node| node.is_match(&matcher))
                .collect()
        }
    }

    pub(crate) fn element<'a>(node: &NodeRef<'a>) -> Ref<'a, Element> {
        node.element_ref()
            .expect("the HTML helpers only pass element nodes")
    }

    pub(crate) fn name(element: &Element) -> &str {
        &element.name.local
    }

    pub(crate) fn attr<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
        element
            .attrs
            .iter()
            .find(|attribute| {
                attribute.name.prefix.is_none()
                    && attribute.name.ns == ns!()
                    && &*attribute.name.local == name
            })
            .map(|attribute| &*attribute.value)
    }

    /// `(local name, value)` pairs ordered by (prefix, namespace, local name).
    pub(crate) fn attrs(element: &Element) -> Vec<(&str, &str)> {
        fn key(attribute: &Attribute) -> (Option<&str>, &str, &str) {
            let name = &attribute.name;
            (name.prefix.as_deref(), &name.ns, &name.local)
        }
        let mut attributes = element.attrs.iter().collect::<Vec<_>>();
        attributes.sort_unstable_by(|left, right| key(left).cmp(&key(right)));
        attributes
            .into_iter()
            .map(|attribute| (&*attribute.name.local, &*attribute.value))
            .collect()
    }

    /// The text node's contents, or `None` for any other kind of node.
    pub(crate) fn text_node(node: &NodeRef<'_>) -> Option<StrTendril> {
        node.query(|node| match &node.data {
            NodeData::Text { contents } => Some(contents.clone()),
            _ => None,
        })
        .flatten()
    }

    /// All descendant text in tree order. Unlike [`NodeRef::text`], this
    /// includes `<template>` contents. Iterative, for any nesting depth.
    pub(crate) fn text(node: &NodeRef<'_>) -> String {
        let mut text = String::new();
        let mut pending = vec![node.id];
        while let Some(id) = pending.pop() {
            let node = NodeRef::new(id, node.tree);
            let template = node
                .query(|node| match &node.data {
                    NodeData::Text { contents } => {
                        text.push_str(contents);
                        None
                    }
                    NodeData::Element(element) => element.template_contents,
                    _ => None,
                })
                .flatten();
            pending.extend(node.children_it(true).map(|child| child.id));
            // Template contents precede the template's (normally absent) children.
            pending.extend(template);
        }
        text
    }

    /// Characters that are invisible, render blank or reorder text, and so can
    /// hide text from the person reading it. ZWNJ and ZWJ (U+200C, U+200D)
    /// are kept: Persian, Indic scripts and emoji need them.
    pub(crate) fn invisible(c: char) -> bool {
        matches!(c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{180E}'
            | '\u{200B}' | '\u{200E}' | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{3164}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0000}'..='\u{E007F}' | '\u{E0100}'..='\u{E01EF}')
    }

    /// Bounded plain-text builder: collapses runs of spaces, keeps at most one
    /// blank line, and drops control and invisible characters.
    struct Plain {
        out: String,
        count: usize,
        max: usize,
        space: bool,
        truncated: bool,
    }
    impl Plain {
        fn new(max: usize) -> Self {
            Self { out: String::new(), count: 0, max, space: false, truncated: false }
        }
        fn full(&self) -> bool {
            self.truncated
        }
        fn push_text(&mut self, text: &str) {
            for c in text.chars() {
                if self.truncated {
                    return;
                }
                if c.is_whitespace() {
                    self.space = !self.out.is_empty() && !self.out.ends_with('\n');
                    continue;
                }
                if c.is_control() || invisible(c) {
                    continue;
                }
                if self.space {
                    self.push(' ');
                    self.space = false;
                }
                self.push(c);
            }
        }
        fn push(&mut self, c: char) {
            if self.count >= self.max {
                self.truncated = true;
                return;
            }
            self.out.push(c);
            self.count += 1;
        }
        fn trim_spaces(&mut self) {
            self.space = false;
            while self.out.ends_with(' ') {
                self.out.pop();
                self.count -= 1;
            }
        }
        /// Add layout without counting it as truncation: only text does.
        fn push_layout(&mut self, c: char) {
            if self.count < self.max {
                self.out.push(c);
                self.count += 1;
            }
        }
        /// End the line; `blank` also leaves an empty line (a paragraph).
        fn line_break(&mut self, blank: bool) {
            self.trim_spaces();
            if self.out.is_empty() {
                return;
            }
            if !self.out.ends_with('\n') {
                self.push_layout('\n');
            }
            if blank && !self.out.ends_with("\n\n") {
                self.push_layout('\n');
            }
        }
        /// Separate table cells on one line.
        fn cell(&mut self) {
            self.trim_spaces();
            if !self.out.is_empty() && !self.out.ends_with('\n') {
                for c in " | ".chars() {
                    self.push_layout(c);
                }
            }
        }
        fn finish(mut self) -> (String, bool) {
            loop {
                if self.out.ends_with(['\n', ' ']) {
                    self.out.pop();
                } else if self.out.ends_with(" |") {
                    // A separator before an empty last cell; content keeps its own "|".
                    self.out.truncate(self.out.len() - 2);
                } else {
                    break;
                }
            }
            (self.out, self.truncated)
        }
    }

    /// Untrusted plain text, bounded and cleaned like [`visible_text`].
    #[cfg_attr(mobile, allow(dead_code))]
    pub(crate) fn plain_text(source: &str, max_chars: usize) -> (String, bool) {
        let mut plain = Plain::new(max_chars);
        for (index, line) in source.split('\n').enumerate() {
            if index > 0 {
                plain.line_break(false);
            }
            plain.push_text(line);
            if plain.full() {
                break;
            }
        }
        plain.finish()
    }

    /// Inline styles that hide an element's text. Stylesheets are not
    /// evaluated, so text hidden by a class still comes through.
    fn hidden_by_style(style: &str) -> bool {
        style.split(';').any(|declaration| {
            let Some((property, value)) = declaration.split_once(':') else { return false; };
            let property = property.trim().to_ascii_lowercase();
            let value = value.trim().trim_end_matches("!important").trim().to_ascii_lowercase();
            let zero = |value: &str| {
                let number = value.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
                number.parse::<f64>().is_ok_and(|number| number == 0.0)
            };
            match property.as_str() {
                "display" => value == "none",
                "visibility" => value == "hidden" || value == "collapse",
                "opacity" => zero(&value),
                "font-size" => zero(&value),
                _ => false,
            }
        })
    }

    /// Elements whose content is never shown as article text.
    fn hidden(element: &Element) -> bool {
        if matches!(
            name(element),
            "head" | "script" | "style" | "template" | "noscript" | "svg" | "math" | "button" | "select"
                | "option" | "input" | "textarea" | "iframe" | "object" | "embed" | "canvas" | "audio"
                | "video" | "map" | "nav"
        ) {
            return true;
        }
        attr(element, "hidden").is_some()
            || attr(element, "aria-hidden").is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
            || attr(element, "style").is_some_and(hidden_by_style)
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Layout {
        Paragraph,
        Line,
        Cell,
        Inline,
    }

    fn layout(name: &str) -> Layout {
        match name {
            "p" | "div" | "ul" | "ol" | "dl" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "table" | "section"
            | "article" | "blockquote" | "pre" | "figure" | "figcaption" | "header" | "footer" | "details"
            | "summary" | "hr" | "main" | "aside" => Layout::Paragraph,
            "br" | "li" | "tr" | "dt" | "dd" | "caption" => Layout::Line,
            "td" | "th" => Layout::Cell,
            _ => Layout::Inline,
        }
    }

    /// The text a reader would see, for agents: skips elements that are never
    /// rendered or are hidden by markup (`hidden`, `aria-hidden`, or an inline
    /// style such as `display:none`), separates paragraphs, lines and table
    /// cells, drops control and invisible characters, and stops at
    /// `max_chars` (the flag reports that). Iterative, for any nesting depth.
    #[cfg_attr(mobile, allow(dead_code))]
    pub(crate) fn visible_text(node: &NodeRef<'_>, max_chars: usize) -> (String, bool) {
        enum Step {
            Enter(NodeId),
            Break(bool),
        }
        let mut plain = Plain::new(max_chars);
        let mut pending = vec![Step::Enter(node.id)];
        while let Some(step) = pending.pop() {
            if plain.full() {
                break;
            }
            let id = match step {
                Step::Break(blank) => {
                    plain.line_break(blank);
                    continue;
                }
                Step::Enter(id) => id,
            };
            let node = NodeRef::new(id, node.tree);
            let (descend, kind) = node
                .query(|data| match &data.data {
                    NodeData::Text { contents } => {
                        plain.push_text(contents);
                        (false, Layout::Inline)
                    }
                    NodeData::Element(element) => {
                        if hidden(element) {
                            (false, Layout::Inline)
                        } else {
                            (true, layout(name(element)))
                        }
                    }
                    _ => (false, Layout::Inline),
                })
                .unwrap_or((false, Layout::Inline));
            if !descend {
                continue;
            }
            match kind {
                Layout::Paragraph | Layout::Line => {
                    let blank = kind == Layout::Paragraph;
                    plain.line_break(blank);
                    pending.push(Step::Break(blank));
                }
                Layout::Cell => plain.cell(),
                Layout::Inline => {}
            }
            pending.extend(node.children_it(true).map(|child| Step::Enter(child.id)));
        }
        plain.finish()
    }

    /// Private control-flow marker: resume_unwind skips the global crash hook.
    /// Only this marker is caught; an interrupted parser is always discarded.
    struct ParserAbort(Refused);

    struct Limits<'a> {
        bounds: &'a Bounds,
        cancelled: Option<&'a CancellationToken>,
        clock: &'a CpuClock,
        slowdown: u32,
        prior_bytes: u64,
        prior_work: u64,
        prior_cpu: Duration,
        fed: Cell<u64>,
        next_check: Cell<u64>,
    }

    impl Limits<'_> {
        fn check(&self, sink: &Sink<'_>) {
            let refused = if self.cancelled.is_some_and(CancellationToken::is_cancelled) {
                Some(Refused::Cancelled)
            } else {
                let fed = self.fed.get();
                let shared = self.prior_bytes.saturating_add(fed);
                let nodes = (self.bounds.nodes.0 as u64).saturating_add(fed / self.bounds.nodes.1.max(1) as u64);
                let attributes = self.bounds.attributes.0.saturating_add(fed / self.bounds.attributes.1.max(1));
                let work = self.bounds.work.0.saturating_add(shared.saturating_mul(self.bounds.work.1));
                let cpu = self.bounds.cpu.0
                    .saturating_add(Duration::from_nanos(shared.saturating_mul(self.bounds.cpu.1)))
                    .saturating_mul(self.slowdown);
                (sink.nodes.get() > nodes
                    || sink.attributes.get() > attributes
                    || self.prior_work.saturating_add(sink.work.get()) > work
                    || self.prior_cpu.saturating_add(self.clock.elapsed()) > cpu)
                    .then_some(Refused::TooComplex)
            };
            if let Some(refused) = refused {
                std::panic::resume_unwind(Box::new(ParserAbort(refused)));
            }
        }
    }

    #[cfg(test)]
    thread_local! {
        static BEFORE_EOF: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }

    /// dom_query's own tree sink, also recording element creation order and
    /// metering the tree builder's work for [`Bounds`]. Parse errors are not
    /// kept.
    struct Sink<'a> {
        document: Document,
        elements: RefCell<Vec<NodeId>>,
        /// Work units: html5ever's scope scans and stack searches ask the sink
        /// about every entry they examine; the formatting comparisons and
        /// attribute merges that do not are charged where they happen.
        work: Cell<u64>,
        /// Nodes created: elements, comments, processing instructions and
        /// text nodes not merged into an adjacent one.
        nodes: Cell<u64>,
        /// Attributes stored on elements (an upper bound).
        attributes: Cell<u64>,
        /// Every formatting element created: (name index, attribute units).
        formatting_elements: RefCell<HashMap<NodeId, (usize, u64)>>,
        /// Distinct formatting elements, open or listed, at the last boundary.
        formatting: RefCell<FormattingTally>,
        /// The "Noah's Ark" charge made in this chunk.
        ark: Cell<u64>,
        limits: Option<&'a Limits<'a>>,
    }

    impl Sink<'_> {
        fn new(document: Document) -> Self {
            Self {
                document,
                elements: RefCell::default(),
                work: Cell::new(0),
                nodes: Cell::new(0),
                attributes: Cell::new(0),
                formatting_elements: RefCell::default(),
                formatting: RefCell::default(),
                ark: Cell::new(0),
                limits: None,
            }
        }
        fn charge(&self, units: u64) {
            self.work.set(self.work.get().saturating_add(units));
            self.check_limits(false);
        }
        fn created(&self) {
            self.nodes.set(self.nodes.get().saturating_add(1));
            self.check_limits(false);
        }
        fn check_limits(&self, force: bool) {
            if let Some(limits) = self.limits {
                let units = self.work.get().saturating_add(self.nodes.get());
                if force || units >= limits.next_check.get() {
                    limits.next_check.set(units.saturating_add(CHUNK as u64));
                    limits.check(self);
                }
            }
        }
        /// Counts the text node an append would create; dom_query merges
        /// text into an adjacent text node instead.
        fn appending(&self, adjacent: Option<NodeRef<'_>>, child: &NodeOrText<NodeId>) {
            if matches!(child, NodeOrText::AppendText(_))
                && !adjacent.is_some_and(|node| node.is_text())
            {
                self.created();
            }
        }
    }

    impl TreeSink for Sink<'_> {
        type Handle = NodeId;
        type Output = Html;
        type ElemName<'a> = Ref<'a, QualName> where Self: 'a;

        fn finish(self) -> Html {
            Html {
                document: self.document,
                elements: self.elements.into_inner(),
                work: self.work.get(),
            }
        }
        fn parse_error(&self, _message: Cow<'static, str>) {}
        fn get_document(&self) -> NodeId {
            self.document.get_document()
        }
        fn elem_name<'a>(&'a self, target: &'a NodeId) -> Ref<'a, QualName> {
            self.charge(1);
            self.document.elem_name(target)
        }
        fn create_element(
            &self,
            name: QualName,
            attrs: Vec<Attribute>,
            flags: ElementFlags,
        ) -> NodeId {
            // Storing the element copies its attributes: re-created
            // formatting elements copy theirs into every later paragraph.
            let count = attrs.len() as u64;
            self.attributes.set(self.attributes.get().saturating_add(count));
            self.charge(count.saturating_mul(2));
            let mut formatting_units = None;
            if let Some(index) = formatting(&name) {
                // A formatting start tag is first compared, without asking the
                // sink, with every active formatting entry back to the last
                // marker ("Noah's Ark"): one unit for another name; for the
                // same name html5ever clones and sorts both attribute lists.
                // A chunk holds at most one formatting start tag, and
                // re-creating stale entries replaces them, so the chunk makes
                // at most one such scan, over the list as traced. Other
                // formatting elements created here (re-created or cloned by the
                // adoption agency) scan nothing; the largest candidate is
                // charged once.
                let units = attribute_units(&attrs);
                formatting_units = Some((index, units));
                let tally = self.formatting.borrow();
                let entries = tally.iter().map(|entry| entry.0).fold(0u64, u64::saturating_add);
                let (same, same_units) = tally[index];
                let scan = entries
                    .saturating_add(same.saturating_mul(units.saturating_add(3)))
                    .saturating_add(same_units);
                if scan > self.ark.get() {
                    self.charge(scan - self.ark.get());
                    self.ark.set(scan);
                }
            }
            self.created();
            if flags.template {
                // dom_query also creates the template's contents fragment.
                self.created();
            }
            let id = self.document.create_element(name, attrs, flags);
            self.elements.borrow_mut().push(id);
            if let Some(units) = formatting_units {
                self.formatting_elements.borrow_mut().insert(id, units);
            }
            id
        }
        fn create_comment(&self, text: StrTendril) -> NodeId {
            self.created();
            self.document.create_comment(text)
        }
        fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
            self.created();
            self.document.create_pi(target, data)
        }
        fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
            self.appending(NodeRef::new(*parent, &self.document.tree).last_child(), &child);
            self.document.append(parent, child);
        }
        fn append_based_on_parent_node(
            &self,
            element: &NodeId,
            prev_element: &NodeId,
            child: NodeOrText<NodeId>,
        ) {
            // dom_query inserts before `element` when it has a parent.
            let element_node = NodeRef::new(*element, &self.document.tree);
            let adjacent = if element_node.parent().is_some() {
                element_node.prev_sibling()
            } else {
                NodeRef::new(*prev_element, &self.document.tree).last_child()
            };
            self.appending(adjacent, &child);
            self.document
                .append_based_on_parent_node(element, prev_element, child);
        }
        fn append_doctype_to_document(
            &self,
            name: StrTendril,
            public_id: StrTendril,
            system_id: StrTendril,
        ) {
            self.document
                .append_doctype_to_document(name, public_id, system_id);
        }
        fn get_template_contents(&self, target: &NodeId) -> NodeId {
            self.document.get_template_contents(target)
        }
        fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
            self.charge(1);
            self.document.same_node(x, y)
        }
        fn set_quirks_mode(&self, mode: QuirksMode) {
            self.document.set_quirks_mode(mode);
        }
        fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
            self.appending(NodeRef::new(*sibling, &self.document.tree).prev_sibling(), &new_node);
            self.document.append_before_sibling(sibling, new_node);
        }
        fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
            // A repeated <html> or <body> tag merges its attributes: dom_query
            // clones every existing name and searches them for each new one.
            let existing = NodeRef::new(*target, &self.document.tree)
                .query(|node| match &node.data {
                    NodeData::Element(element) => element.attrs.len(),
                    _ => 0,
                })
                .unwrap_or(0) as u64;
            self.charge(existing.saturating_mul(attrs.len() as u64 + 1));
            self.attributes.set(self.attributes.get().saturating_add(attrs.len() as u64));
            self.document.add_attrs_if_missing(target, attrs);
        }
        fn remove_from_parent(&self, target: &NodeId) {
            self.document.remove_from_parent(target);
        }
        fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
            self.document.reparent_children(node, new_parent);
        }
        fn is_mathml_annotation_xml_integration_point(&self, handle: &NodeId) -> bool {
            self.document
                .is_mathml_annotation_xml_integration_point(handle)
        }
    }

    #[cfg(test)]
    mod deferred_tests {
        use super::*;

        fn table_text(references: usize, closing: &str) -> String {
            format!("<b>{}<table>{}{closing}", "<span>".repeat(500), "&amp;".repeat(references))
        }

        #[test]
        fn deferred_table_text_cannot_bypass_the_work_budget() {
            for closing in ["", "</table>"] {
                let source = table_text(50_000, closing);
                assert!(source.len() < 256 * 1024);
                assert_eq!(
                    Html::parse_document_bounded(&source, &ARTICLE_BOUNDS, None).err(),
                    Some(Refused::TooComplex),
                    "closing token {closing:?}",
                );
            }
        }

        #[test]
        fn deferred_finalization_consumes_the_shared_export_budget() {
            let source = table_text(200, "");
            let mut spent = Spent::default();
            let first = Html::parse_document_sharing(&source, &ARTICLE_BOUNDS, None, &mut spent).unwrap();
            assert_eq!(spent.work, first.work);
            let bounds = Bounds { work: (first.work + first.work / 2, 0), ..ARTICLE_BOUNDS };
            assert_eq!(
                Html::parse_document_sharing(&source, &bounds, None, &mut spent).err(),
                Some(Refused::TooComplex),
            );
        }

        #[test]
        fn short_eof_work_is_charged_even_below_the_checkpoint_interval() {
            let source = "<b><span><table>&amp;";
            let mut spent = Spent::default();
            let first = Html::parse_document_sharing(source, &ARTICLE_BOUNDS, None, &mut spent).unwrap();
            assert!(first.work < CHUNK as u64);
            assert_eq!(spent.work, first.work);
            let bounds = Bounds { work: (first.work + first.work / 2, 0), ..ARTICLE_BOUNDS };
            assert_eq!(
                Html::parse_document_sharing(source, &bounds, None, &mut spent).err(),
                Some(Refused::TooComplex),
            );
        }

        #[test]
        fn cancellation_is_checked_inside_deferred_eof_processing() {
            let source = table_text(50_000, "");
            let cancelled = CancellationToken::new();
            let request = cancelled.clone();
            BEFORE_EOF.with(|hook| *hook.borrow_mut() = Some(Box::new(move || request.cancel())));
            let bounds = Bounds { work: (u64::MAX, 0), ..ARTICLE_BOUNDS };
            let mut spent = Spent::default();
            assert_eq!(
                Html::parse_document_sharing(&source, &bounds, Some(&cancelled), &mut spent).err(),
                Some(Refused::Cancelled),
            );
            // Full draining performs tens of millions of stack comparisons.
            assert!(spent.work < 2_000_000, "deferred drain completed before cancellation");
        }

        #[test]
        fn bounded_fragment_parse_matches_unbounded_snippet_text() {
            for source in [
                "<b>Water</b> &amp; birds&nbsp;<i>in</i> the <span class=hl>marsh</span>",
                "\u{feff}<b>a</b>\u{feff}b<table><td>cell</table>&notin;&not x",
                "<script>if (a</script>b)</script><noscript>&amp;n</noscript>",
                "<b><i>misnested</b> formatting</i><p>x</b>y",
            ] {
                let unbounded = Html::parse_fragment(source);
                let bounded = Html::parse_fragment_bounded(source, &SNIPPET_BOUNDS, None).unwrap();
                assert_eq!(bounded.serialized(), unbounded.serialized(), "{source}");
                assert_eq!(
                    text(&bounded.root_element()),
                    text(&unbounded.root_element()),
                    "{source}",
                );
            }
        }

        #[test]
        fn bounded_fragment_parse_refuses_hostile_and_cancelled_snippets() {
            let hostile = format!("<b>{}", "<div>".repeat(20_000));
            assert_eq!(
                Html::parse_fragment_bounded(&hostile, &SNIPPET_BOUNDS, None).err(),
                Some(Refused::TooComplex),
            );
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            assert_eq!(
                Html::parse_fragment_bounded("<b>x</b>", &SNIPPET_BOUNDS, Some(&cancelled)).err(),
                Some(Refused::Cancelled),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    #[test]
    fn export_dispatch_accepts_normalized_mime_types_without_false_html_matches() {
        for mime in [
            " TeXt/HtMl ; charset=UTF-8",
            " APPLICATION/XHTML+XML ; charset=utf-8",
        ] {
            checked_mime(mime).unwrap();
            assert!(is_html_mime(mime), "{mime}");
            assert!(is_markup(mime), "{mime}");
            assert!(!is_css(mime), "{mime}");
        }
        for mime in [
            "",
            "text/html-extra",
            "application/xhtml+xml-extra",
            "text/plain; type=text/html",
            "image/svg+xml",
        ] {
            assert!(!is_html_mime(mime), "{mime}");
        }
        assert!(is_markup(" IMAGE/SVG+XML ; charset=UTF-8"));
        assert!(is_css(" TeXt/CsS ; charset=UTF-8"));
        assert!(!is_css("text/css-extra"));
        assert!(!is_markup("image/svg+xml-extra"));
    }

    #[test]
    fn css_discovers_recursive_forms_and_ignores_comments_and_strings() {
        let refs = css_references_both(r#"/* url(no.png) */ @import "theme.css"; @import url('more.css'); a { content:"url(fake)"; src:u\72l(font\20name.woff2); background:URL(../i.png) }"#).unwrap();
        assert_eq!(
            refs,
            ["theme.css", "more.css", "font name.woff2", "../i.png"]
        );
    }
    #[test]
    fn css_references_follow_the_css_tokenizer() {
        // Escaped names are decoded before they are compared, as browsers do.
        for (css, expected) in [
            (r#"a{b:\75 rl(x.png)}"#, &["x.png"][..]),
            (r#"a{b:U\RL(x.png)}"#, &["x.png"]),
            (r#"a{b:\55 RL( "x.png" )}"#, &["x.png"]),
            (r#"a{b:url(\000078.png)}"#, &["x.png"]),
            (
                r#"@\69 mport "x.css"; @IM\PORT url(y.css);"#,
                &["x.css", "y.css"],
            ),
            (r#"a{b:image\-set("x.png" 1x)}"#, &["x.png"]),
            // src() is url() for a string.
            (
                r#"a{b:src("x.png")} @import src("y.css");"#,
                &["x.png", "y.css"],
            ),
            (
                r#"a{b:image-set(url(x.png) 1x, src("y.png") 2x, "z.png" type("image/png") 3x)}"#,
                &["x.png", "y.png", "z.png"],
            ),
            // Not URLs: an escaped newline is not an escape, a hash is not a
            // function, and a dimension ends at the name.
            ("a{b:u\\\nrl(x.png)}", &[]),
            (r#"a{b:#url(x.png) .5url(y.png)}"#, &[]),
            // Comments are not recognised inside an unquoted URL.
            (r#"a{b:url(/**/x.png)}"#, &["/**/x.png"]),
        ] {
            assert_eq!(css_references_both(css).unwrap(), expected, "{css}");
        }
        // Nesting depth costs no stack.
        let deep = format!("{}url(x.png){}", "(".repeat(200_000), ")".repeat(200_000));
        assert_eq!(css_references_both(&deep).unwrap(), ["x.png"]);
    }
    /// Both modes find the same references in well-formed CSS.
    fn css_references_both(css: &str) -> Result<Vec<String>, String> {
        let refused = css_references(css, Malformed::Refuse);
        assert_eq!(css_references(css, Malformed::Ignore), refused, "{css}");
        refused
    }
    #[test]
    fn malformed_css_is_skipped_like_the_browser_for_exports_but_refused_for_pdf() {
        // What the browser skips (bad-url tokens, a url() with more than one
        // string, a string broken by a newline) is no reference for an
        // archive export, and parsing resumes after it as in the browser. What
        // the end of the sheet cuts short, the browser closes and fetches.
        for (css, exported) in [
            ("a{b:url(a(b)); c:url(ok.png)}", &["ok.png"][..]),
            ("a{b:url(a\"b); c:url(ok.png)}", &["ok.png"]),
            ("a{b:url(/**/\"x.png\")}", &[]),
            ("a{b:url(x.png /**/)}", &[]),
            ("a{b:url(x y.png)}", &[]),
            ("a{b:url(\"x.png\" format)} c{d:url(ok.png)}", &["ok.png"]),
            ("a{b:\"x\npng; c:url(ok.png)}", &["ok.png"]),
            ("a{b:url(x.png", &["x.png"]),
            ("a{b:url(\"x.png\"", &["x.png"]),
            ("@import \"x.css", &["x.css"]),
            ("a{b:\"x.png", &[]),
        ] {
            assert_eq!(
                css_references(css, Malformed::Ignore).unwrap(),
                exported,
                "{css}"
            );
            // The PDF sanitizer refuses all of them.
            assert!(css_references(css, Malformed::Refuse).is_err(), "{css}");
        }
        // Through the export's markup path: a <style> and a style attribute.
        let parsed = markup_references(
            "<style>a{b:url(a(b))} c{d:url(one.png)}</style><p style='background:url(a\"b); color:red'>x</p><p style='background:url(two.png)'>y</p>",
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(parsed.values, ["one.png", "two.png"]);
    }
    #[test]
    fn css_image_sets_discover_only_candidate_strings_at_each_function_depth() {
        let refs = css_references_both(
            r#"
            /* image-set("comment.png" 1x) */
            a {
                content: "image-set('incidental.png' 1x)";
                background: image-set(/* first */ "one.png"1x, 'two.png'2x);
                mask-image: -WEBKIT-IMAGE-SET('mask.png' 1x, url(mask2.png) 2x);
                border-image: cross-fade(image-set("nested.png" type("image/png") 1x,
                    linear-gradient(red, blue) 2x, "last.png" 3x), url(other.png), 50%);
                list-style-image: image-set(url(first.png) type("image/avif") 1x,
                    image-set("deep.png" 1x, "deeper.png" 2x) 2x, "final.png" 3x);
                cursor: image-set("escaped\20name.png" 1x, "data:image/png;base64,AAA" 2x);
                font-family: "not-an-asset.png";
            }
        "#,
        )
        .unwrap();
        assert_eq!(
            refs,
            [
                "one.png",
                "two.png",
                "mask.png",
                "mask2.png",
                "nested.png",
                "last.png",
                "other.png",
                "first.png",
                "deep.png",
                "deeper.png",
                "final.png",
                "escaped name.png"
            ]
        );
    }
    #[test]
    fn html_discovers_assets_not_article_links_and_respects_base() {
        let base = Url::parse("http://book.localhost/A/article").unwrap();
        let parsed = markup_references(r#"<base href="../I/"><link rel="stylesheet" href="main.css"><a href="other">Other article</a><img src="a.png" srcset="a.png 1x, b.png 2x"><style>x{background:url(bg.png)}</style>"#, &mut Default::default()).unwrap();
        let base = base.join(parsed.base.as_deref().unwrap()).unwrap();
        let refs: Vec<String> = parsed
            .values
            .iter()
            .map(|value| base.join(value).unwrap().into())
            .collect();
        assert!(refs.contains(&"http://book.localhost/I/main.css".to_string()));
        assert!(refs.contains(&"http://book.localhost/I/b.png".to_string()));
        assert!(refs.contains(&"http://book.localhost/I/bg.png".to_string()));
        assert!(!refs.iter().any(|r| r.ends_with("other")));
    }
    #[test]
    fn mhtml_keeps_original_scripts_embedded_documents_and_media_dependencies() {
        let source = r#"<script src="app.js"></script><iframe src="frame.html"></iframe><video src="large.mp4"></video>"#;
        let references = markup_references(source, &mut Default::default()).unwrap().values;
        for required in ["app.js", "frame.html", "large.mp4"] {
            assert!(references.iter().any(|value| value == required), "{required}");
        }
    }
    #[test]
    fn svg_xlink_assets_and_presentation_urls_are_discovered() {
        let parsed = markup_references(r##"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../I/picture.png"/><use xlink:href="sprites.svg#icon"/><script xlink:href="plot.js"/><path fill="url(pattern.svg#p)"/></svg>"##, &mut Default::default()).unwrap();
        assert_eq!(
            parsed.values,
            [
                "../I/picture.png",
                "sprites.svg#icon",
                "plot.js",
                "pattern.svg#p"
            ]
        );
    }
    #[test]
    fn canonical_archive_urls_preserve_literal_percent_and_leading_slashes() {
        let origin = Url::parse("http://book.localhost/").unwrap();
        for path in [
            "A/École_100%.html",
            "/A/article",
            "I/a%2Fb.png",
            "A/chapter#1.html",
            "I/image?size=2#full.png",
            "A/literal%3F%23?#",
        ] {
            assert_eq!(resource_path(path).unwrap(), path);
            let url = entry_url(&origin, path).unwrap();
            assert_eq!(url.origin(), origin.origin());
            assert_eq!(
                percent_decode_str(url.path().strip_prefix('/').unwrap())
                    .decode_utf8()
                    .unwrap(),
                path
            );
        }
        assert!(entry_url(&origin, "A/../article").is_err());
        assert!(add_reference(
            &mut Vec::new(),
            &format!("data:image/png;base64,{}", "A".repeat(MAX_URL + 1))
        )
        .is_ok());
    }
    #[test]
    fn canonical_resource_paths_reject_invalid_identity_without_location_fallback() {
        assert!(resource_path("").is_err());
        assert!(resource_path("I/image\0.png").is_err());
        assert!(resource_path(&"x".repeat(MAX_URL + 1)).is_err());
        assert_eq!(resource_path("A/chapter.html?").unwrap(), "A/chapter.html?");
        assert_eq!(resource_path("A/chapter.html#").unwrap(), "A/chapter.html#");
    }
    #[test]
    fn inline_and_external_resources_never_become_archive_reads() {
        let origin = Url::parse("http://book.localhost/").unwrap();
        for value in [
            "data:image/png;base64,AAAA",
            "https://remote/image",
            "file:///etc/passwd",
            "//remote/image",
            "#symbol",
        ] {
            assert!(local_reference(&origin, value, &origin).unwrap().is_none());
        }
        let url = local_reference(&origin, "I/a%2520b.png?v=2#x", &origin)
            .unwrap()
            .unwrap();
        assert_eq!(url.as_str(), "http://book.localhost/I/a%2520b.png?v=2");
        let url = local_reference(&origin, "I/a%3Fb%23c%2520.png?variant=2#symbol", &origin)
            .unwrap()
            .unwrap();
        assert_eq!(url.path(), "/I/a%3Fb%23c%2520.png");
        assert_eq!(url.query(), Some("variant=2"));
        assert_eq!(url.fragment(), None);
        assert_eq!(
            percent_decode_str(url.path().strip_prefix('/').unwrap())
                .decode_utf8()
                .unwrap(),
            "I/a?b#c%20.png"
        );
    }
    #[test]
    fn base64_preserves_stream_bytes_and_wraps_mime_lines() {
        let bytes: Vec<u8> = (0..8193).map(|n| (n % 251) as u8).collect();
        let mut output = Vec::new();
        let mut lines = MimeLines {
            output: &mut output,
            column: 0,
        };
        {
            let mut encoder = EncoderWriter::new(&mut lines, &STANDARD);
            for chunk in bytes.chunks(127) {
                encoder.write_all(chunk).unwrap();
            }
            encoder.finish().unwrap();
        }
        lines.finish().unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.split("\r\n").all(|line| line.len() <= 76));
        assert_eq!(STANDARD.decode(text.replace("\r\n", "")).unwrap(), bytes);
    }

    #[cfg(unix)]
    struct ExportDirectory(PathBuf);
    #[cfg(unix)]
    impl ExportDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kiwix-export-durability-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    #[cfg(unix)]
    impl Drop for ExportDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn assert_completed_but_unconfirmed(result: Result<(), String>) {
        result.expect_err("directory synchronization rejection must not report success");
    }

    #[cfg(unix)]
    #[test]
    fn export_directory_sync_failure_reports_completed_new_file() {
        let directory = ExportDirectory::new();
        let destination = directory.0.join("resource.bin");
        FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
        let result = atomic_output(&destination, false, |output| {
            output.write_all(b"complete resource").map_err(display)
        });
        assert_completed_but_unconfirmed(result);
        assert_eq!(fs::read(&destination).unwrap(), b"complete resource");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn export_directory_sync_failure_preserves_explicit_overwrite() {
        let directory = ExportDirectory::new();
        let destination = directory.0.join("article.mhtml");
        fs::write(&destination, b"original").unwrap();
        FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
        let result = atomic_output(&destination, true, |output| {
            output.write_all(b"completed replacement").map_err(display)
        });
        assert_completed_but_unconfirmed(result);
        assert_eq!(fs::read(&destination).unwrap(), b"completed replacement");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn export_completed_warning_survives_user_error_presentation_for_any_filename() {
        let directory = ExportDirectory::new();
        for filename in ["snapshot.mhtml", "database.bin", "record.bin"] {
            let destination = directory.0.join(filename);
            FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
            let error = atomic_output(&destination, false, |output| {
                output.write_all(b"complete resource").map_err(display)
            }).expect_err("directory synchronization rejection must not report success");
            assert_eq!(crate::user_error::plain(&error), error,
                "the presentation boundary must preserve the completed-export warning for {filename}");
            assert_eq!(fs::read(&destination).unwrap(), b"complete resource");
        }
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn export_publication_relinquishes_temporary_name_before_directory_sync() {
        let directory = ExportDirectory::new();
        let destination = directory.0.join("resource.bin");
        let prepared = prepare_output(&destination, |output| {
            output.write_all(b"complete resource").map_err(display)
        }).unwrap();
        let former_temporary = prepared.temporary.0.clone();
        RECREATE_TEMPORARY_ON_DIRECTORY_SYNC.with(|path| *path.borrow_mut() = Some(former_temporary.clone()));
        FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
        let result = prepared.publish(false);
        assert_eq!(fs::read(&destination).unwrap(), b"complete resource");
        assert_eq!(fs::read(&former_temporary).unwrap(), b"unrelated replacement",
            "publication must relinquish the former temporary name before post-commit work");
        assert_completed_but_unconfirmed(result);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn export_relative_filename_syncs_current_directory_without_global_cwd_changes() {
        const CHILD: &str = "KIWIX_EXPORT_RELATIVE_FILENAME_TEST";
        if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
            let directory = ExportDirectory::new();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "exports::tests::export_relative_filename_syncs_current_directory_without_global_cwd_changes", "--test-threads=1"])
                .env(CHILD, "1")
                .current_dir(&directory.0)
                .output()
                .expect("launch isolated relative-filename export");
            assert!(output.status.success(), "isolated relative-filename export failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert_eq!(fs::read(directory.0.join("resource.bin")).unwrap(), b"replacement");
            assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
            return;
        }
        let destination = Path::new("resource.bin");
        atomic_output(destination, false, |output| {
            output.write_all(b"complete resource").map_err(display)
        }).unwrap();
        FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
        let result = atomic_output(destination, true, |output| {
            output.write_all(b"replacement").map_err(display)
        });
        assert_completed_but_unconfirmed(result);
        assert_eq!(fs::read(destination).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(".").unwrap().count(), 1);
    }
    #[test]
    fn export_failure_and_no_overwrite_preserve_user_file() {
        let directory = std::env::temp_dir().join(format!("kiwix-export-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("article.mhtml");
        fs::write(&destination, b"original").unwrap();
        assert!(atomic_output(&destination, true, |output| {
            output.write_all(b"partial").unwrap();
            Err("failure".into())
        })
        .is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert!(atomic_output(&destination, false, |output| output
            .write_all(b"new")
            .map_err(display))
        .is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        atomic_output(&destination, true, |output| {
            output.write_all(b"new").map_err(display)
        })
        .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mhtml_leaves_assets_missing_from_the_archive_unresolved() {
        // fallback.html references missing.png, which the archive never held,
        // beside three images that exist (DevDocs ships styles like this).
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview.zim");
        let archive = Archive::open(&fixture).unwrap();
        let directory = std::env::temp_dir().join(format!("kiwix-export-missing-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("fallback.mhtml");
        save_mhtml(&archive, "fallback.html", &destination, false).unwrap();
        let package = fs::read_to_string(&destination).unwrap();
        let locations: Vec<&str> = package
            .lines()
            .filter_map(|line| line.strip_prefix("Content-Location: "))
            .collect();
        assert!(locations[0].ends_with("/fallback%2Ehtml"), "{locations:?}");
        assert_eq!(locations.len(), 4, "{locations:?}");
        for present in ["/I/vector.svg", "/I/large.png", "/I/photo%20%2523.png"] {
            assert!(locations.iter().any(|location| location.ends_with(present)), "{present} in {locations:?}");
        }
        assert!(!locations.iter().any(|location| location.contains("missing.png")), "{locations:?}");
        // The article itself must exist: a missing root still fails.
        let error = save_mhtml(&archive, "absent.html", &directory.join("absent.mhtml"), false).unwrap_err();
        assert!(crate::archive::is_entry_not_found(&error), "{error}");
        assert!(!directory.join("absent.mhtml").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    fn element_texts(document: &html::Html, name: &str) -> Vec<String> {
        document
            .elements()
            .filter(|element| element.has_name(name))
            .map(|element| html::text(&element))
            .collect()
    }

    #[test]
    fn html_elements_keep_parser_creation_order_and_template_contents() {
        // Foster parenting puts B before the table in the tree, but callers
        // (first <p> preview, first 8 thumbnails, reference order) have always
        // seen elements in the order the parser created them.
        let document =
            html::Html::parse_document("<table><tr><td><p>A</p></td></tr><p>B</p></table>");
        assert_eq!(html::text(&document.root_element()), "BA");
        assert_eq!(element_texts(&document, "p"), ["A", "B"]);
        // Template contents are selectable and part of an ancestor's text.
        let document = html::Html::parse_document(
            "<template><p>T</p><base href='t/'></template><p>Real</p><base href='real/'>",
        );
        assert_eq!(element_texts(&document, "p"), ["T", "Real"]);
        assert_eq!(html::text(&document.root_element()), "TReal");
        let parsed =
            markup_references("<template><base href='t/'></template><base href='real/'>", &mut Default::default()).unwrap();
        assert_eq!(parsed.base.as_deref(), Some("t/"));
    }

    #[test]
    fn html_parsing_uses_scripting_enabled_defaults_and_scraper_attribute_rules() {
        // With scripting enabled, <noscript> in <head> is raw text, so its
        // markup neither becomes elements nor ends the head early.
        let document = html::Html::parse_document(
            "<head><noscript><p>hidden</p></noscript></head><body><p>shown</p>",
        );
        assert_eq!(element_texts(&document, "p"), ["shown"]);
        assert_eq!(html::text(&document.root_element()), "<p>hidden</p>shown");
        let fragment = html::Html::parse_fragment(
            "<p>Fish &amp; chips&nbsp;<b>hot</b>\n<noscript>&amp;n</noscript></p><table><td>cell</td></table>&notin;&not x",
        );
        assert_eq!(
            html::text(&fragment.root_element()),
            "Fish & chips\u{a0}hot\n&amp;ncell\u{2209}\u{ac} x"
        );
        // `href` never reads `xlink:href`, and attributes come out ordered by
        // qualified name, as the PDF sanitizer has always written them.
        let document = html::Html::parse_document(
            r##"<p title="t" class="c" data-b="2" data-a="1">x</p><svg><use xlink:href="#a" fill="red"/></svg>"##,
        );
        let mut elements = document.elements();
        let paragraph = elements.find(|element| element.has_name("p")).unwrap();
        let paragraph = html::element(&paragraph);
        assert_eq!(
            html::attrs(&paragraph),
            [
                ("class", "c"),
                ("data-a", "1"),
                ("data-b", "2"),
                ("title", "t")
            ]
        );
        let icon = elements.find(|element| element.has_name("use")).unwrap();
        let icon = html::element(&icon);
        assert_eq!(html::attr(&icon, "href"), None);
        assert_eq!(html::attr(&icon, "fill"), Some("red"));
        assert_eq!(html::attrs(&icon), [("fill", "red"), ("href", "#a")]);
    }

    #[test]
    fn html_text_is_complete_after_misnested_formatting_elements() {
        // scraper 0.24 (ego-tree 0.10) left stale parent links after the
        // adoption agency moved children, and its text() stopped at "x".
        let document = html::Html::parse_document("<font><article><area>x<dd></font><p>y");
        assert_eq!(html::text(&document.root_element()), "xy");
    }

    #[test]
    fn html5ever_0_39_follows_the_current_parsing_standard() {
        let parent = |document: &html::Html, name: &str| {
            let element = document.elements().find(|e| e.has_name(name)).unwrap();
            let parent = element.parent().unwrap();
            html::name(&html::element(&parent)).to_owned()
        };
        // Customizable <select>: content is kept instead of dropped.
        let document = html::Html::parse_document(
            "<select><div>x</div><img src=a><option>o</option></select>",
        );
        assert_eq!(parent(&document, "div"), "select");
        assert_eq!(parent(&document, "img"), "select");
        // Active formatting elements are reconstructed before <svg>/<math>.
        let document = html::Html::parse_document("<p><font>x</p><svg></svg>");
        assert_eq!(parent(&document, "svg"), "font");
        // annotation-xml with an HTML encoding is an HTML integration point
        // (scraper's tree sink never reported it).
        let document = html::Html::parse_document(
            "<math><annotation-xml encoding='text/html'><p>x</p></annotation-xml></math>",
        );
        assert_eq!(parent(&document, "p"), "annotation-xml");
    }

    fn repeat(open: &str, count: usize, tail: &str) -> String {
        let mut source = String::with_capacity(open.len() * count + tail.len());
        for index in 0..count {
            source.push_str(&open.replace("{}", &index.to_string()));
        }
        source.push_str(tail);
        source
    }

    /// Hostile shapes: deep nesting through each tree-builder path, and
    /// documents that stay under the nesting ceiling but make every token
    /// scan a long stack or formatting list.
    fn hostile_documents() -> Vec<(&'static str, String)> {
        let deep_spans = repeat("<span>", 1000, "");
        let many = (0..1000).map(|index| format!(" a{index}")).collect::<String>();
        vec![
            ("40k nested div", repeat("<div>", 40_000, "x")),
            ("200k nested div", repeat("<div>", 200_000, "x")),
            ("40k distinct formatting", repeat("<b id={}>", 40_000, "x")),
            ("40k nested template", repeat("<template>", 40_000, "x")),
            ("40k nested table cells", repeat("<table><tr><td>", 40_000, "x")),
            ("40k nested svg", format!("<svg>{}", repeat("<g>", 40_000, ""))),
            ("stray end tags at depth 1000", format!("{deep_spans}{}", repeat("</x>", 60_000, ""))),
            ("block starts at depth 1000", format!("{deep_spans}{}", repeat("<p></p>", 40_000, ""))),
            ("noah's ark at depth 500", format!("{}{}", repeat("<b id={}>", 500, ""), repeat("<b></b>", 40_000, ""))),
            ("stale formatting per paragraph", format!("{}{}", repeat("<p><b id={}></p>", 500, ""), repeat("<p>x</p>", 40_000, ""))),
            ("many-attribute formatting at depth 500", format!("{}{}", repeat("<b x={}>", 500, ""), repeat(&format!("<b{many}></b>"), 100, ""))),
            ("formatting against many-attribute entries", format!("{}{}", repeat(&format!("<b id={{}}{many}>"), 500, ""), repeat("<b></b>", 40_000, ""))),
            ("formatting against long attribute names", {
                let prefix = "n".repeat(1000);
                let long = (0..100).map(|index| format!(" {prefix}{index}")).collect::<String>();
                format!("<b{long}>{}", repeat("<b></b>", 40_000, ""))
            }),
            ("repeated body attributes", repeat("<body a{}>", 40_000, "")),
            ("repeated html attributes", repeat("<html a{}>", 40_000, "")),
            // Denser than one node per 4 bytes, the node allowance.
            ("dense comments", "<!>".repeat(1_500_000)),
            ("dense processing instructions", "<?>".repeat(1_500_000)),
            ("alternating text and comments", "x<!>".repeat(1_000_000)),
            // Every paragraph re-creates 140 stale formatting elements with
            // 300 attributes each: gigabytes of copies if only scans count.
            ("re-created attributes", {
                let attributes = repeat(" a{}", 300, "");
                let mut source = String::new();
                for name in ["a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u"] {
                    source.push_str(&repeat(&format!("<p><{name} id={{}}{attributes}></p>"), 10, ""));
                }
                source + &"<p>x</p>".repeat(30_000)
            }),
            // 420 open formatting elements with 1,000 attributes each, then text.
            ("open formatting with many attributes", {
                let attributes = repeat(" a{}", 1000, "");
                let mut source = String::new();
                for name in ["a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u"] {
                    source.push_str(&repeat(&format!("<{name} id={{}}{attributes}>"), 30, ""));
                }
                source + &"x".repeat(2_000_000)
            }),
        ]
    }

    #[test]
    fn bounded_parse_refuses_hostile_nesting_within_milliseconds() {
        for (name, source) in hostile_documents() {
            let started = std::time::Instant::now();
            let refused = html::Html::parse_document_bounded(&source, &html::ARTICLE_BOUNDS, None);
            let elapsed = started.elapsed();
            assert_eq!(refused.err(), Some(html::Refused::TooComplex), "{name}");
            // Unbounded, these take from seconds to hours. Deep nesting is
            // refused at the open-element ceiling within milliseconds; the
            // rest reach the element, work or CPU-time ceiling.
            // Debug builds are much slower, and other tests share the CPU.
            let limit = if name.contains("nested") || name.contains("distinct") { 50 } else { 3000 };
            let limit = if cfg!(debug_assertions) { limit * 40 } else { limit };
            assert!(elapsed < std::time::Duration::from_millis(limit), "{name} took {elapsed:?}");
        }
    }

    fn article(paragraphs: usize) -> String {
        super::test_article(paragraphs)
    }

    #[test]
    fn bounded_parse_accepts_large_articles_with_an_identical_tree() {
        // ~16 MiB, the live snapshot ceiling.
        let source = article(56_000);
        assert!(source.len() > 15 * 1024 * 1024, "{}", source.len());
        html::Html::parse_document_bounded(&source, &html::ARTICLE_BOUNDS, None).unwrap();
        let source = article(2_000);
        let bounded = html::Html::parse_document_bounded(&source, &html::ARTICLE_BOUNDS, None).unwrap();
        assert_eq!(bounded.serialized(), html::Html::parse_document(&source).serialized());
        // The preview reads a 256 KiB prefix, cut anywhere.
        let mut end = 256 * 1024;
        while !source.is_char_boundary(end) { end -= 1; }
        html::Html::parse_document_bounded(&source[..end], &html::ARTICLE_BOUNDS, None).unwrap();
    }

    #[test]
    fn bounded_parse_refuses_what_only_cpu_time_observes() {
        // The tokenizer's duplicate-attribute check is quadratic within one
        // tag and invisible to the tree builder: 80k attributes take seconds.
        let source = repeat(" a{}", 80_000, "").replacen(' ', "<p ", 1);
        let bounds = html::Bounds { cpu: (std::time::Duration::from_millis(50), 100), ..html::ARTICLE_BOUNDS };
        let started = std::time::Instant::now();
        let refused = html::Html::parse_document_bounded(&source, &bounds, None);
        assert_eq!(refused.err(), Some(html::Refused::TooComplex));
        // Debug builds get 25 times the CPU allowance; other tests share the CPU.
        let limit = std::time::Duration::from_secs(if cfg!(debug_assertions) { 60 } else { 5 });
        assert!(started.elapsed() < limit, "{:?}", started.elapsed());
        // The same allowance parses an ordinary document of that length.
        let ordinary = article(2_000);
        assert!(ordinary.len() > source.len());
        html::Html::parse_document_bounded(&ordinary, &bounds, None).unwrap();
        // A cheap tail does not lend its allowance to expensive markup before it.
        let mut front_loaded = repeat(" a{}", 60_000, ">").replacen(' ', "<p ", 1);
        front_loaded.push_str(&format!("<!--{}-->", "c".repeat(16_000_000)));
        let refused = html::Html::parse_document_bounded(&front_loaded, &bounds, None);
        assert_eq!(refused.err(), Some(html::Refused::TooComplex));
    }

    #[test]
    fn parses_sharing_a_budget_share_the_fixed_allowance() {
        // Each part alone fits the fixed work allowance; one export's parts
        // together do not.
        let part = format!("{}{}", "<span>".repeat(1000), "</x>".repeat(300));
        assert!(html::Html::parse_document_bounded(&part, &html::ARTICLE_BOUNDS, None).is_ok());
        let mut spent = html::Spent::default();
        let refused = (0..20).find_map(|_| {
            html::Html::parse_document_sharing(&part, &html::ARTICLE_BOUNDS, None, &mut spent).err()
        });
        assert_eq!(refused, Some(html::Refused::TooComplex));
        let mut spent = html::Spent::default();
        assert!((0..20).any(|_| markup_references(&part, &mut spent).is_err()));
    }

    /// Cheap legitimate shapes that earlier ceilings refused: unclosed legacy
    /// `<font>` per paragraph, dense link indexes and large data tables.
    #[test]
    fn bounded_parse_accepts_dense_legacy_and_link_markup() {
        let styles = ["<font face=Arial size=2 color=#000080>", "<font face=Verdana size=1 color=#333333>", "<font face=Times size=3>"];
        let legacy = (0..12_000).map(|index| format!("<p>{}Paragraph {index} text", styles[index % 3])).collect::<String>();
        let index = (0..60_000).map(|index| format!("<a href={index}.html>{index}</a> ")).collect::<String>();
        let titled = (0..30_000).map(|index| format!("<li><a href=\"/wiki/T{index}\" title=\"T{index}\">T{index}</a></li>")).collect::<String>();
        let table = format!("<table>{}</table>", (0..250_000).map(|row| format!("<tr>\n<td>{}</td>\n<td>{row}</td>\n</tr>\n", row % 97)).collect::<String>());
        assert!(table.len() > 7_000_000);
        for (name, source) in [("legacy font", legacy), ("link index", index), ("titled list", titled), ("pretty-printed table", table)] {
            let (_, (_, _, work)) = html::Html::parse_chunked(&source, &html::ARTICLE_BOUNDS, None, html::CHUNK, &mut Default::default())
                .unwrap_or_else(|refused| panic!("{name}: {refused:?}"));
            // Inside the ceiling of 12 units per byte; legacy font, which
            // html5ever really does re-create in every paragraph, is costliest.
            assert!(work < 10 * source.len() as u64, "{name}: {work} units for {} bytes", source.len());
        }
    }

    #[test]
    fn bounded_parse_keeps_byte_order_marks_after_the_first() {
        let source = "\u{feff}<p title='a\u{feff}b'>\u{feff}x</p><script>s</script>\u{feff}y";
        let document = html::Html::parse_document_bounded(source, &html::ARTICLE_BOUNDS, None).unwrap();
        assert_eq!(html::text(&document.root_element()), "\u{feff}xs\u{feff}y");
        let paragraph = document.elements().find(|element| element.has_name("p")).unwrap();
        assert_eq!(html::attr(&html::element(&paragraph), "title"), Some("a\u{feff}b"));
    }

    #[test]
    fn bounded_parse_is_identical_across_chunk_boundaries() {
        let samples = [
            "\u{feff}<a href=\"a\u{feff}b\">\u{feff}c</a><script>x</script>\u{feff}d",
            "<!doctype html><p title='a&amp;b'>caf\u{e9} \u{1F600} &notin; &notit; &#x1F600;\r\n\rx</p>",
            "<table><b>foster<tr><td>cell</table><p>after",
            "<b><i>misnested</b> formatting</i><p>x</b>y",
            "<template><td>t</td></template><svg><foreignObject><p>f</p></foreignObject><style><b></style></svg>",
            "<script>if (a</script>b) {}</script><style>p{}</style><textarea>&lt;</textarea><!-- c -- -> --><title>t</title>",
            "<math><annotation-xml encoding=text/html><div>m</div></annotation-xml></math><plaintext><b>raw",
        ];
        for sample in samples {
            let whole = html::Html::parse_document(sample);
            for chunk in 1..=9 {
                let (chunked, _) = html::Html::parse_chunked(sample, &html::ARTICLE_BOUNDS, None, chunk, &mut Default::default()).unwrap();
                assert_eq!(chunked.serialized(), whole.serialized(), "{sample} in {chunk}-byte chunks");
                let names = |document: &html::Html| document.elements()
                    .map(|element| html::name(&html::element(&element)).to_owned()).collect::<Vec<_>>();
                assert_eq!(names(&chunked), names(&whole), "{sample} in {chunk}-byte chunks");
            }
        }
    }

    #[test]
    fn bounded_parse_stops_when_cancelled() {
        let cancelled = tokio_util::sync::CancellationToken::new();
        cancelled.cancel();
        assert_eq!(
            html::Html::parse_document_bounded("<p>x</p>", &html::ARTICLE_BOUNDS, Some(&cancelled)).err(),
            Some(html::Refused::Cancelled),
        );
        // Cancellation reaches a parse already in progress.
        let source = article(28_000);
        let cancelled = tokio_util::sync::CancellationToken::new();
        let parse = {
            let cancelled = cancelled.clone();
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                let result = html::Html::parse_document_bounded(&source, &html::ARTICLE_BOUNDS, Some(&cancelled)).err();
                (result, started.elapsed())
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        let requested = std::time::Instant::now();
        cancelled.cancel();
        let (result, _) = parse.join().unwrap();
        assert_eq!(result, Some(html::Refused::Cancelled));
        assert!(requested.elapsed() < std::time::Duration::from_millis(100), "{:?}", requested.elapsed());
    }

    #[test]
    fn mhtml_discovery_refuses_hostile_nesting() {
        assert!(markup_references(&repeat("<div>", 40_000, "<img src=a.png>"), &mut Default::default()).is_err());
    }

    /// `KIWIX_HTML_CORPUS=dir cargo test --lib html_corpus -- --ignored --nocapture`
    /// reports how real pages use the [`html::ARTICLE_BOUNDS`] allowances.
    #[test]
    #[ignore]
    fn html_corpus() {
        let directory = std::env::var("KIWIX_HTML_CORPUS").unwrap();
        let mut paths = fs::read_dir(directory).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            let source = fs::read_to_string(&path).unwrap();
            let started = std::time::Instant::now();
            let (bounded, (open, elements, work)) = html::Html::parse_chunked(&source, &html::ARTICLE_BOUNDS, None, html::CHUNK, &mut Default::default()).unwrap();
            let elapsed = started.elapsed();
            assert_eq!(bounded.serialized(), html::Html::parse_document(&source).serialized());
            println!("{}: {} bytes, {elapsed:?}, peak open {open}, {elements} elements ({:.1} bytes each), work {work} ({:.2}/byte)",
                path.display(), source.len(), source.len() as f64 / elements as f64, work as f64 / source.len() as f64);
        }
    }
}
