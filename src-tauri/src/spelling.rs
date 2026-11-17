// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Explicit, cancellable spelling dictionaries built from public libzim titles.
//! Existing caches load single-flight in read-only workers; `building` covers
//! loading as well as explicit indexing. Interactive calls never scan a cache.
//! No automatic builds. Disabling writes joins indexing before privacy changes;
//! read-only loaders may finish without creating or modifying filesystem state.

use crate::archive::{Archive, ArchiveMetadata, TitleBatch};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use unicode_normalization::UnicodeNormalization;

const VERSION: u32 = 1;
const MAX_TITLES: u32 = 20_000_000;
const MAX_WORDS: u64 = 2_000_000;
const MAX_DB_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_WORD_CHARS: usize = 64;
const MAX_QUERY_BYTES: usize = 512;
const MAX_QUERY_WORDS: usize = 8;
const MAX_CANDIDATES: usize = 8192;
const BATCH: u32 = 256;
// Bounded retained state: an open dictionary owns a SQLite connection (and its
// descriptors) and a cached status is only advisory, so a library larger than
// these limits must not pin one of each per archive for the process lifetime.
const MAX_DICTIONARIES: usize = 8;
const MAX_STATUSES: usize = 256;
const MAX_FAILURES: usize = 64;
// `deletion_keys` yields the word itself plus one key per removed scalar.
const MAX_DELETION_KEYS: usize = MAX_WORD_CHARS + 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellingStatus {
    pub book_id: String,
    pub state: String,
    #[serde(default)]
    pub cancellable: bool,
    pub processed: u32,
    pub total: u32,
    pub words: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpellingCorrection {
    pub original_query: String,
    pub corrected_query: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    version: u32,
    uuid: String,
    archive_bytes: u64,
    articles: u64,
}

impl Identity {
    fn from_metadata(metadata: ArchiveMetadata) -> Self {
        Self {
            version: VERSION,
            uuid: metadata.id,
            archive_bytes: metadata.size,
            articles: metadata.article_count,
        }
    }

    fn key(&self) -> String {
        let data = format!(
            "{}:{}:{}:{}",
            self.version, self.uuid, self.archive_bytes, self.articles
        );
        hex::encode(Sha256::digest(data.as_bytes()))
    }

    fn status(&self, state: &str) -> SpellingStatus {
        SpellingStatus {
            book_id: self.uuid.clone(),
            state: state.into(),
            processed: 0,
            total: 0,
            words: 0,
            error: None,
            cancellable: false,
        }
    }
}

struct Dictionary {
    connection: Connection,
    total: u32,
    words: u64,
}

struct Job {
    book_id: String,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct Inner {
    writable: bool,
    job: Option<Job>,
    // Loaders only read existing files. Retaining their handles owns their
    // lifetime without making an interactive request or privacy cutover join.
    // A loader removes its own entry when it publishes.
    loads: HashMap<String, JoinHandle<()>>,
    // Every archive ever consulted must not retain a SQLite handle (and its
    // descriptors) for the process lifetime, so dictionaries, statuses and
    // failures are bounded and evicted oldest-first. In-flight loaders and the
    // active build keep their status entry; dropping an evicted dictionary
    // closes its connection once the last query releases it.
    statuses: HashMap<String, SpellingStatus>,
    dictionaries: HashMap<String, Arc<Mutex<Dictionary>>>,
    failures: HashMap<String, String>,
    dictionary_order: VecDeque<String>,
    status_order: VecDeque<String>,
    failure_order: VecDeque<String>,
}

impl Inner {
    fn insert_dictionary(&mut self, key: String, dictionary: Arc<Mutex<Dictionary>>) {
        self.dictionaries.insert(key.clone(), dictionary);
        remember(&mut self.dictionary_order, key);
        while self.dictionaries.len() > MAX_DICTIONARIES {
            let Some(key) = self.dictionary_order.pop_front() else { break };
            self.dictionaries.remove(&key);
        }
    }

    /// Mark an eviction-protected dictionary as recently used.
    fn touch_dictionary(&mut self, key: &str) {
        if let Some(position) = self.dictionary_order.iter().position(|entry| entry.as_str() == key) {
            if let Some(key) = self.dictionary_order.remove(position) {
                self.dictionary_order.push_back(key);
            }
        }
    }

    fn insert_status(&mut self, key: String, status: SpellingStatus) {
        self.statuses.insert(key.clone(), status);
        remember(&mut self.status_order, key);
        while self.statuses.len() > MAX_STATUSES {
            let Some(position) = self.status_order.iter().position(|key| self.status_evictable(key))
            else { break };
            let Some(key) = self.status_order.remove(position) else { break };
            self.statuses.remove(&key);
        }
    }

    /// A loader still publishing, or the active build, must keep reporting.
    fn status_evictable(&self, key: &str) -> bool {
        if self.loads.get(key).is_some_and(|handle| !handle.is_finished()) {
            return false;
        }
        !self.job.as_ref().is_some_and(|job| {
            !job.handle.is_finished()
                && self.statuses.get(key).is_some_and(|status| status.book_id == job.book_id)
        })
    }

    fn insert_failure(&mut self, key: String, error: String) {
        self.failures.insert(key.clone(), error);
        remember(&mut self.failure_order, key);
        while self.failures.len() > MAX_FAILURES {
            let Some(key) = self.failure_order.pop_front() else { break };
            self.failures.remove(&key);
        }
    }
}

/// Record `key` as the newest entry of an insertion-ordered cache. Eviction
/// removes the matching map entry and order slot together, so entries that are
/// still protected never lose their place.
fn remember(order: &mut VecDeque<String>, key: String) {
    order.retain(|entry| entry != &key);
    order.push_back(key);
}

pub struct SpellingManager {
    directory: PathBuf,
    inner: Arc<Mutex<Inner>>,
    // Serializes build/cancel/policy changes, including their joins. Workers do
    // not acquire this lock, so joining cannot deadlock their final publication.
    lifecycle: Mutex<()>,
}

impl SpellingManager {
    /// Pure construction: neither creates a directory nor touches a cache.
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            inner: Arc::new(Mutex::new(Inner::default())),
            lifecycle: Mutex::new(()),
        }
    }

    pub fn set_writable(&self, writable: bool) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let job = {
            let mut inner = self.inner.lock().map_err(err)?;
            inner.writable = writable;
            if writable {
                None
            } else {
                inner.job.take()
            }
        };
        if let Some(job) = job {
            job.cancel.store(true, Ordering::Release);
            job.handle
                .join()
                .map_err(|_| "Spelling worker panicked".to_string())?;
        }
        Ok(())
    }

    fn dictionary(&self, identity: &Identity) -> Result<Option<Arc<Mutex<Dictionary>>>, String> {
        self.dictionary_with_loader(identity, open_verified)
    }

    fn dictionary_with_loader<F>(
        &self,
        identity: &Identity,
        load: F,
    ) -> Result<Option<Arc<Mutex<Dictionary>>>, String>
    where
        F: FnOnce(&Path, &Identity) -> Result<Dictionary, String> + Send + 'static,
    {
        let key = identity.key();
        let mut inner = self.inner.lock().map_err(err)?;
        if let Some(dictionary) = inner.dictionaries.get(&key).cloned() {
            // Keep the dictionaries in active use from being evicted first.
            inner.touch_dictionary(&key);
            return Ok(Some(dictionary));
        }
        if let Some(error) = inner.failures.get(&key) {
            return Err(error.clone());
        }
        if inner
            .loads
            .get(&key)
            .is_some_and(|handle| !handle.is_finished())
        {
            return Ok(None);
        }
        let path = self.directory.join(format!("{key}.sqlite"));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
            Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_DB_BYTES => {
                return Err("Invalid or oversized spelling cache file".into());
            }
            Ok(_) => {}
        }
        // The lock covers both reservation and handle insertion. Concurrent
        // cold requests return immediately instead of validating the same file.
        let identity = identity.clone();
        let shared = self.inner.clone();
        let worker_key = key.clone();
        let building = identity.status("building");
        let handle = std::thread::Builder::new()
            .name("spelling-cache".into())
            .spawn(move || {
                // Integrity checks and row counts can scan the entire dictionary;
                // they must never execute while holding the interactive state lock.
                let result = load(&path, &identity);
                if let Ok(mut inner) = shared.lock() {
                    let mut status = identity.status("ready");
                    match result {
                        Ok(dictionary) => {
                            status.processed = dictionary.total;
                            status.total = dictionary.total;
                            status.words = dictionary.words;
                            inner.insert_dictionary(
                                worker_key.clone(), Arc::new(Mutex::new(dictionary)));
                        }
                        Err(error) => {
                            status.state = "error".into();
                            status.error = Some(error.clone());
                            inner.insert_failure(worker_key.clone(), error);
                        }
                    }
                    inner.insert_status(worker_key.clone(), status);
                    // Only in-flight loaders need to be retained; dropping the
                    // finished handle detaches nothing that is still running.
                    inner.loads.remove(&worker_key);
                }
            })
            .map_err(|error| error.to_string())?;
        inner.loads.insert(key.clone(), handle);
        // Reserve the loader status only once its handle is retained: eviction
        // never drops the status of an in-flight load.
        inner.insert_status(key, building);
        Ok(None)
    }

    pub fn status(&self, archive: &Archive) -> Result<SpellingStatus, String> {
        self.status_for_identity(&Identity::from_metadata(archive.metadata()?))
    }

    fn status_for_identity(&self, identity: &Identity) -> Result<SpellingStatus, String> {
        let key = identity.key();
        if let Some(status) = self.inner.lock().map_err(err)?.statuses.get(&key) {
            return Ok(status.clone());
        }
        match self.dictionary(identity) {
            Ok(Some(dictionary)) => {
                let dictionary = dictionary.lock().map_err(err)?;
                let mut status = identity.status("ready");
                status.processed = dictionary.total;
                status.total = dictionary.total;
                status.words = dictionary.words;
                Ok(status)
            }
            Ok(None) => Ok(self
                .inner
                .lock()
                .map_err(err)?
                .statuses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| identity.status("missing"))),
            Err(error) => {
                let mut status = identity.status("error");
                status.error = Some(error);
                Ok(status)
            }
        }
    }

    /// Only an explicit trusted application command may call this method.
    pub fn build(self: &Arc<Self>, archive: Archive) -> Result<SpellingStatus, String> {
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let identity = Identity::from_metadata(archive.metadata()?);
        let key = identity.key();
        let inner = self.inner.lock().map_err(err)?;
        if !inner.writable {
            return Err("Spelling builds are disabled in private or read-only mode".into());
        }
        if let Some(job) = inner.job.as_ref() {
            if !job.handle.is_finished() {
                if job.book_id == identity.uuid {
                    return inner
                        .statuses
                        .get(&key)
                        .cloned()
                        .ok_or_else(|| "Spelling job identity changed".into());
                }
                return Err("Another spelling dictionary is already building".into());
            }
        }
        drop(inner);
        if let Ok(Some(dictionary)) = self.dictionary(&identity) {
            let dictionary = dictionary.lock().map_err(err)?;
            let mut status = identity.status("ready");
            status.processed = dictionary.total;
            status.total = dictionary.total;
            status.words = dictionary.words;
            self.inner
                .lock()
                .map_err(err)?
                .insert_status(key, status.clone());
            return Ok(status);
        }
        let mut inner = self.inner.lock().map_err(err)?;
        if inner.dictionaries.contains_key(&key)
            || (inner.loads.contains_key(&key)
                && inner
                    .statuses
                    .get(&key)
                    .is_some_and(|status| status.state == "building"))
        {
            return inner
                .statuses
                .get(&key)
                .cloned()
                .ok_or_else(|| "Spelling load identity changed".into());
        }
        if let Some(job) = inner.job.take() {
            job.handle
                .join()
                .map_err(|_| "Spelling worker panicked".to_string())?;
        }
        let mut status = identity.status("building");
        status.cancellable = true;
        inner.insert_status(key.clone(), status.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let directory = self.directory.clone();
        let shared = self.inner.clone();
        let book_id = identity.uuid.clone();
        let worker_key = key.clone();
        let handle = std::thread::Builder::new()
            .name("spelling-index".into())
            .spawn(move || {
                let result =
                    build_archive(&archive, &directory, &identity, &worker_cancel, &shared);
                if let Ok(mut inner) = shared.lock() {
                    if let Some(status) = inner.statuses.get_mut(&worker_key) {
                        status.cancellable = false;
                        if let Err(error) = result {
                            status.state = if worker_cancel.load(Ordering::Acquire) {
                                "cancelled"
                            } else {
                                "error"
                            }
                            .into();
                            status.error = if status.state == "cancelled" {
                                None
                            } else {
                                Some(error)
                            };
                        }
                    }
                }
            })
            .map_err(|error| {
                inner.statuses.remove(&key);
                error.to_string()
            })?;
        inner.job = Some(Job {
            book_id,
            cancel,
            handle,
        });
        Ok(status)
    }

    /// Cancel and join whichever build is running, if any. Windows library
    /// moves use this because the build holds its archive open.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn cancel_running(&self) -> Result<(), String> {
        // Same order as `cancel` and `start`: lifecycle, then inner. Holding
        // lifecycle means no build can start between the take and the join.
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let job = self.inner.lock().map_err(err)?.job.take();
        if let Some(job) = job {
            job.cancel.store(true, Ordering::Release);
            job.handle
                .join()
                .map_err(|_| "Spelling worker panicked".to_string())?;
        }
        Ok(())
    }

    pub fn cancel(&self, book_id: &str) -> Result<SpellingStatus, String> {
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let job = {
            let mut inner = self.inner.lock().map_err(err)?;
            if inner.job.as_ref().is_some_and(|job| job.book_id == book_id) {
                inner.job.take()
            } else {
                if inner.statuses.values().any(|status| {
                    status.book_id == book_id && status.state == "building" && !status.cancellable
                }) {
                    return Err("Spelling cache loading cannot be cancelled".into());
                }
                None
            }
        };
        if let Some(job) = job {
            job.cancel.store(true, Ordering::Release);
            job.handle
                .join()
                .map_err(|_| "Spelling worker panicked".to_string())?;
        }
        self.inner
            .lock()
            .map_err(err)?
            .statuses
            .values()
            .find(|status| status.book_id == book_id)
            .cloned()
            .ok_or_else(|| "No spelling job exists for this archive".into())
    }

    pub fn corrections(
        &self,
        archive: &Archive,
        query: &str,
        ordinary_total: u64,
        enabled: bool,
    ) -> Result<Vec<SpellingCorrection>, String> {
        if !eligible_query(query, ordinary_total, enabled) {
            return Ok(Vec::new());
        }
        let identity = Identity::from_metadata(archive.metadata()?);
        let Some(dictionary) = self.dictionary(&identity)? else {
            return Ok(Vec::new());
        };
        let dictionary = dictionary.lock().map_err(err)?;
        query_corrections(&dictionary.connection, query)
    }
}

