// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Explicit, cancellable spelling dictionaries built from public archive titles.
//! Existing caches load single-flight in read-only workers; `building` covers
//! loading as well as explicit indexing. Interactive calls never scan a cache.
//! No automatic builds. Disabling writes joins indexing before privacy changes;
//! read-only loaders may finish without creating or modifying filesystem state.

use crate::archive::{Archive, ArchiveMetadata, TitleBatch};
mod index;
use crate::publication::{publish_verified_noreplace_in, VerifiedFile};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use chksum_hash_sha2_256 as sha256;
use index::{Builder, Lexicon};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use unicode_normalization::UnicodeNormalization;

const VERSION: u32 = 2;
const MAX_TITLES: u32 = 20_000_000;
const MAX_WORDS: u64 = 2_000_000;
const MAX_DB_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_WORD_CHARS: usize = 64;
const MAX_QUERY_BYTES: usize = 512;
const MAX_QUERY_WORDS: usize = 8;
const MAX_CANDIDATES: usize = 8192;
const BATCH: u32 = 256;
// An open dictionary retains a file and checked pages. A larger library must
// not pin a descriptor or every prior advisory status for the process lifetime.
const MAX_DICTIONARIES: usize = 8;
const MAX_COLD_LOADERS: usize = 2;
const MAX_STATUSES: usize = 256;
const MAX_FAILURES: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellingStatus {
    pub book_id: String,
    pub installation_id: String,
    pub source_key: String,
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
pub(crate) struct Identity {
    version: u32,
    uuid: String,
    installation_id: String,
    generation: [u8; 32],
    archive_bytes: u64,
    articles: u64,
}

impl Identity {
    pub(crate) fn from_archive(
        archive: &Archive,
        metadata: &ArchiveMetadata,
        installation_id: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            generation: Self::generation(archive)?,
            version: VERSION,
            uuid: metadata.id.clone(),
            installation_id: installation_id.to_owned(),
            archive_bytes: metadata.size,
            articles: metadata.article_count,
        })
    }

    /// The same identity computed from the retained source's file identity,
    /// without opening the archive. Used only to decide which cache files
    /// still belong to a library entry.
    pub(crate) fn from_content(
        content: &crate::integrity::ContentIdentity,
        metadata: &ArchiveMetadata,
        installation_id: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            generation: Self::content_generation(content)?,
            version: VERSION,
            uuid: metadata.id.clone(),
            installation_id: installation_id.to_owned(),
            archive_bytes: metadata.size,
            articles: metadata.article_count,
        })
    }

    fn generation(archive: &Archive) -> Result<[u8; 32], String> {
        Self::content_generation(&archive.content_identity()?)
    }

    fn content_generation(content: &crate::integrity::ContentIdentity) -> Result<[u8; 32], String> {
        let bytes = serde_json::to_vec(content).map_err(err)?;
        Ok(sha256::hash(&bytes).into())
    }

    pub(crate) fn confirm(&self, archive: &Archive) -> Result<(), String> {
        if Self::generation(archive)? != self.generation {
            return Err("The archive contents changed. Search again.".into());
        }
        Ok(())
    }

    fn digest(&self) -> [u8; 32] {
        let data = format!(
            "{}:{}:{}:{}:{}:{}",
            self.version,
            self.uuid,
            self.archive_bytes,
            self.articles,
            self.installation_id,
            hex::encode(self.generation)
        );
        sha256::hash(data.as_bytes()).into()
    }

    pub(crate) fn key(&self) -> String {
        hex::encode(self.digest())
    }

    // Earlier derived dictionaries have no installation authority. Inspect only
    // their directory entries to explain why an explicit rebuild is available.
    fn unbound_key(&self, version: u32) -> String {
        let data = format!(
            "{version}:{}:{}:{}",
            self.uuid, self.archive_bytes, self.articles
        );
        hex::encode(<[u8; 32]>::from(sha256::hash(data.as_bytes())))
    }

    fn status(&self, state: &str) -> SpellingStatus {
        SpellingStatus {
            book_id: self.uuid.clone(),
            installation_id: self.installation_id.clone(),
            source_key: self.key(),
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
    index: Mutex<Lexicon>,
    total: u32,
    words: u64,
}

enum CacheLookup {
    Missing,
    Pending,
    Ready(Arc<Dictionary>),
}

impl CacheLookup {
    fn ready(self) -> Option<Arc<Dictionary>> {
        match self {
            Self::Ready(dictionary) => Some(dictionary),
            _ => None,
        }
    }
}

struct Job {
    installation_id: String,
    source_key: String,
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
    // Dictionaries, statuses and failures are bounded and evicted oldest-first. In-flight loaders and the
    // active build keep their status entry; dropping an evicted dictionary
    // closes its file once the last query releases it.
    statuses: HashMap<String, SpellingStatus>,
    dictionaries: HashMap<String, Arc<Dictionary>>,
    failures: HashMap<String, String>,
    dictionary_order: VecDeque<String>,
    status_order: VecDeque<String>,
    failure_order: VecDeque<String>,
}

impl Inner {
    fn insert_dictionary(&mut self, key: String, dictionary: Arc<Dictionary>) {
        self.dictionaries.insert(key.clone(), dictionary);
        remember(&mut self.dictionary_order, key);
        while self.dictionaries.len() > MAX_DICTIONARIES {
            let Some(key) = self.dictionary_order.pop_front() else {
                break;
            };
            self.dictionaries.remove(&key);
            if self.statuses.get(&key).is_some_and(|s| s.state == "ready") {
                self.statuses.remove(&key);
                self.status_order.retain(|s| s != &key);
            }
        }
    }

    /// Mark an eviction-protected dictionary as recently used.
    fn touch_dictionary(&mut self, key: &str) {
        if let Some(position) = self
            .dictionary_order
            .iter()
            .position(|entry| entry.as_str() == key)
        {
            if let Some(key) = self.dictionary_order.remove(position) {
                self.dictionary_order.push_back(key);
            }
        }
    }

    fn insert_status(&mut self, key: String, status: SpellingStatus) {
        self.statuses.insert(key.clone(), status);
        remember(&mut self.status_order, key);
        while self.statuses.len() > MAX_STATUSES {
            let Some(position) = self
                .status_order
                .iter()
                .position(|key| self.status_evictable(key))
            else {
                break;
            };
            let Some(key) = self.status_order.remove(position) else {
                break;
            };
            self.statuses.remove(&key);
        }
    }

    /// A loader still publishing, or the active build, must keep reporting.
    fn status_evictable(&self, key: &str) -> bool {
        if self
            .loads
            .get(key)
            .is_some_and(|handle| !handle.is_finished())
        {
            return false;
        }
        !self.job.as_ref().is_some_and(|job| {
            !job.handle.is_finished()
                && self
                    .statuses
                    .get(key)
                    .is_some_and(|status| status.installation_id == job.installation_id)
        })
    }

    fn insert_failure(&mut self, key: String, error: String) {
        self.failures.insert(key.clone(), error);
        remember(&mut self.failure_order, key);
        while self.failures.len() > MAX_FAILURES {
            let Some(key) = self.failure_order.pop_front() else {
                break;
            };
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

    fn dictionary(&self, identity: &Identity) -> Result<CacheLookup, String> {
        self.dictionary_with_loader(identity, open_verified)
    }

    fn dictionary_with_loader<F>(&self, identity: &Identity, load: F) -> Result<CacheLookup, String>
    where
        F: FnOnce(&Path, &Identity) -> Result<Dictionary, String> + Send + 'static,
    {
        let key = identity.key();
        let mut inner = self.inner.lock().map_err(err)?;
        if let Some(dictionary) = inner.dictionaries.get(&key).cloned() {
            // Keep the dictionaries in active use from being evicted first.
            inner.touch_dictionary(&key);
            return Ok(CacheLookup::Ready(dictionary));
        }
        if let Some(error) = inner.failures.get(&key) {
            return Err(error.clone());
        }
        if inner
            .loads
            .get(&key)
            .is_some_and(|handle| !handle.is_finished())
        {
            return Ok(CacheLookup::Pending);
        }
        let name = format!("{key}.words");
        let path = self.directory.join(&name);
        let found =
            open_directory(&self.directory, false).and_then(|dir| dir.symlink_metadata(&name));
        match found {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(CacheLookup::Missing)
            }
            Err(error) => return Err(err(error)),
            Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_DB_BYTES => {
                return Err("Invalid or oversized spelling cache file".into());
            }
            Ok(_) => {}
        }
        // Interactive polling will retry this existing file as soon as a slot
        // opens. A deferred validation never starts an index build.
        if inner.loads.len() >= MAX_COLD_LOADERS {
            return Ok(CacheLookup::Pending);
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
                            inner.insert_dictionary(worker_key.clone(), Arc::new(dictionary));
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
        Ok(CacheLookup::Pending)
    }

    pub(crate) fn status(&self, identity: &Identity) -> Result<SpellingStatus, String> {
        let key = identity.key();
        if let Some(status) = self.inner.lock().map_err(err)?.statuses.get(&key) {
            return Ok(status.clone());
        }
        match self.dictionary(identity) {
            Ok(CacheLookup::Ready(dictionary)) => {
                let mut status = identity.status("ready");
                status.processed = dictionary.total;
                status.total = dictionary.total;
                status.words = dictionary.words;
                Ok(status)
            }
            Ok(CacheLookup::Pending) => Ok(self
                .inner
                .lock()
                .map_err(err)?
                .statuses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| identity.status("building"))),
            Ok(CacheLookup::Missing) => {
                let inner = self.inner.lock().map_err(err)?;
                if let Some(status) = inner.statuses.get(&key) {
                    return Ok(status.clone());
                }
                if inner.writable {
                    for (version, extension) in [(1, "sqlite"), (2, "words")] {
                        let name = format!("{}.{extension}", identity.unbound_key(version));
                        match open_directory(&self.directory, false)
                            .and_then(|dir| dir.symlink_metadata(name))
                        {
                            Ok(metadata) if metadata.is_file() => {
                                return Ok(identity.status("rebuild"))
                            }
                            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(err(e)),
                            _ => {}
                        }
                    }
                }
                Ok(identity.status("missing"))
            }
            Err(error) => {
                let mut status = identity.status("error");
                status.error = Some(error);
                Ok(status)
            }
        }
    }

    /// Only an explicit trusted application command may call this method.
    pub(crate) fn build(
        self: &Arc<Self>,
        archive: Archive,
        identity: Identity,
    ) -> Result<SpellingStatus, String> {
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let key = identity.key();
        let inner = self.inner.lock().map_err(err)?;
        if !inner.writable {
            return Err("Spelling builds are disabled in private or read-only mode".into());
        }
        if let Some(job) = inner.job.as_ref() {
            if !job.handle.is_finished() {
                if job.source_key == key {
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
        let lookup = self.dictionary(&identity);
        if matches!(lookup, Ok(CacheLookup::Pending)) {
            return Ok(self
                .inner
                .lock()
                .map_err(err)?
                .statuses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| identity.status("building")));
        }
        if let Ok(CacheLookup::Ready(dictionary)) = lookup {
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
        let installation_id = identity.installation_id.clone();
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
            installation_id,
            source_key: key,
            cancel,
            handle,
        });
        Ok(status)
    }

    /// Whether a spelling cache directory exists at all (most profiles never
    /// build a dictionary). Metadata only.
    pub(crate) fn cache_present(&self) -> bool {
        fs::symlink_metadata(&self.directory).is_ok_and(|metadata| metadata.is_dir())
    }

    /// Remove cache files no library entry can use: dictionaries (`.words`)
    /// and rollback backups (`.previous`) of keys outside `live`, and every
    /// staging (`.tmp`) or scratch (`.run`) file a killed build left behind.
    /// `live` must describe every library entry; the caller skips the sweep
    /// while any entry is unavailable. Nothing runs while a build or cache
    /// load is active, or while caches are not writable. Only exact cache
    /// names are removed, through the retained no-follow directory, and only
    /// regular files. Returns how many files were removed.
    pub(crate) fn sweep(&self, live: &[Identity]) -> Result<usize, String> {
        // Holding lifecycle keeps a build from starting until the sweep ends.
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let mut keep: std::collections::HashSet<String> = live.iter().map(Identity::key).collect();
        {
            let inner = self.inner.lock().map_err(err)?;
            if !inner.writable
                || inner.job.as_ref().is_some_and(|job| !job.handle.is_finished())
                || inner.loads.values().any(|handle| !handle.is_finished())
            {
                return Ok(0);
            }
            // Whatever this process already resolved stays, even if the
            // caller derived a different key for it.
            keep.extend(inner.dictionaries.keys().cloned());
            keep.extend(inner.statuses.keys().cloned());
            keep.extend(inner.failures.keys().cloned());
        }
        let dir = match open_directory(&self.directory, false) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(err(error)),
        };
        let mut names = Vec::new();
        for entry in dir.entries().map_err(err)? {
            let entry = entry.map_err(err)?;
            if !entry.file_type().map_err(err)?.is_file() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        // A legacy dictionary still explains "rebuild" for its live entry
        // until a current dictionary exists.
        for identity in live {
            if !names.iter().any(|name| *name == format!("{}.words", identity.key())) {
                keep.insert(identity.unbound_key(2));
            }
        }
        let mut removed = 0;
        for name in names {
            let remove = match sweep_name(&name) {
                Some(SweepName::Dictionary(key) | SweepName::Backup(key)) => !keep.contains(key),
                Some(SweepName::Staging) => true,
                None => false,
            };
            if !remove {
                continue;
            }
            check_cache_directory(&dir).map_err(err)?;
            match dir.remove_file(&name) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                // An open file on Windows, for example: the next sweep retries.
                Err(error) => eprintln!("Spelling cache cleanup: {error}"),
            }
        }
        Ok(removed)
    }

    /// Cancel and join whichever build is running, if any. Windows library
    /// moves use this because the build holds its archive open.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn cancel_running(&self) -> Result<(), String> {
        self.stop_build(None)
    }

    pub fn retire_installation(&self, installation: &str) -> Result<(), String> {
        self.stop_build(Some(installation))
    }

    fn stop_build(&self, installation: Option<&str>) -> Result<(), String> {
        // Holding lifecycle prevents another build starting between take and join.
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let job = self.inner.lock().map_err(err)?.job.take_if(|job| {
            installation.is_none_or(|installation| job.installation_id == installation)
        });
        if let Some(job) = job {
            job.cancel.store(true, Ordering::Release);
            job.handle
                .join()
                .map_err(|_| "Spelling worker panicked".to_string())?;
        }
        Ok(())
    }

    pub fn cancel(
        &self,
        installation_id: &str,
        source_key: &str,
    ) -> Result<SpellingStatus, String> {
        let _lifecycle = self.lifecycle.lock().map_err(err)?;
        let job = {
            let mut inner = self.inner.lock().map_err(err)?;
            if inner.job.as_ref().is_some_and(|job| {
                job.installation_id == installation_id && job.source_key == source_key
            }) {
                inner.job.take()
            } else {
                if inner.statuses.get(source_key).is_some_and(|status| {
                    status.installation_id == installation_id
                        && status.state == "building"
                        && !status.cancellable
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
            .get(source_key)
            .filter(|status| status.installation_id == installation_id)
            .cloned()
            .ok_or_else(|| "No spelling job exists for this archive".into())
    }

    pub(crate) fn corrections(
        &self,
        identity: &Identity,
        query: &str,
        ordinary_total: u64,
        enabled: bool,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<SpellingCorrection>, String> {
        if !eligible_query(query, ordinary_total, enabled) {
            return Ok(Vec::new());
        }
        crate::search::check_cancel(cancel)?;
        let Some(dictionary) = self.dictionary(identity)?.ready() else {
            return Ok(Vec::new());
        };
        let index =
            crate::archive::archive_lock(&dictionary.index, Some((cancel, "Search cancelled")))?;
        corrections_while(&index, query, || cancel.is_cancelled())
    }
}

impl Drop for SpellingManager {
    fn drop(&mut self) {
        // Read-only loaders retain only shared state and a read-only descriptor,
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

enum SweepName<'a> {
    Dictionary(&'a str),
    Backup(&'a str),
    Staging,
}

fn cache_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn cache_uuid(text: &str) -> bool {
    uuid::Uuid::try_parse(text).is_ok_and(|uuid| uuid.hyphenated().to_string() == text)
}

/// The exact names this module creates: `{key}.words`,
/// `.{key}.{uuid}.previous`, `.{key}.{uuid}.tmp` and `.spelling-{uuid}.run`.
fn sweep_name(name: &str) -> Option<SweepName<'_>> {
    if let Some(key) = name.strip_suffix(".words") {
        return cache_key(key).then_some(SweepName::Dictionary(key));
    }
    if let Some(uuid) = name.strip_prefix(".spelling-").and_then(|rest| rest.strip_suffix(".run")) {
        return cache_uuid(uuid).then_some(SweepName::Staging);
    }
    let rest = name.strip_prefix('.')?;
    let (key, rest) = rest.split_once('.')?;
    let (uuid, suffix) = rest.split_once('.')?;
    if !cache_key(key) || !cache_uuid(uuid) {
        return None;
    }
    match suffix {
        "previous" => Some(SweepName::Backup(key)),
        "tmp" => Some(SweepName::Staging),
        _ => None,
    }
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

fn open_directory(path: &Path, create: bool) -> io::Result<Dir> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let profile = crate::storage::open_directory_nofollow(parent)?;
    if create {
        #[cfg_attr(windows, allow(unused_mut))]
        let mut builder = cap_std::fs::DirBuilder::new();
        #[cfg(unix)]
        cap_std::fs::DirBuilderExt::mode(&mut builder, 0o700);
        if let Err(error) = profile.create_dir_with(name, &builder) {
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
    }
    let directory = profile.open_dir_nofollow(name)?;
    check_cache_directory(&directory)?;
    Ok(directory)
}

// Scratch and staging cleanup uses names relative to this retained directory.
// The directory itself must exclude writers from other accounts.
fn cache_permissions_error() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied,
        "Spelling storage has unsafe permissions. Use a local, account-owned spelling folder and files with write access restricted to your account or system administrators. On Apple platforms, use a folder without an extended access list.")
}

fn check_cache_directory(directory: &Dir) -> io::Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as UnixMetadata;
        let metadata = directory.dir_metadata()?;
        if metadata.uid() != crate::fs_ext::effective_uid() || metadata.mode() & 0o022 != 0 {
            return Err(cache_permissions_error());
        }
    }
    #[cfg(target_vendor = "apple")]
    if crate::storage::has_extended_file_security(directory)? {
        return Err(cache_permissions_error());
    }
    #[cfg(windows)]
    crate::private_sync_identity::check_cache_directory_access(directory)
        .map_err(|_| cache_permissions_error())?;
    Ok(())
}

fn check_cache_file(file: &cap_std::fs::File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Spelling cache must be a regular file with a single link",
        ));
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as UnixMetadata;
        if metadata.uid() != crate::fs_ext::effective_uid() || metadata.mode() & 0o022 != 0 {
            return Err(cache_permissions_error());
        }
    }
    #[cfg(target_vendor = "apple")]
    if crate::storage::has_extended_file_security(file)? {
        return Err(cache_permissions_error());
    }
    #[cfg(windows)]
    crate::private_sync_identity::check_cache_file_access(file)
        .map_err(|_| cache_permissions_error())?;
    Ok(())
}

fn read_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        // A raced FIFO must not turn a read-only background load into a permanent wait.
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options
}

fn scratch_file(directory: &Dir) -> io::Result<fs::File> {
    let name = PathBuf::from(format!(".spelling-{}.run", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options
        .create_new(true)
        .read(true)
        .write(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        // FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE. Refuse other opens.
        options.share_mode(0).custom_flags(0x100 | 0x0400_0000);
    }
    let file = directory.open_with(&name, &options)?.into_std();
    #[cfg(unix)]
    {
        // Unlink the fresh empty file before writing a single archive-derived byte.
        let empty = VerifiedFile::new(file.try_clone()?, 0)?;
        check_cache_directory(directory)?;
        if !empty.matches_in(directory, &name)? {
            return Err(io::Error::other("Spelling scratch file changed"));
        }
        directory.remove_file(name)?;
    }
    Ok(file)
}

struct Staging<'a> {
    directory: &'a Dir,
    name: PathBuf,
    file: fs::File,
}

impl Drop for Staging<'_> {
    fn drop(&mut self) {
        if let Ok(owned) = self
            .file
            .try_clone()
            .and_then(|f| VerifiedFile::new(f, self.file.metadata()?.len()))
        {
            if check_cache_directory(self.directory).is_ok()
                && owned
                    .matches_in(self.directory, &self.name)
                    .unwrap_or(false)
            {
                let _ = self.directory.remove_file(&self.name);
            }
        }
    }
}

fn open_verified(path: &Path, identity: &Identity) -> Result<Dictionary, String> {
    let directory =
        open_directory(path.parent().ok_or("Missing spelling directory")?, false).map_err(err)?;
    let file = directory
        .open_with(
            path.file_name().ok_or("Missing spelling file")?,
            &read_options(),
        )
        .map_err(err)?;
    check_cache_file(&file).map_err(err)?;
    let (index, total, words) =
        Lexicon::open(file.into_std(), identity.digest(), &AtomicBool::new(false)).map_err(err)?;
    Ok(Dictionary {
        index: Mutex::new(index),
        total,
        words,
    })
}
fn build_archive(
    archive: &Archive,
    directory: &Path,
    identity: &Identity,
    cancel: &AtomicBool,
    shared: &Mutex<Inner>,
) -> Result<(), String> {
    identity.confirm(archive)?;
    build_with_batches(
        Some(archive),
        directory,
        identity,
        cancel,
        shared,
        |offset| archive.spelling_titles(offset, BATCH, cancel),
    )
}

fn build_with_batches<F>(
    archive: Option<&Archive>,
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
    let dir = open_directory(directory, true).map_err(err)?;
    let key = identity.key();
    let final_name = PathBuf::from(format!("{key}.words"));
    let existing = match dir.open_with(&final_name, &read_options()) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(err(error)),
        Ok(file) => {
            check_cache_file(&file).map_err(err)?;
            let len = file.metadata().map_err(err)?.len();
            let file = file.into_std();
            Some(VerifiedFile::new(file, len).map_err(err)?)
        }
    };
    let temp_name = PathBuf::from(format!(".{key}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = dir.open_with(&temp_name, &options).map_err(err)?.into_std();
    let staged = Staging {
        directory: &dir,
        name: temp_name,
        file,
    };
    let scratch = || scratch_file(&dir);
    let mut builder = Builder::new(&scratch, cancel);
    let mut processed: u32 = 0;
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
        for title in &page.titles {
            check_cancel(cancel)?;
            for (start, end) in tokens(title) {
                let word = normalize(&title[start..end]);
                if valid_word(&word) {
                    builder.add(word).map_err(err)?;
                }
            }
        }
        let words = builder.words;
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
            break;
        }
    }
    check_cancel(cancel)?;
    builder
        .finish(
            staged.file.try_clone().map_err(err)?,
            identity.digest(),
            processed,
        )
        .map_err(err)?;
    let readonly = dir
        .open_with(&staged.name, &read_options())
        .map_err(err)?
        .into_std();
    let verified = VerifiedFile::new(
        readonly.try_clone().map_err(err)?,
        staged.file.metadata().map_err(err)?.len(),
    )
    .map_err(err)?;
    let source = VerifiedFile::new(
        staged.file.try_clone().map_err(err)?,
        verified.file().metadata().map_err(err)?.len(),
    )
    .map_err(err)?;
    if !source.matches_in(&dir, &staged.name).map_err(err)?
        || !verified.matches_in(&dir, &staged.name).map_err(err)?
    {
        return Err("Spelling staging file changed during validation".into());
    }
    let (index, total, words) = Lexicon::open(readonly, identity.digest(), cancel).map_err(err)?;
    let dictionary = Dictionary {
        index: Mutex::new(index),
        total,
        words,
    };
    if let Some(archive) = archive {
        identity.confirm(archive)?;
    }
    let mut inner = shared.lock().map_err(err)?;
    check_cancel(cancel)?;
    if !inner.writable {
        return Err("Spelling cache publication disabled by privacy policy".into());
    }
    let confirm_directory = || -> Result<(), String> {
        let current = open_directory(directory, false)
            .map_err(err)?
            .dir_metadata()
            .map_err(err)?;
        let retained = dir.dir_metadata().map_err(err)?;
        if current.dev() != retained.dev() || current.ino() != retained.ino() {
            return Err("Spelling directory changed before publication".into());
        }
        Ok(())
    };
    confirm_directory()?;
    // Explicit recovery retains the previous generation. A concurrent publisher,
    // symlink or hardlink is never overwritten.
    let backup = PathBuf::from(format!(".{key}.{}.previous", uuid::Uuid::new_v4()));
    let result = publish_index(
        &dir,
        &staged.name,
        &final_name,
        &backup,
        &verified,
        existing.as_ref(),
        || {
            confirm_directory()?;
            #[cfg(unix)]
            crate::storage::sync_cap_directory(&dir).map_err(err)?;
            Ok(())
        },
    );
    if let Err(error) = result {
        // A failed rollback can leave a visible file. Require explicit recovery
        // in this process before accepting it as a successfully built cache.
        inner.insert_failure(key, error.clone());
        return Err(error);
    }
    let mut status = identity.status("ready");
    status.processed = dictionary.total;
    status.total = dictionary.total;
    status.words = dictionary.words;
    inner.insert_dictionary(key.clone(), Arc::new(dictionary));
    inner.failures.remove(&key);
    inner.insert_status(key, status);
    Ok(())
}

