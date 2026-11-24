// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit opt-in, receive-only acquisition and selected-library-book offers.
pub(crate) mod discovery;
mod network;
pub(crate) mod library;
mod library_wire;
mod library_transport;
mod library_route;
#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple_transport;
mod protocol;
mod sources;
mod storage;
mod cleanup;
mod import;
pub(crate) use import::{ImportPublication, SavedImportSnapshot};
mod profile;

use crate::{archive::Archive, store::{Book, Store}};
use discovery::Discovery;
#[cfg(test)]
use discovery::Peer;
use network::{RateBudget, SeedServer, Session};
pub(crate) use network::reachable_addresses;
use protocol::{Manifest, SwarmFile};
use storage::{err, Directory, OpenSet, OwnedSet, Stamp};
pub use sources::ConnectionTarget;
use sources::{connect_target, source_candidates, SourceDialer};
use cap_std::fs::Metadata;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use futures_util::{future::BoxFuture, stream::{FuturesUnordered, StreamExt}};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, HashMap, HashSet, VecDeque}, fs::File, io::{Read, Seek, SeekFrom}, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const STATE_LIMIT: u64 = 64 * 1024 * 1024;
const MAX_JOBS: usize = 64;
const MAX_SEEDS: usize = 8;
// Durability window: a verified chunk is written immediately, but the archive
// fsync, the staged-identity check and the job-state checkpoint are batched.
// A commit happens after CHECKPOINT_CHUNKS writes or CHECKPOINT_BYTES, whichever
// comes first. The window is deliberately byte-bounded and not time-bounded: a
// phone fsync or job-state rewrite can take longer than a second, and a time
// interval would re-arm immediately after every slow commit, turning one commit
// per chunk into the throughput ceiling. A crash re-downloads at most one window
// and the committed prefix is always durable.
const CHECKPOINT_CHUNKS: usize = 128;
const CHECKPOINT_BYTES: usize = 256 * 1024 * 1024;
/// Effective durability window. Only the profiling benchmark overrides it, so a
/// single build can measure both a per-chunk commit cadence and the production
/// one against the same simulated storage latency.
fn checkpoint_window() -> (usize, usize) {
    #[cfg(test)]
    {
        let chunks = std::env::var("KIWIX_LOCALSWARM_TEST_CHECKPOINT_CHUNKS").ok()
            .and_then(|value| value.parse().ok()).unwrap_or(CHECKPOINT_CHUNKS);
        let bytes = std::env::var("KIWIX_LOCALSWARM_TEST_CHECKPOINT_BYTES").ok()
            .and_then(|value| value.parse().ok()).unwrap_or(CHECKPOINT_BYTES);
        (chunks.max(1), bytes.max(1))
    }
    #[cfg(not(test))]
    { (CHECKPOINT_CHUNKS, CHECKPOINT_BYTES) }
}
// Chunk indices handed to one connection ahead of its responses. Two request
// windows keep a connection busy while the receiver writes the previous chunk;
// payload bytes are separately capped by network::PREFETCH_BYTES.
const PIPELINE_AHEAD: usize = network::PIPELINE_WINDOW * 2;

/// One live connection owned by its own task. The task waits for index batches
/// so its request window never stops between scheduling rounds.
struct PipelineWorker {
    source: usize,
    available: Arc<Vec<bool>>,
    batches: tokio::sync::mpsc::Sender<Vec<u32>>,
    shutdown: CancellationToken,
    /// Forward-only scan position over the chunk space. A scheduling round costs
    /// the batch it assigns instead of the whole chunk space; releasing an
    /// unverified chunk rewinds every worker to it.
    cursor: usize,
}
type PipelineJoin = BoxFuture<'static, (usize, Result<Session, tokio::task::JoinError>)>;

fn start_pipeline(
    source: usize, session: Session, sender: &tokio::sync::mpsc::Sender<network::PipelineItem>,
    prefetch: &Arc<tokio::sync::Semaphore>, joins: &mut FuturesUnordered<PipelineJoin>,
) -> PipelineWorker {
    let available = session.availability();
    let (batches, batch_receiver) = tokio::sync::mpsc::channel(2);
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(network::pipeline_task(source, session, batch_receiver,
        sender.clone(), prefetch.clone(), shutdown.clone()));
    joins.push(Box::pin(async move { (source, task.await) }));
    PipelineWorker { source, available, batches, shutdown, cursor: 0 }
}

impl PipelineWorker {
    /// Next unverified, unassigned chunks this connection advertised. The
    /// snapshot is taken when the session is established; seeders serve whole
    /// archives.
    fn next_batch(&mut self, verified: &[bool], assigned: &HashMap<u32, usize>) -> Vec<u32> {
        let mut batch = Vec::new();
        while self.cursor < verified.len() && batch.len() < network::PIPELINE_WINDOW {
            let index = self.cursor;
            self.cursor += 1;
            if verified[index] || !self.available.get(index).copied().unwrap_or(false) || assigned.contains_key(&(index as u32)) { continue; }
            batch.push(index as u32);
        }
        batch
    }
}

/// Release every assignment owned by one failed source and report the lowest
/// released index, so forward cursors can rewind behind it.
fn release_source(assigned: &mut HashMap<u32, usize>, source: usize) -> usize {
    let mut released = usize::MAX;
    assigned.retain(|index, owner| {
        let keep = *owner != source;
        if !keep { released = released.min(*index as usize); }
        keep
    });
    released
}

