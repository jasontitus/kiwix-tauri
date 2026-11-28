// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only acquisition verification. Only the service may register verified books.
//!
//! A single worker owns archive checks. Admission
//! retirement invalidates queued/late results immediately; terminal retirement
//! additionally joins that worker before native provider resources are released.
//! There are no profile writes, events, receipt commits, or private caches here.
#![forbid(unsafe_code)]
use crate::{
    archive::Archive,
    store::{new_installation_id, now, Book},
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const MAX_QUEUED: usize = 64;

/// Admission failure is split so a full queue stays a per-job condition: the
/// caller keeps the row visible and retries on a later poll instead of
/// abandoning the whole Downloads list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnqueueError {
    Full,
    Rejected(String),
}

impl std::fmt::Display for EnqueueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => formatter.write_str("Archive verification queue is full"),
            Self::Rejected(error) => formatter.write_str(error),
        }
    }
}

impl From<EnqueueError> for String {
    fn from(error: EnqueueError) -> Self {
        error.to_string()
    }
}

pub(crate) fn load_book(path: &str, managed: bool, verify: bool) -> Result<Book, String> {
    load_book_inner(path, managed, verify, None)
}

/// `load_book` whose verification stops at the next bounded read once `cancel`
/// fires. The caller decides what a cancelled error means: this returns the
/// reader's own error text, never a claim that the file is damaged.
pub(crate) fn load_book_cancellable(
    path: &str,
    managed: bool,
    verify: bool,
    cancel: &CancellationToken,
) -> Result<Book, String> {
    load_book_inner(path, managed, verify, Some(cancel)).map_err(|error| {
        if cancel.is_cancelled() { IMPORT_CANCELLED.to_owned() } else { error }
    })
}

/// A verification stopped by Quit, a privacy or read-only switch, or (mobile)
/// the app moving to the background.
pub(crate) const IMPORT_CANCELLED: &str =
    "Import cancelled because Kiwix closed, moved to the background, or switched privacy mode. Add the archive again to import it.";