fn publish_index(
    dir: &Dir,
    staging: &Path,
    destination: &Path,
    backup: &Path,
    new: &VerifiedFile,
    previous: Option<&VerifiedFile>,
    confirm_and_flush: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // Either verified rename may have moved the file before an I/O error
    // prevented its post-rename identity check. Recover from observed names.
    let backed_up = previous.map_or(Ok(()), |old| {
        publish_verified_noreplace_in(dir, destination, backup, old)
    });
    let published =
        backed_up.and_then(|()| publish_verified_noreplace_in(dir, staging, destination, new));
    let was_published = published.is_ok();
    let error = match published {
        Ok(()) => match confirm_and_flush() {
            Ok(()) => {
                // The replacement is durable: the rollback copy is no longer
                // needed. Remove only the file this publish moved aside; a
                // leftover is collected by the next cache sweep.
                if let Some(old) = previous {
                    if check_cache_directory(dir).is_ok() && old.matches_in(dir, backup).unwrap_or(false) {
                        let _ = dir.remove_file(backup);
                    }
                }
                return Ok(());
            }
            Err(error) => error,
        },
        Err(error) => err(error),
    };
    let recover = || -> Result<(), String> {
        check_cache_directory(dir).map_err(err)?;
        if new.matches_in(dir, destination).map_err(err)? {
            publish_verified_noreplace_in(dir, destination, staging, new).map_err(err)?;
        } else if was_published {
            return Err("The published spelling file changed before recovery".into());
        }
        if let Some(old) = previous {
            if !old.matches_in(dir, destination).map_err(err)? {
                publish_verified_noreplace_in(dir, backup, destination, old).map_err(err)?;
            }
        }
        #[cfg(unix)]
        crate::storage::sync_cap_directory(dir).map_err(err)?;
        Ok(())
    };
    match recover() {
        Ok(()) => Err(format!("The spelling index could not be saved. Check available space and permissions, then retry. {error}")),
        Err(recovery) => Err(format!("The spelling index could not be saved and recovery could not be confirmed. Check the spelling folder, available space and permissions before retrying. Previous backup name, if present: {}. {error}. Recovery: {recovery}", backup.display())),
    }
}