/// Release still-unverified chunks that could not be handed to a connection and
/// report the lowest released index.
fn release_indices(assigned: &mut HashMap<u32, usize>, indices: impl IntoIterator<Item = u32>) -> usize {
    let mut released = usize::MAX;
    for index in indices {
        if assigned.remove(&index).is_some() { released = released.min(index as usize); }
    }
    released
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadJob {
    pub id: String, pub url: String, pub file_name: String, pub destination: String,
    pub status: String, pub downloaded: u64, pub total: Option<u64>, pub error: Option<String>,
    pub speed: u64, pub eta_seconds: Option<u64>, pub created_at: String, pub updated_at: String,
    pub completed_paths: Vec<String>, pub requires_pin: bool, pub transport: String,
    /// Bounded evidence of the route used, e.g. the AWDL interface and local
    /// address. Absent until a payload session is established.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredJob {
    job: DownloadJob, manifest: Arc<Manifest>, host: String, port: u16, owned: OwnedSet,
    verified: Vec<bool>, publishing: bool,
    /// Apple-only Nearby preference captured when this transfer was admitted.
    #[serde(default)]
    direct: bool,
    #[serde(default)]
    history_dismissed: bool,
    #[serde(default)]
    cleanup: Option<cleanup::CleanupIntent>,
}
impl StoredJob {
    fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        if crate::acquisition::monitored_path_matches(path, Path::new(&self.job.destination))? {
            return Ok(true);
        }
        for completed in &self.job.completed_paths {
            if crate::acquisition::monitored_path_matches(path, Path::new(completed))? {
                return Ok(true);
            }
        }
        // The directory rename publishes every split member at once, before
        // completion records its primary import path. Ownership/manifest remain
        // durable on failure and are not proof that replacement bytes are ours.
        for entry in &self.manifest.files {
            if crate::acquisition::monitored_descendant_matches(
                path, &self.owned.parent,
                &[&self.owned.destination, &entry.path],
            )? {
                return Ok(true);
            }
            if self.cleanup.is_some() && crate::acquisition::monitored_descendant_matches(
                path, &self.owned.parent,
                &[&cleanup::cleanup_name(self)?, &entry.path],
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection { swarm_id: String, name: String, total_bytes: u64, chunk_count: usize, files: Vec<InspectedFile>, supported: bool, connected_host: String, #[serde(skip_serializing_if = "Option::is_none")] archive_id: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] route: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] reason: Option<String> }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectedFile { path: String, size_bytes: u64 }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seed { id: String, book_id: String, name: String, swarm_id: String, total_bytes: u64, chunk_count: usize, port: u16, locked: bool, uploaded: u64, upload_speed: u64, peer_count: usize, addresses: Vec<String> }
struct Offer {
    row: Seed,
    source: Arc<SourceSet>,
    stats: Arc<network::SeedStats>,
    cancel: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    last_uploaded: u64,
    last_sample: Instant,
}
impl Drop for Offer {
    fn drop(&mut self) { self.cancel.cancel(); }
}
struct LibraryOffer {
    server: library::LibraryServer,
    advertisement: String,
}
struct ReceiveProgress { downloaded: u64, speed: u64, updated_at: String }
struct Running { cancel: CancellationToken, task: tokio::task::JoinHandle<()>, progress: Option<ReceiveProgress> }
struct State { jobs: BTreeMap<String, StoredJob>, running: HashMap<String, Running> }
/// Orders durable job-map writes. `next` is assigned under the state lock, so a
/// snapshot's sequence follows the mutation it contains; `written` is the newest
/// snapshot that reached the disk. A snapshot captured earlier is therefore
/// never allowed to overwrite a newer one, even though the encode and fsync run
/// outside the state lock.
struct PersistOrder { next: u64, written: u64 }
type StartManifest = (String, Option<String>, Arc<Manifest>);

struct Inner {
    store: Arc<Store>, directory: Directory, peer_id: String, _process_lock: File,
    state: Mutex<State>, discovery: tokio::sync::Mutex<Discovery>,
    offers: tokio::sync::Mutex<HashMap<String, Offer>>, operation: tokio::sync::Mutex<()>,
    library: tokio::sync::Mutex<Option<LibraryOffer>>,
    library_retirement: Mutex<Option<String>>,
    lifetime: Mutex<CancellationToken>, download_budget: Arc<RateBudget>, upload_budget: Arc<RateBudget>,
    slots: tokio::sync::Semaphore,
    receive_activity: Arc<network::ReceiveActivity>,
    #[cfg(target_os = "ios")]
    receive_work: std::sync::atomic::AtomicUsize,
    error: Mutex<Option<String>>,
    // The pending request nonce survives confirmation retries but is consumed
    // on staging admission, so an approval cannot authorize another job.
    last_start_manifest: Mutex<Option<StartManifest>>,
    persist_order: Mutex<PersistOrder>,
    /// Publication digests proven for an exact inode state, so the cleanup
    /// preview and its confirmation do not each re-read a multi-gigabyte
    /// published archive whose bytes provably did not change.
    cleanup_digests: cleanup::CleanupDigestCache,
    #[cfg(target_os = "ios")]
    sharing_admission: std::sync::atomic::AtomicBool,
}
#[cfg(target_os = "ios")]
struct ReceiveWork<'a>(&'a std::sync::atomic::AtomicUsize);
#[cfg(target_os = "ios")]
impl Drop for ReceiveWork<'_> {
    fn drop(&mut self) { self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel); }
}
#[derive(Clone)]
pub struct LocalSwarmManager { inner: Arc<Inner> }

/// Metadata watching carries no source file locks, Store, or manager authority.
/// A stalled filesystem probe can outlive retirement without blocking native
/// shutdown or preventing an explicitly requested archive move/delete.
struct SourceWatch {
    path: PathBuf,
    identity: storage::Identity,
    files: Vec<(String, Stamp)>,
}
impl SourceWatch {
    fn unchanged(&self) -> Result<(), String> {
        let directory = Directory::open(&self.path)?;
        if directory.identity != self.identity { return Err("Offered archive directory changed".into()); }
        for (name, stamp) in &self.files {
            if Stamp::of(&directory.dir.symlink_metadata(name).map_err(err)?)? != *stamp {
                return Err("Offered archive changed; prepare a new offer".into());
            }
        }
        Ok(())
    }
}
struct SourceMonitor {
    cancel: CancellationToken,
    wake: Option<Arc<(Mutex<()>, std::sync::Condvar)>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl SourceMonitor {
    fn start(source: &SourceSet, cancel: CancellationToken, reason: Arc<Mutex<Option<String>>>) -> Result<Self, String> {
        let snapshot = {
            let retained = source.retained.lock().map_err(|_| "LocalSwarm source unavailable")?;
            match &*retained {
                crate::archive_source::RetainedSource::Filesystem(pinned) => SourceWatch {
                    path: pinned.path.clone(),
                    identity: storage::Identity::of(&pinned.directory.dir_metadata().map_err(err)?),
                    files: pinned.files.iter().zip(&pinned.stamps)
                        .map(|((name, _), stamp)| Ok((name.clone(), Stamp::of(stamp)?)))
                        .collect::<Result<Vec<_>, String>>()?,
                },
                #[cfg(mobile)]
                crate::archive_source::RetainedSource::Provider(_) => {
                    // Providers have no ambient pathname authority to watch. Each
                    // authenticated chunk revalidates retained descriptors and its
                    // expected hash; never let a detached metadata probe keep a
                    // native scope/file lease alive after offer retirement.
                    return Ok(Self { cancel, wake: None, thread: None });
                }
            }
        };
        static SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
            std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_SEEDS)));
        let permit = SLOTS.clone().try_acquire_owned().map_err(|_| "Previous source metadata probes are still retiring")?;
        let wake = Arc::new((Mutex::new(()), std::sync::Condvar::new()));
        let worker_wake = wake.clone(); let token = cancel.clone();
        let thread = std::thread::Builder::new().name("localswarm-source-watch".into()).spawn(move || {
            let _permit = permit;
            while !token.is_cancelled() {
                if let Err(error) = snapshot.unchanged() {
                    *reason.lock().unwrap() = Some(error);
                    token.cancel();
                    break;
                }
                let guard = worker_wake.0.lock().unwrap();
                if token.is_cancelled() { break; }
                let _wait = worker_wake.1.wait_timeout(guard, std::time::Duration::from_millis(500));
            }
        }).map_err(|error| format!("Starting LocalSwarm source monitor: {error}"))?;
        Ok(Self { cancel, wake: Some(wake), thread: Some(thread) })
    }
}
impl Drop for SourceMonitor {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(wake) = &self.wake {
            let _guard = wake.0.lock().unwrap();
            wake.1.notify_all();
        }
        if self.thread.as_ref().is_some_and(|thread| thread.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}
fn now() -> String { crate::timestamps::now() }
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> { tokio::task::spawn_blocking(f).await.map_err(|e| format!("LocalSwarm background operation stopped: {e}"))? }
/// Only read-only native verification belongs here. A retired result has no
/// manager, writer handle, publication callback, listener or advertisement.
async fn readonly_verification<T: Send + 'static>(
    cancel: &CancellationToken,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    check_cancel(cancel)?;
    static SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));
    let permit = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("Nearby sharing stopped. Start it again".into()),
        permit = SLOTS.clone().acquire_owned() => permit.map_err(err)?,
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new().name("localswarm-verification".into()).spawn(move || {
        let _permit = permit;
        let _ = sender.send(work());
    }).map_err(|e| format!("Starting LocalSwarm verification: {e}"))?;
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("Nearby sharing stopped. Start it again".into()),
        result = receiver => {
            check_cancel(cancel)?;
            result.map_err(|e| format!("LocalSwarm verification stopped: {e}"))?
        }
    }
}
fn check_cancel(cancel: &CancellationToken) -> Result<(), String> { if cancel.is_cancelled() { Err("LocalSwarm operation stopped".into()) } else { Ok(()) } }
fn supported(manifest: &Manifest) -> Result<(), String> {
    manifest.validate()?;
    if manifest.files.is_empty() || manifest.files.len() > 676 || manifest.files.iter().any(|f| f.size_bytes == 0) { return Err("Select one nonempty ZIM archive or its complete contiguous split set".into()); }
    for f in &manifest.files { storage::basename(&f.path)?; }
    let first = &manifest.files[0].path;
    if manifest.files.len() == 1 && first.ends_with(".zim") { return Ok(()); }
    if !first.ends_with(".zimaa") { return Err("Only one .zim or a complete .zimaa split-ZIM set is supported".into()); }
    let base = &first[..first.len() - 2];
    for (index, f) in manifest.files.iter().enumerate() {
        if f.path != format!("{base}{}{}", char::from(b'a' + (index / 26) as u8), char::from(b'a' + (index % 26) as u8)) { return Err("Split-ZIM parts must be one exact ordered contiguous set beginning with .zimaa".into()); }
    } Ok(())
}
/// Read only the ZIM identity prefix through verified chunks, without creating
/// a job or staging files. Tiny chunks and headers spanning split parts are valid.
/// Keep the transport's cancellable I/O/progress deadlines: a wall-clock limit
/// here would also count intentional receiver and sender bandwidth waits.
async fn incoming_archive_id(session: &mut Session) -> Result<Option<String>, String> {
    let mut header = [0u8; 24];
    let mut filled = 0;
    for file_index in 0..session.manifest().files.len() {
        let file = &session.manifest().files[file_index];
        let (start, end) = (file.start_chunk_index, file.end_chunk_index);
        for index in start..=end {
            let bytes = session.get_chunk(index).await?;
            let length = bytes.len().min(header.len() - filled);
            header[filled..filled + length].copy_from_slice(&bytes[..length]);
            filled += length;
            if filled == header.len() {
                // This is a duplicate preflight, not archive admission.
                // Invalid headers still face the final integrity gate.
                return Ok((&header[..4] == b"ZIM\x04"
                    && matches!(u16::from_le_bytes([header[4], header[5]]), 5 | 6))
                    .then(|| Uuid::from_bytes(header[8..24].try_into().unwrap()).to_string()));
            }
        }
    }
    Ok(None)
}
fn validate_stored(id: &str, saved: &StoredJob) -> Result<(), String> {
    let suffix = id.strip_prefix("localswarm:").ok_or("Invalid LocalSwarm job ID")?;
    if Uuid::parse_str(suffix).map_err(err)?.to_string() != suffix || saved.job.id != id { return Err("Invalid LocalSwarm job identity".into()); }
    supported(&saved.manifest)?;
    if saved.verified.len() != saved.manifest.chunk_hashes.len() || saved.host.is_empty() || saved.host.len() > 253 || saved.port == 0
        || saved.owned.staging != format!(".kiwix-localswarm-{suffix}.part")
        || saved.owned.destination != format!("localswarm-{}-{suffix}", &saved.manifest.swarm_id[..12])
        || saved.job.destination != saved.owned.final_path(&saved.manifest.files[0].path).to_string_lossy()
        || saved.job.file_name != saved.manifest.files[0].path || saved.job.url != format!("localswarm:{}", saved.manifest.swarm_id)
        || saved.job.transport != "localswarm" || saved.job.total != Some(saved.manifest.total_bytes)
        || saved.job.route.as_ref().is_some_and(|route| route.is_empty() || route.len() > 240 || route.chars().any(char::is_control))
        || !matches!(saved.job.status.as_str(), "queued" | "downloading" | "verifying" | "paused" | "failed" | "cancelled" | "complete") { return Err("Invalid persisted LocalSwarm destination or manifest".into()); }
    if saved.job.status == "complete" && saved.job.completed_paths != vec![saved.job.destination.clone()] { return Err("Invalid LocalSwarm completion receipt".into()); }
    if saved.job.status != "complete" && !saved.job.completed_paths.is_empty() { return Err("Incomplete LocalSwarm job contains import paths".into()); }
    if saved.owned.files.len() != saved.manifest.files.len() {
        return Err("LocalSwarm retained ownership does not cover the manifest".into());
    }
    if let Some(cleanup) = &saved.cleanup { cleanup.validate(saved)?; }
    if (saved.history_dismissed || saved.cleanup.is_some()) && !cleanup::terminal(&saved.job.status) {
        return Err("This nearby download cannot be cleaned up yet. Wait for it to finish".into());
    }
    Ok(())
}
fn bounded_json<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, String> {
    struct Output { bytes: Vec<u8>, limit: usize }
    impl std::io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let end = self.bytes.len().checked_add(bytes.len()).filter(|end| *end <= self.limit)
                .ok_or_else(|| std::io::Error::other("LocalSwarm history exceeds the state limit; remove finished history"))?;
            if end > self.bytes.capacity() {
                let capacity = end.max(self.bytes.capacity().saturating_mul(2).max(4096)).min(self.limit);
                self.bytes.try_reserve_exact(capacity - self.bytes.len()).map_err(std::io::Error::other)?;
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut output = Output { bytes: Vec::new(), limit };
    serde_json::to_writer(&mut output, value).map_err(err)?;
    Ok(output.bytes)
}
fn load(directory: &Directory, _store: &Store) -> Result<BTreeMap<String, StoredJob>, String> {
    let Some(bytes) = directory.optional_bytes("jobs.json", STATE_LIMIT)? else { return Ok(BTreeMap::new()); };
    decode_jobs(&bytes, _store)
}

#[cfg(test)]
thread_local! { static HISTORY_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Parse the saved job map. A pending profile relocation is the only thing that
/// can rewrite a saved record, so the bytes are parsed once and the relocation
/// is applied to the record itself; the record is never rebuilt as a JSON tree
/// and parsed a second time.
fn decode_jobs(bytes: &[u8], _store: &Store) -> Result<BTreeMap<String, StoredJob>, String> {
    #[cfg(test)]
    HISTORY_DECODES.with(|count| count.set(count.get() + 1));
    let invalid = |e: serde_json::Error| format!("LocalSwarm saved jobs are invalid; retained data was not changed: {e}");
    let mut jobs: BTreeMap<String, StoredJob> = serde_json::from_slice(bytes).map_err(invalid)?;
    if cfg!(mobile) || _store.portable_mode() { rebase_saved_paths(_store, &mut jobs)?; }
    if jobs.len() > MAX_JOBS { return Err("Too many saved LocalSwarm jobs".into()); }
    for (id, saved) in &mut jobs { validate_stored(id, saved)?; if matches!(saved.job.status.as_str(), "queued" | "downloading" | "verifying") { saved.job.status = "paused".into(); saved.job.speed = 0; saved.job.eta_seconds = None; } }
    Ok(jobs)
}

/// Apply the schema-directed profile relocation to the only filesystem paths a
/// saved LocalSwarm record owns. `profile_paths::rewrite` rewrites those three
/// fields per string and consults no other field, so it runs on a minimal value
/// carrying exactly them; that keeps the rewrite byte-for-byte equivalent to
/// rewriting the whole document while never materializing or re-parsing one.
/// Keep this field list in sync with the `LocalSwarm` arm of that schema.
fn rebase_saved_paths(store: &Store, jobs: &mut BTreeMap<String, StoredJob>) -> Result<(), String> {
    let mut probe = serde_json::Map::with_capacity(jobs.len());
    for (id, saved) in jobs.iter() {
        probe.insert(id.clone(), json!({
            "job": { "destination": saved.job.destination, "completedPaths": saved.job.completed_paths },
            "owned": { "parent": saved.owned.parent.to_string_lossy() },
        }));
    }
    let mut probe = Value::Object(probe);
    #[cfg(mobile)]
    let changed = store.mobile_rebase_saved_json(crate::mobile_rebase::RecordKind::LocalSwarm, &mut probe)?;
    #[cfg(desktop)]
    let changed = store.portable_rebase_saved_json(crate::profile_paths::RecordKind::LocalSwarm, &mut probe)?;
    if !changed { return Ok(()); }
    for (id, saved) in jobs.iter_mut() {
        let row = probe.get(id.as_str()).ok_or("LocalSwarm relocation dropped a saved job")?;
        let job = row.get("job").ok_or("LocalSwarm relocation dropped a saved job")?;
        let owned = row.get("owned").ok_or("LocalSwarm relocation dropped a saved job")?;
        saved.job.destination = job.get("destination").and_then(Value::as_str)
            .ok_or("LocalSwarm relocation damaged a saved destination")?.to_owned();
        saved.job.completed_paths = job.get("completedPaths").and_then(Value::as_array)
            .ok_or("LocalSwarm relocation damaged a saved path list")?
            .iter().map(|path| path.as_str().map(str::to_owned).ok_or("LocalSwarm relocation damaged a saved path"))
            .collect::<Result<Vec<_>, _>>()?;
        saved.owned.parent = PathBuf::from(owned.get("parent").and_then(Value::as_str)
            .ok_or("LocalSwarm relocation damaged a saved parent directory")?);
    }
    Ok(())
}

#[cfg(target_os = "ios")]
pub(crate) fn backup_evidence(store: &Store) -> Result<Vec<(PathBuf, crate::library_files::BackupEvidence)>, String> {
    let saved: BTreeMap<String, StoredJob> = crate::library_files::backup_record(
        store, &["localswarm", "jobs.json"], STATE_LIMIT,
        crate::mobile_rebase::RecordKind::LocalSwarm,
    )?.unwrap_or_default();
    let mut candidates = Vec::new();
    for (id, job) in saved {
        validate_stored(&id, &job)?;
        if job.job.status == "complete" {
            if job.owned.files.len() != job.manifest.files.len() { return Err("Invalid completed LocalSwarm file identities".into()); }
            for (entry, identity) in job.manifest.files.iter().zip(&job.owned.files) {
                candidates.push((job.owned.final_path(&entry.path), crate::library_files::BackupEvidence {
                    size: entry.size_bytes, identity: Some(identity.backup_identity()), sha256: None,
                }));
            }
        }
    }
    Ok(candidates)
}
impl LocalSwarmManager {
    pub(crate) fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        let state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        for saved in state.jobs.values() {
            if saved.claims_monitored_path(path)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Read retained ownership without creating discovery, listeners, or files.
    pub(crate) fn saved_monitored_paths(store: &Store) -> Result<crate::acquisition::MonitoredPathClaims, String> {
        let jobs: BTreeMap<String, StoredJob> = crate::acquisition::saved_monitored_record(
            store, &["localswarm", "jobs.json"], STATE_LIMIT,
            crate::profile_paths::RecordKind::LocalSwarm,
        )?.unwrap_or_default();
        if jobs.len() > MAX_JOBS { return Err("Too many saved LocalSwarm jobs".into()); }
        let mut claims = Vec::new();
        for (id, saved) in jobs {
            validate_stored(&id, &saved)?;
            if saved.cleanup.is_some() {
                let directory = saved.owned.parent.join(cleanup::cleanup_name(&saved)?);
                claims.extend(saved.manifest.files.iter().map(|entry| directory.join(&entry.path)));
            }
            claims.push(PathBuf::from(saved.job.destination));
            claims.extend(saved.job.completed_paths.into_iter().map(PathBuf::from));
            for entry in &saved.manifest.files {
                claims.push(saved.owned.final_path(&entry.path));
            }
        }
        crate::acquisition::MonitoredPathClaims::new(claims)
    }

    /// Staged files of unfinished receives with their durably verified byte
    /// ranges, for another transport to reuse after its own verification.
    /// Reads saved history only; it never opens discovery, sockets or payload.
    pub(crate) fn saved_partial_files(store: &Store) -> Result<Vec<crate::partial_donor::Donor>, String> {
        let jobs: BTreeMap<String, StoredJob> = crate::acquisition::saved_monitored_record(
            store, &["localswarm", "jobs.json"], STATE_LIMIT,
            crate::profile_paths::RecordKind::LocalSwarm,
        ).map_err(|e| e.to_string())?.unwrap_or_default();
        if jobs.len() > MAX_JOBS { return Err("Too many saved LocalSwarm jobs".into()); }
        let mut donors = Vec::new();
        for (id, saved) in jobs {
            validate_stored(&id, &saved)?;
            if saved.publishing || saved.cleanup.is_some() || saved.job.status == "complete"
                || saved.verified.len() != saved.manifest.chunk_hashes.len()
            {
                continue;
            }
            let staging = saved.owned.parent.join(&saved.owned.staging);
            let chunk = u64::from(saved.manifest.chunk_size_bytes);
            for entry in &saved.manifest.files {
                let (start, end) = (entry.start_chunk_index as usize, entry.end_chunk_index as usize);
                if entry.size_bytes == 0 || end >= saved.verified.len() || start > end { continue; }
                let ranges = crate::partial_donor::coalesce((start..=end).filter(|index| saved.verified[*index]).map(|index| {
                    let offset = (index - start) as u64 * chunk;
                    (offset, chunk.min(entry.size_bytes.saturating_sub(offset)))
                }));
                if ranges.is_empty() { continue; }
                donors.push(crate::partial_donor::Donor { path: staging.join(&entry.path), size: entry.size_bytes, ranges: Some(ranges) });
            }
        }
        Ok(donors)
    }

    /// Listing neither creates profile files nor initializes mDNS/QUIC.
    pub fn saved_jobs(store: &Store) -> Result<Vec<DownloadJob>, String> {
        let (path, dir) = store.acquisition_profile_directory()?;
        let identity = storage::Identity::of(&dir.dir_metadata().map_err(err)?);
        let profile = Directory { path, dir, identity };
        match profile.dir.symlink_metadata("localswarm") { Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()), Err(e) => return Err(err(e)), Ok(_) => () }
        let directory = profile.child("localswarm", false)?;
        Ok(load(&directory, store)?.into_values().filter(|saved| !saved.history_dismissed).map(|v| v.job).collect())
    }
    pub fn new(store: Arc<Store>, admission: CancellationToken) -> Result<Self, String> {
        check_cancel(&admission)?;
        if store.private_mode()? || store.read_only()? { return Err("LocalSwarm is disabled in private/read-only mode".into()); }
        #[cfg(feature = "localswarm-profiling")]
        profile::configure();
        let (path, dir) = store.acquisition_profile_directory()?;
        let identity = storage::Identity::of(&dir.dir_metadata().map_err(err)?);
        let profile = Directory { path, dir, identity };
        let directory = match profile.dir.symlink_metadata("localswarm") { Err(e) if e.kind() == std::io::ErrorKind::NotFound => profile.child("localswarm", true)?, Err(e) => return Err(err(e)), Ok(_) => profile.child("localswarm", false)? };
        let process_lock = match directory.dir.symlink_metadata("manager.lock") { Err(e) if e.kind() == std::io::ErrorKind::NotFound => directory.file("manager.lock", true, true)?, Err(e) => return Err(err(e)), Ok(_) => directory.file("manager.lock", true, false)? };
        let peer_id = if let Some(bytes) = directory.optional_bytes("peer-id", 64)? { let id = String::from_utf8(bytes).map_err(err)?; if Uuid::parse_str(&id).map_err(err)?.to_string() != id { return Err("Invalid LocalSwarm peer identity".into()); } id } else { let id = Uuid::new_v4().to_string(); directory.atomic("peer-id", id.as_bytes())?; id };
        let jobs = load(&directory, &store)?; let settings = store.settings()?;
        let discovery = Discovery::new(peer_id.clone())?;
        Ok(Self { inner: Arc::new(Inner { #[cfg(target_os = "ios")] sharing_admission: std::sync::atomic::AtomicBool::new(true), #[cfg(target_os = "ios")] receive_work: std::sync::atomic::AtomicUsize::new(0), store, directory, peer_id, _process_lock: process_lock, state: Mutex::new(State { jobs, running: HashMap::new() }), discovery: tokio::sync::Mutex::new(discovery), offers: tokio::sync::Mutex::new(HashMap::new()), library: tokio::sync::Mutex::new(None), library_retirement: Mutex::new(None), operation: tokio::sync::Mutex::new(()), lifetime: Mutex::new(admission.child_token()), download_budget: Arc::new(RateBudget::new(settings.download_limit)), upload_budget: Arc::new(RateBudget::new(settings.upload_limit)), slots: tokio::sync::Semaphore::new(3), receive_activity: Arc::new(network::ReceiveActivity::default()), error: Mutex::new(None), last_start_manifest: Mutex::new(None), persist_order: Mutex::new(PersistOrder { next: 0, written: 0 }), cleanup_digests: Mutex::new(HashMap::new()) }) })
    }
    fn allowed(&self) -> Result<(), String> { if self.inner.store.private_mode()? || self.inner.store.read_only()? { Err("LocalSwarm is disabled in private/read-only mode".into()) } else { Ok(()) } }
    /// Clone the job map for a durable write. Callers hold the state lock for
    /// this snapshot only: the encode of every job's manifest hashes and the
    /// fsync must never run under it, or a chunk-level checkpoint would stall
    /// progress, status and listing for a multi-megabyte rewrite.
    fn stage(&self, state: &State) -> (u64, BTreeMap<String, StoredJob>) {
        let mut order = self.inner.persist_order.lock().unwrap_or_else(|error| error.into_inner());
        order.next = order.next.saturating_add(1);
        (order.next, state.jobs.clone())
    }
    /// Serialize and durably write a staged job map. `sequence` keeps the newest
    /// mutation authoritative: a snapshot that has already been superseded on
    /// disk is dropped rather than rolled back.
    fn write_staged(&self, sequence: u64, jobs: &BTreeMap<String, StoredJob>) -> Result<(), storage::PersistError> {
        let mut order = self.inner.persist_order.lock().unwrap_or_else(|error| error.into_inner());
        if sequence <= order.written { return Ok(()); }
        let bytes = bounded_json(jobs, STATE_LIMIT as usize)
            .map_err(|message| storage::PersistError { message, committed: false })?;
        let result = self.inner.directory.atomic("jobs.json", &bytes);
        // A committed error means the rename already published these bytes;
        // advance the order so an older staged snapshot cannot overwrite them.
        if result.is_ok() || result.as_ref().is_err_and(|error| error.committed) {
            order.written = sequence;
        }
        result
    }
    /// Snapshot and write while the caller still holds the state lock. Reserved
    /// for command paths whose failure must roll the in-memory state back
    /// before any other mutation can be staged; chunk progress uses `stage`
    /// plus `write_staged` after releasing the lock.
    fn persist(&self, state: &State) -> Result<(), storage::PersistError> {
        let (sequence, jobs) = self.stage(state);
        self.write_staged(sequence, &jobs)
    }
    fn cancel_token(&self) -> CancellationToken { self.inner.lifetime.lock().unwrap().child_token() }
    /// Cancels long accepted setup before Service waits for its mode writer.
    pub fn interrupt(&self) { self.inner.lifetime.lock().unwrap().cancel(); }
    pub fn activate(&self, admission: &CancellationToken) -> Result<(), String> {
        check_cancel(admission)?;
        let mut lifetime = self.inner.lifetime.lock().unwrap();
        if lifetime.is_cancelled() { *lifetime = admission.child_token(); }
        Ok(())
    }
    pub fn limits(&self, download: u64, upload: u64) { self.inner.download_budget.set_limit(download); self.inner.upload_budget.set_limit(upload); }
    /// Apple-only Nearby route preference. It applies to the next connection
    /// and, while browsing, to which resolved interface is published.
    pub async fn set_direct(&self, direct: bool) -> Result<(), String> {
        self.inner.discovery.lock().await.set_direct(direct).await
    }
    pub fn list(&self) -> Result<Vec<DownloadJob>, String> {
        let state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        Ok(state.jobs.values().filter(|saved| !saved.history_dismissed).map(|saved| {
            let mut job = saved.job.clone();
            if job.status == "downloading" {
                if let Some(progress) = state.running.get(&job.id).and_then(|worker| worker.progress.as_ref())
                    .filter(|progress| progress.downloaded > job.downloaded)
                {
                    job.downloaded = progress.downloaded;
                    job.speed = progress.speed;
                    job.eta_seconds = (job.speed > 0).then(|| saved.manifest.total_bytes.saturating_sub(job.downloaded).div_ceil(job.speed));
                    job.updated_at.clone_from(&progress.updated_at);
                }
            }
            job
        }).collect())
    }
    #[cfg(target_os = "ios")]
    pub async fn set_sharing_admission(&self, accepting: bool) {
        self.inner.sharing_admission.store(accepting, std::sync::atomic::Ordering::SeqCst);
        self.inner.receive_activity.set_accepting(accepting);
        if !accepting {
            let _operation = self.inner.operation.lock().await;
            if let Err(error) = self.stop_library_locked().await { *self.inner.error.lock().unwrap() = Some(error); }
        }
        for offer in self.inner.offers.lock().await.values() { offer.stats.set_accepting(accepting); }
    }
    #[cfg(target_os = "ios")]
    pub async fn has_active_transfers(&self) -> Result<bool, String> {
        let receiving = self.inner.receive_work.load(std::sync::atomic::Ordering::Acquire) != 0;
        let library_active = self.library_status().await.peer_count != 0;
        Ok(receiving || library_active || self.inner.offers.lock().await.values()
            .any(|offer| !offer.cancel.is_cancelled() && offer.stats.active_transfers() != 0))
    }
    pub async fn status(&self) -> Result<Value, String> {
        let discovery = self.inner.discovery.lock().await;
        let peers = discovery.peers(); let browsing = discovery.is_browsing();
        let error = discovery.error().or_else(|| self.inner.error.lock().unwrap().clone());
        drop(discovery);
        let mut offers = self.inner.offers.lock().await;
        let seeds: Vec<_> = offers.values_mut().map(|offer| {
            let uploaded = offer.stats.uploaded();
            offer.row.uploaded = uploaded;
            offer.row.peer_count = offer.stats.peer_count();
            offer.row.upload_speed = (uploaded.saturating_sub(offer.last_uploaded) as f64 / offer.last_sample.elapsed().as_secs_f64().max(0.001)) as u64;
            offer.last_uploaded = uploaded; offer.last_sample = Instant::now();
            offer.row.clone()
        }).collect();
        drop(offers);
        let library_sharing = self.library_status().await;
        let status = json!({"discovery":browsing,"peers":peers,"seeds":seeds,"librarySharing":library_sharing,"error":error});
        #[cfg(feature = "localswarm-profiling")]
        let status = {
            let mut status = status;
            status["boundedProfile"] = profile::snapshot();
            status
        };
        Ok(status)
    }
    pub async fn discovery(&self, enabled: bool) -> Result<Value, String> {
        self.allowed()?; let _operation = self.inner.operation.lock().await;
        let mut discovery = self.inner.discovery.lock().await;
        discovery.set_direct(self.inner.store.settings()?.direct_connection).await?;
        if enabled { discovery.start_browse().await?; } else { discovery.stop_browse().await?; }
        drop(discovery); self.status().await
    }
    pub async fn share_library(&self, name: String) -> Result<library::LibrarySharingStatus, String> {
        self.share_library_inner(name, None).await
    }
    pub async fn share_library_on_socket(&self, name: String, socket: std::net::UdpSocket, addresses: Vec<String>,
        route_cancel: CancellationToken) -> Result<library::LibrarySharingStatus, String> {
        self.share_library_inner(name, Some((socket, addresses, route_cancel))).await
    }
    fn next_library_generation(&self) -> Result<u64, String> {
        let current = self.inner.directory.optional_bytes("library-generation", 32)?
            .and_then(|bytes| std::str::from_utf8(&bytes).ok()?.parse::<u64>().ok())
            .unwrap_or(0);
        let next = current.checked_add(1).ok_or("Library generation exhausted")?;
        self.inner.directory.atomic("library-generation", next.to_string().as_bytes())
            .map_err(|error| error.message)?;
        Ok(next)
    }
    async fn share_library_inner(&self, name: String,
        supplied: Option<(std::net::UdpSocket, Vec<String>, CancellationToken)>) -> Result<library::LibrarySharingStatus, String> {
        self.allowed()?;
        let _operation = self.inner.operation.lock().await;
        check_cancel(&self.cancel_token())?;
        self.stop_library_locked().await?;
        let server = if let Some((socket, addresses, route_cancel)) = supplied {
            check_cancel(&route_cancel)?;
            let server = library::LibraryServer::start_on_socket(self.inner.store.clone(), self.inner.peer_id.clone(),
                name, self.inner.upload_budget.clone(), self.cancel_token(), socket, addresses).await?;
            let stopped = server.descriptor.cancel.clone();
            tokio::spawn(async move {
                tokio::select! { _ = stopped.cancelled() => {}, _ = route_cancel.cancelled() => { stopped.cancel(); } }
            });
            server
        } else {
            library::LibraryServer::start(self.inner.store.clone(), self.inner.peer_id.clone(),
                name, self.inner.upload_budget.clone(), self.cancel_token()).await?
        };
        let descriptor = server.descriptor.clone();
        crate::nearby_log!("[nearby] share library port={} addresses={:?}", descriptor.port, descriptor.addresses);
        let generation = self.next_library_generation()?;
        let advertisement = match self.inner.discovery.lock().await.advertise_library(
            &descriptor.session_id, descriptor.port, server.status().library_count, &descriptor.addresses, &descriptor.name, generation, server.native_listener()).await {
            Ok(id) => id,
            Err(error) => { server.shutdown().await; return Err(error); }
        };
        let status = server.status();
        let generation = server.generation.clone();
        *self.inner.library.lock().await = Some(LibraryOffer { server, advertisement });
        self.supervise_library(descriptor, generation);
        Ok(status)
    }
    fn supervise_library(&self, descriptor: library::LibraryPeerDescriptor, generation: String) {
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            descriptor.cancel.cancelled().await;
            let Some(inner) = inner.upgrade() else { return; };
            let manager = LocalSwarmManager { inner };
            let _operation = manager.inner.operation.lock().await;
            let same = manager.inner.library.lock().await.as_ref()
                .is_some_and(|offer| offer.server.generation == generation);
            if same { let _ = manager.stop_library_locked().await; }
        });
    }
    pub async fn rebind_library_on_socket(&self, session_id: &str, socket: std::net::UdpSocket, addresses: Vec<String>,
        route_cancel: CancellationToken) -> Result<library::LibraryPeerDescriptor, String> {
        self.allowed()?;
        let _operation = self.inner.operation.lock().await;
        check_cancel(&self.cancel_token())?; check_cancel(&route_cancel)?;
        let mut offer = self.inner.library.lock().await.take().ok_or("Share your library before connecting a new route")?;
        if offer.server.descriptor.session_id != session_id || !offer.server.status().running {
            *self.inner.library.lock().await = Some(offer);
            return Err("The shared library session changed".into());
        }
        if let Err(error) = self.inner.discovery.lock().await.unadvertise(&offer.advertisement).await {
            *self.inner.library_retirement.lock().unwrap() = Some(offer.advertisement);
            offer.server.shutdown().await;
            return Err(error);
        }
        if let Err(error) = offer.server.rebind(socket, addresses).await {
            offer.server.shutdown().await; return Err(error);
        }
        let descriptor = offer.server.descriptor.clone();
        let generation = self.next_library_generation()?;
        let advertisement = match self.inner.discovery.lock().await.advertise_library(&descriptor.session_id, descriptor.port,
            offer.server.status().library_count, &descriptor.addresses, &descriptor.name, generation, offer.server.native_listener()).await {
            Ok(id) => id,
            Err(error) => { offer.server.shutdown().await; return Err(error); }
        };
        let stopped = descriptor.cancel.clone();
        tokio::spawn(async move {
            tokio::select! { _ = stopped.cancelled() => {}, _ = route_cancel.cancelled() => stopped.cancel() }
        });
        let generation = offer.server.generation.clone();
        *self.inner.library.lock().await = Some(LibraryOffer { server: offer.server, advertisement });
        self.supervise_library(descriptor.clone(), generation);
        Ok(descriptor)
    }
    async fn stop_library_locked(&self) -> Result<(), String> {
        let offer = self.inner.library.lock().await.take();
        if let Some(offer) = offer {
            offer.server.cancel();
            offer.server.shutdown().await;
            *self.inner.library_retirement.lock().unwrap() = Some(offer.advertisement);
        }
        let pending = self.inner.library_retirement.lock().unwrap().clone();
        if let Some(id) = pending {
            self.inner.discovery.lock().await.unadvertise(&id).await?;
            *self.inner.library_retirement.lock().unwrap() = None;
        }
        Ok(())
    }
    pub async fn stop_library(&self) -> Result<library::LibrarySharingStatus, String> {
        let _operation = self.inner.operation.lock().await;
        self.stop_library_locked().await?;
        Ok(self.library_status().await)
    }
    pub async fn library_status(&self) -> library::LibrarySharingStatus {
        self.inner.library.lock().await.as_ref().map_or_else(Default::default, |offer| offer.server.status())
    }
    pub async fn library_descriptor(&self) -> Option<library::LibraryPeerDescriptor> {
        self.inner.library.lock().await.as_ref().filter(|offer| offer.server.status().running)
            .map(|offer| offer.server.descriptor.clone())
    }
    /// Resolve a current browse result, never an arbitrary caller-supplied host.
    pub async fn library_peer(&self, id: &str) -> Result<discovery::Peer, String> {
        self.allowed()?;
        check_cancel(&self.cancel_token())?;
        let discovery = self.inner.discovery.lock().await;
        discovery.library_peer(id)
    }
    pub async fn selected_library_peer(&self, id: &str) -> Result<Option<discovery::Peer>, String> {
        self.allowed()?;
        check_cancel(&self.cancel_token())?;
        let discovery = self.inner.discovery.lock().await;
        discovery.selected_library_peer(id)
    }
    async fn session(
        &self, target: &ConnectionTarget, sid: &str, pin: Option<&str>, cancel: &CancellationToken,
    ) -> Result<(String, Session), String> {
        connect_target(target, sid, pin.filter(|pin| !pin.is_empty()), &self.inner.peer_id,
            cancel, self.inner.download_budget.clone(), &mut HashSet::new()).await
    }
    pub async fn peer_target(&self, id: &str, sid: &str, port: u16, direct: bool) -> Result<ConnectionTarget, String> {
        self.allowed()?;
        let peers = self.inner.discovery.lock().await.peers();
        sources::selected_peer_target(&peers, id, sid, port, direct)
    }
    /// Inspect is in-memory only: no profile identity, lock, state or discovery.
    pub async fn inspect(
        target: ConnectionTarget, sid: &str, pin: Option<&str>,
        cancel: CancellationToken, download_limit: u64, check_identity: bool,
    ) -> Result<Inspection, String> {
        check_cancel(&cancel)?;
        let peer_id = Uuid::new_v4().to_string();
        let (connected_host, mut session) = connect_target(&target, sid, pin.filter(|pin| !pin.is_empty()), &peer_id,
            &cancel, Arc::new(RateBudget::new(download_limit)), &mut HashSet::new()).await?;
        let reason = supported(session.manifest()).err();
        let archive_id = if check_identity && reason.is_none() { incoming_archive_id(&mut session).await? } else { None };
        let manifest = session.manifest();
        let route = session.route().map(str::to_owned);
        let row = Inspection { swarm_id: manifest.swarm_id.clone(), name: manifest.name.clone(), total_bytes: manifest.total_bytes, chunk_count: manifest.chunk_hashes.len(), files: manifest.files.iter().map(|f| InspectedFile { path: f.path.clone(), size_bytes: f.size_bytes }).collect(), supported: reason.is_none(), connected_host, archive_id, route, reason };
        session.shutdown().await; check_cancel(&cancel)?; Ok(row)
    }
    pub async fn start(
        &self, target: ConnectionTarget, sid: String, pin: Option<String>, directory: PathBuf, acknowledgement: Option<String>,
    ) -> Result<DownloadJob, String> {
        self.allowed()?; let _operation = self.inner.operation.lock().await;
        let pin = pin.filter(|pin| !pin.is_empty());
        if self.inner.state.lock().unwrap().jobs.len() >= MAX_JOBS { return Err("Delete retained LocalSwarm downloads and their files before starting another transfer (64 retained jobs maximum)".into()); }
        let cancel = self.cancel_token();
        let port = target.port();
        let direct = target.direct();
        let request = serde_json::to_string(&(sid.as_str(), format!("{target:?}"), &directory)).map_err(err)?;
        let nonce = self.inner.last_start_manifest.lock().unwrap().as_mut()
            .filter(|(key, _, _)| key == &request).map(|(_, nonce, _)| {
                nonce.get_or_insert_with(|| Uuid::new_v4().to_string()).clone()
            }).unwrap_or_else(|| Uuid::new_v4().to_string());
        let identity = serde_json::to_string(&(request.as_str(), nonce.as_str())).map_err(err)?;
        let (host, mut session) = self.session(&target, &sid, pin.as_deref(), &cancel).await?;
        supported(session.manifest())?;
        // No incoming UUID can collide with an empty library. Avoid reading a
        // payload chunk (and waiting on its bandwidth budget) just to prove that.
        if self.inner.store.has_books()? {
            if let Some(id) = incoming_archive_id(&mut session).await? {
                if let Some(existing) = self.inner.store.book(&id)? {
                    session.shutdown().await;
                    return Err(if existing.available {
                        "This archive is already in your library. Open the existing copy instead."
                    } else {
                        "This archive is already in your library, but its file is unavailable. Locate or remove that entry in Library before downloading it again."
                    }.into());
                }
            }
        }
        let initial_route = session.route().map(str::to_owned);
        let manifest = session.into_manifest().await; check_cancel(&cancel)?;
        let directory = blocking(move || {
            crate::fs_ext::create_private_dir_all(&directory).map_err(err)?;
            let directory = std::fs::canonicalize(&directory).map_err(err)?;
            Directory::open(&directory)
        }).await?;
        *self.inner.last_start_manifest.lock().unwrap() = Some((request, Some(nonce), manifest.clone()));
        let space_identity = identity.clone(); let space_acknowledgement = acknowledgement.clone();
        let suffix = Uuid::new_v4().to_string(); let id = format!("localswarm:{suffix}");
        let create_manifest = manifest.clone(); let create_suffix = suffix;
        let owned = blocking(move || {
            let parent = directory;
            OwnedSet::check_space(&parent, &create_manifest, create_manifest.total_bytes, &space_identity, space_acknowledgement.as_deref())?;
            let (owned, open) = OwnedSet::create(parent, &create_manifest, &create_suffix)?;
            drop(open); Ok(owned)
        }).await?;
        if let Some((_, nonce, _)) = self.inner.last_start_manifest.lock().unwrap().as_mut() { nonce.take(); }
        let timestamp = now();
        let job = DownloadJob { id: id.clone(), url: format!("localswarm:{}", manifest.swarm_id), file_name: manifest.files[0].path.clone(), destination: owned.final_path(&manifest.files[0].path).to_string_lossy().into_owned(), status: "queued".into(), downloaded: 0, total: Some(manifest.total_bytes), error: None, speed: 0, eta_seconds: None, created_at: timestamp.clone(), updated_at: timestamp, completed_paths: Vec::new(), requires_pin: pin.is_some(), transport: "localswarm".into(), route: initial_route };
        {
            let mut state = self.inner.state.lock().unwrap();
            let staging = owned.parent.join(&owned.staging);
            state.jobs.insert(id.clone(), StoredJob { job: job.clone(), verified: vec![false; manifest.chunk_hashes.len()], manifest, host, port, owned, publishing: false, direct, history_dismissed: false, cleanup: None });
            if let Err(error) = self.persist(&state) {
                if error.committed {
                    let retained = state.jobs.get_mut(&id).unwrap();
                    retained.job.status = "failed".into();
                    retained.job.error = Some(error.message.clone());
                } else {
                    state.jobs.remove(&id);
                }
                return Err(format!("{}; staged files retained at {}", error.message, staging.display()));
            }
        }
        self.spawn(&id, pin, cancel, identity, acknowledgement)?; Ok(job)
    }
    fn spawn(&self, id: &str, pin: Option<String>, cancel: CancellationToken, space_identity: String, acknowledgement: Option<String>) -> Result<(), String> {
        let manager = self.clone(); let task_id = id.to_string(); let token = cancel.clone();
        // Register the running handle before the receiver can open any staging
        // file. Otherwise a pause that wins the scheduling race finds no handle,
        // returns without waiting, and leaves the receiver holding exclusive
        // file locks that the pause contract promises are released.
        let (start, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started.await;
            let result = manager.receive(&task_id, pin, token.clone(), space_identity, acknowledgement).await;
            if let Err(error) = result {
                let mut state = manager.inner.state.lock().unwrap();
                if let Some(saved) = state.jobs.get_mut(&task_id) {
                    if saved.job.status != "complete" {
                        let cancelled = token.is_cancelled();
                        saved.job.status = if cancelled { "paused" } else { "failed" }.into();
                        saved.job.error = if cancelled { None } else { Some(error) };
                        saved.job.speed = 0;
                        saved.job.eta_seconds = None;
                        saved.job.updated_at = now();
                    }
                }
                if let Err(e) = manager.persist(&state) { eprintln!("Saving LocalSwarm transfer: {e}"); }
            }
        });
        self.inner.state.lock().unwrap().running.insert(id.to_string(), Running { cancel, task, progress: None });
        let _ = start.send(());
        Ok(())
    }
    pub async fn pause(&self, id: &str, cancelled: bool) -> Result<(), String> {
        self.allowed()?; let _operation = self.inner.operation.lock().await; self.pause_locked(id, cancelled).await
    }
    async fn pause_locked(&self, id: &str, cancelled: bool) -> Result<(), String> {
        let running = self.inner.state.lock().unwrap().running.remove(id);
        if let Some(running) = running { running.cancel.cancel(); running.task.await.map_err(|e| format!("Joining LocalSwarm transfer: {e}"))?; }
        let mut state = self.inner.state.lock().unwrap(); let saved = state.jobs.get_mut(id).ok_or("LocalSwarm job not found")?;
        if saved.job.status != "complete" && (cancelled || !matches!(saved.job.status.as_str(), "failed" | "cancelled")) { saved.job.status = if cancelled { "cancelled" } else { "paused" }.into(); saved.job.speed = 0; saved.job.eta_seconds = None; saved.job.error = None; saved.job.updated_at = now(); }
        self.persist(&state).map_err(Into::into)
    }
    pub async fn resume(&self, id: &str, pin: Option<String>, acknowledgement: Option<String>) -> Result<(), String> {
        self.allowed()?; let _operation = self.inner.operation.lock().await;
        if self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?.jobs.get(id)
            .is_some_and(|saved| saved.cleanup.is_some() || saved.history_dismissed) {
            return Err("This LocalSwarm transfer was dismissed or entered file cleanup and cannot be resumed".into());
        }
        let pin = pin.filter(|pin| !pin.is_empty());
        self.pause_locked(id, false).await?;
        let saved = self.inner.state.lock().unwrap().jobs.get(id).cloned().ok_or("LocalSwarm job not found")?;
        if saved.job.status == "complete" { return Ok(()); }
        if saved.job.requires_pin && pin.as_deref().is_none_or(str::is_empty) { return Err("Enter this protected swarm's PIN again to resume".into()); }
        // receive() owns the one retained-prefix rehash under its admitted-work
        // assertion. Do not rehash here while the command's transfer gate hides
        // that work from the native idle-timer reconciler.
        let cancel = self.cancel_token();
        { let mut state = self.inner.state.lock().unwrap(); let saved = state.jobs.get_mut(id).ok_or("LocalSwarm job not found")?;
          if pin.is_some() { saved.job.requires_pin = true; } saved.job.status = "queued".into(); saved.job.error = None; self.persist(&state)?; }
        self.spawn(id, pin, cancel, id.to_string(), acknowledgement)
    }
    async fn candidates(&self, host: String, port: u16, sid: &str, direct: bool) -> VecDeque<ConnectionTarget> {
        let peers = self.inner.discovery.lock().await.peers();
        source_candidates(host, port, sid, peers, direct)
    }
    async fn refresh_candidates(&self, dialer: &mut SourceDialer) -> bool {
        dialer.add_discovered(self.inner.discovery.lock().await.peers())
    }
    async fn receive(&self, id: &str, pin: Option<String>, cancel: CancellationToken, space_identity: String, acknowledgement: Option<String>) -> Result<(), String> {
        let _slot = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("LocalSwarm transfer stopped while queued".into()),
            permit = self.inner.slots.acquire() => permit.map_err(err)?,
        };
        // Meaningful admitted work includes prefix rehash and final validation,
        // but a job merely waiting for the bounded worker slot cannot hold wake.
        #[cfg(target_os = "ios")]
        let _work = {
            self.inner.receive_work.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            ReceiveWork(&self.inner.receive_work)
        };
        // The same admitted window holds the macOS activity assertion, so App
        // Nap cannot coalesce the receive's timers and transport completions.
        // Dropped on completion, error, cancellation and shutdown.
        let _power = crate::power_activity::Guard::acquire("Receiving a LocalSwarm transfer");
        let saved = self.inner.state.lock().unwrap().jobs.get(id).cloned().ok_or("LocalSwarm job not found")?;
        let StoredJob { manifest, owned, host, port, publishing, direct, mut verified, .. } = saved;
        let owned = Arc::new(owned); let open_manifest = manifest.clone(); let open_owned = owned.clone(); let recheck_cancel = cancel.clone();
        let donor_store = self.inner.store.clone();
        let (open, checked) = blocking(move || {
            let mut open = open_owned.open(&open_manifest, publishing)?;
            rehash_verified(&mut open, &open_manifest, &open_owned, &mut verified, &recheck_cancel)?;
            // Adopted chunks consume the same space as downloaded ones, so the
            // space check and its acknowledgement precede any adoption write.
            let remaining = open_manifest.total_bytes.checked_sub(verified_bytes(&open_manifest, &verified)?)
                .ok_or("LocalSwarm verified size exceeds manifest")?;
            OwnedSet::check_space(&open.parent, &open_manifest, remaining, &space_identity, acknowledgement.as_deref())?;
            if !open.published && verified.iter().any(|bit| !bit) {
                let donors = crate::torrent::TorrentManager::saved_partial_files(&donor_store).unwrap_or_else(|error| {
                    eprintln!("Skipping BitTorrent partial reuse: {error}");
                    Vec::new()
                });
                adopt_partial_chunks(&mut open, &open_manifest, &mut verified, &donors, &recheck_cancel)?;
            }
            if !open.published && verified.iter().any(|bit| *bit) {
                // Older journals could snapshot in-flight bits on an unrelated
                // save. Flush checked claims and any adoption writes before
                // recording the repaired checkpoint.
                open.commit_data(&open_manifest, &open_owned)?;
            }
            Ok((Arc::new(Mutex::new(open)), verified))
        }).await?;
        self.checkpoint(id, &checked, "downloading", 0)?;
        let mut verified = checked;
        if verified.iter().any(|v| !v) {
            // Fsync the staged archive and validate its paths before recording
            // exactly the durable prefix. Both stages use the blocking pool.
            // Deliberately ignore cancellation: pausing must retain verified,
            // written bytes. This closure borrows this receive's fixed output.
            let commit = async |verified: &[bool], speed: u64, bytes: usize| -> Result<(), String> {
                let output = open.clone(); let commit_manifest = manifest.clone(); let commit_owned = owned.clone();
                let measured = profile::start();
                blocking(move || output.lock().map_err(|_| "LocalSwarm staging unavailable")?
                    .commit_data(&commit_manifest, &commit_owned)).await?;
                profile::record(7, bytes, measured);
                let manager = self.clone(); let id = id.to_string(); let verified = verified.to_vec();
                let measured = profile::start();
                blocking(move || manager.checkpoint(&id, &verified, "downloading", speed)).await?;
                profile::record(6, bytes, measured);
                Ok(())
            };
            let candidates = self.candidates(host, port, &manifest.swarm_id, direct).await;
            let mut dialer = SourceDialer::new(candidates, manifest.swarm_id.clone(), pin,
                self.inner.peer_id.clone(), cancel.clone(), self.inner.download_budget.clone(), self.inner.receive_activity.clone()).with_direct(direct);
            let transfer: Result<(), String> = async {
                let started = Instant::now(); let initial = verified_bytes(&manifest, &verified)?;
                // Verified chunks are written immediately; only the fsync, the
                // staged-identity check and the job-state checkpoint are batched,
                // so a crash re-downloads at most one window and the recorded
                // prefix is always durable.
                let mut uncommitted = 0usize;
                let mut uncommitted_bytes = 0usize;
                let mut fetched = 0u64;
                let (checkpoint_chunks, checkpoint_bytes) = checkpoint_window();
                let prefetch = Arc::new(tokio::sync::Semaphore::new(network::PREFETCH_BYTES / network::PREFETCH_UNIT));
                let (sender, mut receiver) = tokio::sync::mpsc::channel::<network::PipelineItem>(network::PIPELINE_BUFFERED);
                let mut joins: FuturesUnordered<PipelineJoin> = FuturesUnordered::new();
                let mut workers: Vec<PipelineWorker> = Vec::new();
                let mut assigned: HashMap<u32, usize> = HashMap::new();
                let mut assigned_count: HashMap<usize, usize> = HashMap::new();
                let mut next_source = 0usize;
                // Sources whose byte stream died, with that failure, so their
                // join can schedule a bounded redial instead of retiring the
                // peer for the whole job.
                let mut lost: HashMap<usize, String> = HashMap::new();
                let mut remaining = verified.iter().filter(|bit| !**bit).count();
                // Lowest index returned to the unassigned pool since the last
                // scheduling round; every forward cursor rewinds to it.
                let mut rewind = usize::MAX;
                let outcome: Result<(), String> = async {
                    let mut error: Option<String> = None;
                    while remaining != 0 {
                        if let Err(retired) = check_cancel(&cancel) { error = Some(retired); break; }
                        dialer.fill(workers.len());
                        // A source that can no longer contribute a missing chunk
                        // is retired; its slot may dial a replacement.
                        let mut retire = Vec::new();
                        for (position, worker) in workers.iter().enumerate() {
                            let busy = assigned_count.get(&worker.source).copied().unwrap_or(0) != 0;
                            if !busy && !worker.available.iter().enumerate().any(|(index, has)| *has && !verified[index]) {
                                retire.push(position);
                            }
                        }
                        for position in retire.into_iter().rev() {
                            workers.swap_remove(position).shutdown.cancel();
                        }
                        if rewind != usize::MAX {
                            for worker in &mut workers { worker.cursor = worker.cursor.min(rewind); }
                            rewind = usize::MAX;
                        }
                        // Keep each live connection's request window full. A batch
                        // that does not fit waits for the next scheduling round;
                        // its indices stay unassigned and can go to another source.
                        for worker in &mut workers {
                            let count = assigned_count.get(&worker.source).copied().unwrap_or(0);
                            if count >= PIPELINE_AHEAD { continue; }
                            let batch = worker.next_batch(&verified, &assigned);
                            if batch.is_empty() { continue; }
                            let length = batch.len();
                            for &index in &batch { assigned.insert(index, worker.source); }
                            match worker.batches.try_send(batch) {
                                Ok(()) => *assigned_count.entry(worker.source).or_insert(0) += length,
                                Err(error) => rewind = rewind.min(release_indices(&mut assigned, error.into_inner())),
                            }
                        }
                        // A source that just failed reports its error before its
                        // task is joined; its join is what records a bounded
                        // redial, so the job may not be declared sourceless
                        // while any task is still settling.
                        if workers.is_empty() && joins.is_empty() && !dialer.has_pending() {
                            if self.refresh_candidates(&mut dialer).await { continue; }
                            if !self.inner.receive_activity.accepting() {
                                // Only an established session may continue in
                                // background; closing admission pauses the job.
                                cancel.cancel();
                                if let Err(retired) = check_cancel(&cancel) { error = Some(retired); break; }
                            }
                            error = Some(dialer.error()); break;
                        }
                        let reconnect_wait = dialer.scheduled_wait().unwrap_or(Duration::from_millis(50));
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => { error = Some("LocalSwarm operation cancelled".into()); break; }
                            item = receiver.recv() => {
                                let Some((source, response)) = item else { error = Some(dialer.error()); break; };
                                match response {
                                    Ok(chunk) => {
                                        let network::VerifiedChunk { index, data, permit } = chunk;
                                        if verified[index as usize] { drop(permit); continue; }
                                        let length = data.len();
                                        let output = open.clone(); let write_manifest = manifest.clone(); let token = cancel.clone();
                                        let measured = profile::start();
                                        let written = blocking(move || {
                                            check_cancel(&token)?;
                                            let mut output = output.lock().map_err(|_| "LocalSwarm staging unavailable")?;
                                            let measured = profile::start();
                                            output.write_chunk(&write_manifest, index, &data)?;
                                            profile::record(5, length, measured);
                                            Ok(())
                                        }).await;
                                        profile::record(10, length, measured);
                                        drop(permit);
                                        if let Err(retired) = written { error = Some(retired); break; }
                                        verified[index as usize] = true;
                                        remaining -= 1;
                                        assigned.remove(&index);
                                        if let Some(count) = assigned_count.get_mut(&source) { *count = count.saturating_sub(1); }
                                        uncommitted += 1;
                                        uncommitted_bytes += length;
                                        fetched += length as u64;
                                        // In-memory progress is cheap and keeps the UI
                                        // moving; only the byte window is durable.
                                        let speed = (fetched as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
                                        self.advance(id, initial + fetched, speed);
                                        if uncommitted >= checkpoint_chunks || uncommitted_bytes >= checkpoint_bytes {
                                            let committed = match verified_bytes(&manifest, &verified) {
                                                Ok(downloaded) => commit(&verified,
                                                    ((downloaded - initial) as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64,
                                                    uncommitted_bytes).await,
                                                Err(retired) => Err(retired),
                                            };
                                            if let Err(retired) = committed { error = Some(retired); break; }
                                            uncommitted = 0; uncommitted_bytes = 0;
                                        }
                                    },
                                    Err(retired) => {
                                        // A corrupt, rejected or lost response
                                        // retires its source; its unfetched
                                        // chunks return to the unassigned pool.
                                        // A transport loss is recorded so the
                                        // join below can redial the same peer.
                                        if network::transient_transport_failure(&retired) { lost.insert(source, retired.clone()); }
                                        dialer.fail(retired);
                                        rewind = rewind.min(release_source(&mut assigned, source));
                                        assigned_count.remove(&source);
                                        if let Some(position) = workers.iter().position(|worker| worker.source == source) {
                                            workers.swap_remove(position).shutdown.cancel();
                                        }
                                    },
                                }
                            }
                            session = dialer.next(), if dialer.has_dials() => {
                                if let Some(session) = session {
                                    if let Some(route) = session.route() { self.record_route(id, route); }
                                    let worker = start_pipeline(next_source, session, &sender, &prefetch, &mut joins);
                                    next_source += 1;
                                    workers.push(worker);
                                }
                            }
                            _ = tokio::time::sleep(reconnect_wait), if dialer.has_scheduled() => {}
                            joined = joins.next(), if !joins.is_empty() => {
                                if let Some((source, result)) = joined {
                                    if let Ok(session) = result {
                                        // A lost link is retired as a connection
                                        // and redialed as the same peer; every
                                        // other failure is already terminal.
                                        if let Some(error) = lost.remove(&source) { dialer.retire(&session, &error); }
                                        session.shutdown().await;
                                    }
                                    if let Some(position) = workers.iter().position(|worker| worker.source == source) {
                                        workers.swap_remove(position).shutdown.cancel();
                                    }
                                    rewind = rewind.min(release_source(&mut assigned, source));
                                    assigned_count.remove(&source);
                                }
                            }
                        }
                    }
                    // Stop every connection before the final durability flush: no
                    // worker may hold a response or the staging lock while we commit.
                    drop(receiver);
                    for worker in workers.drain(..) { worker.shutdown.cancel(); }
                    while let Some((_, result)) = joins.next().await {
                        if let Ok(session) = result { session.shutdown().await; }
                    }
                    drop(sender);
                    match error { Some(error) => Err(error), None => Ok(()) }
                }.await;
                let downloaded = verified_bytes(&manifest, &verified)?;
                let speed = ((downloaded - initial) as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
                // Durability before completion, pause, cancellation and shutdown.
                let committed = commit(&verified, speed, uncommitted_bytes).await;
                match outcome { Ok(()) => committed, Err(error) => { let _ = committed; Err(error) } }
            }.await;
            dialer.stop().await;
            transfer?;
        }
        check_cancel(&cancel)?; self.checkpoint(id, &verified, "verifying", 0)?;
        let validate_open = open.clone(); let validate_manifest = manifest.clone(); let validate_owned = owned.clone(); let token = cancel.clone();
        let (validate_path, validate_files) = blocking(move || {
            check_cancel(&token)?; let mut open = validate_open.lock().map_err(|_| "LocalSwarm staging unavailable")?;
            open.seal(&validate_manifest, &validate_owned)?;
            let mut final_bits = vec![true; validate_manifest.chunk_hashes.len()];
            rehash_verified(&mut open, &validate_manifest, &validate_owned, &mut final_bits, &token)?;
            if final_bits.iter().any(|bit| !bit) { return Err("LocalSwarm staged bytes changed after chunk verification".into()); }
            Ok((open.directory.path.clone(), open.verification_files(&validate_manifest, &validate_owned)?))
        }).await?;
        let validate_manifest = manifest.clone();
        let token = cancel.clone();
        readonly_verification(&cancel, move || validate_zim(&validate_path, &validate_files, &validate_manifest, &token)).await?;
        check_cancel(&cancel)?;
        { let mut state = self.inner.state.lock().unwrap(); state.jobs.get_mut(id).ok_or("LocalSwarm job not found")?.publishing = true; self.persist(&state)?; }
        let publish_open = open.clone(); let publish_manifest = manifest.clone(); let publish_owned = owned.clone(); let store = self.inner.store.clone(); let token = cancel.clone();
        blocking(move || {
            check_cancel(&token)?; let mut open = publish_open.lock().map_err(|_| "LocalSwarm staging unavailable")?; open.unchanged(&publish_manifest, &publish_owned)?;
            let paths: Vec<_> = publish_manifest.files.iter().map(|f| publish_owned.final_path(&f.path)).collect();
            store.with_acquisition_destinations(&paths, || {
                check_cancel(&token)?;
                if !open.published {
                    let files = publish_manifest.files.iter().zip(&open.files).map(|(entry, file)| Ok((PathBuf::from(&entry.path), crate::publication::VerifiedFile::new(file.try_clone().map_err(err)?, entry.size_bytes).map_err(err)?))).collect::<Result<Vec<_>, String>>()?;
                    open.parent.unchanged()?; open.unchanged(&publish_manifest, &publish_owned)?;
                    open.publish(&publish_manifest, &publish_owned, files)?;
                }
                // Also cover a restart after the rename but before its sync.
                open.parent.sync()?;
                for (file, path) in open.files.iter().zip(&paths) { crate::mobile_backup::archive_policy(file, path)?; }
                open.unchanged(&publish_manifest, &publish_owned)
            })
        }).await?;
        let mut state = self.inner.state.lock().unwrap(); let saved = state.jobs.get_mut(id).ok_or("LocalSwarm job not found")?;
        saved.job.status = "complete".into(); saved.job.downloaded = manifest.total_bytes; saved.job.speed = 0; saved.job.eta_seconds = None; saved.job.error = None; saved.job.completed_paths = vec![saved.job.destination.clone()]; saved.job.updated_at = now(); self.persist(&state).map_err(Into::into)
    }
    /// In-flight progress belongs to this worker, never to the persisted job
    /// map. Route changes and writes by other jobs may snapshot the map before
    /// this worker's next payload flush.
    fn advance(&self, id: &str, downloaded: u64, speed: u64) {
        let mut state = self.inner.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(worker) = state.running.get_mut(id) {
            worker.progress = Some(ReceiveProgress { downloaded, speed, updated_at: now() });
        }
    }
    /// Persist the route that actually carried payload. Sessions are bounded
    /// (four source slots, bounded reconnects), and this writes only when the
    /// route changes, so the receipt cannot become a write amplifier.
    fn record_route(&self, id: &str, route: &str) {
        if route.is_empty() || route.len() > 240 || route.chars().any(char::is_control) { return; }
        let (sequence, jobs) = {
            let mut state = self.inner.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(saved) = state.jobs.get_mut(id) else { return };
            if saved.job.route.as_deref() == Some(route) { return; }
            saved.job.route = Some(route.to_string());
            self.stage(&state)
        };
        if let Err(error) = self.write_staged(sequence, &jobs) { eprintln!("Saving LocalSwarm route receipt: {}", error.message); }
    }
    fn checkpoint(&self, id: &str, verified: &[bool], status: &str, speed: u64) -> Result<(), String> {
        // Snapshot under the lock, then encode and fsync outside it: a
        // checkpoint after every durable window must not stall chunk progress,
        // status or listing behind a multi-megabyte rewrite.
        let (sequence, jobs) = {
            let mut state = self.inner.state.lock().unwrap();
            let saved = state.jobs.get_mut(id).ok_or("LocalSwarm job not found")?;
            saved.verified.copy_from_slice(verified); saved.job.downloaded = verified_bytes(&saved.manifest, verified)?; saved.job.status = status.into(); saved.job.speed = speed; saved.job.eta_seconds = if speed > 0 { Some((saved.manifest.total_bytes - saved.job.downloaded).div_ceil(speed)) } else { None }; saved.job.updated_at = now();
            self.stage(&state)
        };
        self.write_staged(sequence, &jobs).map_err(Into::into)
    }
    pub async fn seed(&self, book_id: &str, pin: Option<String>) -> Result<Seed, String> {
        self.allowed()?; let _operation = self.inner.operation.lock().await;
        let pin = pin.filter(|pin| !pin.is_empty());
        if self.inner.offers.lock().await.len() >= MAX_SEEDS { return Err("Stop an active LocalSwarm offer before adding another".into()); }
        let book = self.inner.store.book(book_id)?.filter(|b| b.available).ok_or("Select an available library archive")?;
        let cancel = self.cancel_token();
        let settings = self.inner.store.settings()?;
        let (manifest, source) = SourceSet::prepare(self.inner.store.clone(), book, cancel.clone()).await?;
        check_cancel(&cancel)?; source.unchanged()?;
        let source_reader = source.clone();
        let reason = Arc::new(Mutex::new(None));
        let monitor = SourceMonitor::start(&source, cancel.clone(), reason.clone())?;
        let reader_cancel = cancel.clone(); let reader_reason = reason.clone();
        let reader: network::ChunkReader = Arc::new(move |index| source_reader.chunk(index));
        let retire: network::SeedSourceFailure = Arc::new(move |error| {
            *reader_reason.lock().unwrap() = Some(error.to_owned());
            reader_cancel.cancel();
        });
        let server = network::seed(manifest.clone(), self.inner.peer_id.clone(), pin.clone(), reader,
            self.inner.upload_budget.clone(), cancel.clone(), Some(retire)).await?;
        if let Err(error) = source.unchanged().and_then(|_| check_cancel(&cancel)) { server.shutdown().await; return Err(error); }
        #[cfg(target_os = "ios")]
        server.stats.set_accepting(self.inner.sharing_admission.load(std::sync::atomic::Ordering::SeqCst));
        let advertisement = match self.inner.discovery.lock().await.advertise_addresses(&manifest, server.port, pin.is_some(), &server.addresses, crate::publisher::name(&settings)).await { Ok(id) => id, Err(e) => { server.shutdown().await; return Err(e); } };
        let row = Seed { id: format!("localswarm-seed:{}", Uuid::new_v4()), book_id: book_id.into(), name: manifest.name.clone(), swarm_id: manifest.swarm_id.clone(), total_bytes: manifest.total_bytes, chunk_count: manifest.chunk_hashes.len(), port: server.port, locked: pin.is_some(), uploaded: 0, upload_speed: 0, peer_count: 0, addresses: server.addresses.clone() };
        let stats = server.stats.clone(); let offer_id = row.id.clone();
        let weak = Arc::downgrade(&self.inner); let supervisor_source = source.clone();
        let (start, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started.await;
            Self::supervise_offer(weak, offer_id, supervisor_source, reason, monitor, server, advertisement).await
        });
        *self.inner.error.lock().unwrap() = None;
        let mut offers = self.inner.offers.lock().await;
        #[cfg(target_os = "ios")]
        stats.set_accepting(self.inner.sharing_admission.load(std::sync::atomic::Ordering::SeqCst));
        offers.insert(row.id.clone(), Offer {
            row: row.clone(), source, stats, cancel, task: Some(task),
            last_uploaded: 0, last_sample: Instant::now(),
        });
        drop(offers);
        let _ = start.send(());
        Ok(row)
    }
    async fn supervise_offer(
        weak: std::sync::Weak<Inner>, id: String, source: Arc<SourceSet>,
        reason: Arc<Mutex<Option<String>>>,
        monitor: SourceMonitor, server: SeedServer, advertisement: String,
    ) -> Result<(), String> {
        // The monitor already owns this exact offer-lifetime token.
        let cancel = monitor.cancel.clone();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = tick.tick() => {
                    if server.is_finished() {
                        *reason.lock().unwrap() = Some("The nearby-sharing listener stopped. Start sharing again to make this archive available.".into());
                        cancel.cancel();
                        break;
                    }
                    if let Err(error) = source.principal() {
                        *reason.lock().unwrap() = Some(error);
                        cancel.cancel();
                        break;
                    }
                }
            }
        }
        cancel.cancel();
        drop(monitor);
        server.shutdown().await;
        let Some(inner) = weak.upgrade() else { return Ok(()); };
        let result = inner.discovery.lock().await.unadvertise(&advertisement).await;
        let failure = reason.lock().unwrap().clone().or_else(|| result.as_ref().err().cloned());
        if let Some(error) = failure { *inner.error.lock().unwrap() = Some(error); }
        inner.offers.lock().await.remove(&id);
        result
    }
    async fn finish_offer(mut offer: Offer) -> Result<(), String> {
        offer.cancel.cancel();
        match offer.task.take() {
            Some(task) => task.await.map_err(|error| format!("Joining LocalSwarm source retirement: {error}"))?,
            None => Ok(()),
        }
    }
    /// Keep this admission guard through the caller's source mutation. A seed
    /// setup cannot publish between revocation and the principal/path commit.
    pub async fn quiesce_sources(&self, book_id: Option<&str>) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        let guard = self.inner.operation.lock().await;
        let retire_library = self.inner.library.lock().await.as_ref().is_some_and(|offer| offer.server.contains(book_id));
        if retire_library { self.stop_library_locked().await?; }
        let offers = {
            let mut offers = self.inner.offers.lock().await;
            let ids: Vec<_> = offers.iter().filter(|(_, offer)| book_id.is_none_or(|id| offer.row.book_id == id)).map(|(id, _)| id.clone()).collect();
            ids.into_iter().filter_map(|id| offers.remove(&id)).collect::<Vec<_>>()
        };
        for offer in &offers { offer.cancel.cancel(); }
        let mut errors = Vec::new();
        for offer in offers { if let Err(error) = Self::finish_offer(offer).await { errors.push(error); } }
        if errors.is_empty() { Ok(guard) } else { Err(errors.join("; ")) }
    }
    pub async fn revoke_invalid_sources(&self) -> Result<(), String> {
        let _guard = self.inner.operation.lock().await;
        let invalid_library = self.inner.library.lock().await.as_ref().is_some_and(|offer| !offer.server.valid_sources());
        if invalid_library { self.stop_library_locked().await?; }
        let offers = {
            let mut offers = self.inner.offers.lock().await;
            let ids: Vec<_> = offers.iter().filter(|(_, offer)| offer.source.principal().is_err()).map(|(id, _)| id.clone()).collect();
            ids.into_iter().filter_map(|id| offers.remove(&id)).collect::<Vec<_>>()
        };
        for offer in &offers { offer.cancel.cancel(); }
        let mut errors = Vec::new();
        for offer in offers { if let Err(error) = Self::finish_offer(offer).await { errors.push(error); } }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
    pub async fn stop_seed(&self, id: &str) -> Result<(), String> {
        let _operation = self.inner.operation.lock().await; let offer = self.inner.offers.lock().await.remove(id).ok_or("LocalSwarm offer not found")?;
        Self::finish_offer(offer).await
    }
    /// Background Android receives may continue under a foreground service,
    /// but offers and discovery must not retain a hidden network presence.
    #[cfg(target_os = "android")]
    pub async fn stop_sharing(&self) -> Result<(), String> {
        let _operation = self.inner.operation.lock().await;
        let library_error = self.stop_library_locked().await.err();
        let offers = std::mem::take(&mut *self.inner.offers.lock().await);
        for offer in offers.values() { offer.cancel.cancel(); }
        let mut errors: Vec<_> = library_error.into_iter().collect();
        for (_, offer) in offers {
            if let Err(error) = Self::finish_offer(offer).await { errors.push(error); }
        }
        if let Err(error) = self.inner.discovery.lock().await.shutdown().await {
            errors.push(error);
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
    pub async fn stop(&self) -> Result<Value, String> {
        self.inner.lifetime.lock().unwrap().cancel();
        let _operation = self.inner.operation.lock().await;
        let mut errors: Vec<_> = self.stop_library_locked().await.err().into_iter().collect();
        let ids: Vec<_> = self.inner.state.lock().unwrap().running.keys().cloned().collect();
        for id in ids { if let Err(e) = self.pause_locked(&id, false).await { errors.push(e); } }
        let offers = std::mem::take(&mut *self.inner.offers.lock().await);
        for (_, offer) in offers { if let Err(error) = Self::finish_offer(offer).await { errors.push(error); } }
        if let Err(e) = self.inner.discovery.lock().await.shutdown().await { errors.push(e); }
        if errors.is_empty() { self.status().await } else { Err(errors.join("; ")) }
    }
}
fn verified_bytes(manifest: &Manifest, bits: &[bool]) -> Result<u64, String> { bits.iter().enumerate().filter(|(_, b)| **b).try_fold(0u64, |n, (i, _)| n.checked_add(manifest.chunk_len(i as u32)? as u64).ok_or_else(|| "LocalSwarm verified size overflow".into())) }
/// Check durable claims only. Unclaimed staging bytes may be unwritten or
/// from a lost checkpoint window; the receiver must fetch them again. A set
/// already renamed by our durable publishing intent may only be revalidated
/// in full, never repaired.
fn rehash_verified(
    open: &mut OpenSet, manifest: &Manifest, owned: &OwnedSet,
    bits: &mut [bool], cancel: &CancellationToken,
) -> Result<(), String> {
    open.unchanged(manifest, owned)?;
    for (index, bit) in bits.iter_mut().enumerate() {
        check_cancel(cancel)?;
        if *bit || open.published {
            let bytes = open.read_chunk(manifest, index as u32)?;
            *bit = hex::encode(Sha256::digest(&bytes)) == manifest.chunk_hashes[index];
        }
    }
    if open.published && bits.iter().any(|bit| !bit) {
        return Err("Published LocalSwarm bytes no longer match their verified manifest".into());
    }
    open.unchanged(manifest, owned)
}
/// Copies chunks that are still missing from same-sized partial files owned by
/// other acquisitions (e.g. an unfinished BitTorrent download of the same
/// archive). A chunk is accepted only when its SHA-256 equals the manifest
/// hash, exactly as for a network chunk; donor provenance grants nothing.
/// Unwritten parts of a partial download read as zeros, so an all-zero chunk is
/// compared with a cached zero-chunk hash rather than hashed, and is never
/// counted as a content mismatch. A donor whose first `MISMATCH_LIMIT`
/// fully written (no all-zero 16 KiB block) candidate chunks all fail is
/// abandoned as different content.
/// Donor I/O failures skip that donor; staging write failures are returned.
/// Cancellation stops early and keeps what was adopted. Returns adopted bytes.
fn adopt_partial_chunks(
    open: &mut OpenSet, manifest: &Manifest, bits: &mut [bool],
    donors: &[crate::partial_donor::Donor], cancel: &CancellationToken,
) -> Result<u64, String> {
    let mut adopted = 0u64;
    let chunk = u64::from(manifest.chunk_size_bytes);
    let mut zero_hashes: HashMap<usize, String> = HashMap::new();
    for entry in &manifest.files {
        let (start, end) = (entry.start_chunk_index, entry.end_chunk_index);
        for donor in donors.iter().filter(|donor| donor.size == entry.size_bytes && entry.size_bytes > 0) {
            if !(start..=end).any(|index| !bits[index as usize]) { break; }
            let source = match crate::partial_donor::open(&donor.path, donor.size) {
                Ok(source) => source,
                Err(error) => { eprintln!("Skipping partial download {}: {error}", donor.path.display()); continue; }
            };
            let (mut matched, mut mismatched) = (0usize, 0usize);
            for index in start..=end {
                if cancel.is_cancelled() { return Ok(adopted); }
                if bits[index as usize] { continue; }
                let offset = u64::from(index - start) * chunk;
                let length = manifest.chunk_len(index)?;
                if !crate::partial_donor::covers(donor.ranges.as_deref(), offset, length as u64) { continue; }
                let mut bytes = vec![0u8; length];
                if let Err(error) = crate::partial_donor::read_at(&source, offset, &mut bytes) {
                    eprintln!("Stopped reading partial download {}: {error}", donor.path.display());
                    break;
                }
                if bytes.iter().all(|byte| *byte == 0) {
                    // Unwritten or genuinely zero: compare with the cached
                    // zero-chunk hash instead of hashing, and never count an
                    // unwritten region as a content mismatch.
                    let zero = zero_hashes.entry(length).or_insert_with(|| hex::encode(Sha256::digest(&bytes)));
                    if *zero != manifest.chunk_hashes[index as usize] { continue; }
                } else if hex::encode(Sha256::digest(&bytes)) != manifest.chunk_hashes[index as usize] {
                    // A chunk holding an unwritten 16 KiB block is most likely a
                    // piece still in flight, not evidence of different content.
                    if bytes.chunks(16 * 1024).any(|block| block.iter().all(|byte| *byte == 0)) { continue; }
                    mismatched += 1;
                    if matched == 0 && mismatched >= crate::partial_donor::MISMATCH_LIMIT { break; }
                    continue;
                }
                open.write_chunk(manifest, index, &bytes)?;
                bits[index as usize] = true;
                matched += 1;
                adopted += length as u64;
            }
            if matched != 0 {
                eprintln!("Reused {matched} verified chunks from partial download {}", donor.path.display());
            }
        }
    }
    Ok(adopted)
}
fn zim_header(files: &[(String, File)]) -> Result<[u8; 80], String> {
    let mut header = [0u8; 80];
    let mut filled = 0;
    for (_, file) in files {
        let mut reader = file;
        reader.seek(SeekFrom::Start(0)).map_err(err)?;
        while filled < header.len() {
            let count = reader.read(&mut header[filled..]).map_err(err)?;
            if count == 0 { break; }
            filled += count;
        }
        if filled == header.len() { return Ok(header); }
    }
    Err("LocalSwarm ZIM header is truncated".into())
}
fn validate_zim(directory: &Path, files: &[(String, File)], manifest: &Manifest, cancel: &CancellationToken) -> Result<(), String> {
    check_cancel(cancel)?;
    let expected = files.iter().map(|(_, file)| Stamp::of(&Metadata::from_file(file).map_err(err)?)).collect::<Result<Vec<_>, _>>()?;
    let header = zim_header(files)?;
    if &header[..4] != b"ZIM\x04" || u64::from_le_bytes(header[72..80].try_into().map_err(err)?).checked_add(16) != Some(manifest.total_bytes) {
        return Err("This archive is damaged. Remove it from your library and add it again".into());
    }
    let archive = Archive::open_files(directory, files)?;
    let metadata = archive.metadata_cancellable(cancel)?;
    if metadata.id.is_empty() || !archive.check_integrity_cancellable(cancel, |_, _| {})? {
        return Err("This archive is damaged. Remove it from your library and add it again".into());
    }
    check_cancel(cancel)?;
    for ((_, file), expected) in files.iter().zip(&expected) {
        if Stamp::of(&Metadata::from_file(file).map_err(err)?)? != *expected {
            return Err("This archive changed while Kiwix was checking it. Remove it from your library and add it again".into());
        }
    }
    Ok(())
}
struct SourceSet {
    store: Arc<Store>,
    book: Book,
    retained: Mutex<crate::archive_source::RetainedSource>,
    #[cfg(mobile)]
    stamps: Vec<Stamp>,
    manifest: Arc<Manifest>,
}
impl SourceSet {
    async fn prepare(store: Arc<Store>, book: Book, cancel: CancellationToken) -> Result<(Arc<Manifest>, Arc<Self>), String> {
        check_cancel(&cancel)?;
        let source_book = book.clone();
        if book.source_reference.is_some() {
            let (retained, identity) = readonly_verification(&cancel, move || {
                let retained = crate::archive_source::retain(&source_book)?;
                let identity = retained.identity()?;
                let archive = retained.open()?;
                if archive.metadata()?.id != source_book.metadata.id || !archive.check_integrity()? {
                    return Err("Kiwix could not verify this archive. Choose the file again".into());
                }
                if retained.identity()? != identity { return Err("This file changed while Kiwix was checking it. Choose it again".into()); }
                Ok((retained, identity))
            }).await?;
            blocking(move || Self::from_retained(store, book, cancel, retained, identity)).await
        } else {
            let pinned = readonly_verification(&cancel, move || crate::library_files::pin_archive_files(&source_book)).await?;
            blocking(move || Self::from_pinned(store, book, cancel, pinned)).await
        }
    }
    fn from_pinned(store: Arc<Store>, book: Book, cancel: CancellationToken,
        pinned: crate::library_files::PinnedArchive) -> Result<(Arc<Manifest>, Arc<Self>), String> {
        let retained = crate::archive_source::RetainedSource::Filesystem(pinned);
        retained.unchanged()?;
        let identity = retained.identity()?;
        Self::from_retained(store, book, cancel, retained, identity)
    }
    fn from_retained(store: Arc<Store>, book: Book, cancel: CancellationToken,
        mut retained: crate::archive_source::RetainedSource,
        identity: crate::integrity::ContentIdentity) -> Result<(Arc<Manifest>, Arc<Self>), String> {
        check_cancel(&cancel)?;
        retained.unchanged()?;
        if retained.identity()? != identity { return Err("Verified LocalSwarm source changed".into()); }
        let mut stamps = Vec::with_capacity(retained.files().len());
        let mut total = 0u64;
        for (_, file) in retained.files() {
            let stamp = Stamp::read_only_source(&Metadata::from_file(file).map_err(err)?)?;
            if stamp.size == 0 { return Err("Empty LocalSwarm archive parts are not supported".into()); }
            total = total.checked_add(stamp.size).ok_or("Archive size overflow")?;
            stamps.push(stamp);
        }
        let header = zim_header(retained.files())?;
        if &header[..4] != b"ZIM\x04" ||
            u64::from_le_bytes(header[72..80].try_into().map_err(err)?).checked_add(16) != Some(total) {
            return Err("LocalSwarm source parts must exactly cover the ZIM checksum boundary".into());
        }
        let mut chunk_size = 1024u32 * 1024;
        while stamps.iter().map(|s| s.size.div_ceil(u64::from(chunk_size))).sum::<u64>() > 262_144 {
            chunk_size = chunk_size.checked_mul(2).filter(|n| *n <= 16 * 1024 * 1024)
                .ok_or("Archive exceeds LocalSwarm manifest limits")?;
        }
        let mut entries = Vec::with_capacity(stamps.len());
        let mut hashes = Vec::new();
        let mut bytes = vec![0u8; chunk_size as usize];
        for ((name, file), stamp) in retained.files_mut().iter_mut().zip(&stamps) {
            let start = hashes.len() as u32;
            let mut left = stamp.size;
            file.seek(SeekFrom::Start(0)).map_err(err)?;
            while left > 0 {
                check_cancel(&cancel)?;
                let length = left.min(u64::from(chunk_size)) as usize;
                file.read_exact(&mut bytes[..length]).map_err(err)?;
                hashes.push(hex::encode(Sha256::digest(&bytes[..length])));
                left -= length as u64;
            }
            entries.push(SwarmFile { path: name.clone(), size_bytes: stamp.size,
                start_chunk_index: start, end_chunk_index: hashes.len() as u32 - 1 });
        }
        retained.unchanged()?;
        if retained.identity()? != identity { return Err("Archive changed while preparing LocalSwarm offer".into()); }
        let mut manifest = Manifest { zim: None, protocol_version: 1, swarm_id: String::new(),
            name: book.metadata.title.clone(), total_bytes: total, chunk_size_bytes: chunk_size,
            chunk_hash_algo: "sha256".into(), files: entries, chunk_hashes: hashes };
        manifest.swarm_id = manifest.content_address();
        supported(&manifest)?;
        let manifest = Arc::new(manifest);
        let source = Arc::new(Self { store, book, retained: Mutex::new(retained),
            #[cfg(mobile)]
            stamps,
            manifest: manifest.clone() });
        check_cancel(&cancel)?;
        source.unchanged()?;
        Ok((manifest, source))
    }
    fn principal(&self) -> Result<(), String> {
        // The store compares in place under its own lock: serving a chunk must
        // not clone a Book (or the whole state) per identity check.
        if self.store.owner_matches(&self.book)? { return Ok(()); }
        // Only the failure path pays for distinguishing the two verdicts.
        let installed = self.store.book(&self.book.metadata.id)?.filter(|b| b.available).is_some();
        Err(if installed { "Offered archive installation changed" } else { "Offered archive is no longer installed" }.into())
    }
    fn unchanged(&self) -> Result<(), String> {
        self.principal()?;
        let retained = self.retained.lock().map_err(|_| "LocalSwarm source unavailable")?;
        retained.unchanged()?;
        #[cfg(mobile)]
        if let crate::archive_source::RetainedSource::Provider(_) = &*retained {
            for ((_, file), stamp) in retained.files().iter().zip(&self.stamps) {
                if Stamp::read_only_source(&Metadata::from_file(file).map_err(err)?)? != *stamp {
                    return Err("Offered external archive changed; stop and prepare a new offer".into());
                }
            }
        }
        Ok(())
    }
    // Read the exact offered source. The shared transport hashes these copied
    // bytes once before responding, including for Apple QUIC.
    fn chunk(&self, index: u32) -> Result<Vec<u8>, String> {
        let measured = profile::start();
        self.unchanged()?;
        let (file, offset) = storage::chunk_position(&self.manifest, index)?;
        let mut bytes = vec![0u8; self.manifest.chunk_len(index)?];
        {
            let mut retained = self.retained.lock().map_err(|_| "LocalSwarm source unavailable")?;
            let file = &mut retained.files_mut()[file].1;
            file.seek(SeekFrom::Start(offset)).map_err(err)?;
            file.read_exact(&mut bytes).map_err(err)?;
        }
        self.unchanged()?;
        profile::record(0, bytes.len(), measured);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod monitored_claim_tests {
    use super::*;
    use std::fs;

    #[test]
    fn split_publication_claims_survive_unacknowledged_rename_and_lazy_restart() {
        let root = std::env::temp_dir().join(format!("kiwix-swarm-monitor-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let store = Arc::new(Store::new(root.join("profile")).unwrap());
        let external = root.join("external.zim");
        assert!(!LocalSwarmManager::saved_monitored_paths(&store).unwrap().claims_monitored_path(&external).unwrap());
        assert!(!store.data_dir().join("localswarm").exists());

        let bytes = include_bytes!("../../tests/fixtures/small.zim");
        let midpoint = bytes.len() / 2;
        let parts = [&bytes[..midpoint], &bytes[midpoint..]];
        let mut manifest = Manifest { zim: None,
            protocol_version: 1, swarm_id: String::new(), name: "Split monitor publication".into(),
            total_bytes: bytes.len() as u64, chunk_size_bytes: bytes.len() as u32,
            chunk_hash_algo: "sha256".into(),
            files: ["book.zimaa", "book.zimab"].iter().zip(parts).enumerate().map(|(index, (name, part))| SwarmFile {
                path: (*name).into(), size_bytes: part.len() as u64,
                start_chunk_index: index as u32, end_chunk_index: index as u32,
            }).collect(),
            chunk_hashes: parts.iter().map(|part| hex::encode(Sha256::digest(part))).collect(),
        };
        manifest.swarm_id = manifest.content_address();
        supported(&manifest).unwrap();
        let manifest = Arc::new(manifest);
        let suffix = Uuid::new_v4().to_string();
        let id = format!("localswarm:{suffix}");
        let (owned, mut open) = OwnedSet::create(Directory::open(&root).unwrap(), &manifest, &suffix).unwrap();
        for (index, part) in parts.iter().enumerate() {
            open.write_chunk(&manifest, index as u32, part).unwrap();
        }
        open.seal(&manifest, &owned).unwrap();
        let checked = open.verification_files(&manifest, &owned).unwrap();
        validate_zim(&open.directory.path, &checked, &manifest, &CancellationToken::new()).unwrap();
        drop(checked);
        let primary = owned.final_path("book.zimaa");
        let secondary = owned.final_path("book.zimab");
        let unrelated = owned.final_path("external.zim");
        let job = DownloadJob {
            id: id.clone(), url: format!("localswarm:{}", manifest.swarm_id), file_name: "book.zimaa".into(),
            destination: primary.to_str().unwrap().into(), status: "verifying".into(),
            downloaded: manifest.total_bytes, total: Some(manifest.total_bytes), error: None,
            speed: 0, eta_seconds: None, created_at: now(), updated_at: now(),
            completed_paths: Vec::new(), requires_pin: false, transport: "localswarm".into(), route: None,
        };
        let saved = StoredJob {
            job, manifest: manifest.clone(), host: "127.0.0.1".into(), port: 1,
            owned: owned.clone(), verified: vec![true; manifest.chunk_hashes.len()], publishing: true,
            direct: false, history_dismissed: false, cleanup: None,
        };
        validate_stored(&id, &saved).unwrap();
        let history = Directory::open(store.data_dir()).unwrap().child("localswarm", true).unwrap();
        history.atomic("jobs.json", &bounded_json(&BTreeMap::from([(id, saved)]), STATE_LIMIT as usize).unwrap()).unwrap();
        let claims = LocalSwarmManager::saved_monitored_paths(&store).unwrap();
        assert!(claims.claims_monitored_path(&primary).unwrap());
        assert!(claims.claims_monitored_path(&secondary).unwrap());
        assert!(!claims.claims_monitored_path(&unrelated).unwrap());

        let paths = [&primary, &secondary];
        let verified = manifest.files.iter().zip(&open.files).map(|(entry, file)| (
            PathBuf::from(&entry.path),
            crate::publication::VerifiedFile::new(file.try_clone().unwrap(), entry.size_bytes).unwrap(),
        )).collect::<Vec<_>>();
        store.with_acquisition_destinations(&paths, || open.publish(&manifest, &owned, verified)).unwrap();
        drop(open);
        // No completion checkpoint or manager initialization has occurred.
        let claims = LocalSwarmManager::saved_monitored_paths(&store).unwrap();
        assert!(claims.claims_monitored_path(&primary).unwrap());
        assert!(claims.claims_monitored_path(&secondary).unwrap());
        assert!(!claims.claims_monitored_path(&unrelated).unwrap());
        assert!(!store.data_dir().join("localswarm/peer-id").exists());
        assert!(owned.open(&manifest, true).unwrap().published);

        let manager = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
        assert!(manager.claims_monitored_path(&primary).unwrap());
        assert!(manager.claims_monitored_path(&secondary).unwrap());
        assert!(!manager.claims_monitored_path(&unrelated).unwrap());
        // Exclusion is not verification authority for replacement bytes.
        fs::rename(&secondary, root.join("retained-part")).unwrap();
        fs::write(&secondary, b"foreign replacement").unwrap();
        assert!(owned.open(&manifest, true).is_err());
        assert!(manager.claims_monitored_path(&secondary).unwrap());
        assert!(LocalSwarmManager::saved_monitored_paths(&store).unwrap().claims_monitored_path(&secondary).unwrap());
        drop(manager);
        drop(history);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod pipeline_cursor_tests {
    use super::*;

    fn worker(available: Vec<bool>) -> PipelineWorker {
        let (batches, _receiver) = tokio::sync::mpsc::channel(1);
        PipelineWorker { source: 0, available: Arc::new(available), batches, shutdown: CancellationToken::new(), cursor: 0 }
    }

    #[test]
    fn scheduling_cursor_advances_and_a_released_chunk_is_reoffered() {
        let verified = vec![false; 100];
        let mut assigned = HashMap::new();
        let mut worker = worker(vec![true; 100]);
        let first = worker.next_batch(&verified, &assigned);
        assert_eq!(first.len(), network::PIPELINE_WINDOW);
        assert_eq!(worker.cursor, network::PIPELINE_WINDOW);
        for &index in &first { assigned.insert(index, 0); }
        let second = worker.next_batch(&verified, &assigned);
        assert_eq!(second.first().copied(), Some(network::PIPELINE_WINDOW as u32));
        assert_eq!(worker.cursor, network::PIPELINE_WINDOW * 2);
        for &index in &second { assigned.insert(index, 0); }
        // Only a rewind re-offers a chunk the cursor already passed.
        let released = release_indices(&mut assigned, [second[0]]);
        assert_eq!(released, second[0] as usize);
        assert!(worker.next_batch(&verified, &assigned).iter().all(|index| *index > second[0]));
        worker.cursor = worker.cursor.min(released);
        assert_eq!(worker.next_batch(&verified, &assigned).first().copied(), Some(second[0]));
    }

    #[test]
    fn releasing_one_source_reports_its_lowest_chunk() {
        let mut assigned = HashMap::from([(3u32, 1usize), (9, 1), (4, 2)]);
        assert_eq!(release_source(&mut assigned, 1), 3);
        assert_eq!(assigned.keys().copied().collect::<Vec<_>>(), vec![4]);
        assert_eq!(release_source(&mut assigned, 7), usize::MAX);
    }
}