fn load_book_inner(path: &str, managed: bool, verify: bool, cancel: Option<&CancellationToken>) -> Result<Book, String> {
    let canonical = std::fs::canonicalize(path).map_err(|failure| {
        crate::archive::archive_open_error(
            std::path::Path::new(path),
            &format!("Opening archive path: {failure}"),
        )
    })?;
    let archive = Archive::open(&canonical)?;
    if verify {
        let intact = match cancel {
            Some(token) => archive.check_integrity_cancellable(token, |_, _| {}),
            None => archive.check_integrity(),
        }.map_err(crate::archive::integrity_failure)?;
        if !intact {
            // One sentence for both downloaded and manually chosen archives:
            // the failure must say the file was refused, not that it was added.
            return Err(crate::archive::CORRUPT_ARCHIVE.into());
        }
    }
    Ok(Book {
        metadata: match cancel {
            Some(token) => archive.metadata_cancellable(token)?,
            None => archive.metadata()?,
        },
        installation_id: new_installation_id(),
        path: canonical.to_string_lossy().into_owned(),
        added_at: now(),
        available: true,
        managed,
        source_reference: None,
        integrity: None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ImportState {
    Queued,
    Verifying,
    Ready,
    Failed,
    Cancelled,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportStatus {
    #[serde(serialize_with = "serialize_arc")]
    pub(crate) receipt: Arc<str>,
    #[serde(serialize_with = "serialize_arc")]
    pub(crate) paths: Arc<[String]>,
    pub(crate) state: ImportState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) verified_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) verified_total: u64,
}

fn serialize_arc<T: ?Sized + Serialize, S: serde::Serializer>(
    value: &Arc<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_ref().serialize(serializer)
}

struct Entry {
    cancel: CancellationToken,
    status: ImportStatus,
    books: Option<Arc<[Book]>>,
    sources: Option<Arc<[crate::archive_source::RetainedSource]>>,
    publication: Option<Result<Arc<crate::acquisition::ImportPublication>, String>>,
    identities: Option<Arc<[VerifiedIdentity]>>,
}

/// Volatile content-change proof, kept only with the verifier's open handles.
/// Unix ContentIdentity includes ctime even if mtime is restored. Windows
/// retained filesystem handles deny FILE_SHARE_WRITE for this entire lifetime.
/// The persisted library/cleanup stamp formats remain unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VerifiedIdentity {
    content: crate::integrity::ContentIdentity,
}

impl VerifiedIdentity {
    fn capture(source: &crate::archive_source::RetainedSource) -> Result<Self, String> {
        Ok(Self {
            content: source.identity()?,
        })
    }

    fn unchanged(&self, source: &crate::archive_source::RetainedSource) -> Result<(), String> {
        source.unchanged()?;
        if Self::capture(source)? != *self {
            return Err("Archive changed after verification; retry the import".into());
        }
        Ok(())
    }
}


#[derive(Default)]
struct State {
    stopped: bool,
    queue: VecDeque<String>,
    entries: HashMap<String, Entry>,
    retired: HashSet<String>,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    changed: Notify,
    #[cfg(test)]
    hooks: Mutex<TestHooks>,
}

/// A cheap ownership ticket, not an authorization to commit. Metadata stays in
/// an Arc, so polling ready work never clones archive metadata. Use with_ready
/// under the service's global writable-mode guard before accessing its books.
pub(crate) struct ReadyImport {
    receipt: String,
    books: Arc<[Book]>,
    sources: Arc<[crate::archive_source::RetainedSource]>,
    publication: Option<Arc<crate::acquisition::ImportPublication>>,
    identities: Arc<[VerifiedIdentity]>,
}

impl ReadyImport {
    pub(crate) fn receipt(&self) -> &str {
        &self.receipt
    }

    pub(crate) fn sources_unchanged(&self) -> Result<(), String> {
        if self.sources.len() != self.identities.len() {
            return Err("Archive verification omitted its retained content identities".into());
        }
        self.sources.iter().zip(self.identities.iter())
            .try_for_each(|(source, identity)| identity.unchanged(source))
    }
}

enum WorkerRetirement {
    Running(Option<std::thread::JoinHandle<()>>),
    Joining(tokio::task::JoinHandle<Result<(), String>>),
    Complete(Result<(), String>),
}

pub(crate) struct ImportManager {
    shared: Arc<Shared>,
    retirement: tokio::sync::Mutex<WorkerRetirement>,
}

impl ImportManager {
    pub(crate) fn new() -> Result<Self, String> {
        Self::start(Arc::new(Shared {
            state: Mutex::new(State::default()),
            work: Condvar::new(),
            changed: Notify::new(),
            #[cfg(test)]
            hooks: Mutex::new(TestHooks::default()),
        }))
    }

    fn start(shared: Arc<Shared>) -> Result<Self, String> {
        let worker = shared.clone();
        let handle = std::thread::Builder::new()
            .name("archive-integrity".into())
            .spawn(move || {
                verify_pending(&worker);
                #[cfg(test)]
                if let Some(exited) = worker.hooks.lock().unwrap().exited.take() {
                    let _ = exited.send(());
                }
            })
            .map_err(|e| format!("Starting archive integrity worker: {e}"))?;
        Ok(Self {
            shared,
            retirement: tokio::sync::Mutex::new(WorkerRetirement::Running(Some(handle))),
        })
    }

    #[cfg(test)]
    pub(crate) fn block_before_read(&self) -> (
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::SyncSender<()>,
    ) {
        let (entered, entering) = std::sync::mpsc::sync_channel(1);
        let (release, releasing) = std::sync::mpsc::sync_channel(1);
        self.shared.hooks.lock().unwrap().before_read = Some(TestGate { entered, release: releasing });
        (entering, release)
    }

    /// Receipt identity is stable: a duplicate cannot replace its paths or
    /// restart queued, active, ready, or failed work. Only retry restarts errors.
    pub(crate) fn enqueue(
        &self,
        receipt: &str,
        paths: Vec<String>,
    ) -> Result<ImportStatus, EnqueueError> {
        self.enqueue_inner(receipt, paths, None)
    }

    pub(crate) fn enqueue_publication(
        &self, receipt: &str, paths: Vec<String>,
        publication: Result<crate::acquisition::ImportPublication, String>,
    ) -> Result<ImportStatus, EnqueueError> {
        self.enqueue_inner(receipt, paths, Some(publication))
    }

    fn enqueue_inner(
        &self, receipt: &str, paths: Vec<String>,
        publication: Option<Result<crate::acquisition::ImportPublication, String>>,
    ) -> Result<ImportStatus, EnqueueError> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(EnqueueError::Rejected(
                "Archive verification is shut down".into(),
            ));
        }
        if let Some(entry) = state.entries.get(receipt) {
            if entry.status.paths.as_ref() != paths.as_slice() {
                return Err(EnqueueError::Rejected(
                    "Acquisition receipt already belongs to different archive paths".into(),
                ));
            }
            return Ok(entry.status.clone());
        }
        if state.retired.iter().any(|acquisition| belongs_to_acquisition(receipt, acquisition)) {
            return Ok(ImportStatus {
                receipt: receipt.into(),
                paths: paths.into(),
                state: ImportState::Cancelled,
                error: None,
                verified_bytes: 0,
                verified_total: 0,
            });
        }
        if receipt.is_empty() || paths.is_empty() {
            return Err(EnqueueError::Rejected(
                "Archive verification requires a receipt and archive paths".into(),
            ));
        }
        if state.queue.len() >= MAX_QUEUED {
            return Err(EnqueueError::Full);
        }
        let status = ImportStatus {
            receipt: receipt.into(),
            paths: paths.into(),
            state: ImportState::Queued,
            error: None,
            verified_bytes: 0,
            verified_total: 0,
        };
        state.entries.insert(
            receipt.into(),
            Entry {
                cancel: CancellationToken::new(),
                status: status.clone(),
                books: None,
                sources: None,
                publication: publication.map(|value| value.map(Arc::new)),
                identities: None,
            },
        );
        state.queue.push_back(receipt.into());
        self.shared.work.notify_one();
        self.shared.changed.notify_one();
        Ok(status)
    }

    pub(crate) fn status(&self, receipt: &str) -> Option<ImportStatus> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .get(receipt)
            .map(|entry| entry.status.clone())
    }

    /// Avoid loading large saved producer journals for a row that cannot enter
    /// the verifier queue. enqueue_inner still rechecks capacity atomically.
    pub(crate) fn has_admission_capacity(&self) -> Result<bool, String> {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped { return Err("Archive verification is shut down".into()); }
        Ok(state.queue.len() < MAX_QUEUED)
    }

    /// Revoke one acquisition and all of its publication receipts before any
    /// owned files are deleted. The tombstone also rejects an enqueue from a
    /// completed-job snapshot taken before retirement, even if no entry exists
    /// yet. In-flight verification observes the per-receipt cancellation token.
    /// Undo `retire_acquisition` when the deletion/forget it guarded was
    /// refused before touching the acquisition (for example, the job had been
    /// resumed from another window and is no longer terminal). Without this the
    /// tombstone outlives the refusal and the job's eventual completion can
    /// never be imported before restart. Cancelled entries for it are dropped
    /// so a fresh completion snapshot can enqueue again.
    pub(crate) fn unretire_acquisition(&self, acquisition: &str) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.retired.remove(acquisition) {
            return;
        }
        state.entries.retain(|receipt, entry| {
            !(belongs_to_acquisition(receipt, acquisition) && entry.status.state == ImportState::Cancelled)
        });
        self.shared.changed.notify_one();
    }

    /// Returns whether this call added the tombstone. Only that caller may
    /// `unretire_acquisition`: a concurrent deletion that found it already
    /// present must not lift another deletion's tombstone.
    pub(crate) fn retire_acquisition(&self, acquisition: &str) -> bool {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let inserted = state.retired.insert(acquisition.into());
        state.queue.retain(|receipt| !belongs_to_acquisition(receipt, acquisition));
        for (receipt, entry) in &mut state.entries {
            if belongs_to_acquisition(receipt, acquisition) {
                entry.cancel.cancel();
                entry.status.state = ImportState::Cancelled;
                entry.status.error = None;
                entry.books = None;
                entry.sources = None;
                entry.identities = None;
            }
        }
        self.shared.work.notify_one();
        self.shared.changed.notify_one();
        inserted
    }

    pub(crate) fn ready(&self) -> Vec<ReadyImport> {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state
            .entries
            .iter()
            .filter_map(|(receipt, entry)| {
                entry.books.as_ref().and_then(|books| Some(ReadyImport {
                    receipt: receipt.clone(),
                    books: books.clone(),
                    sources: entry.sources.as_ref()?.clone(),
                    publication: entry.publication.as_ref().and_then(|value| value.as_ref().ok()).cloned(),
                    identities: entry.identities.as_ref()?.clone(),
                }))
            })
            .collect()
    }

    /// The service must first hold its global mode guard and verify writable
    /// settings. The synchronous callback must only perform the short atomic
    /// Store registration + receipt commit, and must not reenter this manager.
    /// Holding this ownership guard makes shutdown linearize before or after
    /// registration, never between checking a ticket and committing its books.
    /// Registration errors become retryable per-receipt failures; they must not
    /// block unrelated acquisitions or hide the Downloads list.
    pub(crate) fn with_ready(
        &self,
        ready: &ReadyImport,
        register: impl FnOnce(&[Book]) -> Result<(), String>,
    ) -> Result<bool, String> {
        self.with_ready_publications(ready, |books, _, _| register(books))
    }

    /// Ordinary acquisition registration also needs the exact source handles
    /// so its producer can verify original publication authority. Failures stay
    /// on this receipt and do not hide unrelated completed downloads.
    pub(crate) fn with_ready_publications(
        &self,
        ready: &ReadyImport,
        register: impl FnOnce(&[Book], &[crate::archive_source::RetainedSource], Option<&crate::acquisition::ImportPublication>) -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped || !owns_ready(&state, ready) {
            return Err("Verified import is no longer available".into());
        }
        let registration = ready.sources_unchanged()
            .and_then(|()| register(&ready.books, &ready.sources, ready.publication.as_deref()));
        match registration {
            Ok(()) => Ok(true),
            Err(error) => {
                let entry = state.entries.get_mut(&ready.receipt).unwrap();
                entry.status.state = ImportState::Failed;
                entry.status.error = Some(error);
                entry.books = None;
                entry.sources = None;
                entry.identities = None;
                self.shared.changed.notify_one();
                Ok(false)
            }
        }
    }

    /// Edition cutover retains the verified descriptors and the publication
    /// verified by the worker throughout the isolated Store transaction. A
    /// failed admission does not itself discard verified output.
    pub(crate) fn with_ready_sources<T>(
        &self,
        ready: &ReadyImport,
        commit: impl FnOnce(&[Book], &[crate::archive_source::RetainedSource], Option<&crate::acquisition::ImportPublication>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped || !owns_ready(&state, ready) {
            return Err("Verified import is no longer available".into());
        }
        if let Err(error) = ready.sources_unchanged() {
            let entry = state.entries.get_mut(&ready.receipt).unwrap();
            entry.status.state = ImportState::Failed;
            entry.status.error = Some(error.clone());
            entry.books = None;
            entry.sources = None;
            entry.identities = None;
            self.shared.changed.notify_one();
            return Err(error);
        }
        commit(&ready.books, &ready.sources, ready.publication.as_deref())
    }

    /// A definitive edition artifact/publication mismatch is retryable
    /// verification failure, unlike an expired cutover authorization.
    pub(crate) fn fail_ready(&self, ready: &ReadyImport, error: &str) -> bool {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped || !owns_ready(&state, ready) { return false; }
        let entry = state.entries.get_mut(&ready.receipt).unwrap();
        entry.status.state = ImportState::Failed;
        entry.status.error = Some(error.into());
        entry.books = None;
        entry.sources = None;
        entry.identities = None;
        self.shared.changed.notify_one();
        true
    }

    /// Call only AFTER successful atomic library registration + receipt commit.
    /// Retained tickets cannot acknowledge another batch with the same receipt.
    /// Acknowledgement evicts the entry; the durable receipt is the service's
    /// authority for rejecting already consumed acquisitions after eviction.
    pub(crate) fn ack(&self, ready: &ReadyImport) -> bool {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !owns_ready(&state, ready) {
            return false;
        }
        state.entries.remove(&ready.receipt);
        self.shared.changed.notify_one();
        true
    }

    pub(crate) fn retry(&self, receipt: &str) -> Result<ImportStatus, String> {
        self.retry_inner(receipt, None)
    }

    pub(crate) fn retry_publication(
        &self, receipt: &str, publication: Result<crate::acquisition::ImportPublication, String>,
    ) -> Result<ImportStatus, String> {
        self.retry_inner(receipt, Some(publication))
    }

    fn retry_inner(
        &self, receipt: &str,
        publication: Option<Result<crate::acquisition::ImportPublication, String>>,
    ) -> Result<ImportStatus, String> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err("Archive verification is shut down".into());
        }
        if state.queue.len() >= MAX_QUEUED {
            return Err("Archive verification queue is full".into());
        }
        let entry = state
            .entries
            .get_mut(receipt)
            .ok_or("Unknown acquisition receipt")?;
        if entry.status.state != ImportState::Failed {
            return Err("Only failed archive verification can be retried".into());
        }
        if publication.is_some() { entry.publication = publication.map(|value| value.map(Arc::new)); }
        entry.status.state = ImportState::Queued;
        entry.status.error = None;
        entry.status.verified_bytes = 0;
        entry.status.verified_total = 0;
        let status = entry.status.clone();
        state.queue.push_back(receipt.into());
        self.shared.work.notify_one();
        self.shared.changed.notify_one();
        Ok(status)
    }

    /// Cancel active verification and invalidate queued and ready batches without joining.
    /// Already retained ReadyImport tickets are invalidated by with_ready.
    pub(crate) fn shutdown(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return;
        }
        state.stopped = true;
        state.queue.clear();
        for entry in state.entries.values_mut() {
            entry.cancel.cancel();
            if matches!(
                entry.status.state,
                ImportState::Queued | ImportState::Verifying | ImportState::Ready
            ) {
                entry.status.state = ImportState::Cancelled;
                entry.status.error = None;
            }
            entry.books = None;
            entry.sources = None;
            entry.identities = None;
        }
        self.shared.work.notify_one();
        self.shared.changed.notify_one();
    }

    /// Terminal retirement owns the join even if its awaiting future is dropped.
    /// Later callers await the same task rather than treating a taken handle as
    /// proof that the archive verification worker has finished.
    pub(crate) async fn shutdown_and_wait(&self) -> Result<(), String> {
        self.shutdown();
        let mut retirement = self.retirement.lock().await;
        if let WorkerRetirement::Running(worker) = &mut *retirement {
            let worker = worker.take().ok_or("Archive verification worker ownership missing")?;
            *retirement = WorkerRetirement::Joining(tokio::task::spawn_blocking(move || {
                worker.join().map_err(|_| "Archive verification worker panicked".to_string())
            }));
        }
        if let WorkerRetirement::Joining(worker) = &mut *retirement {
            let result = worker.await
                .map_err(|error| format!("Joining archive verification worker: {error}"))
                .and_then(|result| result);
            *retirement = WorkerRetirement::Complete(result);
        }
        match &*retirement {
            WorkerRetirement::Complete(result) => result.clone(),
            _ => Err("Archive verification worker did not retire".into()),
        }
    }

    /// Single maintenance consumer: notify_one retains a wake permit if a state
    /// transition happens before that consumer starts awaiting notified().
    #[cfg(any(desktop, test))]
    pub(crate) fn changed(&self) -> &Notify {
        &self.shared.changed
    }
}