impl Drop for SpellingManager {
    fn drop(&mut self) {
        // Read-only loaders retain only shared state and their file connection,
        // never the manager. They may finish after drop without publishing files.
        if let Ok(mut inner) = self.inner.lock() {
            inner.writable = false;
            if let Some(job) = inner.job.take() {
                job.cancel.store(true, Ordering::Release);
                drop(inner);
                let _ = job.handle.join();
            }
        }
    }
}

fn eligible_query(query: &str, ordinary_total: u64, enabled: bool) -> bool {
    enabled && ordinary_total == 0 && query.len() <= MAX_QUERY_BYTES && query.chars().count() > 2
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Spelling build cancelled".into())
    } else {
        Ok(())
    }
}

fn normalize(word: &str) -> String {
    let word: String = word.nfkc().collect();
    crate::casefold::fold(&word).nfkc().collect()
}

fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        let is_word = character.is_alphanumeric()
            || (start.is_some() && unicode_normalization::char::is_combining_mark(character));
        if is_word {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            words.push((begin, index));
        }
    }
    if let Some(begin) = start {
        words.push((begin, text.len()));
    }
    words
}

fn valid_word(word: &str) -> bool {
    word.len() <= 256
        && (2..=MAX_WORD_CHARS).contains(&word.chars().count())
        && word.chars().any(char::is_alphabetic)
}

