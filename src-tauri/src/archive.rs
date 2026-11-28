// SPDX-License-Identifier: GPL-3.0-or-later
//! Positioned ZIM reads from retained, caller-authorized files.
//!
//! Archive paths retain namespaces, percent signs,
//! and Unicode exactly. The caller handles URL decoding once at its HTTP edge.
//! Reads are capped at 16 MiB per call; stream larger media in successive ranges.
//! Search pages are capped at 1000 hits, queries at 64 KiB, metadata strings at
//! 1 MiB, search responses at 32 MiB, and redirect chains at 256 hops. These bounds
//! reject oversized/malformed input explicitly rather than silently truncating it.
//! Full-text totals count matches. Suggestion totals can include collapsed hits. Title fallback is case/diacritic-sensitive
//! prefix search when the archive has no suggestion index.

#![forbid(unsafe_code)]

use crate::archive_search::{Parts, SearchReader};

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zimru::archive::PositionedInput;
use zimru::{Archive as Zim, ArchiveLimits, ClusterReadLimits, Entry, Error as ZimError, Item};

/// Acquire the archive or search-cache lock, allowing synchronous workers to
/// withdraw while queued. The uncontended path takes the lock immediately.
pub(super) fn archive_lock<'a, T>(
    mutex: &'a std::sync::Mutex<T>,
    cancel: Option<(&CancellationToken, &'static str)>,
) -> Result<std::sync::MutexGuard<'a, T>, String> {
    let Some((token, reason)) = cancel else {
        return mutex.lock().map_err(|_| "Archive reader unavailable. Reopen the archive.".to_string());
    };
    interruptible_lock(mutex, reason, || token.is_cancelled())
}

fn interruptible_lock<'a, T>(
    mutex: &'a std::sync::Mutex<T>, reason: &str, stopped: impl Fn() -> bool,
) -> Result<std::sync::MutexGuard<'a, T>, String> {
    let unavailable = || "Archive reader unavailable. Reopen the archive.".to_string();
    loop {
        if stopped() {
            return Err(reason.into());
        }
        match mutex.try_lock() {
            Ok(_) if stopped() => return Err(reason.into()),
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(unavailable()),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(2))
            }
        }
    }
}

pub const MAX_READ_LENGTH: u64 = 16 * 1024 * 1024;

/// Recognize HTML media types independently of case, whitespace, and parameters.
pub(crate) fn is_html_mime(mime: &str) -> bool {
    let media_type = mime.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("text/html")
        || media_type.eq_ignore_ascii_case("application/xhtml+xml")
}

/// Category used for an absent archive entry.
///
/// Callers that resolve a user-chosen navigation target must treat this category
/// as a recoverable miss (the archive is readable, the entry is not there), not as
/// an archive failure.
pub const ENTRY_NOT_FOUND: &str = "Entry not found";

/// Whether an archive error is the `EntryNotFound` category.
pub fn is_entry_not_found(error: &str) -> bool {
    error
        .strip_prefix(ENTRY_NOT_FOUND)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
}

/// Category used for structurally invalid input.
pub(crate) const MALFORMED_ARCHIVE: &str = "Malformed archive";

/// Actionable replacements for raw filesystem/libzim text on open failure.
pub(crate) const MISSING_ARCHIVE: &str = "The archive file was moved or deleted. Choose it again.";
pub(crate) const ARCHIVE_ACCESS_DENIED: &str =
    "The operating system denied access to this archive. Check the file permissions and try again.";
pub(crate) const EMPTY_ARCHIVE: &str = "This file is empty, so it is not a ZIM archive.";
pub(crate) const INVALID_ARCHIVE: &str =
    "This is not a valid ZIM archive, or the file is truncated.";
pub(crate) const CORRUPT_ARCHIVE: &str = "Kiwix could not verify this archive, so it was not added. Download it again or choose another copy";
/// Fallback for native failures that match no known cause; the raw text is
/// logged rather than shown because it names libzim internals.
pub(crate) const ARCHIVE_OPEN_FAILED: &str = "The archive could not be opened.";

/// Turn a raw archive-open failure into a message that names the likely cause.
///
/// The path is inspected after the native failure so a vanished, denied,
/// directory, or empty target is reported instead of libzim's low-level text.
/// A regular file is opened for a readability probe, so a zero-length file the
/// process cannot read reports access denial rather than emptiness. The path
/// must be the one the caller tried to open; retained-descriptor opens classify
/// with the size observed on the descriptor instead. Anything unrecognized
/// becomes [`ARCHIVE_OPEN_FAILED`]; the raw detail goes to stderr only.
pub(crate) fn archive_open_error(path: &Path, error: &str) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => INVALID_ARCHIVE.into(),
        Ok(metadata) => match open_archive_file(path) {
            Ok(_) => classify_archive_failure(Some(metadata.len()), error),
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                MISSING_ARCHIVE.into()
            }
            Err(failure) if failure.kind() == std::io::ErrorKind::PermissionDenied => {
                ARCHIVE_ACCESS_DENIED.into()
            }
            Err(_) => classify_archive_failure(Some(metadata.len()), error),
        },
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => MISSING_ARCHIVE.into(),
        Err(failure) if failure.kind() == std::io::ErrorKind::PermissionDenied => {
            ARCHIVE_ACCESS_DENIED.into()
        }
        Err(_) => classify_archive_failure(None, error),
    }
}

// Opening a discovered split sibling must return even if another process has
// replaced its name with a pipe. Validate the type on the open handle.
fn open_archive_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options.open(path)
}

// Keep all parent directories and part names protected against deletion or
// replacement for the life of the reader. I/O still uses the authenticated
// retained file, including after another process changes its working directory.
#[cfg(windows)]
fn windows_path_leases(
    directory: &Path,
    files: &[(String, std::fs::File)],
) -> Result<Vec<std::fs::File>, String> {
    use cap_fs_ext::MetadataExt as _;
    use cap_std::fs::Metadata;
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::path::{Component, PathBuf, Prefix};

    fn lease(path: &Path, directory: bool) -> Result<File, String> {
        const FILE_READ_ATTRIBUTES: u32 = 0x80;
        const FILE_SHARE_READ_WRITE: u32 = 3;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        let mut options = OpenOptions::new();
        options.read(true).share_mode(FILE_SHARE_READ_WRITE);
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if directory {
            options.access_mode(FILE_READ_ATTRIBUTES);
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        let file = options
            .custom_flags(flags)
            .open(path)
            .map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (directory && !meta.is_dir())
            || (!directory && !meta.is_file())
        {
            return Err("Archive path lease is not the expected regular entry".into());
        }
        Ok(file)
    }

    let path = directory
        .to_str()
        .ok_or("Archive directory must be valid Unicode")?;
    let mut components = directory.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err("Retained archive path must be absolute".into());
    };
    let valid_prefix = match prefix.kind() {
        Prefix::Disk(_) | Prefix::VerbatimDisk(_) => true,
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            !server.is_empty() && !share.is_empty()
        }
        Prefix::Verbatim(volume) => volume
            .to_str()
            .is_some_and(|v| v.starts_with("Volume{") && v.ends_with('}')),
        Prefix::DeviceNS(_) => false,
    };
    if !valid_prefix || !directory.is_absolute() || path.contains('/') {
        return Err("Unsupported retained archive filesystem root".into());
    }
    // Components folds "." and repeated separators. Check the spelling before
    // constructing prefixes so leases cannot refer to a different path.
    let prefix_text = prefix
        .as_os_str()
        .to_str()
        .ok_or("Invalid archive filesystem root")?;
    let tail = path[prefix_text.len()..]
        .strip_prefix('\\')
        .unwrap_or(&path[prefix_text.len()..]);
    let tail = tail.strip_suffix('\\').unwrap_or(tail);
    let mut current = PathBuf::from(prefix.as_os_str());
    current.push(r"\");
    let mut leases = vec![lease(&current, true)?];
    if !tail.is_empty() {
        for dir in tail.split('\\') {
            if dir.is_empty() || dir == "." || dir == ".." || dir.contains(':') {
                return Err("Retained archive directory is not normalized".into());
            }
            current.push(dir);
            leases.push(lease(&current, true)?);
        }
    }
    for (name, expected) in files {
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
            return Err("Invalid retained archive filename".into());
        }
        let guard = lease(&current.join(name), false)?;
        let actual = Metadata::from_file(&guard).map_err(|e| e.to_string())?;
        let expected = Metadata::from_file(expected).map_err(|e| e.to_string())?;
        if actual.nlink() != 1
            || actual.dev() != expected.dev()
            || actual.ino() != expected.ino()
            || actual.len() != expected.len()
        {
            return Err("Retained archive part identity changed".into());
        }
        leases.push(guard);
    }
    Ok(leases)
}