impl Drop for ImportManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn belongs_to_acquisition(receipt: &str, acquisition: &str) -> bool {
    receipt == acquisition
        || receipt.strip_prefix(acquisition).is_some_and(|suffix| suffix.starts_with(':'))
}

fn owns_ready(state: &State, ready: &ReadyImport) -> bool {
    state
        .entries
        .get(&ready.receipt)
        .and_then(|entry| entry.books.as_ref())
        .is_some_and(|books| Arc::ptr_eq(books, &ready.books))
}

fn verify_pending(shared: &Shared) {
    loop {
        let (receipt, paths, publication, cancel) = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !state.stopped && state.queue.is_empty() {
                state = shared
                    .work
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.stopped {
                return;
            }
            let receipt = state.queue.pop_front().unwrap();
            let entry = state.entries.get_mut(&receipt).unwrap();
            entry.status.state = ImportState::Verifying;
            entry.status.verified_bytes = 0;
            entry.status.verified_total = 0;
            let paths = entry.status.paths.clone();
            shared.changed.notify_one();
            (receipt, paths, entry.publication.clone(), entry.cancel.clone())
        };
        #[cfg(test)]
        test_gate(shared, true);
        // Cancellation belongs to this admission, even if the same receipt is
        // retired and then re-enqueued. Never hold the state mutex over I/O.
        let mut books = Vec::with_capacity(paths.len());
        let mut sources = Vec::with_capacity(paths.len());
        let mut identities = Vec::with_capacity(paths.len());
        let mut error = publication.as_ref().and_then(|value| value.as_ref().err()).cloned();
        // Estimate remaining hashed bytes (ZIM checksum covers all bytes except
        // the trailing 16-byte MD5) so multi-archive batches advance monotonically
        // and reach exact `verified_bytes == verified_total` on the last file.
        let file_estimates: Vec<u64> = paths
            .iter()
            .map(|path| {
                std::fs::metadata(path)
                    .map(|metadata| metadata.len().saturating_sub(16))
                    .unwrap_or(0)
            })
            .collect();
        let mut completed_bytes = 0u64;
        for (file_index, path) in paths.iter().enumerate() {
            if error.is_some() { break; }
            if cancel.is_cancelled() { break; }
            #[cfg(test)]
            {
                shared.hooks.lock().unwrap().reads += 1;
            }
            let file_base = completed_bytes;
            let remaining_after: u64 = file_estimates
                .iter()
                .skip(file_index + 1)
                .copied()
                .fold(0u64, u64::saturating_add);
            let mut file_total = 0u64;
            let verified = (|| {
                let mut book = load_book_inner(path, true, false, Some(&cancel))?;
                let source = crate::archive_source::retain(&book)?;
                let identity = VerifiedIdentity::capture(&source)?;
                if let Some(Ok(publication)) = &publication {
                    publication.verify(&source, || cancel.is_cancelled())?;
                }
                if cancel.is_cancelled() { return Err("Archive verification was cancelled".into()); }
                let archive = source.open()?;
                let mut last_progress = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(1))
                    .unwrap_or_else(std::time::Instant::now);
                let intact = archive.check_integrity_cancellable(&cancel, |checked, total| {
                    file_total = total;
                    #[cfg(test)]
                    checksum_progress(shared, checked, total);
                    if checked == total || last_progress.elapsed() >= std::time::Duration::from_millis(250) {
                        last_progress = std::time::Instant::now();
                        let reported_bytes = file_base.saturating_add(checked);
                        let reported_total = file_base
                            .saturating_add(total)
                            .saturating_add(remaining_after)
                            .max(reported_bytes);
                        let mut state = shared
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        if let Some(entry) = state.entries.get_mut(&receipt) {
                            if !cancel.is_cancelled() && entry.status.state == ImportState::Verifying {
                                entry.status.verified_bytes = reported_bytes;
                                entry.status.verified_total = reported_total;
                            }
                        }
                    }
                }).map_err(|error| {
                    if cancel.is_cancelled() { error } else { crate::archive::integrity_failure(error) }
                })?;
                if !intact {
                    return Err(crate::archive::CORRUPT_ARCHIVE.into());
                }
                book.metadata = archive.metadata_cancellable(&cancel)?;
                identity.unchanged(&source)?;
                Ok((book, source, identity))
            })();
            match verified {
                Ok((book, source, identity)) => {
                    completed_bytes = completed_bytes.saturating_add(file_total);
                    books.push(book);
                    sources.push(source);
                    identities.push(identity);
                }
                Err(failure) => {
                    error = Some(failure);
                    break;
                }
            }
        }
        #[cfg(test)]
        test_gate(shared, false);
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return;
        }
        let Some(entry) = state.entries.get_mut(&receipt) else {
            continue;
        };
        if entry.status.state != ImportState::Verifying || cancel.is_cancelled() {
            continue;
        }
        if let Some(error) = error {
            entry.status.state = ImportState::Failed;
            entry.status.error = Some(error);
        } else {
            entry.books = Some(books.into());
            entry.sources = Some(sources.into());
            entry.identities = Some(identities.into());
            entry.status.state = ImportState::Ready;
            entry.status.error = None;
        }
        shared.changed.notify_one();
    }
}

