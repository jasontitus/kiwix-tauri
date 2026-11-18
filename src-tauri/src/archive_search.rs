// SPDX-License-Identifier: GPL-3.0-or-later
//! Search the embedded Glass database over the very files admitted to the archive.
#![forbid(unsafe_code)]

use crate::archive::{SearchHit, SearchResults};
use parking_lot::{Mutex, MutexGuard};
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zimfile_xapian::{Index, ReaderLimits, SnippetQuery, Source};
use zimru::archive::PositionedInput;
use zimru::{Archive as Zim, ArchiveLimits, ClusterReadLimits, Entry, Error as ZimError, Item};

#[derive(Debug)]
pub(super) struct Parts {
    files: Vec<(File, u64)>,
    ends: Vec<u64>,
    modified: Vec<Option<std::time::SystemTime>>,
}

impl Parts {
    pub(super) fn new(files: Vec<(File, u64)>) -> Result<Self, String> {
        let mut ends = Vec::with_capacity(files.len());
        let mut total = 0u64;
        for (_, size) in &files {
            total = total
                .checked_add(*size)
                .ok_or("Archive part lengths overflow")?;
            ends.push(total);
        }
        let modified = files
            .iter()
            .map(|(file, _)| {
                file.metadata()
                    .map(|metadata| metadata.modified().ok())
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            files,
            ends,
            modified,
        })
    }

    // A descriptor remains authoritative after a rename. Reject observable
    // in-place edits before reusing cached directory and index blocks.
    fn unchanged(&self) -> Result<(), String> {
        for ((file, len), modified) in self.files.iter().zip(&self.modified) {
            let now = file.metadata().map_err(|e| e.to_string())?;
            if now.len() != *len || now.modified().ok() != *modified {
                return Err("Archive contents changed while searching. Reopen the archive.".into());
            }
        }
        Ok(())
    }

    pub(super) fn files(&self) -> &[(File, u64)] {
        &self.files
    }

    pub(super) fn len(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }
}

impl PositionedInput for Parts {
    fn read_exact_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > self.len())
        {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let mut i = self.ends.partition_point(|&end| end <= offset);
        while !buf.is_empty() {
            let (file, size) = &self.files[i];
            let end = self.ends[i];
            let count = (end - offset).min(buf.len() as u64) as usize;
            let (chunk, rest) = buf.split_at_mut(count);
            read_file(file, offset - (end - size), chunk)?;
            buf = rest;
            offset += count as u64;
            i += 1;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn read_file(file: &File, pos: u64, bytes: &mut [u8]) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, bytes, pos)
}

