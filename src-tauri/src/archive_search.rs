// SPDX-License-Identifier: GPL-3.0-or-later
//! Search the embedded Glass database over the very files admitted to the archive.
#![forbid(unsafe_code)]

use crate::archive::{
    archive_error, entry_by_path, public_path, resolve_cancellable as resolve, SearchHit,
    RankedHit, SearchResults,
};
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;
use zimfile_xapian::{Index, ReaderLimits, SnippetQuery, Source};
use zimru::archive::PositionedInput;
use zimru::{Archive as Zim, Entry, Error as ZimError, Item};

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
    pub(super) fn unchanged(&self) -> Result<(), String> {
        for ((file, len), modified) in self.files.iter().zip(&self.modified) {
            let now = file.metadata().map_err(|e| e.to_string())?;
            if now.len() != *len || now.modified().ok() != *modified {
                return Err("Archive contents changed. Reopen the archive.".into());
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
            entry => resolve(entry.map_err(archive_error)?, &|| false)?
                .get_item(false)
                .map(Some)
                .map_err(archive_error),
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
            .map_err(archive_error)?;
        let mut info = [0; 1];
        if archive.read_at(offset, &mut info).map_err(archive_error)? != 1 {
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
                    .map_err(archive_error)?;
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
                        let item = resolve(result.map_err(archive_error)?, &|| false)?
                            .get_item(false)
                            .map_err(archive_error)?;
                        let mut prefix = [0; 1025];
                        let n = item.read_at(0, &mut prefix).map_err(archive_error)?;
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
    prefixes: PrefixCache,
    title: IndexSlot,
    fulltext: IndexSlot,
}

pub(super) struct SearchReader {
    archive: Zim,
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

// Interactive pages frequently return the same articles for adjacent queries.
// Keep one 50-hit UI page (at most 3.125 MiB of prefixes) per archive.
#[derive(Default)]
struct PrefixCache(VecDeque<(u32, Box<[u8]>)>);

impl PrefixCache {
    fn read(&mut self, entry: &Entry) -> Result<&[u8], ZimError> {
        let index = entry.index();
        if let Some(i) = self.0.iter().position(|(id, _)| *id == index) {
            if i != 0 {
                let hit = self.0.remove(i).expect("Known prefix cache entry");
                self.0.push_front(hit);
            }
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

    fn excerpt(
        &mut self, entry: &Entry, query: &str, prepared: &mut Option<SnippetQuery>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        // A missing or oversized body still has a valid ranked hit.
        let Ok(prefix) = self.read(entry) else { return Ok(String::new()) };
        let terms = prepared.get_or_insert_with(|| SnippetQuery::new(query));
        let text = terms.excerpt(&String::from_utf8_lossy(prefix), cancelled).map_err(|e| e.to_string())?;
        Ok(escape_text(&text))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SnippetMode { Disabled, Immediate, Deferred }

struct ResultPage<'a, T> {
    rows: SearchResults<T>,
    bytes: usize,
    snippets: SnippetMode,
    fulltext: bool,
    query: &'a str,
    prepared: Option<SnippetQuery>,
    prefixes: &'a mut PrefixCache,
    make_row: fn(SearchHit, Option<u32>) -> T,
}

impl<T> ResultPage<'_, T> {
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
        let path = public_path(&entry, modern)?;
        if title.len() > 1 << 20 {
            return Err("Search result field exceeds 1 MiB".into());
        }
        let (snippet, excerpt_entry) = match (self.snippets, stored) {
            (SnippetMode::Disabled, _) => (String::new(), None),
            (_, Some(text)) => (text, None),
            (_, None) if !self.fulltext => (escape_text(&title), None),
            (SnippetMode::Deferred, None) => (String::new(), Some(entry.index())),
            (SnippetMode::Immediate, None) => (
                self.prefixes.excerpt(&entry, self.query, &mut self.prepared, cancelled)?, None,
            ),
        };
        self.bytes += title.len() + path.len() + snippet.len();
        if self.bytes > MAX_RESULT_BYTES {
            return Err("Search response exceeds 32 MiB limit".into());
        }
        self.rows.hits.push((self.make_row)(SearchHit {
            path,
            title,
            snippet,
            score,
        }, excerpt_entry));
        Ok(())
    }
}

impl SearchReader {
    pub(super) fn new(parts: Arc<Parts>, archive: Zim) -> Self {
        Self {
            parts,
            archive,
            cache: Mutex::new(Cache::default()),
        }
    }

    fn lock(&self, cancel: Option<&CancellationToken>) -> Result<MutexGuard<'_, Cache>, String> {
        crate::archive::archive_lock(&self.cache, cancel.map(|token| (token, "Search cancelled")))
    }

    pub(super) fn capabilities(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> Result<(bool, bool), String> {
        self.parts.unchanged()?;
        let cache = self.lock(cancel)?;
        let archive = &self.archive;
        let indexes = (
            cache.fulltext.available(archive, true)?,
            cache.title.available(archive, false)?,
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
        let mode = if snippets { SnippetMode::Immediate } else { SnippetMode::Disabled };
        self.page_with(q, fulltext, offset, limit, mode, cancel, |hit, _| hit)
    }

    pub(super) fn ranked(
        &self, q: &str, fulltext: bool, offset: u32, limit: u32, snippets: bool, cancel: &CancellationToken,
    ) -> Result<SearchResults<RankedHit>, String> {
        let mode = if snippets { SnippetMode::Deferred } else { SnippetMode::Disabled };
        self.page_with(q, fulltext, offset, limit, mode, Some(cancel), |hit, excerpt_entry| RankedHit { hit, excerpt_entry })
    }

    #[allow(clippy::too_many_arguments)]
    fn page_with<T>(
        &self, q: &str, fulltext: bool, offset: u32, limit: u32, snippets: SnippetMode,
        cancel: Option<&CancellationToken>, make_row: fn(SearchHit, Option<u32>) -> T,
    ) -> Result<SearchResults<T>, String> {
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
        let archive = &self.archive;
        let modern = archive.header().uses_new_namespaces();
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
            make_row,
        };
        match slot.open(archive, &self.parts, fulltext)? {
            Some(open) if fulltext => {
                let page = open
                    .index
                    .search_page(
                        q,
                        offset as usize,
                        limit as usize,
                        &open.language,
                        snippets != SnippetMode::Disabled,
                        &cancelled,
                    )
                    .map_err(|e| e.to_string())?;
                result.rows.total = page.total;
                for row in page.hits {
                    check()?;
                    result.add(
                        entry_by_path(archive, &row.hit.path)?,
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
                        entry_by_path(archive, &path)?,
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
                    .map_err(archive_error)?;
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
                        archive.entry_by_title_index(i).map_err(archive_error)?,
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

    pub(super) fn excerpts(
        &self, q: &str, rows: &mut [&mut RankedHit], cancel: &CancellationToken,
    ) -> Result<(), String> {
        let mut cache = self.lock(Some(cancel))?;
        self.parts.unchanged()?;
        let cancelled = || cancel.is_cancelled();
        let mut prepared = None;
        let mut bytes = 0;
        for row in rows {
            crate::search::check_cancel(cancel)?;
            if let Some(index) = row.excerpt_entry {
                let entry = self.archive.entry_by_url_index(index).map_err(archive_error)?;
                row.hit.snippet = cache.prefixes.excerpt(&entry, q, &mut prepared, &cancelled)?;
            }
            bytes += row.hit.path.len() + row.hit.title.len() + row.hit.snippet.len();
            if bytes > MAX_RESULT_BYTES {
                return Err("Search response exceeds 32 MiB limit".into());
            }
        }
        crate::search::check_cancel(cancel)?;
        self.parts.unchanged()
    }

    #[cfg(test)]
    pub(super) fn cached_prefixes(&self) -> usize {
        self.cache.lock().unwrap().prefixes.0.len()
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
    fn reading_an_old_prefix_keeps_the_previous_hottest_prefix() {
        let archive = Zim::open(fixture()).unwrap();
        let hot = archive.entry_by_ns_path(b'C', "Venus").unwrap();
        let old = archive.entry_by_ns_path(b'C', "Earth & sky").unwrap();
        let new = archive.entry_by_ns_path(b'C', "Mars%20").unwrap();
        let mut cache = PrefixCache::default();
        cache.0.push_back((hot.index(), Box::from(&b"hot"[..])));
        for id in u32::MAX - 47..=u32::MAX {
            cache.0.push_back((id, Box::default()));
        }
        cache.0.push_back((old.index(), Box::from(&b"old"[..])));

        assert_eq!(cache.read(&old).unwrap(), b"old");
        cache.read(&new).unwrap();
        assert_eq!(cache.0.len(), 50);
        assert_eq!(cache.0[0].0, new.index());
        assert_eq!(cache.0[1].0, old.index());
        assert_eq!(cache.0[2].0, hot.index());
        assert!(cache.0.iter().all(|(id, _)| *id != u32::MAX));
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
        let checksum = md5::compute(&bytes[..end]).0;
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
        let parts = Arc::new(Parts::new(vec![(file, size)]).unwrap());
        let archive = crate::archive::open_positioned(&parts).unwrap();
        let reader = Arc::new(SearchReader::new(parts, archive));
        let token = CancellationToken::new();
        let held = reader.cache.lock().unwrap();
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

    #[test]
    fn merged_pages_only_read_visible_fallback_prefixes() {
        use crate::search::{run, Mode};
        let cancel = CancellationToken::new();
        for mode in [Mode::Fulltext, Mode::Combined] {
            let first = Archive::open(&fixture()).unwrap();
            let second = Archive::open(&fixture()).unwrap();
            let archives = || vec![("first".into(), Ok(first.clone())), ("second".into(), Ok(second.clone()))];
            let hidden = run(archives(), "basalt", mode, 1, 1, false, &cancel).unwrap();
            assert_eq!(hidden["hits"][0]["snippet"], "");
            assert_eq!((first.cached_search_prefixes(), second.cached_search_prefixes()), (0, 0));

            let visible = run(archives(), "basalt", mode, 1, 1, true, &cancel).unwrap();
            assert_eq!(visible["hits"][0]["path"], hidden["hits"][0]["path"]);
            assert_eq!(visible["hits"][0]["bookId"], "second");
            assert!(visible["hits"][0]["snippet"].as_str().unwrap().to_lowercase().contains("basalt"));
            assert_eq!((first.cached_search_prefixes(), second.cached_search_prefixes()), (0, 1));
        }
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