#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    before_read: Option<TestGate>,
    during_checksum: Option<TestGate>,
    finished_checksums: usize,
    after_read: Option<TestGate>,
    exited: Option<std::sync::mpsc::SyncSender<()>>,
    reads: usize,
}

#[cfg(test)]
struct TestGate {
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
fn checksum_progress(shared: &Shared, checked: u64, total: u64) {
    let gate = {
        let mut hooks = shared.hooks.lock().unwrap();
        if checked == total { hooks.finished_checksums += 1; }
        hooks.during_checksum.take()
    };
    if let Some(gate) = gate {
        let _ = gate.entered.send(());
        let _ = gate.release.recv();
    }
}

#[cfg(test)]
fn test_gate(shared: &Shared, before: bool) {
    let gate = {
        let mut hooks = shared.hooks.lock().unwrap();
        if before {
            hooks.before_read.take()
        } else {
            hooks.after_read.take()
        }
    };
    if let Some(gate) = gate {
        let _ = gate.entered.send(());
        let _ = gate.release.recv();
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_refused_deletion_releases_its_acquisition_tombstone() {
        let manager = ImportManager::new().unwrap();
        assert!(manager.retire_acquisition("job"));
        // A concurrent second deletion of the same job does not own it.
        assert!(!manager.retire_acquisition("job"));
        assert_eq!(manager.enqueue("job:archive", paths()).unwrap().state, ImportState::Cancelled);
        // The deletion was refused (the job is running again): its later
        // completion must still be importable.
        manager.unretire_acquisition("job");
        assert_eq!(manager.enqueue("job:archive", paths()).unwrap().state, ImportState::Queued);
        // Unrelated tombstones are untouched.
        manager.retire_acquisition("other");
        manager.unretire_acquisition("job");
        assert_eq!(manager.enqueue("other:archive", paths()).unwrap().state, ImportState::Cancelled);
        manager.shutdown_and_wait().await.unwrap();
    }

    use super::*;
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    const DEADLINE: Duration = Duration::from_secs(10);

    struct TempArchive(PathBuf);
    impl TempArchive {
        fn corrupt() -> Self {
            let path =
                std::env::temp_dir().join(format!("kiwix-import-{}.zim", uuid::Uuid::new_v4()));
            let mut bytes = std::fs::read(fixture()).unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
        fn paths(&self) -> Vec<String> {
            vec![self.0.to_string_lossy().into_owned()]
        }
    }
    impl Drop for TempArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")
    }
    fn paths() -> Vec<String> {
        vec![fixture().to_string_lossy().into_owned()]
    }
    fn gate() -> (TestGate, mpsc::Receiver<()>, mpsc::SyncSender<()>) {
        let (entered, entering) = mpsc::sync_channel(1);
        let (release, releasing) = mpsc::sync_channel(1);
        (
            TestGate {
                entered,
                release: releasing,
            },
            entering,
            release,
        )
    }
    fn manager(hooks: TestHooks) -> ImportManager {
        ImportManager::start(Arc::new(Shared {
            state: Mutex::new(State::default()),
            work: Condvar::new(),
            changed: Notify::new(),
            hooks: Mutex::new(hooks),
        }))
        .unwrap()
    }
    async fn wait_for(
        manager: &ImportManager,
        receipt: &str,
        expected: ImportState,
    ) -> ImportStatus {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let changed = manager.changed().notified();
                if let Some(status) = manager.status(receipt) {
                    if status.state == expected {
                        return status;
                    }
                }
                changed.await;
            }
        })
        .await
        .unwrap()
    }

    #[test]
    fn missing_import_path_reports_the_classified_message() {
        let missing = std::env::temp_dir()
            .join(format!("kiwix-missing-{}.zim", uuid::Uuid::new_v4()));
        let error = load_book(&missing.to_string_lossy(), true, true).unwrap_err();
        assert_eq!(error, crate::archive::MISSING_ARCHIVE);
    }

    #[test]
    fn actual_fixture_metadata_and_embedded_checksum_are_checked() {
        let book = load_book(&paths()[0], true, true).unwrap();
        assert!(book.available && book.managed);
        assert!(!book.metadata.id.is_empty());
        assert_eq!(
            book.path,
            std::fs::canonicalize(fixture()).unwrap().to_string_lossy()
        );
        let corrupt = TempArchive::corrupt();
        let error = load_book(&corrupt.paths()[0], true, true).unwrap_err();
        assert_eq!(error, crate::archive::CORRUPT_ARCHIVE);
        // The same archive remains openable: this is an embedded-checksum failure,
        // not a stand-in malformed-file/open error.
        assert!(load_book(&corrupt.paths()[0], false, false).is_ok());
    }

    #[tokio::test]
    async fn changed_ready_source_fails_only_its_receipt_and_can_retry() {
        let temporary = TempArchive(std::env::temp_dir().join(format!("kiwix-import-changed-{}.zim", uuid::Uuid::new_v4())));
        std::fs::copy(fixture(), &temporary.0).unwrap();
        let manager = ImportManager::new().unwrap();
        manager.enqueue("changed", temporary.paths()).unwrap();
        manager.enqueue("unrelated", paths()).unwrap();
        wait_for(&manager, "unrelated", ImportState::Ready).await;
        let changed = manager.ready().into_iter().find(|ready| ready.receipt() == "changed").unwrap();
        // Keep the verified inode open, but replace its directory entry with a
        // different file. A matching pathname must not authorize registration.
        let replacement = temporary.0.with_extension("replacement");
        std::fs::copy(fixture(), &replacement).unwrap();
        std::fs::remove_file(&temporary.0).unwrap();
        std::fs::rename(&replacement, &temporary.0).unwrap();
        assert!(!manager.with_ready(&changed, |_| panic!("Changed source must not register")).unwrap());
        let failed = manager.status("changed").unwrap();
        assert_eq!(failed.state, ImportState::Failed);
        assert!(failed.error.is_some());
        assert!(manager.shared.state.lock().unwrap().entries["changed"].sources.is_none());
        let unrelated = manager.ready().into_iter().find(|ready| ready.receipt() == "unrelated").unwrap();
        assert!(manager.with_ready(&unrelated, |_| Ok(())).unwrap());
        assert!(manager.ack(&unrelated));
        manager.retry("changed").unwrap();
        wait_for(&manager, "changed", ImportState::Ready).await;
        let retry = manager.ready().into_iter().find(|ready| ready.receipt() == "changed").unwrap();
        assert!(manager.with_ready(&retry, |_| Ok(())).unwrap());
        manager.shutdown();
    }

    #[tokio::test]
    async fn edition_source_changes_are_retryable_but_admission_errors_retain_ready() {
        let temporary = TempArchive(std::env::temp_dir().join(format!("kiwix-edition-import-{}.zim", uuid::Uuid::new_v4())));
        std::fs::copy(fixture(), &temporary.0).unwrap();
        let manager = ImportManager::new().unwrap();
        manager.enqueue("edition", temporary.paths()).unwrap();
        wait_for(&manager, "edition", ImportState::Ready).await;
        let ready = manager.ready().into_iter().find(|ready| ready.receipt() == "edition").unwrap();
        let denied: Result<(), String> = manager.with_ready_sources(&ready, |_, _, proof| {
            assert!(proof.is_none(), "A generic import does not confer publication authority");
            Err("Authorization expired".into())
        });
        assert!(denied.is_err());
        assert_eq!(manager.status("edition").unwrap().state, ImportState::Ready);
        assert!(manager.with_ready_sources(&ready, |_, _, _| Ok(())).is_ok());
        let replacement = temporary.0.with_extension("replacement");
        std::fs::copy(fixture(), &replacement).unwrap();
        std::fs::remove_file(&temporary.0).unwrap();
        std::fs::rename(&replacement, &temporary.0).unwrap();
        let changed: Result<(), String> = manager.with_ready_sources(&ready, |_, _, _| panic!("Changed edition must not commit"));
        assert!(changed.is_err());
        assert_eq!(manager.status("edition").unwrap().state, ImportState::Failed);
        assert!(manager.shared.state.lock().unwrap().entries["edition"].sources.is_none());
        manager.retry("edition").unwrap();
        wait_for(&manager, "edition", ImportState::Ready).await;
        let retry = manager.ready().into_iter().find(|ready| ready.receipt() == "edition").unwrap();
        assert!(manager.with_ready_sources(&retry, |_, _, _| Ok(())).is_ok());
        manager.shutdown();
    }

    #[tokio::test]
    async fn queued_active_and_ready_duplicates_do_not_repeat_verification() {
        let (before_read, entered, release) = gate();
        let manager = manager(TestHooks {
            before_read: Some(before_read),
            ..TestHooks::default()
        });
        let queued = manager.enqueue("first", paths()).unwrap();
        assert_eq!(
            serde_json::to_value(&queued).unwrap(),
            serde_json::json!({
                "receipt": "first", "paths": paths(), "state": "queued"
            })
        );
        entered.recv_timeout(DEADLINE).unwrap();
        assert_eq!(
            manager.enqueue("first", paths()).unwrap().state,
            ImportState::Verifying
        );
        manager.enqueue("second", paths()).unwrap();
        assert_eq!(
            manager.enqueue("second", paths()).unwrap().state,
            ImportState::Queued
        );
        assert!(manager
            .enqueue("first", vec!["different.zim".into()])
            .is_err());
        release.send(()).unwrap();
        wait_for(&manager, "second", ImportState::Ready).await;
        assert_eq!(
            manager.enqueue("first", paths()).unwrap().state,
            ImportState::Ready
        );
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 2);
        let first = manager
            .ready()
            .into_iter()
            .find(|ready| ready.receipt() == "first")
            .unwrap();
        let same = manager
            .ready()
            .into_iter()
            .find(|ready| ready.receipt() == "first")
            .unwrap();
        assert!(Arc::ptr_eq(&first.books, &same.books));
        manager
            .with_ready(&first, |books| {
                // The ownership guard must span the commit, not just its precheck.
                assert!(manager.shared.state.try_lock().is_err());
                assert_eq!(books.len(), 1);
                assert!(!books[0].metadata.id.is_empty());
                Ok(())
            })
            .unwrap();
        assert!(manager.ack(&first));
        assert!(!manager.ack(&same));
        assert!(manager.status("first").is_none());
        assert!(manager.with_ready(&same, |_| Ok(())).is_err());
        // A stale ticket cannot evict a newly verified batch with the same id.
        manager.enqueue("first", paths()).unwrap();
        wait_for(&manager, "first", ImportState::Ready).await;
        assert!(!manager.ack(&same));
        assert_eq!(manager.status("first").unwrap().state, ImportState::Ready);
    }

    #[tokio::test]
    async fn checksum_error_is_retained_until_explicit_retry() {
        let corrupt = TempArchive::corrupt();
        let manager = ImportManager::new().unwrap();
        manager.enqueue("damaged", corrupt.paths()).unwrap();
        let failed = wait_for(&manager, "damaged", ImportState::Failed).await;
        assert_eq!(failed.error.as_deref(), Some(crate::archive::CORRUPT_ARCHIVE));
        assert!(manager.ready().is_empty());
        std::fs::copy(fixture(), &corrupt.0).unwrap();
        for _ in 0..5 {
            assert_eq!(manager.enqueue("damaged", corrupt.paths()).unwrap(), failed);
            assert_eq!(manager.status("damaged").unwrap(), failed);
        }
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 1);
        manager.retry("damaged").unwrap();
        wait_for(&manager, "damaged", ImportState::Ready).await;
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 2);
        assert!(manager.retry("damaged").is_err());
    }

    #[tokio::test]
    async fn one_bad_checksum_rejects_the_entire_acquisition_batch() {
        let corrupt = TempArchive::corrupt();
        let manager = ImportManager::new().unwrap();
        let mut batch = paths();
        batch.extend(corrupt.paths());
        manager.enqueue("batch", batch).unwrap();
        wait_for(&manager, "batch", ImportState::Failed).await;
        assert!(manager.ready().is_empty());
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 2);
        assert!(manager.shared.state.lock().unwrap().entries["batch"]
            .books
            .is_none());
    }

    #[tokio::test]
    async fn acquisition_retirement_revokes_active_queued_and_late_publications_only() {
        let (after_read, entered, release) = gate();
        let manager = manager(TestHooks {
            after_read: Some(after_read),
            ..TestHooks::default()
        });
        manager.enqueue("torrent:one:first", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        manager.enqueue("torrent:one:second", paths()).unwrap();
        manager.enqueue("torrent:one-other:first", paths()).unwrap();
        manager.retire_acquisition("torrent:one");
        manager.retire_acquisition("torrent:one");
        for receipt in ["torrent:one:first", "torrent:one:second"] {
            assert_eq!(manager.status(receipt).unwrap().state, ImportState::Cancelled);
            assert!(manager.retry(receipt).is_err());
        }
        // A publication snapshot taken before deletion must not reestablish
        // import authority after the deletion, even if it was never queued.
        assert_eq!(
            manager.enqueue("torrent:one:late", paths()).unwrap().state,
            ImportState::Cancelled
        );
        release.send(()).unwrap();
        wait_for(&manager, "torrent:one-other:first", ImportState::Ready).await;
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 2);
        assert_eq!(manager.ready().len(), 1);
        assert_eq!(manager.ready()[0].receipt(), "torrent:one-other:first");
        assert_eq!(manager.status("torrent:one:first").unwrap().state, ImportState::Cancelled);
        manager.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn acquisition_retirement_invalidates_retained_ready_tickets_without_stopping_others() {
        let manager = ImportManager::new().unwrap();
        manager.enqueue("deleted", paths()).unwrap();
        wait_for(&manager, "deleted", ImportState::Ready).await;
        let ready = manager.ready().pop().unwrap();
        manager.retire_acquisition("deleted");
        let mut committed = false;
        assert!(manager.with_ready(&ready, |_| {
            committed = true;
            Ok(())
        }).is_err());
        assert!(!committed);
        assert!(!manager.ack(&ready));
        assert!(manager.ready().is_empty());
        assert_eq!(manager.enqueue("deleted", paths()).unwrap().state, ImportState::Cancelled);
        manager.enqueue("kept", paths()).unwrap();
        wait_for(&manager, "kept", ImportState::Ready).await;
        assert_eq!(manager.ready().len(), 1);
        manager.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn retirement_before_the_first_read_skips_the_cancelled_batch() {
        let (before_read, entered, release) = gate();
        let manager = manager(TestHooks {
            before_read: Some(before_read),
            ..TestHooks::default()
        });
        manager.enqueue("deleted", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        manager.retire_acquisition("deleted");
        manager.enqueue("kept", paths()).unwrap();
        release.send(()).unwrap();
        wait_for(&manager, "kept", ImportState::Ready).await;
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 1);
        assert_eq!(manager.status("deleted").unwrap().state, ImportState::Cancelled);
        manager.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn stopping_imports_interrupts_the_current_checksum_before_join() {
        let (during_checksum, entered, release) = gate();
        let manager = manager(TestHooks { during_checksum: Some(during_checksum), ..TestHooks::default() });
        manager.enqueue("checking", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        manager.shutdown();
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), manager.shutdown_and_wait()).await.unwrap().unwrap();
        assert_eq!(manager.status("checking").unwrap().state, ImportState::Cancelled);
        assert_eq!(manager.shared.hooks.lock().unwrap().finished_checksums, 0);
        assert!(manager.ready().is_empty());
    }

    #[tokio::test]
    async fn retired_checksum_cannot_finish_or_publish_into_reused_receipt() {
        let (during_checksum, entered, release) = gate();
        let manager = manager(TestHooks { during_checksum: Some(during_checksum), ..TestHooks::default() });
        manager.enqueue("book:edition", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        manager.retire_acquisition("book");
        manager.unretire_acquisition("book");
        manager.enqueue("book:edition", paths()).unwrap();
        release.send(()).unwrap();
        wait_for(&manager, "book:edition", ImportState::Ready).await;
        assert_eq!(manager.shared.hooks.lock().unwrap().finished_checksums, 1);
        assert_eq!(manager.ready().len(), 1);
        manager.shutdown_and_wait().await.unwrap();
    }

    #[test]
    fn shutdown_does_not_wait_for_current_read_or_publish_its_late_result() {
        let (after_read, entered, release) = gate();
        let (exited, exit) = mpsc::sync_channel(1);
        let manager = manager(TestHooks {
            after_read: Some(after_read),
            exited: Some(exited),
            ..TestHooks::default()
        });
        manager.enqueue("active", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        // This gate is after real archive verification but before publication.
        manager.enqueue("pending", paths()).unwrap();
        manager.shutdown();
        assert_eq!(
            manager.status("active").unwrap().state,
            ImportState::Cancelled
        );
        assert_eq!(
            manager.status("pending").unwrap().state,
            ImportState::Cancelled
        );
        assert!(manager.ready().is_empty());
        assert!(manager.enqueue("other", paths()).is_err());
        assert!(manager.retry("active").is_err());
        release.send(()).unwrap();
        exit.recv_timeout(DEADLINE).unwrap();
        assert!(manager.ready().is_empty());
        assert_eq!(manager.shared.hooks.lock().unwrap().reads, 1);
        assert_eq!(
            manager.status("active").unwrap().state,
            ImportState::Cancelled
        );
    }

    #[tokio::test]
    async fn terminal_retirement_joins_active_work_after_an_await_is_dropped() {
        let (after_read, entered, release) = gate();
        let manager = manager(TestHooks { after_read: Some(after_read), ..TestHooks::default() });
        manager.enqueue("active", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        {
            let retirement = manager.shutdown_and_wait();
            tokio::pin!(retirement);
            assert!(futures_util::poll!(&mut retirement).is_pending());
        }
        let retirement = manager.shutdown_and_wait();
        tokio::pin!(retirement);
        assert!(futures_util::poll!(&mut retirement).is_pending());
        release.send(()).unwrap();
        tokio::time::timeout(DEADLINE, retirement).await.unwrap().unwrap();
        assert!(manager.ready().is_empty());
        assert_eq!(manager.status("active").unwrap().state, ImportState::Cancelled);
        manager.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_invalidates_retained_ready_ownership_without_committing() {
        let manager = ImportManager::new().unwrap();
        manager.enqueue("ready", paths()).unwrap();
        wait_for(&manager, "ready", ImportState::Ready).await;
        let ready = manager.ready().pop().unwrap();
        manager.shutdown();
        let mut committed = false;
        assert!(manager
            .with_ready(&ready, |_| {
                committed = true;
                Ok(())
            })
            .is_err());
        assert!(!committed);
        assert!(!manager.ack(&ready));
        assert_eq!(
            manager.status("ready").unwrap().state,
            ImportState::Cancelled
        );
    }

    #[test]
    fn queue_is_bounded_and_drop_cancels_without_joining() {
        let (before_read, entered, release) = gate();
        let (exited, exit) = mpsc::sync_channel(1);
        let manager = manager(TestHooks {
            before_read: Some(before_read),
            exited: Some(exited),
            ..TestHooks::default()
        });
        manager.enqueue("active", paths()).unwrap();
        entered.recv_timeout(DEADLINE).unwrap();
        for number in 0..MAX_QUEUED {
            manager
                .enqueue(&format!("pending-{number}"), paths())
                .unwrap();
        }
        assert_eq!(
            manager.enqueue("overflow", paths()).unwrap_err(),
            EnqueueError::Full
        );
        assert!(manager.status("overflow").is_none());
        let shared = manager.shared.clone();
        drop(manager);
        assert!(shared.state.lock().unwrap().stopped);
        release.send(()).unwrap();
        exit.recv_timeout(DEADLINE).unwrap();
        assert_eq!(shared.hooks.lock().unwrap().reads, 0);
    }

    #[tokio::test]
    async fn verification_records_byte_progress_and_resets_on_retry() {
        let manager = ImportManager::new().unwrap();
        manager.enqueue("progress", paths()).unwrap();
        let ready = wait_for(&manager, "progress", ImportState::Ready).await;
        assert!(ready.verified_total > 0);
        assert_eq!(ready.verified_bytes, ready.verified_total);
        let single_total = ready.verified_total;
        let pair = vec![paths()[0].clone(), paths()[0].clone()];
        manager.enqueue("batch", pair).unwrap();
        let batch_ready = wait_for(&manager, "batch", ImportState::Ready).await;
        assert_eq!(batch_ready.verified_total, single_total * 2);
        assert_eq!(batch_ready.verified_bytes, batch_ready.verified_total);
        let corrupt = TempArchive::corrupt();
        manager.enqueue("retryable", corrupt.paths()).unwrap();
        let failed = wait_for(&manager, "retryable", ImportState::Failed).await;
        assert!(failed.verified_total > 0);
        let retried = manager.retry("retryable").unwrap();
        assert_eq!(retried.verified_bytes, 0);
        assert_eq!(retried.verified_total, 0);
        wait_for(&manager, "retryable", ImportState::Failed).await;
        manager.shutdown();
    }
}
