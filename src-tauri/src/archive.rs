// SPDX-License-Identifier: GPL-3.0-or-later
//! Headless libzim access; this module has no Tauri dependency.
//!
//! Archive paths are libzim paths, not URLs: retain namespaces, percent signs,
//! and Unicode exactly. The caller handles URL decoding once at its HTTP edge.
//! Reads are capped at 16 MiB per call; stream larger media in successive ranges.
//! Search pages are capped at 1000 hits, queries at 64 KiB, metadata strings at
//! 1 MiB, JSON responses at 32 MiB, and redirect chains at 256 hops. These bounds
//! reject oversized/malformed input explicitly rather than silently truncating it.
//! Full-text totals count matches. Suggestion totals can include collapsed hits. Title fallback is case/diacritic-sensitive
//! prefix search when the archive has no suggestion index.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use md5::{Digest, Md5};
use crate::archive_search::{Parts, SearchReader};
use zimru::archive::PositionedInput;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;

pub const MAX_READ_LENGTH: u64 = 16 * 1024 * 1024;

/// Recognize HTML media types independently of case, whitespace, and parameters.
pub(crate) fn is_html_mime(mime: &str) -> bool {
    let media_type = mime.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("text/html")
        || media_type.eq_ignore_ascii_case("application/xhtml+xml")
}

/// Category [`NativeResult::check`] reports for libzim's `EntryNotFound`.
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

/// Category [`NativeResult::check`] reports for structurally invalid input.
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
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => MISSING_ARCHIVE.into(),
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

fn classify_archive_failure(size: Option<u64>, error: &str) -> String {
    if size == Some(0) {
        return EMPTY_ARCHIVE.into();
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
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total: u64,
}

/// A bounded window of public libzim article titles, without reading bodies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TitleBatch {
    pub titles: Vec<String>,
    pub total: u32,
}

#[repr(C)]
struct NativeArchive {
    _opaque: [u8; 0],
}

#[repr(C)]
struct NativeResult {
    data: *mut u8,
    len: usize,
    code: std::ffi::c_int,
}

#[repr(C)]
struct NativeFileInput {
    handle: isize,
    size: u64,
    path: *const u8,
    path_len: usize,
}

unsafe extern "C" {
    fn kz_open_files(files: *const NativeFileInput, count: usize, out: *mut *mut NativeArchive) -> NativeResult;
    fn kz_close(archive: *mut NativeArchive);
    fn kz_free(data: *mut u8);
    fn kz_query(
        archive: *mut NativeArchive,
        operation: std::ffi::c_int,
        input: *const u8,
        input_len: usize,
        offset: u32,
        limit: u32,
    ) -> NativeResult;
    fn kz_read(
        archive: *mut NativeArchive,
        path: *const u8,
        path_len: usize,
        offset: u64,
        length: u64,
        destination: *mut u8,
        capacity: usize,
        illustration: std::ffi::c_int,
    ) -> NativeResult;
}

impl NativeResult {
    fn bytes(&self) -> Result<&[u8], String> {
        if self.len == 0 {
            return Ok(&[]);
        }
        if self.data.is_null() || self.len > isize::MAX as usize {
            return Err("Invalid native archive response buffer".into());
        }
        // SAFETY: query/error buffers are malloc-owned by this result, remain
        // live until Drop, and the native ABI supplies their exact length.
        Ok(unsafe { std::slice::from_raw_parts(self.data, self.len) })
    }

    fn check(&self) -> Result<(), String> {
        if self.code == 0 {
            return Ok(());
        }
        let category = match self.code {
            1 => ENTRY_NOT_FOUND,
            2 => "Malformed archive",
            3 => "Unsupported archive operation",
            4 => "Invalid archive argument",
            _ => "Native archive error",
        };
        let detail = String::from_utf8_lossy(self.bytes()?);
        if detail.is_empty() {
            Err(category.to_string())
        } else {
            Err(format!("{category}: {detail}"))
        }
    }
}

impl Drop for NativeResult {
    fn drop(&mut self) {
        // SAFETY: only kz_free releases this ABI's malloc-owned buffers; null is
        // valid, including successful read results whose len is a byte count.
        unsafe { kz_free(self.data) };
    }
}