fn deletion_keys(word: &str) -> BTreeSet<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut keys = BTreeSet::new();
    keys.insert(word.to_string());
    for removed in 0..chars.len() {
        keys.insert(
            chars
                .iter()
                .enumerate()
                .filter_map(|(index, ch)| (index != removed).then_some(*ch))
                .collect(),
        );
    }
    keys
}

fn create_schema(connection: &Connection) -> Result<(), String> {
    connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
        PRAGMA page_size=4096; PRAGMA max_page_count=262144; PRAGMA trusted_schema=OFF;
        CREATE TABLE words(word TEXT PRIMARY KEY, length INTEGER NOT NULL, frequency INTEGER NOT NULL) WITHOUT ROWID;
        CREATE INDEX word_lengths ON words(length,word);
        CREATE TABLE deletes(key TEXT NOT NULL, word TEXT NOT NULL, PRIMARY KEY(key,word)) WITHOUT ROWID;
        CREATE TABLE identity(value TEXT NOT NULL, total INTEGER NOT NULL, words INTEGER NOT NULL);")
        .map_err(err)
}

fn add_titles(
    connection: &mut Connection,
    titles: &[String],
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let transaction = connection.transaction().map_err(err)?;
    let mut added = 0;
    {
        let mut insert = transaction.prepare_cached("INSERT INTO words VALUES(?1,?2,1) ON CONFLICT(word) DO UPDATE SET frequency=frequency+1").map_err(err)?;
        let mut exists = transaction
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM words WHERE word=?1)")
            .map_err(err)?;
        let mut delete = transaction
            .prepare_cached("INSERT OR IGNORE INTO deletes VALUES(?1,?2)")
            .map_err(err)?;
        for title in titles {
            check_cancel(cancel)?;
            for (start, end) in tokens(title) {
                let word = normalize(&title[start..end]);
                if !valid_word(&word) {
                    continue;
                }
                let present: bool = exists.query_row([&word], |row| row.get(0)).map_err(err)?;
                insert
                    .execute(params![word, word.chars().count() as u32])
                    .map_err(err)?;
                if !present {
                    added += 1;
                    for key in deletion_keys(&word) {
                        delete.execute(params![key, word]).map_err(err)?;
                    }
                }
            }
        }
    }
    transaction.commit().map_err(err)?;
    Ok(added)
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn build_archive(
    archive: &Archive,
    directory: &Path,
    identity: &Identity,
    cancel: &AtomicBool,
    shared: &Mutex<Inner>,
) -> Result<(), String> {
    build_with_batches(directory, identity, cancel, shared, |offset| {
        archive.title_batch(offset, BATCH)
    })
}