fn classify_archive_failure(size: Option<u64>, error: &str) -> String {
    if size == Some(0) {
        return EMPTY_ARCHIVE.into();
    }
    if reader_limitation(error) {
        return error.to_owned();
    }
    if error.contains(MALFORMED_ARCHIVE) || error.contains("ZIM file format") {
        return INVALID_ARCHIVE.into();
    }
    eprintln!("archive open failed: {error}");
    ARCHIVE_OPEN_FAILED.into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMetadata {
    pub id: String,
    pub title: String,
    pub description: String,
    pub language: String,
    pub creator: String,
    pub publisher: String,
    pub date: String,
    pub name: String,
    pub tags: Vec<String>,
    pub size: u64,
    pub article_count: u64,
    pub media_count: u64,
    #[serde(default)]
    pub has_fulltext_index: bool,
    #[serde(default)]
    pub has_title_index: bool,
    pub main_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryInfo {
    pub path: String,
    pub title: String,
    pub mime: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults<T = SearchHit> {
    pub hits: Vec<T>,
    pub total: u64,
}

#[derive(Debug)]
pub(crate) struct RankedHit {
    pub(crate) hit: SearchHit,
    // Exact resolved URL index, not an ambiguous public path. It belongs
    // only to the retained archive that produced this hit.
    pub(crate) excerpt_entry: Option<u32>,
}

/// A bounded window of public libzim article titles, without reading bodies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TitleBatch {
    pub titles: Vec<String>,
    pub total: u32,
}

const MAX_STRING: usize = 1 << 20;
// The archive reader sorts one u64 offset per cluster on its first payload read.
const MAX_CLUSTERS: u32 = 8 * 1024 * 1024;
const CLUSTER_LIMIT: &str = "This archive exceeds the reader limit of 8,388,608 clusters.";

const CODEC_LIMIT: &str = "The reader could not decompress this archive within its size limits. Choose a different edition or copy.";
const RECORD_LIMIT: &str =
    "An archive metadata entry exceeds the reader’s size limit. Choose a different edition.";
const UNSUPPORTED_FORMAT: &str =
    "This archive uses a format that this reader does not support. Choose a different edition.";

fn reader_limitation(error: &str) -> bool {
    matches!(
        error,
        CLUSTER_LIMIT | CODEC_LIMIT | RECORD_LIMIT | UNSUPPORTED_FORMAT
    )
}

pub(crate) fn integrity_failure(error: String) -> String {
    if reader_limitation(&error) {
        return error;
    }
    eprintln!("Archive integrity check failed: {error}");
    CORRUPT_ARCHIVE.to_owned()
}

// The reader reports exceeding its resource policy and a frame's declared
// output size through these same pinned decoder diagnostics.
pub(super) fn decoder_limit(error: &ZimError) -> bool {
    let ZimError::Decompression(detail) = error else {
        return false;
    };
    matches!(
        detail.as_str(),
        "stored cluster exceeds byte limit"
            | "zstd: window size exceeds decoder limit"
            | "zstd: output limit exceeded"
            | "xz: decoded output exceeds byte limit"
            | "xz: declared output exceeds byte limit"
            | "xz: declared dictionary exceeds decoder memory limit"
            | "xz: dictionary growth exceeds decoder memory limit"
    )
}

pub(super) fn archive_error(error: ZimError) -> String {
    match error {
        ZimError::EntryNotFound | ZimError::NoMainEntry => ENTRY_NOT_FOUND.to_owned(),
        ZimError::UnsupportedVersion { .. } | ZimError::UnsupportedCompression(_) => {
            UNSUPPORTED_FORMAT.to_owned()
        }
        ref error if decoder_limit(error) => CODEC_LIMIT.to_owned(),
        ZimError::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidData => {
            match e.to_string().as_str() {
                "archive counts exceed configured limits" => CLUSTER_LIMIT.to_owned(),
                "ZIM metadata record exceeds configured byte limit" => RECORD_LIMIT.to_owned(),
                _ => format!("{MALFORMED_ARCHIVE}: {error}"),
            }
        }
        ZimError::Io(ref e)
            if !matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData
            ) =>
        {
            format!("Archive read error: {e}")
        }
        _ => format!("{MALFORMED_ARCHIVE}: {error}"),
    }
}

pub(super) fn open_positioned(parts: &Arc<Parts>) -> Result<Zim, String> {
    open_source(parts.clone(), parts.len())
}

pub(super) fn open_source(source: Arc<dyn PositionedInput>, len: u64) -> Result<Zim, String> {
    let limits = ArchiveLimits {
        max_metadata_record_bytes: 2 * MAX_STRING + 512,
        max_clusters: MAX_CLUSTERS,
        cluster: Some(ClusterReadLimits {
            max_decoded_bytes: 64 << 20,
            max_stored_bytes: 65 << 20,
            xz_memory_bytes: 128 << 20,
            zstd_window_log: 26,
        }),
        ..Default::default()
    };
    let archive = Zim::from_positioned(source, len, limits).map_err(archive_error)?;
    validate_layout(&archive)?;
    // Mobile processes have much tighter memory ceilings (Jetsam/LMK) than
    // desktop, so keep per-archive decoded cluster caches smaller on mobile.
    #[cfg(mobile)]
    const CLUSTER_CACHE_MAX_BYTES: usize = 8 << 20;
    #[cfg(not(mobile))]
    const CLUSTER_CACHE_MAX_BYTES: usize = 32 << 20;
    archive.set_cluster_cache_max_bytes(CLUSTER_CACHE_MAX_BYTES);
    Ok(archive)
}

// Check table extents before returning a usable handle. Opening is bounded and does
// not decode every cluster or scan every entry (the integrity check does that).
fn validate_layout(z: &Zim) -> Result<(), String> {
    let h = z.header();
    let size = z.file_len();
    let end = if h.has_checksum() {
        h.checksum_pos
    } else {
        size
    };
    let table_fits = |offset: u64, count: u32, width: u64| {
        offset >= h.mime_list_pos
            && offset
                .checked_add(u64::from(count) * width)
                .is_some_and(|stop| stop <= end)
    };
    let bad_layout = || format!("{MALFORMED_ARCHIVE}: Invalid archive header or table extent");
    if size < 80
        || !matches!(h.mime_list_pos, 72 | 80)
        || (h.entry_count == 0) != (h.cluster_count == 0)
        || h.cluster_count > h.entry_count
        || (h.mime_list_pos != 72 && h.checksum_pos.checked_add(16) != Some(size))
        || !table_fits(h.url_ptr_pos, h.entry_count, 8)
        || !table_fits(h.cluster_ptr_pos, h.cluster_count, 8)
        || (h.title_ptr_pos != u64::MAX && !table_fits(h.title_ptr_pos, h.entry_count, 4))
    {
        return Err(bad_layout());
    }
    if h.cluster_count > 0 {
        let mut bytes = [0; 8];
        let last = h.cluster_ptr_pos + u64::from(h.cluster_count - 1) * 8;
        if z.read_at(last, &mut bytes).map_err(archive_error)? != bytes.len()
            || !(h.mime_list_pos..end).contains(&u64::from_le_bytes(bytes))
        {
            return Err(bad_layout());
        }
    }
    Ok(())
}

fn illustration_entry(z: &Zim) -> Result<Option<Entry>, String> {
    for (namespace, path) in [
        (b'M', "Illustration_48x48@1"),
        (b'-', "favicon"),
        (b'-', "favicon.png"),
        (b'I', "favicon"),
        (b'I', "favicon.png"),
    ] {
        match z.entry_by_ns_path(namespace, path) {
            Ok(entry) => return Ok(Some(entry)),
            Err(ZimError::EntryNotFound) => (),
            Err(error) => return Err(archive_error(error)),
        }
    }
    for entry in z.by_prefix(b'M', "Illustration_") {
        let entry = entry.map_err(archive_error)?;
        if illustration_name(entry.path()) {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

fn illustration_name(name: &str) -> bool {
    let Some(dimensions) = name.strip_prefix("Illustration_") else {
        return false;
    };
    let mut fields = dimensions.split(';');
    let Some((dimensions, scale)) = fields.next().and_then(|s| s.split_once('@')) else {
        return false;
    };
    let Some((width, height)) = dimensions.split_once('x') else {
        return false;
    };
    if ![width, height]
        .iter()
        .all(|s| s.parse::<u32>().is_ok_and(|n| n.to_string() == *s))
        || !scale
            .parse::<f32>()
            .is_ok_and(|n| n.is_finite() && n >= 0.0 && n.to_string() == scale)
    {
        return false;
    }
    let mut last = None;
    for attr in fields {
        let Some((key, _)) = attr.split_once('=') else {
            return false;
        };
        if last.is_some_and(|k| k >= key) {
            return false;
        }
        last = Some(key);
    }
    true
}

pub(super) fn entry_by_path(z: &Zim, path: &str) -> Result<Entry, String> {
    if path.len() > MAX_STRING || path.as_bytes().contains(&0) {
        return Err("Invalid archive argument: invalid or oversized archive path".into());
    }
    let tail = path.strip_prefix('/').unwrap_or(path);
    let qualified = match tail.as_bytes() {
        [ns, b'/', ..] if ns.is_ascii() && *ns != b'/' => Some((*ns, &tail[2..])),
        [ns] if ns.is_ascii() && *ns != b'/' => Some((*ns, "")),
        _ => None,
    };
    let by = |ns, key: &str| z.entry_by_ns_path(ns, key);
    if z.header().uses_new_namespaces() {
        match by(b'C', path) {
            Err(ZimError::EntryNotFound) => {}
            result => return result.map_err(archive_error),
        }
        if let Some((_, key)) = qualified {
            return by(b'C', key).map_err(archive_error);
        }
    } else {
        if let Some((ns, key)) = qualified {
            match by(ns, key) {
                Err(ZimError::EntryNotFound) => {}
                result => return result.map_err(archive_error),
            }
        }
        for &ns in b"AIJ-" {
            match by(ns, path) {
                Err(ZimError::EntryNotFound) => {}
                result => return result.map_err(archive_error),
            }
        }
    }
    Err(ENTRY_NOT_FOUND.into())
}

pub(super) fn resolve(entry: Entry) -> Result<Entry, String> {
    resolve_cancellable(entry, &|| false)
}

pub(super) fn resolve_cancellable(
    mut entry: Entry,
    cancelled: &dyn Fn() -> bool,
) -> Result<Entry, String> {
    for _ in 0..256 {
        if !entry.is_redirect() {
            return Ok(entry);
        }
        if cancelled() {
            return Err("Search cancelled".into());
        }
        entry = entry.get_redirect_entry().map_err(archive_error)?;
    }
    if entry.is_redirect() {
        return Err(format!(
            "{MALFORMED_ARCHIVE}: Cyclic or excessive archive redirect chain (limit 256)"
        ));
    }
    Ok(entry)
}

pub(super) fn public_path(entry: &Entry, modern: bool) -> Result<String, String> {
    let prefix = if modern {
        0
    } else {
        char::from(entry.namespace()).len_utf8() + 1
    };
    if entry.path().len() > MAX_STRING - prefix {
        return Err(RECORD_LIMIT.to_owned());
    }
    Ok(if modern {
        entry.path().to_owned()
    } else {
        format!("{}/{}", char::from(entry.namespace()), entry.path())
    })
}

/// How a serving caller asks for one archive entry.
///
/// Lookup policy is part of the serving contract: documents accept the exact
/// path or its single trailing-slash variant, an empty document location opens
/// the main page (or a random article when the archive has none), and raw
/// resource fetches stay exact.
#[derive(Clone, Copy)]
pub(crate) enum EntryLookup<'a> {
    Exact(&'a str),
    ExactOrTrailingSlash(&'a str),
    MainOrRandom,
}

/// The requested entry's decompressed bytes, retained from the resolution.
///
/// Compressed items decompress their cluster once, so the reported size and
/// every selected byte range come from the same decompressed payload.
pub(crate) struct RetainedItem<'a> {
    archive: &'a Zim,
    index: u32,
    content: ItemContent,
}

enum ItemContent {
    Direct { file_offset: u64, size: u64 },
    Blob(zimru::Blob),
}

impl ItemContent {
    fn load(item: &Item) -> Result<Self, String> {
        let direct = item
            .archive()
            .blob_direct_access(item.cluster_index(), item.blob_index())
            .map_err(archive_error)?;
        if direct.is_direct {
            return Ok(Self::Direct {
                file_offset: direct.file_offset,
                size: direct.size,
            });
        }
        Ok(Self::Blob(item.get_data().map_err(archive_error)?))
    }

    fn size(&self) -> u64 {
        match self {
            Self::Direct { size, .. } => *size,
            Self::Blob(blob) => blob.size() as u64,
        }
    }
}

impl RetainedItem<'_> {
    /// Exact URL index of the resolved entry, stable for this archive's
    /// retained files. Streaming must use this instead of re-resolving a
    /// public path, which can name a different namespace entry.
    pub(crate) fn index(&self) -> u32 {
        self.index
    }

    /// Read an item range from the already-resolved payload, preserving the
    /// clamping and error semantics of [`item_range`].
    pub(crate) fn read_range(&self, offset: u64, length: u64) -> Result<Vec<u8>, String> {
        match &self.content {
            ItemContent::Blob(blob) => {
                let start = usize::try_from(offset)
                    .ok()
                    .filter(|n| *n <= blob.data().len())
                    .ok_or("Invalid archive argument: Archive read offset exceeds item size")?;
                let count = (length as usize).min(blob.data().len() - start);
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(count)
                    .map_err(|_| "Unable to allocate archive range")?;
                bytes.extend_from_slice(&blob.data()[start..start + count]);
                Ok(bytes)
            }
            ItemContent::Direct { file_offset, size } => {
                let remaining = size.checked_sub(offset).ok_or(
                    "Invalid archive argument: Archive read offset exceeds item size",
                )?;
                let count = remaining.min(length) as usize;
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(count)
                    .map_err(|_| "Unable to allocate archive range")?;
                bytes.resize(count, 0);
                let at = file_offset
                    .checked_add(offset)
                    .ok_or("Archive item offset overflow")?;
                if self
                    .archive
                    .read_at(at, &mut bytes)
                    .map_err(archive_error)?
                    != count
                {
                    return Err(format!("{MALFORMED_ARCHIVE}: Truncated archive item range"));
                }
                Ok(bytes)
            }
        }
    }
}

fn lookup_entry(z: &Zim, lookup: EntryLookup<'_>) -> Result<Entry, String> {
    match lookup {
        EntryLookup::Exact(path) => entry_by_path(z, path),
        EntryLookup::ExactOrTrailingSlash(path) => {
            let primary = entry_by_path(z, path);
            match primary {
                Ok(entry) => Ok(entry),
                Err(error) if is_entry_not_found(&error) => {
                    let alternate = if path.is_empty() {
                        None
                    } else if let Some(trimmed) = path.strip_suffix('/') {
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    } else {
                        Some(format!("{path}/"))
                    };
                    let Some(alternate) = alternate else { return Err(error); };
                    match entry_by_path(z, &alternate) {
                        Ok(entry) => Ok(entry),
                        Err(alternate_error) if is_entry_not_found(&alternate_error) => Err(error),
                        Err(alternate_error) => Err(alternate_error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        EntryLookup::MainOrRandom => match crate::archive_metadata::main_entry(z) {
            Ok(entry) => Ok(entry),
            Err(error) if is_entry_not_found(&error) => {
                // Index flags are irrelevant here; only the article count
                // decides whether a random entry would be a real document.
                if crate::archive_metadata::metadata(z, (false, false))?.article_count == 0 {
                    return Err(error);
                }
                match crate::archive_metadata::random_entry(z) {
                    Ok(entry) => Ok(entry),
                    Err(random) if is_entry_not_found(&random) => Err(error),
                    Err(random) => Err(random),
                }
            }
            Err(error) => Err(error),
        },
    }
}

fn item_mime<'a>(z: &'a Zim, entry: &Entry) -> Result<&'a str, String> {
    let zimru::Dirent::Article(article) = entry.dirent() else {
        return Err(format!("{MALFORMED_ARCHIVE}: Expected an archive item"));
    };
    let mime = z
        .mime_list()
        .get(article.mimetype)
        .ok_or_else(|| format!("{MALFORMED_ARCHIVE}: Unknown archive MIME type"))?;
    if mime.len() > MAX_STRING {
        return Err(RECORD_LIMIT.to_owned());
    }
    Ok(mime)
}

pub(super) fn entry_info(entry: Entry) -> Result<EntryInfo, String> {
    let entry = resolve(entry)?;
    let item = entry.get_item(false).map_err(archive_error)?;
    if entry.title().len() > MAX_STRING {
        return Err(RECORD_LIMIT.to_owned());
    }
    Ok(EntryInfo {
        path: public_path(&entry, item.archive().header().uses_new_namespaces())?,
        title: entry.title().to_owned(),
        mime: item_mime(item.archive(), &entry)?.to_owned(),
        size: item.size().map_err(archive_error)?,
    })
}

fn entry_info_with_item(entry: &Entry, item: &Item, size: u64) -> Result<EntryInfo, String> {
    if entry.title().len() > MAX_STRING {
        return Err(RECORD_LIMIT.to_owned());
    }
    Ok(EntryInfo {
        path: public_path(entry, item.archive().header().uses_new_namespaces())?,
        title: entry.title().to_owned(),
        mime: item_mime(item.archive(), entry)?.to_owned(),
        size,
    })
}

fn item_range(item: &Item, offset: u64, length: u64) -> Result<Vec<u8>, String> {
    let direct = item
        .archive()
        .blob_direct_access(item.cluster_index(), item.blob_index())
        .map_err(archive_error)?;
    if !direct.is_direct {
        let blob = item.get_data().map_err(archive_error)?;
        let start = usize::try_from(offset)
            .ok()
            .filter(|n| *n <= blob.data().len())
            .ok_or("Invalid archive argument: Archive read offset exceeds item size")?;
        let count = (length as usize).min(blob.data().len() - start);
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(count)
            .map_err(|_| "Unable to allocate archive range")?;
        bytes.extend_from_slice(&blob.data()[start..start + count]);
        return Ok(bytes);
    }
    let remaining = direct
        .size
        .checked_sub(offset)
        .ok_or("Invalid archive argument: Archive read offset exceeds item size")?;
    let count = remaining.min(length) as usize;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| "Unable to allocate archive range")?;
    bytes.resize(count, 0);
    let at = direct
        .file_offset
        .checked_add(offset)
        .ok_or("Archive item offset overflow")?;
    if item
        .archive()
        .read_at(at, &mut bytes)
        .map_err(archive_error)?
        != count
    {
        return Err(format!("{MALFORMED_ARCHIVE}: Truncated archive item range"));
    }
    Ok(bytes)
}

fn integrity_cancel(cancel: Option<&CancellationToken>) -> Result<(), String> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        Err("Integrity check cancelled".into())
    } else {
        Ok(())
    }
}

fn retained_checksum_cancellable(
    parts: &Parts,
    cancel: Option<&CancellationToken>,
    progress: &mut impl FnMut(u64, u64),
) -> Result<bool, String> {
    integrity_cancel(cancel)?;
    for (file, size) in parts.files() {
        if file.metadata().map_err(|e| e.to_string())?.len() != *size {
            return Err("Retained archive part size changed".into());
        }
    }
    let Some(body) = parts.len().checked_sub(16) else {
        return Ok(false);
    };
    if body < 80 {
        return Ok(false);
    }
    let mut header = [0; 80];
    parts
        .read_exact_at(0, &mut header)
        .map_err(|e| e.to_string())?;
    // The legacy 72-byte header has no checksum offset. For modern headers,
    // hash only when the declared trailer covers the final 16 bytes.
    if header[56..64] == 72u64.to_le_bytes() || header[72..80] != body.to_le_bytes() {
        return Ok(false);
    }
    let mut digest = md5::Context::new();
    let mut checked = 0;
    let mut buffer = [0u8; 64 * 1024];
    progress(0, body);
    while checked < body {
        integrity_cancel(cancel)?;
        let count = (body - checked).min(buffer.len() as u64) as usize;
        parts
            .read_exact_at(checked, &mut buffer[..count])
            .map_err(|e| e.to_string())?;
        digest.consume(&buffer[..count]);
        checked += count as u64;
        if checked == body || checked % (1024 * 1024) == 0 {
            progress(checked, body);
        }
    }
    let mut stored = [0; 16];
    parts
        .read_exact_at(body, &mut stored)
        .map_err(|e| e.to_string())?;
    integrity_cancel(cancel)?;
    Ok(digest.finalize().0 == stored)
}

#[derive(Clone, Debug)]
pub struct Archive {
    search_reader: Arc<SearchReader>,
    archive: Zim,
    parts: Arc<Parts>,
    read_lock: Arc<std::sync::Mutex<()>>,
    #[cfg(test)]
    lock_acquisitions: Arc<std::sync::atomic::AtomicU64>,
    #[cfg(windows)]
    _path_leases: Arc<Vec<std::fs::File>>,
    _scope: Option<Arc<dyn std::fmt::Debug + Send + Sync>>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self, String> {
        // Retain the same objects even if their paths change during a read.
        // Preserve libzim's exact-file preference and its base-name / .zimaa split convention.
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let directory = path
            .parent()
            .ok_or_else(|| archive_open_error(&path, "no parent"))?;
        let split = path.extension().is_some_and(|ext| ext == "zimaa");
        let mut files = Vec::new();
        let add = |files: &mut Vec<_>, name: &Path, file: std::fs::File| -> Result<(), String> {
            let meta = file.metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() {
                return Err(INVALID_ARCHIVE.into());
            }
            if meta.len() == 0 {
                return Err(EMPTY_ARCHIVE.into());
            }
            #[cfg(windows)]
            let name = name
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("Archive filename must be valid Unicode")?
                .to_owned();
            #[cfg(unix)]
            let name = name
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            files.push((name, file));
            Ok(())
        };
        if !split {
            match open_archive_file(&path) {
                Ok(file) => add(&mut files, &path, file)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(archive_open_error(&path, &e.to_string())),
            }
        }
        if files.is_empty() {
            let base = if split {
                path.with_extension("zim")
            } else {
                path.clone()
            };
            for i in 0..676u16 {
                let mut name = base.as_os_str().to_os_string();
                name.push(format!(
                    "{}{}",
                    char::from(b'a' + (i / 26) as u8),
                    char::from(b'a' + (i % 26) as u8)
                ));
                let name = std::path::PathBuf::from(name);
                match open_archive_file(&name) {
                    Ok(file) => {
                        let metadata = file.metadata().map_err(|e| e.to_string())?;
                        if !metadata.is_file() {
                            return Err(INVALID_ARCHIVE.into());
                        }
                        if !files.is_empty() && metadata.len() == 0 {
                            break;
                        }
                        add(&mut files, &name, file)?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound && !files.is_empty() => {
                        break
                    }
                    Err(e) => return Err(archive_open_error(&path, &e.to_string())),
                }
            }
        }
        Self::open_files_scoped(directory, &files, None)
    }

    /// Open exactly the retained regular files, not whatever their old names
    /// may identify later. Windows leases those names against replacement for
    /// the archive lifetime; all reads use the original retained files.
    pub(crate) fn open_files(
        directory: &Path,
        files: &[(String, std::fs::File)],
    ) -> Result<Self, String> {
        Self::open_files_scoped(directory, files, None)
    }

    /// The shared archive drops its positioned readers and retained descriptors
    /// before releasing the caller’s security scope.
    pub(crate) fn open_files_scoped(
        directory: &Path,
        files: &[(String, std::fs::File)],
        scope: Option<Arc<dyn std::fmt::Debug + Send + Sync>>,
    ) -> Result<Self, String> {
        if files.is_empty() || files.len() > 676 {
            return Err("Invalid retained archive part count".into());
        }
        #[cfg(windows)]
        let leases = windows_path_leases(directory, files)?;
        #[cfg(unix)]
        let _ = directory;
        let retained = files
            .iter()
            .map(|(_, file)| {
                #[cfg(unix)]
                let file = file.try_clone().map_err(|e| e.to_string())?;
                #[cfg(windows)]
                let file =
                    cap_fs_ext::Reopen::reopen(file, cap_std::fs::OpenOptions::new().read(true))
                        .map_err(|e| e.to_string())?;
                let meta = file.metadata().map_err(|e| e.to_string())?;
                if !meta.is_file() {
                    return Err(INVALID_ARCHIVE.to_owned());
                }
                if meta.len() == 0 {
                    return Err(EMPTY_ARCHIVE.to_owned());
                }
                Ok((file, meta.len()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let parts = Arc::new(Parts::new(retained)?);
        let archive =
            open_positioned(&parts).map_err(|e| classify_archive_failure(Some(parts.len()), &e))?;
        parts.unchanged()?;
        Ok(Self {
            search_reader: Arc::new(SearchReader::new(parts.clone(), archive.clone())),
            archive,
            parts,
            read_lock: Arc::new(std::sync::Mutex::new(())),
            #[cfg(test)]
            lock_acquisitions: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            #[cfg(windows)]
            _path_leases: Arc::new(leases),
            _scope: scope,
        })
    }

    // Decompressing a single cluster can reach the reader’s full codec budget.
    // Admit one operation at a time across all clones, including metadata and
    // search snippets. The search cache is always acquired inside this gate.
    fn lock_reader(
        &self,
        cancel: Option<(&CancellationToken, &'static str)>,
    ) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        let guard = archive_lock(&self.read_lock, cancel)?;
        #[cfg(test)]
        self.lock_acquisitions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(guard)
    }

    #[cfg(test)]
    pub(crate) fn lock_acquisitions(&self) -> u64 {
        self.lock_acquisitions
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn with_unchanged<T>(
        &self,
        cancel: Option<(&CancellationToken, &'static str)>,
        action: impl FnOnce(&Zim) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.lock_reader(cancel)?;
        self.with_retained_parts(action)
    }

    fn with_retained_parts<T>(&self, action: impl FnOnce(&Zim) -> Result<T, String>) -> Result<T, String> {
        self.parts.unchanged()?;
        let result = action(&self.archive)?;
        self.parts.unchanged()?;
        Ok(result)
    }

    /// Whether the retained files still have the length and modification time
    /// they were opened with (filesystem metadata only, no decoder lock). A
    /// descriptor on a removed volume fails here as well.
    pub(crate) fn retained_unchanged(&self) -> Result<(), String> {
        self.parts.unchanged()
    }

    /// Identify the actual retained files, including each part of a split archive.
    /// Reading filesystem metadata here never waits for the archive decoder.
    pub(crate) fn content_identity(&self) -> Result<crate::integrity::ContentIdentity, String> {
        crate::integrity::ContentIdentity::from_handles(self.parts.files().iter().map(|(file, _)| file))
    }

    pub fn metadata(&self) -> Result<ArchiveMetadata, String> {
        self.read_metadata(None)
    }

    pub(crate) fn metadata_cancellable(
        &self,
        cancel: &CancellationToken,
    ) -> Result<ArchiveMetadata, String> {
        self.read_metadata(Some(cancel))
    }

    pub(crate) fn search_capabilities(&self) -> Result<(bool, bool), String> {
        let _guard = self.lock_reader(None)?;
        self.search_reader.capabilities(None)
    }

    pub(crate) fn search_capabilities_cancellable(&self, cancel: &CancellationToken) -> Result<(bool, bool), String> {
        let _guard = self.lock_reader(Some((cancel, "Search cancelled")))?;
        self.search_reader.capabilities(Some(cancel))
    }

    fn read_metadata(&self, cancel: Option<&CancellationToken>) -> Result<ArchiveMetadata, String> {
        let metadata =
            self.with_unchanged(cancel.map(|token| (token, "Search cancelled")), |z| {
                let indexes = self.search_reader.capabilities(cancel)?;
                crate::archive_metadata::metadata(z, indexes)
            })?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err("Search cancelled".into());
        }
        Ok(metadata)
    }

    pub fn entry(&self, path: &str) -> Result<EntryInfo, String> {
        self.with_unchanged(None, |z| entry_info(entry_by_path(z, path)?))
    }

    /// Resolve one entry and run a serving decision against its retained bytes.
    ///
    /// The entry, its metadata and any selected bytes all come from one read
    /// lock acquisition and one retained file identity check. A compressed
    /// cluster is decompressed at most once: [`EntryInfo::size`] and every
    /// [`RetainedItem::read_range`] call use the same payload. The lock wait
    /// observes `cancel`, so reader retirement can withdraw queued resources
    /// instead of stalling shutdown.
    pub(crate) fn with_entry<T>(
        &self,
        lookup: EntryLookup<'_>,
        cancel: &CancellationToken,
        action: impl FnOnce(&EntryInfo, &RetainedItem<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_unchanged(
            Some((cancel, "Reading stopped. Reload the article")),
            |z| {
                let entry = resolve(lookup_entry(z, lookup)?)?;
                let item = entry.get_item(false).map_err(archive_error)?;
                let content = ItemContent::load(&item)?;
                let info = entry_info_with_item(&entry, &item, content.size())?;
                let retained = RetainedItem {
                    archive: z,
                    index: entry.index(),
                    content,
                };
                action(&info, &retained)
            },
        )
    }

    pub fn read(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>, String> {
        self.read_range(path, offset, length, None)
    }

    /// Read a streamed range from the exact entry index resolved for the
    /// response. Re-resolving the public path here could select a different
    /// namespace entry that shares the same path and splice its bytes into a
    /// response whose size and ETag describe the original entry.
    ///
    /// The lock wait observes `cancel`, so reader retirement can withdraw a
    /// queued streaming read.
    pub(crate) fn read_index_cancellable(
        &self,
        index: u32,
        offset: u64,
        length: u64,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, String> {
        if length > MAX_READ_LENGTH {
            return Err("Archive read exceeds 16 MiB range limit; use successive ranges".into());
        }
        self.with_unchanged(
            Some((cancel, "Reading stopped. Reload the article")),
            |z| {
                let item = z
                    .entry_by_url_index(index)
                    .map_err(archive_error)?
                    .get_item(false)
                    .map_err(archive_error)?;
                item_range(&item, offset, length)
            },
        )
    }

    fn read_range(
        &self,
        path: &str,
        offset: u64,
        length: u64,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, String> {
        if length > MAX_READ_LENGTH {
            return Err("Archive read exceeds 16 MiB range limit; use successive ranges".into());
        }
        self.with_unchanged(
            cancel.map(|token| (token, "Reading stopped. Reload the article")),
            |z| {
                let item = resolve(entry_by_path(z, path)?)?
                    .get_item(false)
                    .map_err(archive_error)?;
                item_range(&item, offset, length)
            },
        )
    }

    pub fn main_entry(&self) -> Result<EntryInfo, String> {
        self.with_unchanged(None, |z| {
            entry_info(crate::archive_metadata::main_entry(z)?)
        })
    }

    pub fn random_entry(&self) -> Result<EntryInfo, String> {
        self.with_unchanged(None, |z| {
            entry_info(crate::archive_metadata::random_entry(z)?)
        })
    }

    /// Whether the archive has at least one front article.
    ///
    /// `getRandomEntry` falls back to metadata and media entries when an
    /// archive has no articles, which would surface raw text; callers check
    /// this before treating a random entry as a readable document.
    pub fn has_articles(&self) -> Result<bool, String> {
        Ok(self.metadata()?.article_count > 0)
    }

    /// The document to show when an archive has no main page: a random article.
    ///
    /// Returns `(entry, true)` for that fallback and `(main, false)` otherwise.
    /// An `EntryNotFound` error means the archive has no main page *and* no
    /// articles, so there is nothing readable to open.
    pub fn main_or_random_entry(&self) -> Result<(EntryInfo, bool), String> {
        match self.main_entry() {
            Ok(entry) => Ok((entry, false)),
            Err(error) if is_entry_not_found(&error) => {
                if !self.has_articles()? {
                    return Err(error);
                }
                match self.random_entry() {
                    Ok(entry) => Ok((entry, true)),
                    Err(random) if is_entry_not_found(&random) => Err(error),
                    Err(random) => Err(random),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Full-text mode requires an index; title mode uses suggestions with the
    /// title-prefix fallback. Snippets contain untrusted archive HTML.
    #[cfg(any(
        test,
        all(
            feature = "android-evaluation",
            target_os = "android",
            debug_assertions
        )
    ))]
    pub fn search(
        &self,
        query: &str,
        fulltext: bool,
        offset: u32,
        limit: u32,
    ) -> Result<SearchResults, String> {
        let _guard = self.lock_reader(None)?;
        self.search_reader
            .page(query, fulltext, offset, limit, true, None)
    }

    /// Interactive search observes cancellation while ranking and building the page.
    pub(crate) fn search_cancellable(
        &self,
        query: &str,
        fulltext: bool,
        offset: u32,
        limit: u32,
        snippets: bool,
        cancel: &CancellationToken,
    ) -> Result<SearchResults, String> {
        let _guard = self.lock_reader(Some((cancel, "Search cancelled")))?;
        self.search_reader
            .page(query, fulltext, offset, limit, snippets, Some(cancel))
    }

    pub(crate) fn rank_cancellable(
        &self, query: &str, fulltext: bool, offset: u32, limit: u32,
        snippets: bool, cancel: &CancellationToken,
    ) -> Result<SearchResults<RankedHit>, String> {
        let _guard = self.lock_reader(Some((cancel, "Search cancelled")))?;
        self.search_reader.ranked(query, fulltext, offset, limit, snippets, cancel)
    }

    pub(crate) fn fill_search_excerpts(
        &self, query: &str, rows: &mut [&mut RankedHit], cancel: &CancellationToken,
    ) -> Result<(), String> {
        let _guard = self.lock_reader(Some((cancel, "Search cancelled")))?;
        self.search_reader.excerpts(query, rows, cancel)
    }

    #[cfg(test)]
    pub(crate) fn cached_search_prefixes(&self) -> usize {
        self.search_reader.cached_prefixes()
    }

    #[cfg(test)]
    pub fn title_batch(&self, offset: u32, limit: u32) -> Result<TitleBatch, String> {
        self.with_unchanged(None, |z| {
            crate::archive_metadata::title_batch(z, offset, limit)
        })
    }

    pub(crate) fn spelling_titles(
        &self, offset: u32, limit: u32, cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<TitleBatch, String> {
        let _guard = interruptible_lock(&self.read_lock, "Spelling indexing cancelled", ||
            cancel.load(std::sync::atomic::Ordering::Acquire))?;
        self.with_retained_parts(|z| crate::archive_metadata::title_batch(z, offset, limit))
    }

    /// Validate the retained archive checksum and structural integrity.
    pub fn check_integrity(&self) -> Result<bool, String> {
        self.check_integrity_with_cancel(None, &mut |_, _| {})
    }

    /// Checksum and structural checks observe cancellation between bounded reads.
    pub(crate) fn check_integrity_cancellable(
        &self,
        cancel: &CancellationToken,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<bool, String> {
        self.check_integrity_with_cancel(Some(cancel), &mut progress)
    }

    fn check_integrity_with_cancel(
        &self,
        cancel: Option<&CancellationToken>,
        progress: &mut impl FnMut(u64, u64),
    ) -> Result<bool, String> {
        integrity_cancel(cancel)?;
        self.parts.unchanged()?;
        if !retained_checksum_cancellable(&self.parts, cancel, progress)? {
            self.parts.unchanged()?;
            return Ok(false);
        }
        self.with_unchanged(
            cancel.map(|token| (token, "Integrity check cancelled")),
            |_| crate::archive_metadata::structure(&self.parts, cancel),
        )
    }

    #[cfg(all(
        feature = "android-evaluation",
        target_os = "android",
        debug_assertions
    ))]
    pub(crate) fn evaluation_checksum(&self) -> Result<bool, String> {
        retained_checksum_cancellable(&self.parts, None, &mut |_, _| {})
    }

    #[cfg(all(
        feature = "android-evaluation",
        target_os = "android",
        debug_assertions
    ))]
    pub(crate) fn evaluation_structure(&self) -> Result<bool, String> {
        self.with_unchanged(None, |_| {
            crate::archive_metadata::structure(&self.parts, None)
        })
    }

    pub fn illustration(&self) -> Result<Option<(String, Vec<u8>)>, String> {
        self.with_unchanged(None, |z| {
            let Some(entry) = illustration_entry(z)? else {
                return Ok(None);
            };
            let entry = resolve(entry)?;
            let item = entry.get_item(false).map_err(archive_error)?;
            let mime = item_mime(z, &entry)?.to_owned();
            let size = item.size().map_err(archive_error)?;
            if size > 2 << 20 {
                return Err("Archive illustration exceeds 2 MiB".into());
            }
            Ok(Some((mime, item_range(&item, 0, size)?)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_not_found_category_is_distinct_from_archive_failures() {
        assert!(is_entry_not_found(ENTRY_NOT_FOUND));
        assert!(is_entry_not_found("Entry not found: Cannot find entry"));
        assert!(!is_entry_not_found("Malformed archive: bad header"));
        assert!(!is_entry_not_found("Entry not foundness"));
        assert!(!is_entry_not_found(""));
    }

    #[test]
    fn disabling_snippets_keeps_title_search_hits() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let titles = archive.title_batch(0, 1).unwrap();
        let query = titles
            .titles
            .first()
            .expect("Fixture must contain an article");
        let cancel = CancellationToken::new();
        let enabled = archive
            .search_cancellable(query, false, 0, 50, true, &cancel)
            .unwrap();
        let disabled = archive
            .search_cancellable(query, false, 0, 50, false, &cancel)
            .unwrap();
        assert!(!enabled.hits.is_empty());
        assert_eq!(enabled.total, disabled.total);
        assert_eq!(enabled.hits.len(), disabled.hits.len());
        for (with_snippet, without_snippet) in enabled.hits.iter().zip(&disabled.hits) {
            assert_eq!(with_snippet.path, without_snippet.path);
            assert_eq!(with_snippet.title, without_snippet.title);
            assert_eq!(with_snippet.score, without_snippet.score);
            assert!(without_snippet.snippet.is_empty());
        }
    }

    #[test]
    fn spelling_can_cancel_or_enter_private_mode_while_reader_is_held() {
        use crate::spelling::{Identity, SpellingManager};
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let identity = Identity::from_archive(&archive, &archive.metadata().unwrap(), "test-installation").unwrap();
        for privacy in [false, true] {
            let cache = std::env::temp_dir().canonicalize().unwrap()
                .join(format!("kiwix-spelling-gate-{}", uuid::Uuid::new_v4()));
            let manager = Arc::new(SpellingManager::new(cache.clone()));
            manager.set_writable(true).unwrap();
            let gate = archive.read_lock.lock().unwrap();
            let status = manager.build(archive.clone(), identity.clone()).unwrap();
            assert!(status.cancellable);
            let deadline = Instant::now() + Duration::from_secs(3);
            let staged = loop {
                if std::fs::read_dir(&cache).is_ok_and(|mut entries| entries.any(|entry|
                    entry.is_ok_and(|e| e.path().extension().is_some_and(|ext| ext == "tmp")))) {
                    break true;
                }
                if Instant::now() >= deadline { break false; }
                std::thread::sleep(Duration::from_millis(5));
            };
            let (tx, rx) = mpsc::channel();
            let worker = Arc::clone(&manager);
            let source_key = identity.key();
            let handle = std::thread::spawn(move || {
                let result = if privacy { worker.set_writable(false) } else {
                    worker.cancel("test-installation", &source_key).map(|_| ())
                };
                let _ = tx.send(result);
            });
            let result = rx.recv_timeout(Duration::from_secs(2));
            drop(gate);
            handle.join().unwrap();
            assert!(staged, "worker must reach the archive input before cancellation");
            result.expect("cancellation must finish while the reader is held").unwrap();
            assert_eq!(manager.status(&identity).unwrap().state, "cancelled");
            assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
            std::fs::remove_dir(cache).unwrap();
        }
    }

    #[test]
    fn waiting_metadata_search_and_integrity_cancel_before_the_reader_is_released() {
        use std::sync::{mpsc, Barrier};
        use std::time::Duration;

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let gate = archive.read_lock.lock().unwrap();
        let token = CancellationToken::new();
        let start = Barrier::new(4);
        let (sent, received) = mpsc::channel();

        std::thread::scope(|scope| {
            for operation in 0..3 {
                let reader = &archive;
                let start = &start;
                let cancel = &token;
                let done = sent.clone();
                scope.spawn(move || {
                    start.wait();
                    let result = match operation {
                        0 => reader
                            .search_cancellable("Test", false, 0, 50, true, cancel)
                            .map(|_| ()),
                        1 => reader.metadata_cancellable(cancel).map(|_| ()),
                        _ => reader
                            .check_integrity_cancellable(cancel, |_, _| {})
                            .map(|_| ()),
                    };
                    done.send((operation, result)).unwrap();
                });
            }
            start.wait();
            let waiting = received.recv_timeout(Duration::from_millis(40)).is_err();
            token.cancel();
            let results: Vec<_> = (0..3)
                .map(|_| received.recv_timeout(Duration::from_secs(2)))
                .collect();
            drop(gate);
            assert!(waiting);
            for result in results {
                let (operation, result) = result.unwrap();
                assert_eq!(
                    result.unwrap_err(),
                    if operation == 2 {
                        "Integrity check cancelled"
                    } else {
                        "Search cancelled"
                    }
                );
            }
        });
        assert!(archive.search("Test", false, 0, 50).is_ok());
        assert!(archive.metadata().is_ok());
        assert!(archive.check_integrity().unwrap());
    }

    #[test]
    fn serving_entry_cancels_while_queued_for_the_reader() {
        use std::sync::{mpsc, Barrier};
        use std::time::Duration;

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let gate = archive.read_lock.lock().unwrap();
        let token = CancellationToken::new();
        let start = Barrier::new(2);
        let (sent, received) = mpsc::channel();

        std::thread::scope(|scope| {
            let reader = &archive;
            let start = &start;
            let cancel = &token;
            scope.spawn(move || {
                start.wait();
                let result = reader
                    .with_entry(EntryLookup::MainOrRandom, cancel, |_, _| Ok(()))
                    .map(|_| ());
                sent.send(result).unwrap();
            });
            start.wait();
            assert!(
                received.recv_timeout(Duration::from_millis(40)).is_err(),
                "serving must wait for the retained reader"
            );
            token.cancel();
            let result = received.recv_timeout(Duration::from_secs(2));
            drop(gate);
            assert_eq!(
                result.unwrap().unwrap_err(),
                "Reading stopped. Reload the article"
            );
        });
    }

    /// A minimal modern-namespace ZIM with a `C/shared.txt` article and an
    /// `I/shared.txt` media entry. Both have the public path `shared.txt`, so
    /// path lookup selects the article; only the URL index names the media.
    fn namespace_collision_archive(article: &[u8], media: &[u8]) -> std::path::PathBuf {
        const MIME: &[u8] = b"text/plain\0\0";
        let dirent = |namespace: u8, cluster: u32, title: &str| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&[0, namespace]);
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&cluster.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(b"shared.txt");
            bytes.push(0);
            bytes.extend_from_slice(title.as_bytes());
            bytes.push(0);
            bytes
        };
        let article_dirent = dirent(b'C', 0, "Article");
        let media_dirent = dirent(b'I', 1, "Image");
        let url_ptr_pos = 80 + MIME.len() as u64;
        let title_ptr_pos = url_ptr_pos + 16;
        let cluster_ptr_pos = title_ptr_pos + 8;
        let dirent_start = cluster_ptr_pos + 16;
        let article_offset = dirent_start;
        let media_offset = article_offset + article_dirent.len() as u64;
        let cluster0 = media_offset + media_dirent.len() as u64;
        let cluster1 = cluster0 + 1 + 16 + article.len() as u64;
        let checksum_pos = cluster1 + 1 + 16 + media.len() as u64;
        let mut bytes = vec![0u8; 80];
        bytes[..4].copy_from_slice(b"ZIM\x04");
        bytes[4..6].copy_from_slice(&5u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
        bytes[24..28].copy_from_slice(&2u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&2u32.to_le_bytes());
        bytes[32..40].copy_from_slice(&url_ptr_pos.to_le_bytes());
        bytes[40..48].copy_from_slice(&title_ptr_pos.to_le_bytes());
        bytes[48..56].copy_from_slice(&cluster_ptr_pos.to_le_bytes());
        bytes[56..64].copy_from_slice(&80u64.to_le_bytes());
        bytes[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[68..72].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[72..80].copy_from_slice(&checksum_pos.to_le_bytes());
        bytes.extend_from_slice(MIME);
        bytes.extend_from_slice(&article_offset.to_le_bytes());
        bytes.extend_from_slice(&media_offset.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&cluster0.to_le_bytes());
        bytes.extend_from_slice(&cluster1.to_le_bytes());
        bytes.extend_from_slice(&article_dirent);
        bytes.extend_from_slice(&media_dirent);
        for data in [article, media] {
            bytes.push(0x11);
            bytes.extend_from_slice(&16u64.to_le_bytes());
            bytes.extend_from_slice(&(16 + data.len() as u64).to_le_bytes());
            bytes.extend_from_slice(data);
        }
        assert_eq!(bytes.len() as u64, checksum_pos);
        bytes.extend_from_slice(&[0u8; 16]);
        let directory =
            std::env::temp_dir().join(format!("kiwix-collision-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("collision.zim");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn index_pinned_range_read_keeps_the_resolved_namespace_entry() {
        let media = vec![b'I'; 1024 * 1024 + 4096];
        let article = b"ARTICLE";
        let path = namespace_collision_archive(article, &media);
        let archive = Archive::open(&path).unwrap();
        assert_eq!(archive.entry("shared.txt").unwrap().size, article.len() as u64);
        let resolved = archive.archive.entry_by_url_index(1).unwrap();
        assert_eq!(resolved.namespace(), b'I');
        assert_eq!(resolved.path(), "shared.txt");
        assert_eq!(archive.read("shared.txt", 0, 4).unwrap(), b"ARTI");
        let cancel = CancellationToken::new();
        assert_eq!(
            archive
                .read_index_cancellable(resolved.index(), 0, 4, &cancel)
                .unwrap(),
            b"IIII"
        );
        assert_eq!(
            archive
                .read_index_cancellable(resolved.index(), 0, media.len() as u64, &cancel)
                .unwrap(),
            media
        );
        drop(archive);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reader_limits_survive_open_entry_and_import_verification() {
        let dir =
            std::env::temp_dir().join(format!("kiwix-reader-limits-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("bounded.zim");
        let mut bytes = vec![0; 80];
        bytes[..4].copy_from_slice(b"ZIM\x04");
        bytes[4] = 6;
        bytes[24..28].copy_from_slice(&1u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&1u32.to_le_bytes());
        bytes[64..72].fill(0xff);
        bytes.extend_from_slice(b"text/html\0\0");
        let path_pos = bytes.len();
        let title_pos = path_pos + 8;
        let cluster_pos = title_pos + 4;
        let entry_pos = cluster_pos + 8;
        bytes.resize(entry_pos + 16, 0);
        bytes[entry_pos + 3] = b'A';
        bytes.extend_from_slice(b"Entry\0\0");
        let cluster = bytes.len() as u64;
        // Legal non-single-segment Zstandard frame, 128 MiB declared window,
        // one raw final block containing the two offsets of an empty item.
        bytes.extend_from_slice(&[5, 0x28, 0xb5, 0x2f, 0xfd, 0, 17 << 3, 65, 0, 0]);
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&8u32.to_le_bytes());
        let end = bytes.len() as u64;
        for (at, value) in [
            (32, path_pos as u64),
            (40, title_pos as u64),
            (48, cluster_pos as u64),
            (56, 80),
            (72, end),
            (path_pos, entry_pos as u64),
            (cluster_pos, cluster),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        let digest = md5::compute(&bytes).0;
        bytes.extend_from_slice(&digest);
        std::fs::write(&file, &bytes).unwrap();
        let archive = Archive::open(&file).unwrap();
        assert_eq!(archive.entry("A/Entry").unwrap_err(), CODEC_LIMIT);
        assert_eq!(
            integrity_failure(archive.check_integrity().unwrap_err()),
            CODEC_LIMIT
        );
        drop(archive);
        bytes[28..32].copy_from_slice(&(MAX_CLUSTERS + 1).to_le_bytes());
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(Archive::open(&file).unwrap_err(), CLUSTER_LIMIT);
        assert_eq!(
            integrity_failure(archive_error(ZimError::Decompression(
                "bad checksum".into()
            ))),
            CORRUPT_ARCHIVE
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_leases_hold_validated_names_until_the_last_archive_drops() {
        let dir = std::env::temp_dir().join(format!("kiwix-retained-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("a.zim");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
            &path,
        )
        .unwrap();
        let archive = Archive::open(&path).unwrap();
        let clone = archive.clone();
        drop(archive);
        let new_name = dir.join("b.zim");
        assert!(std::fs::rename(&path, &new_name).is_err());
        assert!(std::fs::rename(&dir, dir.with_extension("moved")).is_err());
        assert!(clone.check_integrity().unwrap());
        drop(clone);
        std::fs::rename(&path, &new_name).unwrap();
        std::fs::hard_link(&new_name, &path).unwrap();
        assert!(Archive::open(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_split_archive_preserves_zero_length_tables_and_cross_part_checksum() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-empty-split-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        // Valid legacy-format ZIM with no entries/clusters, empty MIME list,
        // zero-length pointer tables at 81, and a checksum spanning both parts.
        let mut bytes = vec![0u8; 81];
        bytes[..4].copy_from_slice(b"ZIM\x04");
        bytes[4] = 5;
        bytes[64..68].copy_from_slice(&u32::MAX.to_le_bytes()); // No main page.
        bytes[68..72].copy_from_slice(&u32::MAX.to_le_bytes()); // No layout page.
        for (offset, value) in [(32, 81u64), (40, 81), (48, 81), (56, 80), (72, 81)] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        let checksum = md5::compute(&bytes).0;
        bytes.extend_from_slice(&checksum);
        std::fs::write(directory.join("empty.zimaa"), &bytes[..90]).unwrap();
        std::fs::write(directory.join("empty.zimab"), &bytes[90..]).unwrap();
        let files = ["empty.zimaa", "empty.zimab"]
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    std::fs::File::open(directory.join(name)).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let archive = Archive::open_files(&directory, &files).unwrap();
        assert_eq!(archive.metadata().unwrap().article_count, 0);
        assert!(archive.check_integrity().unwrap());
        drop(archive);
        drop(files);
        let archive = Archive::open(&directory.join("empty.zimaa")).unwrap();
        assert_eq!(archive.metadata().unwrap().size, 97);
        assert!(archive.check_integrity().unwrap());
        drop(archive);
        // A stray empty next-letter sibling ends name discovery instead of
        // failing the import.
        std::fs::write(directory.join("empty.zimac"), b"").unwrap();
        let archive = Archive::open(&directory.join("empty.zimaa")).unwrap();
        assert_eq!(archive.metadata().unwrap().size, 97);
        assert!(archive.check_integrity().unwrap());
        drop(archive);
        // A non-empty stray sibling would join name discovery, but never an
        // explicitly retained set.
        std::fs::write(directory.join("empty.zimac"), b"stray bytes").unwrap();
        let files = ["empty.zimaa", "empty.zimab"]
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    std::fs::File::open(directory.join(name)).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let archive = Archive::open_files(&directory, &files).unwrap();
        assert_eq!(archive.metadata().unwrap().size, 97);
        assert!(archive.check_integrity().unwrap());
        drop(archive);
        drop(files);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sparse_large_raw_blob_preserves_bounded_u64_reads_and_following_blob() {
        use std::io::{Seek, SeekFrom, Write};
        let directory =
            std::env::temp_dir().join(format!("kiwix-large-range-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        // Reader-only legacy ZIM with two raw blobs in one extended-offset
        // cluster. The sparse hole is addressing evidence, NOT checksum,
        // physical storage capacity, composition or throughput evidence.
        let blob_size = (1u64 << 32) + 128;
        let cluster = 4096u64;
        let data_start = cluster + 1 + 24;
        let checksum_position = data_start + blob_size + 4;
        let mut header = vec![0u8; 124];
        header[..4].copy_from_slice(b"ZIM\x04");
        header[4] = 5;
        header[24..28].copy_from_slice(&2u32.to_le_bytes());
        header[28..32].copy_from_slice(&1u32.to_le_bytes());
        header[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
        header[68..72].copy_from_slice(&u32::MAX.to_le_bytes());
        for (offset, value) in [
            (32, 92u64),
            (40, 108),
            (48, 116),
            (56, 80),
            (72, checksum_position),
        ] {
            header[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        header[80..92].copy_from_slice(b"text/plain\0\0");
        header[116..124].copy_from_slice(&cluster.to_le_bytes());
        for (index, (path, title)) in [("large.txt", "Large range"), ("next.txt", "Next range")]
            .into_iter()
            .enumerate()
        {
            let entry_offset = header.len() as u64;
            header[92 + index * 8..100 + index * 8].copy_from_slice(&entry_offset.to_le_bytes());
            header[108 + index * 4..112 + index * 4].copy_from_slice(&(index as u32).to_le_bytes());
            header.extend_from_slice(&0u16.to_le_bytes()); // MIME index.
            header.extend_from_slice(&[0, b'A']); // Parameter size and namespace.
            header.extend_from_slice(&0u32.to_le_bytes()); // Revision.
            header.extend_from_slice(&0u32.to_le_bytes()); // Cluster index.
            header.extend_from_slice(&(index as u32).to_le_bytes());
            header.extend_from_slice(path.as_bytes());
            header.push(0);
            header.extend_from_slice(title.as_bytes());
            header.push(0);
        }
        let path = directory.join("sparse.zim");
        let mut file = std::fs::File::create_new(&path).unwrap();
        file.write_all(&header).unwrap();
        file.seek(SeekFrom::Start(cluster)).unwrap();
        file.write_all(&[0x11]).unwrap(); // Uncompressed, 64-bit offset table.
        for offset in [24u64, 24 + blob_size, 28 + blob_size] {
            file.write_all(&offset.to_le_bytes()).unwrap();
        }
        file.write_all(b"HEAD").unwrap();
        file.seek(SeekFrom::Start(data_start + (1u64 << 32) + 32))
            .unwrap();
        file.write_all(b"HIGH").unwrap();
        file.seek(SeekFrom::Start(data_start + blob_size - 4))
            .unwrap();
        file.write_all(b"TAILNEXT").unwrap();
        // Deliberately no 4 GiB hash: only bounded range behavior is asserted.
        file.set_len(checksum_position + 16).unwrap();
        drop(file);
        let files = vec![("sparse.zim".into(), std::fs::File::open(&path).unwrap())];
        let archive = Archive::open_files(&directory, &files).unwrap();
        assert_eq!(archive.entry("A/large.txt").unwrap().size, blob_size);
        assert_eq!(archive.read("A/large.txt", 0, 4).unwrap(), b"HEAD");
        assert_eq!(
            archive.read("A/large.txt", (1u64 << 32) + 32, 4).unwrap(),
            b"HIGH"
        );
        assert_eq!(
            archive.read("A/large.txt", blob_size - 4, 4).unwrap(),
            b"TAIL"
        );
        assert_eq!(archive.read("A/next.txt", 0, 4).unwrap(), b"NEXT");
        assert!(archive.read("A/large.txt", blob_size + 1, 4).is_err());
        assert!(archive.read("A/large.txt", 0, blob_size).is_err());
        assert!(archive
            .read("A/large.txt", blob_size, 4)
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn discovered_split_part_must_be_a_regular_file() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-split-pipe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let first = directory.join("book.zimaa");
        std::fs::write(&first, b"first part").unwrap();
        let fifo = directory.join("book.zimab");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let (sent, result) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || sent.send(Archive::open(&first)).unwrap());
        let result = result.recv_timeout(std::time::Duration::from_secs(2));
        if result.is_err() {
            // Release a blocking reader so a regression cannot hold the suite.
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo);
        }
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(result.unwrap().unwrap_err(), INVALID_ARCHIVE);
        thread.join().unwrap();
    }

    #[test]
    fn archive_open_errors_name_missing_empty_and_malformed_files() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-open-errors-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        assert_eq!(
            Archive::open(&directory.join("missing.zim")).unwrap_err(),
            MISSING_ARCHIVE
        );
        let empty = directory.join("empty.zim");
        std::fs::write(&empty, []).unwrap();
        assert_eq!(Archive::open(&empty).unwrap_err(), EMPTY_ARCHIVE);
        let text = directory.join("notes.zim");
        std::fs::write(&text, b"this is not a ZIM archive").unwrap();
        assert_eq!(Archive::open(&text).unwrap_err(), INVALID_ARCHIVE);
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let bytes = std::fs::read(&fixture).unwrap();
        let truncated = directory.join("truncated.zim");
        std::fs::write(&truncated, &bytes[..4096]).unwrap();
        assert_eq!(Archive::open(&truncated).unwrap_err(), INVALID_ARCHIVE);
        let archive = Archive::open(&fixture).unwrap();
        assert!(!archive.metadata().unwrap().id.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn archive_open_errors_name_file_permission_denied() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("kiwix-open-denied-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("denied.zim");
        std::fs::write(&path, b"not readable").unwrap();
        // Restore permissions before removal: the directory itself stays readable,
        // so only the file-level probe can report denial.
        struct Cleanup {
            file: std::path::PathBuf,
            directory: std::path::PathBuf,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ =
                    std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o644));
                let _ = std::fs::set_permissions(
                    &self.directory,
                    std::fs::Permissions::from_mode(0o755),
                );
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
        let _cleanup = Cleanup {
            file: path.clone(),
            directory: directory.clone(),
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let denied = std::fs::File::open(&path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied);
        let message = archive_open_error(&path, "Native archive error: cannot open file");
        if !denied {
            return; // Privileged accounts read through mode 000; nothing to classify.
        }
        assert_eq!(message, ARCHIVE_ACCESS_DENIED);
    }

    #[cfg(unix)]
    #[test]
    fn archive_open_errors_name_empty_but_unreadable_files_as_denied() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("kiwix-open-denied-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("denied-empty.zim");
        std::fs::write(&path, []).unwrap();
        struct Cleanup {
            file: std::path::PathBuf,
            directory: std::path::PathBuf,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ =
                    std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o644));
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
        let _cleanup = Cleanup {
            file: path.clone(),
            directory: directory.clone(),
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let denied = std::fs::File::open(&path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied);
        let message = archive_open_error(&path, "Native archive error: cannot open file");
        if !denied {
            return; // Privileged accounts read through mode 000; nothing to classify.
        }
        assert_eq!(message, ARCHIVE_ACCESS_DENIED);
    }

    #[test]
    fn archive_open_errors_reject_directories() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-open-directory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        assert_eq!(
            archive_open_error(&directory, "Native archive error: cannot open file"),
            INVALID_ARCHIVE
        );
    }

    #[test]
    fn archive_open_errors_hide_unclassified_native_text() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-open-generic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let path = directory.join("opaque.zim");
        std::fs::write(&path, b"opaque bytes").unwrap();
        let message = archive_open_error(&path, "Native archive error: libzim internal detail");
        assert_eq!(message, ARCHIVE_OPEN_FAILED);
        assert!(!message.contains("libzim"));
    }
}