#[derive(Debug)]
struct Handle {
    native: NonNull<NativeArchive>,
    parts: Arc<Parts>,
    /// Retains the caller's path/capability leases until after kz_close.
    _scope: Option<Arc<dyn std::fmt::Debug + Send + Sync>>,
}

// SAFETY: `Handle` owns the only pointer to its libzim archive. libzim objects
// are not safe for concurrent use, so every native operation runs while holding the `Mutex`
// inside `Archive::handle`, and no borrowed pointer or iterator outlives that
// guard. Moving the handle between threads is therefore safe: it holds no
// thread-affine state, the `Arc` keeps it alive until the last `Archive` clone
// is dropped, and `Handle` deliberately does not implement `Sync`, so the
// pointer cannot be shared without that mutex. Enforced by `Archive::handle`
// being private and reachable only through `lock()`.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this is the unique owner; Arc/Mutex prevent simultaneous calls
        // or destruction while another thread is using the handle.
        unsafe { kz_close(self.native.as_ptr()) };
    }
}

// Verify MD5 independently over the exact caller-retained split byte stream.
// The patched SDK also preserves descriptor offsets; Rust's progress/cancellation
// contract remains separate from libzim's complete structural validation.

fn integrity_cancel(cancel: Option<&CancellationToken>) -> Result<(), String> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        Err("Integrity check cancelled".into())
    } else { Ok(()) }
}

fn retained_checksum_cancellable(parts: &Parts, cancel: Option<&CancellationToken>,
    progress: &mut impl FnMut(u64, u64)) -> Result<bool, String> {
    integrity_cancel(cancel)?;
    for (file, size) in parts.files() {
        if file.metadata().map_err(|e| e.to_string())?.len() != *size {
            return Err("Retained archive part size changed".into());
        }
    }
    let Some(body) = parts.len().checked_sub(16) else { return Ok(false); };
    if body < 80 { return Ok(false); }
    let mut header = [0; 80];
    parts.read_exact_at(0, &mut header).map_err(|e| e.to_string())?;
    // The legacy 72-byte header has no checksum offset. For modern headers,
    // hash only when the declared trailer covers the final 16 bytes.
    if header[56..64] == 72u64.to_le_bytes() || header[72..80] != body.to_le_bytes() {
        return Ok(false);
    }
    let mut digest = Md5::new();
    let mut checked = 0;
    let mut buffer = [0u8; 64 * 1024];
    progress(0, body);
    while checked < body {
        integrity_cancel(cancel)?;
        let count = (body - checked).min(buffer.len() as u64) as usize;
        parts.read_exact_at(checked, &mut buffer[..count]).map_err(|e| e.to_string())?;
        digest.update(&buffer[..count]);
        checked += count as u64;
        if checked == body || checked % (1024 * 1024) == 0 { progress(checked, body); }
    }
    let mut stored = [0; 16];
    parts.read_exact_at(body, &mut stored).map_err(|e| e.to_string())?;
    integrity_cancel(cancel)?;
    Ok(digest.finalize().as_slice() == stored)
}