#[cfg(windows)]
fn read_file(file: &File, mut pos: u64, mut buf: &mut [u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, pos) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => {
                pos += count as u64;
                buf = &mut buf[count..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

struct IndexBytes {
    parts: Arc<Parts>,
    range: Range<u64>,
}

impl Source for IndexBytes {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> zimfile_xapian::Result<()> {
        let length = self.range.end - self.range.start;
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > length)
        {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        Ok(self.parts.read_exact_at(self.range.start + offset, buf)?)
    }
}

struct OpenIndex {
    index: Index,
    language: String,
}

#[derive(Default)]
struct IndexSlot {
    // Cache absence, but allow a transient I/O error to be retried.
    extent: Option<Option<Range<u64>>>,
    open: Option<OpenIndex>,
}

impl IndexSlot {
    fn entry(archive: &Zim, fulltext: bool) -> Result<Entry, ZimError> {
        if fulltext {
            match archive.entry_by_ns_path(b'X', "fulltext/xapian") {
                Err(ZimError::EntryNotFound) => {
                    archive.entry_by_ns_path(b'Z', "/fulltextIndex/xapian")
                }
                other => other,
            }
        } else {
            archive.entry_by_ns_path(b'X', "title/xapian")
        }
    }

    fn item(archive: &Zim, fulltext: bool) -> Result<Option<Item>, String> {
        match Self::entry(archive, fulltext) {
            Err(ZimError::EntryNotFound) => Ok(None),
            entry => resolve(entry.map_err(|e| e.to_string())?, &|| false)?
                .get_item(false)
                .map(Some)
                .map_err(|e| e.to_string()),
        }
    }

    // Capability reporting runs during import. It needs the storage type,
    // independent of the number or ordering of all other clusters.
    fn available(&self, archive: &Zim, fulltext: bool) -> Result<bool, String> {
        if let Some(range) = &self.extent {
            return Ok(range.is_some());
        }
        let Some(item) = Self::item(archive, fulltext)? else {
            return Ok(false);
        };
        let offset = archive
            .cluster_offset(item.cluster_index())
            .map_err(|e| e.to_string())?;
        let mut info = [0; 1];
        if archive
            .read_at(offset, &mut info)
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("Truncated search index cluster".into());
        }
        Ok(matches!(info[0] & 0x0f, 0 | 1))
    }

    fn extent(&mut self, archive: &Zim, fulltext: bool) -> Result<Option<Range<u64>>, String> {
        if let Some(range) = &self.extent {
            return Ok(range.clone());
        }
        let range = match Self::item(archive, fulltext)? {
            None => None,
            Some(item) => {
                let extent = archive
                    .blob_direct_access(item.cluster_index(), item.blob_index())
                    .map_err(|e| e.to_string())?;
                if extent.is_direct {
                    let end = extent
                        .file_offset
                        .checked_add(extent.size)
                        .filter(|&end| end <= archive.file_len())
                        .ok_or("Embedded search index exceeds the archive")?;
                    Some(extent.file_offset..end)
                } else {
                    None
                }
            }
        };
        self.extent = Some(range.clone());
        Ok(range)
    }

    fn open(
        &mut self,
        archive: &Zim,
        parts: &Arc<Parts>,
        fulltext: bool,
    ) -> Result<Option<&OpenIndex>, String> {
        if self.open.is_none() {
            let Some(range) = self.extent(archive, fulltext)? else {
                return Ok(None);
            };
            let src = IndexBytes {
                parts: parts.clone(),
                range,
            };
            let index = Index::open_with_limits(
                Box::new(src),
                ReaderLimits {
                    max_query_bytes: MAX_QUERY,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            let expected = if fulltext {
                b"fulltext".as_slice()
            } else {
                b"title"
            };
            let declared = index
                .metadata("kind")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            if !declared.is_empty() && declared != expected {
                return Err("Unexpected embedded search index kind".into());
            }
            let language = if index
                .metadata("language")
                .map_err(|e| e.to_string())?
                .is_none_or(|v| v.is_empty())
            {
                match archive.entry_by_ns_path(b'M', "Language") {
                    Err(ZimError::EntryNotFound) => String::new(),
                    result => {
                        let item = result
                            .and_then(|entry| entry.get_item(true))
                            .map_err(|e| e.to_string())?;
                        let mut prefix = [0; 1025];
                        let n = item.read_at(0, &mut prefix).map_err(|e| e.to_string())?;
                        let lang = prefix[..n].split(|&b| b == b',').next().unwrap_or_default();
                        if lang.len() > 1024 {
                            return Err("Archive language tag exceeds 1024 bytes".into());
                        }
                        std::str::from_utf8(lang)
                            .map_err(|e| e.to_string())?
                            .trim()
                            .to_owned()
                    }
                }
            } else {
                String::new()
            };
            self.open = Some(OpenIndex { index, language });
        }
        Ok(self.open.as_ref())
    }
}

#[derive(Default)]
struct Cache {
    archive: Option<Zim>,
    prefixes: PrefixCache,
    title: IndexSlot,
    fulltext: IndexSlot,
}

impl Cache {
    fn archive(&mut self, parts: &Arc<Parts>) -> Result<Zim, String> {
        if let Some(archive) = &self.archive {
            return Ok(archive.clone());
        }
        let limits = ArchiveLimits {
            max_metadata_record_bytes: 2 * 1024 * 1024 + 512,
            cluster: Some(ClusterReadLimits {
                max_decoded_bytes: 64 << 20,
                max_stored_bytes: 65 << 20,
                xz_memory_bytes: 128 << 20,
                zstd_window_log: 26,
            }),
            ..Default::default()
        };
        let archive =
            Zim::from_positioned(parts.clone(), parts.len(), limits).map_err(|e| e.to_string())?;
        archive.set_cluster_cache_max_bytes(4 << 20);
        self.archive = Some(archive.clone());
        Ok(archive)
    }
}

pub(super) struct SearchReader {
    parts: Arc<Parts>,
    cache: Mutex<Cache>,
}

impl std::fmt::Debug for SearchReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchReader")
            .field("bytes", &self.parts.len())
            .finish_non_exhaustive()
    }
}

const MAX_QUERY: usize = 64 * 1024;
const MAX_RESULT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SNIPPET_INPUT: usize = 64 * 1024;

fn original_entry(archive: &Zim, data: &str) -> Result<Entry, String> {
    if data.len() > 1 << 20 || data.as_bytes().contains(&0) {
        return Err("Invalid indexed path".into());
    }
    let path = data.strip_prefix('/').unwrap_or(data);
    let qualified = match path.as_bytes() {
        [ns, b'/', ..] if ns.is_ascii() && *ns != b'/' => Some((*ns, &path[2..])),
        [ns] if ns.is_ascii() && *ns != b'/' => Some((*ns, "")),
        _ => None,
    };
    if archive.header().uses_new_namespaces() {
        match archive.entry_by_ns_path(b'C', data) {
            Err(ZimError::EntryNotFound) => {
                let (_, tail) =
                    qualified.ok_or("Search index points to an absent archive entry")?;
                return archive
                    .entry_by_ns_path(b'C', tail)
                    .map_err(|e| e.to_string());
            }
            other => return other.map_err(|e| e.to_string()),
        }
    }
    if let Some((ns, tail)) = qualified {
        match archive.entry_by_ns_path(ns, tail) {
            Err(ZimError::EntryNotFound) => {}
            other => return other.map_err(|e| e.to_string()),
        }
    }
    for ns in *b"AIJ-" {
        match archive.entry_by_ns_path(ns, data) {
            Err(ZimError::EntryNotFound) => {}
            other => return other.map_err(|e| e.to_string()),
        }
    }
    Err("Search index points to an absent archive entry".into())
}

// Interactive pages frequently return the same articles for adjacent queries.
// Keep one 50-hit UI page (at most 3.125 MiB of prefixes) per archive.
#[derive(Default)]
struct PrefixCache(VecDeque<(u32, Box<[u8]>)>);

impl PrefixCache {
    fn read(&mut self, entry: &Entry) -> Result<&[u8], ZimError> {
        let index = entry.index();
        if let Some(i) = self.0.iter().position(|(id, _)| *id == index) {
            self.0.swap(i, 0);
        } else {
            let mut bytes = vec![0; MAX_SNIPPET_INPUT];
            let count = entry.get_item(false)?.read_at(0, &mut bytes)?;
            bytes.truncate(count);
            if self.0.len() == 50 {
                self.0.pop_back();
            }
            self.0.push_front((index, bytes.into_boxed_slice()));
        }
        Ok(&self.0[0].1)
    }
}

fn resolve(mut entry: Entry, cancelled: &dyn Fn() -> bool) -> Result<Entry, String> {
    for _ in 0..256 {
        if !entry.is_redirect() {
            return Ok(entry);
        }
        if cancelled() {
            return Err("Search cancelled".into());
        }
        entry = entry.get_redirect_entry().map_err(|e| e.to_string())?;
    }
    if entry.is_redirect() {
        return Err("Cyclic or excessive archive redirect chain".into());
    }
    Ok(entry)
}

struct ResultPage<'a> {
    rows: SearchResults,
    bytes: usize,
    snippets: bool,
    fulltext: bool,
    query: &'a str,
    prepared: Option<SnippetQuery>,
    prefixes: &'a mut PrefixCache,
}

impl ResultPage<'_> {
    fn add(
        &mut self,
        entry: Entry,
        modern: bool,
        score: f64,
        stored: Option<String>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let title = entry.title().to_owned();
        let entry = resolve(entry, cancelled)?;
        let path = if modern {
            entry.path().to_owned()
        } else {
            format!("{}/{}", char::from(entry.namespace()), entry.path())
        };
        if path.len() > 1 << 20 || title.len() > 1 << 20 {
            return Err("Search result field exceeds 1 MiB".into());
        }
        let snippet = if !self.snippets {
            String::new()
        } else if let Some(text) = stored {
            text
        } else if !self.fulltext {
            escape_text(&title)
        } else {
            // A missing or oversized body still has a valid ranked hit.
            match self.prefixes.read(&entry) {
                Ok(prefix) => {
                    let terms = self
                        .prepared
                        .get_or_insert_with(|| SnippetQuery::new(self.query));
                    escape_text(
                        &terms
                            .excerpt(&String::from_utf8_lossy(prefix), cancelled)
                            .map_err(|e| e.to_string())?,
                    )
                }
                Err(_) => String::new(),
            }
        };
        self.bytes += title.len() + path.len() + snippet.len();
        if self.bytes > MAX_RESULT_BYTES {
            return Err("Search response exceeds 32 MiB limit".into());
        }
        self.rows.hits.push(SearchHit {
            path,
            title,
            snippet,
            score,
        });
        Ok(())
    }
}