fn build_with_batches<F>(
    directory: &Path,
    identity: &Identity,
    cancel: &AtomicBool,
    shared: &Mutex<Inner>,
    mut batch: F,
) -> Result<(), String>
where
    F: FnMut(u32) -> Result<TitleBatch, String>,
{
    check_cancel(cancel)?;
    if !shared.lock().map_err(err)?.writable {
        return Err("Spelling builds are disabled in private or read-only mode".into());
    }
    fs::create_dir_all(directory).map_err(err)?;
    let temporary =
        Temporary(directory.join(format!(".{}.{}.tmp", identity.key(), uuid::Uuid::new_v4())));
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary.0)
        .map_err(err)?;
    let mut connection =
        Connection::open_with_flags(&temporary.0, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(err)?;
    create_schema(&connection)?;
    let mut processed: u32 = 0;
    let mut words: u64 = 0;
    let mut total = None;
    loop {
        check_cancel(cancel)?;
        let page = batch(processed)?;
        if page.total > MAX_TITLES
            || page.titles.len() > BATCH as usize
            || page.titles.iter().any(|title| title.len() > 16 * 1024)
            || total.is_some_and(|expected| expected != page.total)
            || processed.saturating_add(page.titles.len() as u32) > page.total
            || (page.titles.is_empty() && processed < page.total)
        {
            return Err("Invalid, changing, or oversized spelling title listing (maximum 20 million titles)".into());
        }
        total = Some(page.total);
        words += add_titles(&mut connection, &page.titles, cancel)?;
        processed += page.titles.len() as u32;
        if words > MAX_WORDS {
            return Err("Spelling dictionary exceeds 2 million distinct words".into());
        }
        if let Some(status) = shared
            .lock()
            .map_err(err)?
            .statuses
            .get_mut(&identity.key())
        {
            status.processed = processed;
            status.total = page.total;
            status.words = words;
        }
        if processed == page.total {
            connection
                .execute(
                    "INSERT INTO identity VALUES(?1,?2,?3)",
                    params![
                        serde_json::to_string(identity).map_err(err)?,
                        processed,
                        words
                    ],
                )
                .map_err(err)?;
            break;
        }
    }
    check_cancel(cancel)?;
    connection.close().map_err(|(_, error)| error.to_string())?;
    let verified = open_verified(&temporary.0, identity)?;
    let total = verified.total;
    let words = verified.words;
    // SQLite's Windows VFS does not promise FILE_SHARE_DELETE. Close the
    // verifier before moving the file; reopen the same verified bytes after it.
    drop(verified);
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary.0)
        .and_then(|file| file.sync_all())
        .map_err(err)?;
    // Publication and policy changes share a lock. set_writable(false) waits for
    // this section and joins before returning, so private mode cannot publish.
    let mut inner = shared.lock().map_err(err)?;
    check_cancel(cancel)?;
    if !inner.writable {
        return Err("Spelling cache publication disabled by privacy policy".into());
    }
    let key = identity.key();
    let final_path = directory.join(format!("{key}.sqlite"));
    fs::rename(&temporary.0, &final_path).map_err(err)?;
    let dictionary = Dictionary {
        connection: open_readonly(&final_path)?,
        total,
        words,
    };
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(err)?;
    let mut status = identity.status("ready");
    status.processed = dictionary.total;
    status.total = dictionary.total;
    status.words = dictionary.words;
    inner.insert_dictionary(key.clone(), Arc::new(Mutex::new(dictionary)));
    inner.failures.remove(&key);
    inner.insert_status(key, status);
    Ok(())
}