#[derive(Clone, Debug)]
pub struct Archive {
    // Drop search caches and their file references before the native provider lease.
    search_reader: Arc<SearchReader>,
    handle: Arc<Mutex<Handle>>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self, String> {
        // Both engines must see the same open objects even if names change during a search.
        // Preserve libzim's exact-file preference and its base-name / .zimaa split convention.
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let directory = path.parent().ok_or_else(|| archive_open_error(&path, "no parent"))?;
        let split = path.extension().is_some_and(|ext| ext == "zimaa");
        let mut files = Vec::new();
        let add = |files: &mut Vec<_>, name: &Path, file: std::fs::File| -> Result<(), String> {
            let meta = file.metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() { return Err(INVALID_ARCHIVE.into()); }
            if meta.len() == 0 { return Err(EMPTY_ARCHIVE.into()); }
            #[cfg(windows)]
            let name = name.file_name().and_then(|name| name.to_str()).ok_or("Archive filename must be valid Unicode")?.to_owned();
            #[cfg(unix)]
            let name = name.file_name().unwrap_or_default().to_string_lossy().into_owned();
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
            let base = if split { path.with_extension("zim") } else { path.clone() };
            for i in 0..676u16 {
                let mut name = base.as_os_str().to_os_string();
                name.push(format!("{}{}", char::from(b'a' + (i / 26) as u8), char::from(b'a' + (i % 26) as u8)));
                let name = std::path::PathBuf::from(name);
                match open_archive_file(&name) {
                    Ok(file) => {
                        let metadata = file.metadata().map_err(|e| e.to_string())?;
                        if !metadata.is_file() { return Err(INVALID_ARCHIVE.into()); }
                        if !files.is_empty() && metadata.len() == 0 { break; }
                        add(&mut files, &name, file)?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound && !files.is_empty() => break,
                    Err(e) => return Err(archive_open_error(&path, &e.to_string())),
                }
            }
        }
        Self::open_files_scoped(directory, &files, None)
    }

    /// Open exactly the retained regular files, not whatever their old names
    /// may identify later. Windows leases those names against replacement for
    /// the complete native archive lifetime; POSIX uses patched libzim FdInput.
    pub(crate) fn open_files(
        directory: &Path,
        files: &[(String, std::fs::File)],
    ) -> Result<Self, String> {
        Self::open_files_scoped(directory, files, None)
    }

    /// Scope authority belongs to the same shared handle as libzim and its exact
    /// descriptor duplicates. Handle::drop closes libzim before fields drop:
    /// retained descriptors first, then the native security-scope lease.
    pub(crate) fn open_files_scoped(
        directory: &Path,
        files: &[(String, std::fs::File)],
        scope: Option<Arc<dyn std::fmt::Debug + Send + Sync>>,
    ) -> Result<Self, String> {
        if files.is_empty() || files.len() > 676 {
            return Err("Invalid retained archive part count".into());
        }
        #[cfg(windows)]
        let paths: Vec<Vec<u8>> = files.iter().map(|(name, _)| {
            directory.join(name).to_str()
                .map(|path| path.as_bytes().to_vec())
                .ok_or_else(|| "Archive filesystem path is not valid UTF-8".to_string())
        }).collect::<Result<_, _>>()?;
        #[cfg(unix)]
        let _ = directory;
        // Keep descriptors for the native handle and the Rust positioned source
        // even when the caller immediately closes its own handles.
        let retained = files.iter().map(|(_, file)| {
            #[cfg(unix)]
            let file = file.try_clone().map_err(|error| error.to_string())?;
            // Windows seek_read updates the file object's cursor. Keep our positional
            // reads independent from consumers of the caller's handle.
            #[cfg(windows)]
            let file = cap_fs_ext::Reopen::reopen(file, cap_std::fs::OpenOptions::new().read(true))
                .map_err(|error| error.to_string())?;
            let size = file.metadata().map_err(|error| error.to_string())?.len();
            Ok((file, size))
        }).collect::<Result<Vec<_>, String>>()?;
        let parts = Arc::new(Parts::new(retained)?);
        let inputs: Vec<_> = parts.files().iter().enumerate().map(|(index, (file, size))| {
            #[cfg(unix)]
            let handle = {
                use std::os::fd::AsRawFd;
                let _ = index;
                file.as_raw_fd() as isize
            };
            #[cfg(windows)]
            let handle = {
                use std::os::windows::io::AsRawHandle;
                file.as_raw_handle() as isize
            };
            #[cfg(unix)]
            let (path, path_len) = (std::ptr::null(), 0);
            #[cfg(windows)]
            let (path, path_len) = (paths[index].as_ptr(), paths[index].len());
            NativeFileInput { handle, size: *size, path, path_len }
        }).collect();
        let mut raw = std::ptr::null_mut();
        // SAFETY: parts owns the input descriptors and moves into Handle
        // below, so they outlive every native operation.
        // Windows additionally retains its identity-checked path leases.
        let response = unsafe { kz_open_files(inputs.as_ptr(), inputs.len(), &mut raw) };
        if let Err(error) = response.check() {
            // The retained descriptors are authoritative here: their directory
            // entries may already have been renamed or replaced.
            return Err(classify_archive_failure(Some(parts.files()[0].1), &error));
        }
        let handle = NonNull::new(raw).ok_or("Native retained archive open returned a null handle")?;
        Ok(Self {
            search_reader: Arc::new(SearchReader::new(parts.clone())),
            handle: Arc::new(Mutex::new(Handle { native: handle, parts, _scope: scope })),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Handle>, String> {
        self.handle
            .lock()
            .map_err(|_| "Archive operation lock is poisoned".to_string())
    }

    fn query<T: DeserializeOwned>(
        &self,
        operation: std::ffi::c_int,
        input: &str,
        offset: u32,
        limit: u32,
        cancel: Option<&CancellationToken>,
    ) -> Result<T, String> {
        if cancel.is_some_and(CancellationToken::is_cancelled) { return Err("Search cancelled".into()); }
        let response = {
            let handle = self.lock()?;
            // A cancelled waiter may still acquire this blocking mutex, but it
            // must not start another native operation after gaining the lock.
            if cancel.is_some_and(CancellationToken::is_cancelled) { return Err("Search cancelled".into()); }
            // SAFETY: the mutex serializes the complete native operation and
            // guarantees lifetime; input is borrowed only for this call.
            unsafe {
                kz_query(
                    handle.native.as_ptr(),
                    operation,
                    input.as_ptr(),
                    input.len(),
                    offset,
                    limit,
                )
            }
        };
        // libzim itself has no interruption hook. Discard its completed native
        // buffer before JSON decoding when the request retired during the call.
        if cancel.is_some_and(CancellationToken::is_cancelled) { return Err("Search cancelled".into()); }
        response.check()?;
        serde_json::from_slice(response.bytes()?)
            .map_err(|error| format!("Invalid native archive JSON response: {error}"))
    }

    pub fn metadata(&self) -> Result<ArchiveMetadata, String> {
        self.read_metadata(None)
    }

    pub(crate) fn metadata_cancellable(&self, cancel: &CancellationToken) -> Result<ArchiveMetadata, String> {
        self.read_metadata(Some(cancel))
    }

    pub(crate) fn search_capabilities(&self) -> Result<(bool, bool), String> {
        self.search_reader.capabilities(None)
    }

    fn read_metadata(&self, cancel: Option<&CancellationToken>) -> Result<ArchiveMetadata, String> {
        let mut metadata: ArchiveMetadata = self.query(0, "", 0, 0, cancel)?;
        (metadata.has_fulltext_index, metadata.has_title_index) = self.search_reader.capabilities(cancel)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) { return Err("Search cancelled".into()); }
        Ok(metadata)
    }

    /// Resolve redirects and return the canonical item path and MIME type.
    pub fn entry(&self, path: &str) -> Result<EntryInfo, String> {
        self.query(1, path, 0, 0, None)
    }

    /// Read at most length bytes. EOF is empty; offsets past EOF are errors.
    /// The native getData(offset, count) call never requests the entire media.
    pub fn read(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>, String> {
        self.read_inner(path, offset, length, false)
    }

    fn read_inner(
        &self,
        path: &str,
        offset: u64,
        length: u64,
        illustration: bool,
    ) -> Result<Vec<u8>, String> {
        if length > MAX_READ_LENGTH {
            return Err("Archive read exceeds 16 MiB range limit; use successive ranges".into());
        }
        let capacity = usize::try_from(length)
            .map_err(|_| "Archive range does not fit this platform's address space".to_string())?;
        // Reads on this handle are serialized. Wait before reserving the output
        // so queued resource requests do not each hold a full range buffer.
        let handle = self.lock()?;
        let mut bytes = Vec::<u8>::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|error| format!("Cannot allocate archive range: {error}"))?;
        // SAFETY: the pointer has capacity bytes of allocated writable storage,
        // but its uninitialized contents are not exposed as Rust values. The ABI
        // writes only the returned byte count; the mutex keeps the handle live.
        let response = unsafe {
            kz_read(
                handle.native.as_ptr(),
                path.as_ptr(),
                path.len(),
                offset,
                length,
                bytes.as_mut_ptr(),
                capacity,
                i32::from(illustration),
            )
        };
        response.check()?;
        if response.len > capacity || !response.data.is_null() {
            return Err("Native archive read returned an invalid byte count or buffer".into());
        }
        // SAFETY: exactly response.len bytes were initialized by kz_read above.
        unsafe { bytes.set_len(response.len) };
        Ok(bytes)
    }

    pub fn main_entry(&self) -> Result<EntryInfo, String> {
        self.query(2, "", 0, 0, None)
    }

    pub fn random_entry(&self) -> Result<EntryInfo, String> {
        self.query(3, "", 0, 0, None)
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
    #[cfg(any(test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
    pub fn search(
        &self,
        query: &str,
        fulltext: bool,
        offset: u32,
        limit: u32,
    ) -> Result<SearchResults, String> {
        self.search_reader.page(query, fulltext, offset, limit, true, None)
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
        self.search_reader.page(query, fulltext, offset, limit, snippets, Some(cancel))
    }

    /// Explicit background dictionary builders only, not interactive searches.
    /// Maximum 256 titles, each at most 16 KiB; offsets seek directly.
    pub fn title_batch(&self, offset: u32, limit: u32) -> Result<TitleBatch, String> {
        self.query(7, "", offset, limit, None)
    }

    /// Expensive: validates checksum and all libzim structural integrity checks.
    pub fn check_integrity(&self) -> Result<bool, String> {
        self.check_integrity_with_cancel(None, &mut |_, _| {})
    }

    /// Retained-byte MD5 is interruptible at each bounded read. Libzim's
    /// structural call is not preemptible: cancellation discards its result.
    pub(crate) fn check_integrity_cancellable(
        &self, cancel: &CancellationToken, mut progress: impl FnMut(u64, u64),
    ) -> Result<bool, String> {
        self.check_integrity_with_cancel(Some(cancel), &mut progress)
    }

    fn check_integrity_with_cancel(&self, cancel: Option<&CancellationToken>,
        progress: &mut impl FnMut(u64, u64)) -> Result<bool, String> {
        integrity_cancel(cancel)?;
        let handle = self.lock()?;
        integrity_cancel(cancel)?;
        if !retained_checksum_cancellable(&handle.parts, cancel, progress)? { return Ok(false); }
        integrity_cancel(cancel)?;
        // SAFETY: the same mutex serializes checksum reads, native checks, and
        // destruction. No input buffer is required for the structural integrity opcode.
        let response = unsafe { kz_query(handle.native.as_ptr(), 8, std::ptr::null(), 0, 0, 0) };
        integrity_cancel(cancel)?;
        response.check()?;
        serde_json::from_slice(response.bytes()?)
            .map_err(|error| format!("Invalid native archive JSON response: {error}"))
    }

    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    pub(crate) fn evaluation_checksum(&self) -> Result<bool, String> {
        let handle = self.lock()?;
        retained_checksum_cancellable(&handle.parts, None, &mut |_, _| {})
    }

    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    pub(crate) fn evaluation_structure(&self) -> Result<bool, String> {
        self.query(8, "", 0, 0, None)
    }

    pub fn illustration(&self) -> Result<Option<(String, Vec<u8>)>, String> {
        let info: Option<EntryInfo> = self.query(6, "", 0, 0, None)?;
        match info {
            Some(info) => {
                if info.size > 2 * 1024 * 1024 {
                    return Err("Archive illustration exceeds 2 MiB".into());
                }
                let bytes = self.read_inner("", 0, info.size, true)?;
                Ok(Some((info.mime, bytes)))
            }
            None => Ok(None),
        }
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
    fn disabling_native_snippets_keeps_title_search_hits() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let titles = archive.title_batch(0, 1).unwrap();
        let query = titles.titles.first().expect("Fixture must contain an article");
        let cancel = CancellationToken::new();
        let enabled = archive.search_cancellable(query, false, 0, 50, true, &cancel).unwrap();
        let disabled = archive.search_cancellable(query, false, 0, 50, false, &cancel).unwrap();
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
    fn cancelled_search_does_not_wait_for_another_archive_consumer() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let reader = archive.clone();
        let lock = archive.lock().unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (completed, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            completed.send(reader.search_cancellable("Test", false, 0, 50, true, &cancel)).unwrap();
        });
        let result = result.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(result.unwrap_err(), "Search cancelled");
        drop(lock);
        worker.join().unwrap();
        assert!(archive.search("Test", false, 0, 50).is_ok());
    }

    #[test]
    fn empty_split_archive_preserves_zero_length_tables_and_cross_part_checksum() {
        let directory = std::env::temp_dir().join(format!("kiwix-empty-split-{}", uuid::Uuid::new_v4()));
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
        let checksum = Md5::digest(&bytes);
        bytes.extend_from_slice(&checksum);
        std::fs::write(directory.join("empty.zimaa"), &bytes[..90]).unwrap();
        std::fs::write(directory.join("empty.zimab"), &bytes[90..]).unwrap();
        let files = ["empty.zimaa", "empty.zimab"].into_iter()
            .map(|name| (name.to_string(), std::fs::File::open(directory.join(name)).unwrap()))
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
        let files = ["empty.zimaa", "empty.zimab"].into_iter()
            .map(|name| (name.to_string(), std::fs::File::open(directory.join(name)).unwrap()))
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
        let directory = std::env::temp_dir().join(format!("kiwix-large-range-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
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
        for (offset, value) in [(32, 92u64), (40, 108), (48, 116), (56, 80), (72, checksum_position)] {
            header[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        header[80..92].copy_from_slice(b"text/plain\0\0");
        header[116..124].copy_from_slice(&cluster.to_le_bytes());
        for (index, (path, title)) in [("large.txt", "Large range"), ("next.txt", "Next range")].into_iter().enumerate() {
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
        file.seek(SeekFrom::Start(data_start + (1u64 << 32) + 32)).unwrap();
        file.write_all(b"HIGH").unwrap();
        file.seek(SeekFrom::Start(data_start + blob_size - 4)).unwrap();
        file.write_all(b"TAILNEXT").unwrap();
        // Deliberately no 4 GiB hash: only bounded range behavior is asserted.
        file.set_len(checksum_position + 16).unwrap();
        drop(file);
        let files = vec![("sparse.zim".into(), std::fs::File::open(&path).unwrap())];
        let archive = Archive::open_files(&directory, &files).unwrap();
        assert_eq!(archive.entry("A/large.txt").unwrap().size, blob_size);
        assert_eq!(archive.read("A/large.txt", 0, 4).unwrap(), b"HEAD");
        assert_eq!(archive.read("A/large.txt", (1u64 << 32) + 32, 4).unwrap(), b"HIGH");
        assert_eq!(archive.read("A/large.txt", blob_size - 4, 4).unwrap(), b"TAIL");
        assert_eq!(archive.read("A/next.txt", 0, 4).unwrap(), b"NEXT");
        assert!(archive.read("A/large.txt", blob_size + 1, 4).is_err());
        assert!(archive.read("A/large.txt", 0, blob_size).is_err());
        assert!(archive.read("A/large.txt", blob_size, 4).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn discovered_split_part_must_be_a_regular_file() {
        let directory = std::env::temp_dir().join(format!("kiwix-split-pipe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let first = directory.join("book.zimaa");
        std::fs::write(&first, b"first part").unwrap();
        let fifo = directory.join("book.zimab");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let (sent, result) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || sent.send(Archive::open(&first)).unwrap());
        let result = result.recv_timeout(std::time::Duration::from_secs(2));
        if result.is_err() {
            // Release a blocking reader so a regression cannot hold the suite.
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(&fifo);
        }
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(result.unwrap().unwrap_err(), INVALID_ARCHIVE);
        thread.join().unwrap();
    }

    #[test]
    fn archive_open_errors_name_missing_empty_and_malformed_files() {
        let directory = std::env::temp_dir().join(format!("kiwix-open-errors-{}", uuid::Uuid::new_v4()));
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
        let directory = std::env::temp_dir().join(format!("kiwix-open-denied-{}", uuid::Uuid::new_v4()));
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
                let _ = std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o644));
                let _ = std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o755));
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
        let _cleanup = Cleanup { file: path.clone(), directory: directory.clone() };
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
        let directory = std::env::temp_dir().join(format!("kiwix-open-denied-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("denied-empty.zim");
        std::fs::write(&path, []).unwrap();
        struct Cleanup {
            file: std::path::PathBuf,
            directory: std::path::PathBuf,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o644));
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
        let _cleanup = Cleanup { file: path.clone(), directory: directory.clone() };
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
        let directory = std::env::temp_dir().join(format!("kiwix-open-directory-{}", uuid::Uuid::new_v4()));
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
        let directory = std::env::temp_dir().join(format!("kiwix-open-generic-{}", uuid::Uuid::new_v4()));
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