impl SearchReader {
    pub(super) fn new(parts: Arc<Parts>) -> Self {
        Self {
            parts,
            cache: Mutex::new(Cache::default()),
        }
    }

    fn lock(&self, cancel: Option<&CancellationToken>) -> Result<MutexGuard<'_, Cache>, String> {
        let Some(token) = cancel else {
            return Ok(self.cache.lock());
        };
        loop {
            if token.is_cancelled() {
                return Err("Search cancelled".into());
            }
            if let Some(guard) = self
                .cache
                .try_lock_for(std::time::Duration::from_millis(10))
            {
                return Ok(guard);
            }
        }
    }

    pub(super) fn capabilities(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> Result<(bool, bool), String> {
        self.parts.unchanged()?;
        let mut cache = self.lock(cancel)?;
        let archive = cache.archive(&self.parts)?;
        let indexes = (
            cache.fulltext.available(&archive, true)?,
            cache.title.available(&archive, false)?,
        );
        self.parts.unchanged()?;
        Ok(indexes)
    }

    pub(super) fn page(
        &self,
        q: &str,
        fulltext: bool,
        offset: u32,
        limit: u32,
        snippets: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<SearchResults, String> {
        let cancelled = || cancel.is_some_and(CancellationToken::is_cancelled);
        let check = || {
            if cancelled() {
                Err("Search cancelled".to_string())
            } else {
                Ok(())
            }
        };
        check()?;
        if q.len() > MAX_QUERY || limit > 1000 || offset.saturating_add(limit) > i32::MAX as u32 {
            return Err("Search exceeds query or pagination limits".into());
        }
        let mut cache = self.lock(cancel)?;
        check()?;
        self.parts.unchanged()?;
        let archive = cache.archive(&self.parts)?;
        let modern = archive.header().uses_new_namespaces();
        // The MIT reader sorts one u64 offset per cluster on the first extent
        // lookup. Bound this search-only allocation while preserving import and
        // article access for archives above the limit.
        if archive.cluster_count() > 8 * 1024 * 1024 {
            return Err(
                "This archive exceeds the search memory limit of 8,388,608 clusters.".into(),
            );
        }
        let Cache {
            title,
            fulltext: text,
            prefixes,
            ..
        } = &mut *cache;
        let slot = if fulltext { text } else { title };
        let mut result = ResultPage {
            rows: SearchResults {
                hits: Vec::new(),
                total: 0,
            },
            bytes: 0,
            snippets,
            fulltext,
            query: q,
            prepared: None,
            prefixes,
        };
        match slot.open(&archive, &self.parts, fulltext)? {
            Some(open) if fulltext => {
                let page = open
                    .index
                    .search_page(
                        q,
                        offset as usize,
                        limit as usize,
                        &open.language,
                        snippets,
                        &cancelled,
                    )
                    .map_err(|e| e.to_string())?;
                result.rows.total = page.total;
                for row in page.hits {
                    check()?;
                    result.add(
                        original_entry(&archive, &row.hit.path)?,
                        modern,
                        f64::from(row.hit.percent),
                        row.snippet,
                        &cancelled,
                    )?;
                }
            }
            Some(open) => {
                let page = open
                    .index
                    .suggest_page(
                        q,
                        offset as usize,
                        limit as usize,
                        &open.language,
                        &cancelled,
                    )
                    .map_err(|e| e.to_string())?;
                result.rows.total = page.total;
                for path in page.hits {
                    check()?;
                    result.add(
                        original_entry(&archive, &path)?,
                        modern,
                        0.0,
                        None,
                        &cancelled,
                    )?;
                }
            }
            None if fulltext => return Err("Archive has no full-text search index".into()),
            None if !q.is_empty() => {
                let range = archive
                    .title_prefix_range(if modern { b'C' } else { b'A' }, q)
                    .map_err(|e| e.to_string())?;
                result.rows.total = u64::from(range.end - range.start);
                for i in range.start.saturating_add(offset)
                    ..range
                        .start
                        .saturating_add(offset)
                        .saturating_add(limit)
                        .min(range.end)
                {
                    check()?;
                    result.add(
                        archive.entry_by_title_index(i).map_err(|e| e.to_string())?,
                        modern,
                        0.0,
                        Some(String::new()),
                        &cancelled,
                    )?;
                }
            }
            None => {}
        }
        check()?;
        self.parts.unchanged()?;
        Ok(result.rows)
    }
}

// Existing consumers accept snippet HTML and extract plain text. Preserve literal
// angle brackets and ampersands from the Rust engine's plain-text excerpts.
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/indexed_search.zim")
    }

    #[test]
    fn redirected_index_uses_the_target_blob_for_capabilities_and_search() {
        let archive = Zim::open(fixture()).unwrap();
        let index = archive.entry_by_ns_path(b'X', "fulltext/xapian").unwrap();
        let target = archive.entry_by_ns_path(b'C', "Venus").unwrap();
        let mut bytes = std::fs::read(fixture()).unwrap();
        let row = |id: u32| {
            let at = archive.header().url_ptr_pos as usize + id as usize * 8;
            u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
        };
        let original = row(index.index());
        let other = row(target.index());
        // Give the other dirent the original index blob and replace the index
        // dirent with a redirect. Its key remains in the same sort position.
        bytes.copy_within(original + 8..original + 16, other + 8);
        bytes[original..original + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        bytes[original + 8..original + 12].copy_from_slice(&target.index().to_le_bytes());
        let key_len = "fulltext/xapian".len() + 1;
        bytes.copy_within(original + 16..original + 16 + key_len, original + 12);
        bytes[original + 12 + key_len] = 0;
        let end = bytes.len() - 16;
        let checksum = <md5::Md5 as md5::Digest>::digest(&bytes[..end]);
        bytes[end..].copy_from_slice(&checksum);
        let path = std::env::temp_dir().join(format!("kiwix-index-{}.zim", uuid::Uuid::new_v4()));
        std::fs::write(&path, &bytes).unwrap();
        let reader = Archive::open(&path).unwrap();
        assert!(reader.search_capabilities().unwrap().0);
        assert_eq!(
            reader
                .search_cancellable("basalt", true, 0, 1, false, &CancellationToken::new())
                .unwrap()
                .total,
            3
        );
        drop(reader);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn queued_query_observes_cancellation_before_the_cache_is_released() {
        let file = File::open(fixture()).unwrap();
        let size = file.metadata().unwrap().len();
        let reader = Arc::new(SearchReader::new(Arc::new(
            Parts::new(vec![(file, size)]).unwrap(),
        )));
        let token = CancellationToken::new();
        let held = reader.cache.lock();
        let (started, waiting) = std::sync::mpsc::channel();
        let (done, result) = std::sync::mpsc::channel();
        let task = {
            let reader = reader.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                started.send(()).unwrap();
                done.send(reader.page("basalt", true, 0, 1, false, Some(&token)))
                    .unwrap();
            })
        };
        waiting.recv().unwrap();
        token.cancel();
        assert_eq!(
            result
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .unwrap_err(),
            "Search cancelled"
        );
        drop(held);
        task.join().unwrap();
    }

    #[test]
    fn native_written_fulltext_index_pages_and_snippets_use_public_paths() {
        let archive = Archive::open(&fixture()).unwrap();
        let token = CancellationToken::new();
        let all = archive
            .search_cancellable("basalt", true, 0, 10, false, &token)
            .unwrap();
        assert_eq!(all.total, 3);
        assert_eq!(all.hits.len(), 3);
        let mut paths: Vec<_> = all.hits.iter().map(|h| h.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["Earth & sky", "Mars%20", "Venus"]);
        for (rank, hit) in all.hits.iter().enumerate() {
            assert!(hit.snippet.is_empty());
            let page = archive.search("basalt", true, rank as u32, 1).unwrap();
            assert_eq!(page.total, all.total);
            assert_eq!(page.hits[0].path, hit.path);
            assert_eq!(page.hits[0].score, hit.score);
            assert!(page.hits[0].snippet.to_lowercase().contains("basalt"));
        }
        assert_eq!(archive.search("basalt", true, 0, 0).unwrap().total, 3);
        assert!(archive
            .search("basalt", true, 3, 1)
            .unwrap()
            .hits
            .is_empty());
    }

    // Windows explicitly leases the file against writers.
    #[cfg(unix)]
    #[test]
    fn warm_index_rejects_a_truncated_file() {
        let path = std::env::temp_dir().join(format!("kiwix-index-{}.zim", uuid::Uuid::new_v4()));
        std::fs::copy(fixture(), &path).unwrap();
        let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let archive = Archive::open(&path).unwrap();
        assert_eq!(archive.search("basalt", true, 0, 1).unwrap().total, 3);
        writer.set_len(64).unwrap();
        assert!(archive
            .search("basalt", true, 0, 1)
            .unwrap_err()
            .contains("changed"));
        drop(archive);
        drop(writer);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn named_open_keeps_its_index_and_checksum_after_a_replacement() {
        let path = std::env::temp_dir().join(format!("kiwix-index-{}.zim", uuid::Uuid::new_v4()));
        std::fs::copy(fixture(), &path).unwrap();
        let archive = Archive::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        assert!(archive.check_integrity().unwrap());
        assert_eq!(archive.search("basalt", true, 0, 1).unwrap().total, 3);
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn search_uses_retained_split_parts_after_the_caller_closes_them() {
        let root = std::env::temp_dir().join(format!("kiwix-split-index-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let bytes = std::fs::read(fixture()).unwrap();
        let source = Zim::open(fixture()).unwrap();
        let extent = IndexSlot::default().extent(&source, true).unwrap().unwrap();
        let inside_index = (extent.start + (extent.end - extent.start) / 2) as usize;
        let cuts = [0, 37, inside_index, bytes.len() - 9, bytes.len()];
        let mut files = Vec::new();
        for (i, cut) in cuts.windows(2).enumerate() {
            let name = format!("held.zima{}", char::from(b'a' + i as u8));
            let path = root.join(&name);
            std::fs::write(&path, &bytes[cut[0]..cut[1]]).unwrap();
            files.push((name, File::open(path).unwrap()));
        }
        let archive = Archive::open_files(&root, &files).unwrap();
        drop(files);
        // Windows leases the names against replacement. POSIX continues reading
        // the exact previously admitted descriptors after all names disappear.
        #[cfg(unix)]
        for entry in std::fs::read_dir(&root).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        assert!(archive.metadata().unwrap().has_fulltext_index);
        assert!(archive.check_integrity().unwrap());
        let rows = archive.search("basalt", true, 0, 3).unwrap();
        assert_eq!(rows.hits.len(), 3);
        assert!(rows
            .hits
            .iter()
            .all(|h| h.snippet.to_lowercase().contains("basalt")));
        assert_eq!(
            archive.search("Mar", false, 0, 1).unwrap().hits[0].path,
            "Mars%20"
        );
    }
}