/// Optimal-string-alignment distance <= 1: insert/delete/substitute or transpose
/// two adjacent Unicode scalars. No byte-distance corruption for non-ASCII text.
fn distance_one(left: &str, right: &str) -> bool {
    if left.is_ascii() && right.is_ascii() {
        adjacent(left.as_bytes(), right.as_bytes())
    } else {
        adjacent(
            &left.chars().collect::<Vec<_>>(),
            &right.chars().collect::<Vec<_>>(),
        )
    }
}

fn adjacent<T: Eq>(left: &[T], right: &[T]) -> bool {
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

#[cfg(test)]
fn query_corrections(index: &Lexicon, query: &str) -> Result<Vec<SpellingCorrection>, String> {
    corrections_while(index, query, || false)
}
fn corrections_while(
    index: &Lexicon,
    query: &str,
    is_cancelled: impl Fn() -> bool,
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
        if is_cancelled() {
            return Err("Search cancelled".into());
        }
        let original = &query[start..end];
        let word = normalize(original);
        if word.len() > 256 || word.chars().count() > MAX_WORD_CHARS {
            return Ok(Vec::new());
        }
        let known = !valid_word(&word) || index.known(&word).map_err(err)?;
        let replacements = if known {
            vec![(original.to_string(), 0)]
        } else {
            changed += 1;
            if changed > 2 {
                return Ok(Vec::new());
            }
            index
                .candidates(&word, &is_cancelled)
                .map_err(err)?
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

    pub(super) struct TestDirectory(pub(super) PathBuf);
    impl TestDirectory {
        pub(super) fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .canonicalize()
                    .unwrap()
                    .join(format!("kiwix-spelling-{}", uuid::Uuid::new_v4())),
            )
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
            generation: [0; 32],
            installation_id: "fixture-installation".into(),
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

    /// Wait for a released loader. A loader that already finished removed its
    /// own handle in the same locked step that published its result, so a
    /// missing handle means there is nothing left to wait for.
    fn finish_load(manager: &SpellingManager, identity: &Identity) {
        let handle = manager
            .inner
            .lock()
            .unwrap()
            .loads
            .remove(&identity.key());
        if let Some(handle) = handle {
            handle.join().unwrap();
        }
    }

    fn fixture(directory: &Path) -> Dictionary {
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
        let id = identity();
        build_with_batches(
            None,
            directory,
            &id,
            &AtomicBool::new(false),
            &writable(&id),
            |_| {
                Ok(TitleBatch {
                    titles: titles.clone(),
                    total: titles.len() as u32,
                })
            },
        )
        .unwrap();
        open_verified(&directory.join(format!("{}.words", id.key())), &id).unwrap()
    }

    #[test]
    fn unicode_casefold_normalization_and_adjacent_transpositions() {
        assert_eq!(normalize("STRAẞE"), normalize("strasse"));
        assert_eq!(normalize("Cafe\u{301}"), normalize("CAFÉ"));
        assert_eq!(normalize("ΚΌΣΜΟΣ"), normalize("κόσμος"));
        // Georgian Mtavruli (Unicode 11) folds to Mkhedruli.
        assert_eq!(
            normalize("\u{1C90}\u{1C91}\u{1C92}"),
            "\u{10D0}\u{10D1}\u{10D2}"
        );
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
        let dir = TestDirectory::new();
        let fixture = fixture(&dir.0);
        let connection = &*fixture.index.lock().unwrap();
        let typo = query_corrections(connection, "Planer EARHT!").unwrap();
        assert_eq!(
            typo[0],
            SpellingCorrection {
                original_query: "Planer EARHT!".into(),
                corrected_query: "Planer EARTH!".into(),
            }
        );
        let two = query_corrections(connection, "Plnaet, earht").unwrap();
        assert_eq!(two[0].corrected_query, "Planet, earth");
        assert_eq!(
            query_corrections(connection, "planrt").unwrap()[0].corrected_query,
            "planet"
        );
        assert_eq!(
            query_corrections(connection, "cafè").unwrap()[0].corrected_query,
            "café"
        );
        assert_eq!(
            query_corrections(connection, "a plnaet").unwrap()[0].corrected_query,
            "a planet"
        );
        assert!(query_corrections(connection, "Cafe\u{301}")
            .unwrap()
            .is_empty());
        assert!(query_corrections(connection, "STRASSE").unwrap().is_empty());
        assert!(
            query_corrections(connection, "no plausible correction whatsoever")
                .unwrap()
                .is_empty()
        );
        assert!(query_corrections(connection, "a b c d e f g h i")
            .unwrap()
            .is_empty());
        assert!(
            query_corrections(connection, &"a".repeat(MAX_QUERY_BYTES + 1))
                .unwrap()
                .is_empty()
        );
        assert_eq!(query_corrections(connection, "dat").unwrap().len(), 5);
    }

    #[test]
    fn bounded_title_batches_publish_verified_uuid_version_cache_with_progress() {
        let directory = TestDirectory::new();
        let identity = identity();
        let shared = writable(&identity);
        let mut offsets = Vec::new();
        build_with_batches(
            None,
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
        let final_path = directory.0.join(format!("{}.words", identity.key()));
        let dictionary = open_verified(&final_path, &identity).unwrap();
        assert_eq!((dictionary.total, dictionary.words), (BATCH + 1, 3));
        assert_eq!(
            query_corrections(&dictionary.index.lock().unwrap(), "plnaet").unwrap()[0]
                .corrected_query,
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
        assert!(manager.dictionary(&identity).unwrap().ready().is_none());
        assert!(!directory.0.exists());
        let denied = Mutex::new(Inner::default());
        assert!(build_with_batches(
            None,
            &directory.0,
            &identity,
            &AtomicBool::new(false),
            &denied,
            |_| panic!("read-only build must not enumerate titles")
        )
        .is_err());
        assert!(!directory.0.exists());
        build_with_batches(
            None,
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
        let path = directory.0.join(format!("{}.words", identity.key()));
        let before = fs::read(&path).unwrap();
        assert!(manager.dictionary(&identity).unwrap().ready().is_none());
        finish_load(&manager, &identity);
        let dictionary = manager.dictionary(&identity).unwrap().ready().unwrap();
        assert_eq!(
            query_corrections(&dictionary.index.lock().unwrap(), "plnaet").unwrap()[0]
                .corrected_query,
            "planet"
        );

        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn cold_validation_is_single_flight_and_never_blocks_interactive_requests_or_cutover() {
        use std::sync::{atomic::AtomicUsize, mpsc, Barrier};
        use std::time::Duration;

        let directory = TestDirectory::new();
        let identity = identity();
        build_with_batches(
            None,
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
        let path = directory.0.join(format!("{}.words", identity.key()));
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
            .ready()
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
                    .ready()
                    .is_none());
                assert_eq!(manager.status(&identity).unwrap().state, "building");
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
        assert!(!manager.status(&identity).unwrap().cancellable);
        assert!(manager
            .cancel(&identity.installation_id, &identity.key())
            .unwrap_err()
            .contains("cannot be cancelled"));

        // A read-only loader is allowed to outlive privacy cutover. It must not
        // become a joining writer or create a second cache file afterward.
        manager.set_writable(false).unwrap();
        release.send(()).unwrap();
        finish_load(&manager, &identity);
        let status = manager.status(&identity).unwrap();
        assert_eq!(
            (status.state.as_str(), status.total, status.words),
            ("ready", 1, 1)
        );
        assert!(!status.cancellable);
        let dictionary = manager.dictionary(&identity).unwrap().ready().unwrap();
        assert_eq!(
            query_corrections(&dictionary.index.lock().unwrap(), "plnaet").unwrap()[0]
                .corrected_query,
            "planet"
        );

        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn legacy_cache_is_only_identified_by_name_and_requires_explicit_rebuild() {
        let directory = TestDirectory::new();
        open_directory(&directory.0, true).unwrap();
        let id = identity();
        let path = directory.0.join(format!("{}.sqlite", id.unbound_key(1)));
        let marker = b"Legacy file is preserved without parsing";
        fs::write(&path, marker).unwrap();
        let manager = SpellingManager::new(directory.0.clone());
        manager.set_writable(false).unwrap();
        assert_eq!(manager.status(&id).unwrap().state, "missing");
        manager.set_writable(true).unwrap();
        assert_eq!(manager.status(&id).unwrap().state, "rebuild");
        assert!(manager.dictionary(&id).unwrap().ready().is_none());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
        let dictionary = fixture(&directory.0);
        assert_eq!(
            query_corrections(&dictionary.index.lock().unwrap(), "plnaet").unwrap()[0]
                .corrected_query,
            "planet"
        );
        assert_eq!(fs::read(path).unwrap(), marker);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn spelling_failed_directory_flush_preserves_old_cache_and_can_retry_and_reopen() {
        let templates = TestDirectory::new();
        drop(fixture(&templates.0));
        let id = identity();
        let name = PathBuf::from(format!("{}.words", id.key()));
        let checked = |path: &Path| {
            let file = fs::File::open(path).unwrap();
            let size = file.metadata().unwrap().len();
            VerifiedFile::new(file, size).unwrap()
        };
        for with_old in [false, true] {
            let profile = TestDirectory::new();
            let dir = open_directory(&profile.0, true).unwrap();
            let path = profile.0.join(&name);
            let staging = Path::new("temporary");
            let backup = Path::new("prior");
            let stage = profile.0.join(staging);
            fs::copy(templates.0.join(&name), &stage).unwrap();
            if with_old {
                fs::copy(templates.0.join(&name), &path).unwrap();
            }
            fs::write(profile.0.join("unrelated"), b"preserved").unwrap();
            let old = with_old.then(|| checked(&path));
            let new = checked(&stage);
            let failed = publish_index(&dir, staging, &name, backup, &new, old.as_ref(), || {
                assert!(new.matches_in(&dir, &name).unwrap());
                #[cfg(any(target_os = "linux", target_os = "android"))]
                {
                    // This is the real kernel error from syncing a cap-std O_PATH
                    // descriptor that triggered the packaged Linux regression.
                    let error = dir
                        .try_clone()
                        .unwrap()
                        .into_std_file()
                        .sync_all()
                        .unwrap_err();
                    assert_eq!(error.raw_os_error(), Some(libc::EBADF));
                    Err(err(error))
                }
                #[cfg(not(any(target_os = "linux", target_os = "android")))]
                Err(err(io::Error::from_raw_os_error(libc::EIO)))
            });
            assert!(failed.unwrap_err().contains("could not be saved"));
            assert!(new.matches_in(&dir, staging).unwrap());
            assert_eq!(path.exists(), with_old);
            if let Some(old) = &old {
                assert!(old.matches_in(&dir, &name).unwrap());
                open_verified(&path, &id).unwrap();
            }
            assert!(!profile.0.join(backup).exists());
            assert_eq!(fs::read(profile.0.join("unrelated")).unwrap(), b"preserved");
            publish_index(&dir, staging, &name, backup, &new, old.as_ref(), || {
                crate::storage::sync_cap_directory(&dir).map_err(err)
            })
            .unwrap();
            assert!(new.matches_in(&dir, &name).unwrap());
            assert!(open_verified(&path, &id).unwrap().words > 0);
            // A successful publish keeps no rollback copy behind.
            assert!(!profile.0.join(backup).exists());
            if let Some(old) = old {
                assert!(!old.matches_in(&dir, backup).unwrap_or(false));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn spelling_uncertain_backup_move_restores_only_the_retained_prior_file() {
        for replaced_destination in [false, true] {
            let profile = TestDirectory::new();
            let dir = open_directory(&profile.0, true).unwrap();
            let stage = Path::new("new");
            let destination = Path::new("final");
            let backup = Path::new("owned-backup");
            let new = VerifiedFile::new(dir.create(stage).unwrap().into_std(), 0).unwrap();
            let previous =
                VerifiedFile::new(dir.create(destination).unwrap().into_std(), 0).unwrap();
            // The original is already under its backup name, as it could be
            // when the post-rename check fails. No other name is ours to move.
            dir.rename(destination, &dir, backup).unwrap();
            if replaced_destination {
                dir.write(destination, b"unrelated").unwrap();
            }
            let error = publish_index(
                &dir,
                stage,
                destination,
                backup,
                &new,
                Some(&previous),
                || panic!("failed initial move must never reach the final flush"),
            )
            .unwrap_err();
            assert!(new.matches_in(&dir, stage).unwrap());
            if replaced_destination {
                assert!(
                    error.contains("owned-backup")
                        && error.contains("recovery could not be confirmed")
                );
                assert!(previous.matches_in(&dir, backup).unwrap());
                assert_eq!(dir.read(destination).unwrap(), b"unrelated");
            } else {
                assert!(error.contains("could not be saved"));
                assert!(previous.matches_in(&dir, destination).unwrap());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn spelling_rollback_keeps_replacements_at_all_owned_names() {
        for changed in ["final", "staging", "backup"] {
            let profile = TestDirectory::new();
            let dir = open_directory(&profile.0, true).unwrap();
            let stage = Path::new("staging");
            let destination = Path::new("final");
            let backup = Path::new("backup");
            let make = |name: &Path, bytes: &[u8]| {
                let path = profile.0.join(name);
                fs::write(&path, bytes).unwrap();
                VerifiedFile::new(fs::File::open(&path).unwrap(), bytes.len() as u64).unwrap()
            };
            let old = make(destination, b"old");
            let new = make(stage, b"new");
            let result = publish_index(&dir, stage, destination, backup, &new, Some(&old), || {
                let path = profile.0.join(changed);
                if path.exists() {
                    fs::rename(&path, profile.0.join("displaced")).unwrap();
                }
                fs::write(&path, b"another file").unwrap();
                Err("simulated flush error".into())
            });
            assert!(result
                .unwrap_err()
                .contains("recovery could not be confirmed"));
            assert_eq!(fs::read(profile.0.join(changed)).unwrap(), b"another file");
            assert!(
                old.matches_in(&dir, backup).unwrap()
                    || old.matches_in(&dir, Path::new("displaced")).unwrap()
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spelling_refuses_extended_apple_acls_on_private_mode_files_and_directories() {
        let directory = TestDirectory::new();
        let id = identity();
        drop(fixture(&directory.0));
        let words = directory.0.join(format!("{}.words", id.key()));
        let before = fs::read(&words).unwrap();
        for (path, acl) in [
            (&words, "group:staff allow write"),
            (&directory.0, "group:staff allow add_file,delete_child"),
        ] {
            assert!(std::process::Command::new("/bin/chmod")
                .args(["+a", acl])
                .arg(path)
                .status()
                .unwrap()
                .success());
            assert!(open_verified(&words, &id)
                .err()
                .unwrap()
                .contains("write access"));
            assert_eq!(fs::read(&words).unwrap(), before);
            assert!(std::process::Command::new("/bin/chmod")
                .arg("-N")
                .arg(path)
                .status()
                .unwrap()
                .success());
        }
        open_verified(&words, &id).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shared_spelling_storage_is_refused_without_modifying_existing_files() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TestDirectory::new();
        let id = identity();
        let dictionary = fixture(&directory.0);
        drop(dictionary);
        let words = directory.0.join(format!("{}.words", id.key()));
        let before = fs::read(&words).unwrap();
        let sentinel = directory.0.join("unrelated");
        fs::write(&sentinel, b"preserved").unwrap();
        for (dir_mode, file_mode) in [(0o775, 0o600), (0o700, 0o664)] {
            fs::set_permissions(&directory.0, fs::Permissions::from_mode(dir_mode)).unwrap();
            fs::set_permissions(&words, fs::Permissions::from_mode(file_mode)).unwrap();
            assert!(open_verified(&words, &id)
                .err()
                .unwrap()
                .contains("write access"));
            let result = build_with_batches(
                None,
                &directory.0,
                &id,
                &AtomicBool::new(false),
                &writable(&id),
                |_| panic!("insecure cache input must not start a build"),
            );
            assert!(result.unwrap_err().contains("write access"));
            assert_eq!(fs::read(&words).unwrap(), before);
            assert_eq!(fs::read(&sentinel).unwrap(), b"preserved");
            assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
            assert_eq!(
                fs::metadata(&directory.0).unwrap().permissions().mode() & 0o777,
                dir_mode
            );
            assert_eq!(
                fs::metadata(&words).unwrap().permissions().mode() & 0o777,
                file_mode
            );
        }
    }

    #[test]
    fn cancellation_never_publishes_or_replaces_verified_cache() {
        let directory = TestDirectory::new();
        let identity = identity();
        let shared = writable(&identity);
        let cancel = AtomicBool::new(true);
        assert!(
            build_with_batches(None, &directory.0, &identity, &cancel, &shared, |_| panic!(
                "pre-cancelled build must not enumerate"
            ))
            .is_err()
        );
        assert!(!directory.0.exists());
        cancel.store(false, Ordering::Release);
        build_with_batches(None, &directory.0, &identity, &cancel, &shared, |_| {
            Ok(TitleBatch {
                titles: vec!["Planet".into()],
                total: 1,
            })
        })
        .unwrap();
        let path = directory.0.join(format!("{}.words", identity.key()));
        let before = fs::read(&path).unwrap();
        let result = build_with_batches(None, &directory.0, &identity, &cancel, &shared, |_| {
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
        open_directory(&directory.0, true).unwrap();
        let path = directory.0.join(format!("{}.words", identity.key()));
        fs::write(&path, b"not a database").unwrap();
        let manager = SpellingManager::new(directory.0.clone());
        assert!(manager.dictionary(&identity).unwrap().ready().is_none());
        finish_load(&manager, &identity);
        assert!(manager.dictionary(&identity).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not a database");
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"KIWISPL2").unwrap();
        assert!(open_verified(&path, &identity).is_err());
        fs::remove_file(&path).unwrap();
        let result = build_with_batches(
            None,
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

    /// Run against the pinned English Climate mini archive. This exercises the
    /// actual archive-title producer and the consumer's zero-ordinary-hit gate.
    #[test]
    #[ignore = "requires KIWIX_SPELLING_TEST_ZIM pointing to the English Climate archive"]
    fn real_archive_build_correct_search_and_reopen() {
        let source = std::env::var_os("KIWIX_SPELLING_TEST_ZIM").expect("KIWIX_SPELLING_TEST_ZIM");
        let archive = Archive::open(Path::new(&source)).unwrap();
        let identity =
            Identity::from_archive(&archive, &archive.metadata().unwrap(), "test-installation")
                .unwrap();
        let dir = TestDirectory::new();
        let manager = Arc::new(SpellingManager::new(dir.0.clone()));
        manager.set_writable(true).unwrap();
        assert_eq!(
            manager
                .build(archive.clone(), identity.clone())
                .unwrap()
                .state,
            "building"
        );
        let job = manager.inner.lock().unwrap().job.take().unwrap();
        job.handle.join().unwrap();
        let status = manager.status(&identity).unwrap();
        assert_eq!(status.state, "ready", "{:?}", status.error);
        assert_eq!((status.total, status.words), (3821, 4204));
        let none = archive.search("climtae", false, 0, 5).unwrap();
        assert_eq!(none.total, 0);
        let token = tokio_util::sync::CancellationToken::new();
        for stage in 0..2 {
            let current = if stage == 0 {
                manager.clone()
            } else {
                let reloaded = Arc::new(SpellingManager::new(dir.0.clone()));
                assert!(reloaded.dictionary(&identity).unwrap().ready().is_none());
                finish_load(&reloaded, &identity);
                reloaded
            };
            let guesses = current
                .corrections(&identity, "climtae", none.total, true, &token)
                .unwrap();
            assert_eq!(guesses[0].corrected_query, "climate");
            let hits = archive
                .search(&guesses[0].corrected_query, false, 0, 5)
                .unwrap();
            assert!(!hits.hits.is_empty() && hits.total > 0);
            assert!(current
                .corrections(&identity, "climate", hits.total, true, &token)
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn reopened_archive_replacement_cannot_reuse_spelling_from_the_same_registration() {
        let profile = TestDirectory::new();
        open_directory(&profile.0, true).unwrap();
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let stored = profile.0.join("book.zim");
        fs::copy(&input, &stored).unwrap();
        let original = Archive::open(&stored).unwrap();
        let metadata = original.metadata().unwrap();
        let first = Identity::from_archive(&original, &metadata, "same-registration").unwrap();
        let reopened = Archive::open(&stored).unwrap();
        assert_eq!(
            first,
            Identity::from_archive(&reopened, &metadata, "same-registration").unwrap()
        );
        let cache = profile.0.join("spelling");
        build_archive(
            &original,
            &cache,
            &first,
            &AtomicBool::new(false),
            &writable(&first),
        )
        .unwrap();
        drop(original);
        drop(reopened);
        fs::rename(&stored, profile.0.join("previous.zim")).unwrap();
        fs::copy(&input, &stored).unwrap();
        let replacement = Archive::open(&stored).unwrap();
        let next = Identity::from_archive(&replacement, &metadata, "same-registration").unwrap();
        assert_ne!(first.key(), next.key());
        assert!(first.confirm(&replacement).is_err());
        let manager = SpellingManager::new(cache);
        assert_eq!(manager.status(&next).unwrap().state, "missing");
        assert!(manager.dictionary(&next).unwrap().ready().is_none());
    }

    #[test]
    fn replacement_installation_cannot_reuse_disk_or_memory_dictionary() {
        let directory = TestDirectory::new();
        open_directory(&directory.0, true).unwrap();
        let first = identity();
        let mut replacement = first.clone();
        replacement.installation_id = "replacement-installation".into();
        let _built = fixture(&directory.0);
        let manager = SpellingManager::new(directory.0.clone());
        assert!(manager.dictionary(&first).unwrap().ready().is_none());
        finish_load(&manager, &first);
        assert!(manager.dictionary(&first).unwrap().ready().is_some());
        assert_eq!(manager.status(&replacement).unwrap().state, "missing");
        assert!(manager.dictionary(&replacement).unwrap().ready().is_none());
        let first_file = directory.0.join(format!("{}.words", first.key()));
        assert!(open_verified(&first_file, &replacement).is_err());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
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
        let installation_id = identity.installation_id.clone();
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
                installation_id,
                source_key: identity.key(),
                cancel,
                handle,
            });
        }
        manager
            .retire_installation("replacement-installation")
            .unwrap();
        assert!(manager
            .cancel("replacement-installation", &identity.key())
            .is_err());
        assert!(manager.inner.lock().unwrap().job.is_some());
        manager.set_writable(false).unwrap();
        let inner = manager.inner.lock().unwrap();
        assert!(!inner.writable);
        assert!(inner.job.is_none());
        assert_eq!(inner.statuses[&identity.key()].state, "cancelled");
        assert!(!directory.0.exists());
    }

    #[test]
    fn sweep_removes_only_cache_files_that_no_live_entry_uses() {
        let profile = TestDirectory::new();
        fs::create_dir_all(&profile.0).unwrap();
        let directory = profile.0.join("spelling");
        drop(fixture(&directory));
        let live = identity();
        let mut removed = identity();
        removed.installation_id = "removed-installation".into();
        let words = |identity: &Identity| directory.join(format!("{}.words", identity.key()));
        fs::copy(words(&live), words(&removed)).unwrap();
        let run = uuid::Uuid::new_v4();
        let names = [
            format!(".{}.{run}.previous", removed.key()),
            format!(".{}.{run}.previous", live.key()),
            format!(".{}.{run}.tmp", live.key()),
            format!(".spelling-{run}.run"),
            // Not names this module creates: never touched.
            "notes.words".to_string(),
            format!("{}.sqlite", removed.key()),
            format!("{}.words.bak", removed.key()),
            format!(".{}.{run}.other", removed.key()),
        ];
        for name in &names {
            fs::write(directory.join(name), b"cache").unwrap();
        }
        fs::create_dir(directory.join(format!("{}.words", "b".repeat(64)))).unwrap();
        #[cfg(unix)]
        let outside = {
            let outside = profile.0.join("outside");
            fs::write(&outside, b"outside").unwrap();
            std::os::unix::fs::symlink(&outside, directory.join(format!("{}.words", "c".repeat(64)))).unwrap();
            outside
        };
        let manager = SpellingManager::new(directory.clone());
        // Caches are not writable (private or read-only mode): nothing runs.
        assert_eq!(manager.sweep(&[]).unwrap(), 0);
        manager.set_writable(true).unwrap();
        assert_eq!(manager.sweep(std::slice::from_ref(&live)).unwrap(), 4);
        assert!(words(&live).is_file());
        assert!(!words(&removed).exists());
        assert!(directory.join(&names[1]).is_file(), "a live entry's backup is kept for recovery");
        for name in &names[4..] {
            assert!(directory.join(name).is_file(), "{name}");
        }
        assert!(directory.join(format!("{}.words", "b".repeat(64))).is_dir());
        #[cfg(unix)]
        {
            assert!(fs::symlink_metadata(directory.join(format!("{}.words", "c".repeat(64)))).is_ok());
            assert_eq!(fs::read(&outside).unwrap(), b"outside");
        }

        // A legacy dictionary keeps explaining "rebuild" until a current one exists.
        let legacy = directory.join(format!("{}.words", live.unbound_key(2)));
        fs::write(&legacy, b"legacy").unwrap();
        fs::remove_file(words(&live)).unwrap();
        assert_eq!(manager.sweep(std::slice::from_ref(&live)).unwrap(), 0);
        assert!(legacy.is_file());

        // No live entries: every cache file goes, nothing else does.
        assert_eq!(manager.sweep(&[]).unwrap(), 2);
        assert!(!legacy.exists());
        assert!(!directory.join(&names[1]).exists());
        for name in &names[4..] {
            assert!(directory.join(name).is_file(), "{name}");
        }
    }

    #[test]
    fn sweep_name_accepts_only_this_modules_cache_names() {
        let key = "0123456789abcdef".repeat(4);
        let run = uuid::Uuid::new_v4();
        assert!(matches!(sweep_name(&format!("{key}.words")), Some(SweepName::Dictionary(found)) if found == key));
        assert!(matches!(sweep_name(&format!(".{key}.{run}.previous")), Some(SweepName::Backup(found)) if found == key));
        assert!(matches!(sweep_name(&format!(".{key}.{run}.tmp")), Some(SweepName::Staging)));
        assert!(matches!(sweep_name(&format!(".spelling-{run}.run")), Some(SweepName::Staging)));
        for name in [
            format!("{}.words", key.to_uppercase()),
            format!("{}.words", &key[1..]),
            format!("{key}.sqlite"),
            format!(".{key}.{}.tmp", run.simple()),
            format!(".{key}.{run}.tmp.extra"),
            format!("{key}.{run}.tmp"),
            ".spelling-x.run".to_string(),
            "../escape.words".to_string(),
        ] {
            assert!(sweep_name(&name).is_none(), "{name}");
        }
    }
}