fn open_readonly(path: &Path) -> Result<Connection, String> {
    // Published dictionaries are immutable snapshots. READ_ONLY alone can still
    // create WAL shared-memory files in a writable directory; immutable also
    // prevents those side effects during private/read-only background loading.
    let absolute;
    let path = if path.is_absolute() {
        path
    } else {
        absolute = std::env::current_dir().map_err(err)?.join(path);
        &absolute
    };
    let mut uri =
        url::Url::from_file_path(path).map_err(|_| "Invalid spelling cache path".to_string())?;
    uri.query_pairs_mut().append_pair("immutable", "1");
    let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(err)?;
    connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA cache_size=-4096; PRAGMA temp_store=MEMORY;").map_err(err)?;
    Ok(connection)
}

fn open_verified(path: &Path, identity: &Identity) -> Result<Dictionary, String> {
    let metadata = fs::symlink_metadata(path).map_err(err)?;
    if !metadata.is_file() || metadata.len() > MAX_DB_BYTES {
        return Err("Invalid or oversized spelling cache".into());
    }
    let connection = open_readonly(path)?;
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(err)?;
    if check != "ok" {
        return Err("Spelling cache integrity verification failed".into());
    }
    let schema_valid: bool = connection.query_row("SELECT
        (SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('words','deletes','identity'))=3
        AND (SELECT group_concat(name,',') FROM (SELECT name FROM pragma_index_info('word_lengths') ORDER BY seqno))='length,word'
        AND EXISTS(SELECT 1 FROM pragma_table_info('words') WHERE name='word' AND pk=1)
        AND EXISTS(SELECT 1 FROM pragma_table_info('deletes') WHERE name='key' AND pk=1)
        AND EXISTS(SELECT 1 FROM pragma_table_info('deletes') WHERE name='word' AND pk=2)",
        [], |row| row.get(0)).map_err(err)?;
    if !schema_valid {
        return Err("The spelling dictionary is damaged. Rebuild it in Settings".into());
    }
    let (stored, total, words): (String, u32, u64) = connection
        .query_row(
            "SELECT substr(value,1,4097),total,words FROM identity",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(err)?;
    if stored.len() > 4096
        || serde_json::from_str::<Identity>(&stored).map_err(err)? != *identity
        || total > MAX_TITLES
        || words > MAX_WORDS
    {
        return Err("Spelling cache archive identity/version or bounds mismatch".into());
    }
    let actual: u64 = connection
        .query_row("SELECT count(*) FROM words", [], |row| row.get(0))
        .map_err(err)?;
    if actual != words {
        return Err("Incomplete spelling cache dictionary".into());
    }
    Ok(Dictionary {
        connection,
        total,
        words,
    })
}

/// Optimal-string-alignment distance <= 1: insert/delete/substitute or transpose
/// two adjacent Unicode scalars. No byte-distance corruption for non-ASCII text.
fn distance_one(left: &str, right: &str) -> bool {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    let mut index = 0;
    while index < left.len().min(right.len()) && left[index] == right[index] {
        index += 1;
    }
    if index == left.len().min(right.len()) {
        return left != right;
    }
    if left.len() == right.len() {
        left[index + 1..] == right[index + 1..]
            || (index + 1 < left.len()
                && left[index] == right[index + 1]
                && left[index + 1] == right[index]
                && left[index + 2..] == right[index + 2..])
    } else if left.len() > right.len() {
        left[index + 1..] == right[index..]
    } else {
        left[index..] == right[index + 1..]
    }
}

fn candidates(connection: &Connection, word: &str) -> Result<Vec<(String, u64)>, String> {
    // The statement cache is keyed by SQL text, so building the IN list from the
    // deletion-key count re-prepares on nearly every keystroke. Bind a fixed
    // 65-slot list instead: unused slots bind the empty string, which is never a
    // deletion key (valid words hold 2..=64 scalars), and the length bounds are
    // bound too. The final ordering is applied below, so the query plan's row
    // order stays irrelevant.
    static SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let placeholders = std::iter::repeat_n("?", MAX_DELETION_KEYS)
            .collect::<Vec<_>>()
            .join(",");
        // No SQL LIMIT: the deletion-key join returns a distance-two superset, so
        // capping rows before the distance filter would fail (or drop real
        // candidates) for a word with many non-adjacent matches. The join itself
        // is tight — each key of length k only matches words of length k or k+1 —
        // and the filtered cap is applied below.
        format!("SELECT DISTINCT substr(w.word,1,257),w.frequency FROM deletes d JOIN words w ON w.word=d.word
        WHERE d.key IN ({placeholders}) AND w.length BETWEEN ? AND ?")
    });
    let keys = deletion_keys(word);
    if keys.len() > MAX_DELETION_KEYS {
        return Err("Invalid spelling query length".into());
    }
    let length = word.chars().count() as i64;
    let mut bound: Vec<SqlValue> = keys.into_iter().map(SqlValue::Text).collect();
    bound.resize(MAX_DELETION_KEYS, SqlValue::Text(String::new()));
    bound.push(SqlValue::Integer(length.saturating_sub(1)));
    bound.push(SqlValue::Integer(length + 1));
    let mut statement = connection.prepare_cached(&SQL).map_err(err)?;
    let rows = statement
        .query_map(params_from_iter(bound.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(err)?;
    let mut result = Vec::new();
    for row in rows {
        let (candidate, frequency) = row.map_err(err)?;
        if candidate.len() > 256 || !valid_word(&candidate) {
            return Err("Invalid spelling dictionary word".into());
        }
        if !distance_one(word, &candidate) {
            continue;
        }
        // The cap counts adjacent candidates only: a large superset of distant
        // words is not a failure.
        if result.len() >= MAX_CANDIDATES {
            return Err("Spelling candidate limit exceeded".into());
        }
        result.push((candidate, frequency));
    }
    // Deterministic result order: most frequent first, then by word.
    result.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    result.truncate(5);
    Ok(result)
}

fn match_case(original: &str, replacement: &str) -> String {
    if original
        .chars()
        .filter(|ch| ch.is_alphabetic())
        .all(|ch| ch.is_uppercase())
    {
        replacement.to_uppercase()
    } else if original.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = replacement.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    } else {
        replacement.to_string()
    }
}

fn query_corrections(
    connection: &Connection,
    query: &str,
) -> Result<Vec<SpellingCorrection>, String> {
    if query.len() > MAX_QUERY_BYTES || query.chars().count() <= 2 {
        return Ok(Vec::new());
    }
    let words = tokens(query);
    if words.is_empty() || words.len() > MAX_QUERY_WORDS {
        return Ok(Vec::new());
    }
    // At most two corrected words per query. Beam width five bounds Cartesian
    // growth while still correcting common multiword queries with two typos.
    let mut beam = vec![(String::new(), 0_u64)];
    let mut cursor = 0;
    let mut changed = 0;
    for (start, end) in words {
        let original = &query[start..end];
        let word = normalize(original);
        if word.len() > 256 || word.chars().count() > MAX_WORD_CHARS {
            return Ok(Vec::new());
        }
        let known: bool = !valid_word(&word)
            || connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM words WHERE word=?1)",
                    [&word],
                    |row| row.get(0),
                )
                .map_err(err)?;
        let replacements = if known {
            vec![(original.to_string(), 0)]
        } else {
            changed += 1;
            if changed > 2 {
                return Ok(Vec::new());
            }
            candidates(connection, &word)?
                .into_iter()
                .map(|(word, score)| (match_case(original, &word), score))
                .collect()
        };
        if replacements.is_empty() {
            return Ok(Vec::new());
        }
        let mut next = Vec::new();
        for (prefix, score) in &beam {
            for (replacement, frequency) in &replacements {
                next.push((
                    format!("{}{}{}", prefix, &query[cursor..start], replacement),
                    score.saturating_add(*frequency),
                ));
            }
        }
        next.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        next.dedup_by(|a, b| a.0 == b.0);
        next.truncate(5);
        beam = next;
        cursor = end;
    }
    if changed == 0 {
        return Ok(Vec::new());
    }
    Ok(beam
        .into_iter()
        .filter_map(|(mut corrected, _)| {
            corrected.push_str(&query[cursor..]);
            (corrected != query).then(|| SpellingCorrection {
                original_query: query.to_string(),
                corrected_query: corrected,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("kiwix-spelling-{}", uuid::Uuid::new_v4())))
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn identity() -> Identity {
        Identity {
            version: VERSION,
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            archive_bytes: 12345,
            articles: 257,
        }
    }

    fn writable(identity: &Identity) -> Mutex<Inner> {
        let mut inner = Inner {
            writable: true,
            ..Inner::default()
        };
        inner
            .statuses
            .insert(identity.key(), identity.status("building"));
        Mutex::new(inner)
    }

    fn finish_load(manager: &SpellingManager, identity: &Identity) {
        let handle = manager
            .inner
            .lock()
            .unwrap()
            .loads
            .remove(&identity.key())
            .unwrap();
        handle.join().unwrap();
    }

    fn fixture(connection: &mut Connection) {
        create_schema(connection).unwrap();
        let mut titles = vec!["Planet Earth".to_string(); 12];
        titles.extend(
            [
                "Planer",
                "Plant",
                "Café",
                "Straße",
                "κόσμος",
                "cat",
                "bat",
                "hat",
                "mat",
                "pat",
                "rat",
                "sat",
            ]
            .into_iter()
            .map(str::to_string),
        );
        add_titles(connection, &titles, &AtomicBool::new(false)).unwrap();
    }

    #[test]
    fn unicode_casefold_normalization_and_adjacent_transpositions() {
        assert_eq!(normalize("STRAẞE"), normalize("strasse"));
        assert_eq!(normalize("Cafe\u{301}"), normalize("CAFÉ"));
        assert_eq!(normalize("ΚΌΣΜΟΣ"), normalize("κόσμος"));
        // Georgian Mtavruli (Unicode 11) folds to Mkhedruli.
        assert_eq!(normalize("\u{1C90}\u{1C91}\u{1C92}"), "\u{10D0}\u{10D1}\u{10D2}");
        for (left, right) in [
            ("plnaet", "planet"),
            ("cafe", "café"),
            ("plane", "planet"),
            ("planett", "planet"),
            ("caéf", "café"),
        ] {
            assert!(distance_one(left, right), "{left} / {right}");
        }
        assert!(!distance_one("plnaett", "planet"));
        assert!(!distance_one("planet", "planet"));
        assert!(!distance_one("ab", "baaa"));
    }

    #[test]
    fn consumer_queries_preserve_original_case_and_separators_and_rank_frequency() {
        let mut connection = Connection::open_in_memory().unwrap();
        fixture(&mut connection);
        let typo = query_corrections(&connection, "Planer EARHT!").unwrap();
        assert_eq!(
            typo[0],
            SpellingCorrection {
                original_query: "Planer EARHT!".into(),
                corrected_query: "Planer EARTH!".into(),
            }
        );
        let two = query_corrections(&connection, "Plnaet, earht").unwrap();
        assert_eq!(two[0].corrected_query, "Planet, earth");
        assert_eq!(
            query_corrections(&connection, "planrt").unwrap()[0].corrected_query,
            "planet"
        );
        assert_eq!(
            query_corrections(&connection, "cafè").unwrap()[0].corrected_query,
            "café"
        );
        assert_eq!(
            query_corrections(&connection, "a plnaet").unwrap()[0].corrected_query,
            "a planet"
        );
        assert!(query_corrections(&connection, "Cafe\u{301}")
            .unwrap()
            .is_empty());
        assert!(query_corrections(&connection, "STRASSE")
            .unwrap()
            .is_empty());
        assert!(
            query_corrections(&connection, "no plausible correction whatsoever")
                .unwrap()
                .is_empty()
        );
        assert!(query_corrections(&connection, "a b c d e f g h i")
            .unwrap()
            .is_empty());
        assert!(
            query_corrections(&connection, &"a".repeat(MAX_QUERY_BYTES + 1))
                .unwrap()
                .is_empty()
        );
        assert_eq!(query_corrections(&connection, "dat").unwrap().len(), 5);
    }

    #[test]
    fn bounded_title_batches_publish_verified_uuid_version_cache_with_progress() {
        let directory = TestDirectory::new();
        let identity = identity();
        let shared = writable(&identity);
        let mut offsets = Vec::new();
        build_with_batches(
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &shared,
            |offset| {
                offsets.push(offset);
                Ok(if offset == 0 {
                    TitleBatch {
                        titles: vec!["Planet Earth".into(); BATCH as usize],
                        total: BATCH + 1,
                    }
                } else {
                    let inner = shared.lock().unwrap();
                    let status = &inner.statuses[&identity.key()];
                    assert_eq!(
                        (status.processed, status.total, status.words),
                        (BATCH, BATCH + 1, 2)
                    );
                    TitleBatch {
                        titles: vec!["Café".into()],
                        total: BATCH + 1,
                    }
                })
            },
        )
        .unwrap();
        assert_eq!(offsets, vec![0, BATCH]);
        assert_eq!(
            shared.lock().unwrap().statuses[&identity.key()].state,
            "ready"
        );
        let final_path = directory.0.join(format!("{}.sqlite", identity.key()));
        let dictionary = open_verified(&final_path, &identity).unwrap();
        assert_eq!((dictionary.total, dictionary.words), (BATCH + 1, 3));
        assert_eq!(
            query_corrections(&dictionary.connection, "plnaet").unwrap()[0].corrected_query,
            "planet"
        );
        let mut wrong = identity.clone();
        wrong.version += 1;
        assert!(open_verified(&final_path, &wrong).is_err());
        wrong = identity.clone();
        wrong.uuid.push('x');
        assert!(open_verified(&final_path, &wrong).is_err());
        wrong = identity.clone();
        wrong.archive_bytes += 1;
        assert!(open_verified(&final_path, &wrong).is_err());
    }

    #[test]
    fn readonly_lookup_never_creates_cache_and_existing_cache_stays_readonly() {
        let directory = TestDirectory::new();
        let identity = identity();
        let manager = SpellingManager::new(directory.0.clone());
        manager.set_writable(false).unwrap();
        assert!(manager.dictionary(&identity).unwrap().is_none());
        assert!(!directory.0.exists());
        let denied = Mutex::new(Inner::default());
        assert!(build_with_batches(
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &denied,
            |_| panic!("read-only build must not enumerate titles")
        )
        .is_err());
        assert!(!directory.0.exists());
        build_with_batches(
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &writable(&identity),
            |_| {
                Ok(TitleBatch {
                    titles: vec!["Planet".into()],
                    total: 1,
                })
            },
        )
        .unwrap();
        let path = directory.0.join(format!("{}.sqlite", identity.key()));
        let before = fs::read(&path).unwrap();
        assert!(manager.dictionary(&identity).unwrap().is_none());
        finish_load(&manager, &identity);
        let dictionary = manager.dictionary(&identity).unwrap().unwrap();
        let dictionary = dictionary.lock().unwrap();
        assert_eq!(
            query_corrections(&dictionary.connection, "plnaet").unwrap()[0].corrected_query,
            "planet"
        );
        assert!(dictionary
            .connection
            .execute("DELETE FROM words", [])
            .is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn cold_validation_is_single_flight_and_never_blocks_interactive_requests_or_cutover() {
        use std::sync::{atomic::AtomicUsize, mpsc, Barrier};
        use std::time::Duration;

        let directory = TestDirectory::new();
        let identity = identity();
        build_with_batches(
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &writable(&identity),
            |_| {
                Ok(TitleBatch {
                    titles: vec!["Planet".into()],
                    total: 1,
                })
            },
        )
        .unwrap();
        let path = directory.0.join(format!("{}.sqlite", identity.key()));
        let before = fs::read(&path).unwrap();
        let manager = Arc::new(SpellingManager::new(directory.0.clone()));
        manager.set_writable(true).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let timeout = Duration::from_secs(10);
        let caller = std::thread::current().id();
        assert!(manager
            .dictionary_with_loader(&identity, move |path, identity| {
                worker_calls.fetch_add(1, Ordering::AcqRel);
                entered.send(std::thread::current().id()).unwrap();
                blocked
                    .recv_timeout(timeout)
                    .expect("test must release cache validation");
                open_verified(path, identity)
            })
            .unwrap()
            .is_none());
        assert_ne!(started.recv_timeout(timeout).unwrap(), caller);

        // Keep validation blocked while simultaneous cold lookups and polling
        // complete. A duplicated or synchronous load cannot pass this gate.
        let barrier = Arc::new(Barrier::new(5));
        let (completed, responses) = mpsc::channel();
        let mut requests = Vec::new();
        for _ in 0..4 {
            let manager = manager.clone();
            let identity = identity.clone();
            let barrier = barrier.clone();
            let completed = completed.clone();
            requests.push(std::thread::spawn(move || {
                barrier.wait();
                assert!(manager
                    .dictionary_with_loader(&identity, |_, _| panic!(
                        "cold cache validation must be single-flight"
                    ))
                    .unwrap()
                    .is_none());
                assert_eq!(
                    manager.status_for_identity(&identity).unwrap().state,
                    "building"
                );
                completed.send(()).unwrap();
            }));
        }
        barrier.wait();
        for _ in 0..4 {
            responses.recv_timeout(timeout).unwrap();
        }
        for request in requests {
            request.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert!(!manager.status_for_identity(&identity).unwrap().cancellable);
        assert!(manager
            .cancel(&identity.uuid)
            .unwrap_err()
            .contains("cannot be cancelled"));

        // A read-only loader is allowed to outlive privacy cutover. It must not
        // become a joining writer or create a second cache file afterward.
        manager.set_writable(false).unwrap();
        release.send(()).unwrap();
        finish_load(&manager, &identity);
        let status = manager.status_for_identity(&identity).unwrap();
        assert_eq!(
            (status.state.as_str(), status.total, status.words),
            ("ready", 1, 1)
        );
        assert!(!status.cancellable);
        let dictionary = manager.dictionary(&identity).unwrap().unwrap();
        let dictionary = dictionary.lock().unwrap();
        assert_eq!(
            query_corrections(&dictionary.connection, "plnaet").unwrap()[0].corrected_query,
            "planet"
        );
        assert!(dictionary
            .connection
            .execute("DELETE FROM words", [])
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn readonly_wal_cache_validation_never_creates_sidecar_files() {
        let directory = TestDirectory::new();
        let identity = identity();
        build_with_batches(
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &writable(&identity),
            |_| {
                Ok(TitleBatch {
                    titles: vec!["Planet".into()],
                    total: 1,
                })
            },
        )
        .unwrap();
        let path = directory.0.join(format!("{}.sqlite", identity.key()));
        let connection = Connection::open(&path).unwrap();
        let journal: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        drop(connection);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
        let before = fs::read(&path).unwrap();

        let dictionary = open_verified(&path, &identity).unwrap();
        assert_eq!(
            query_corrections(&dictionary.connection, "plnaet").unwrap()[0].corrected_query,
            "planet"
        );
        assert!(dictionary
            .connection
            .execute("DELETE FROM words", [])
            .is_err());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn cancellation_never_publishes_or_replaces_verified_cache() {
        let directory = TestDirectory::new();
        let identity = identity();
        let shared = writable(&identity);
        let cancel = AtomicBool::new(true);
        assert!(
            build_with_batches(&directory.0, &identity, &cancel, &shared, |_| panic!(
                "pre-cancelled build must not enumerate"
            ))
            .is_err()
        );
        assert!(!directory.0.exists());
        cancel.store(false, Ordering::Release);
        build_with_batches(&directory.0, &identity, &cancel, &shared, |_| {
            Ok(TitleBatch {
                titles: vec!["Planet".into()],
                total: 1,
            })
        })
        .unwrap();
        let path = directory.0.join(format!("{}.sqlite", identity.key()));
        let before = fs::read(&path).unwrap();
        let result = build_with_batches(&directory.0, &identity, &cancel, &shared, |_| {
            cancel.store(true, Ordering::Release);
            Ok(TitleBatch {
                titles: vec!["Replacement".into()],
                total: 1,
            })
        });
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn malformed_or_incomplete_cache_and_listing_are_rejected_without_publication() {
        let directory = TestDirectory::new();
        let identity = identity();
        fs::create_dir_all(&directory.0).unwrap();
        let path = directory.0.join(format!("{}.sqlite", identity.key()));
        fs::write(&path, b"not a database").unwrap();
        let manager = SpellingManager::new(directory.0.clone());
        assert!(manager.dictionary(&identity).unwrap().is_none());
        finish_load(&manager, &identity);
        assert!(manager.dictionary(&identity).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not a database");
        fs::remove_file(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        create_schema(&connection).unwrap();
        drop(connection);
        assert!(open_verified(&path, &identity).is_err());
        fs::remove_file(&path).unwrap();
        let result = build_with_batches(
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &writable(&identity),
            |_| {
                Ok(TitleBatch {
                    titles: vec![],
                    total: 1,
                })
            },
        );
        assert!(result.is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn corrections_require_explicit_optin_zero_ordinary_results_and_long_enough_query() {
        assert!(eligible_query("plnaet", 0, true));
        assert!(!eligible_query("plnaet", 0, false));
        assert!(!eligible_query("plnaet", 1, true));
        assert!(!eligible_query("éß", 0, true));
        assert!(!eligible_query(&"x".repeat(MAX_QUERY_BYTES + 1), 0, true));
    }

    #[test]
    fn disabling_writes_cancels_and_joins_owned_worker_before_returning() {
        let directory = TestDirectory::new();
        let manager = SpellingManager::new(directory.0.clone());
        manager.set_writable(true).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let shared = manager.inner.clone();
        let identity = identity();
        let key = identity.key();
        let book_id = identity.uuid.clone();
        let handle = std::thread::spawn(move || {
            while !worker_cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            shared.lock().unwrap().statuses.get_mut(&key).unwrap().state = "cancelled".into();
        });
        {
            let mut inner = manager.inner.lock().unwrap();
            inner
                .statuses
                .insert(identity.key(), identity.status("building"));
            inner.job = Some(Job {
                book_id,
                cancel,
                handle,
            });
        }
        manager.set_writable(false).unwrap();
        let inner = manager.inner.lock().unwrap();
        assert!(!inner.writable);
        assert!(inner.job.is_none());
        assert_eq!(inner.statuses[&identity.key()].state, "cancelled");
        assert!(!directory.0.exists());
    }
}
