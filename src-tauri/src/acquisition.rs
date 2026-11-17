// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent HTTP/QUIC acquisitions with fail-closed resume validation and no-clobber publication.
//!
//! The private `.kiwix-<uuid>.part` lives beside its destination. Publication uses an
//! atomic no-replace move, including on exFAT. The verified handle stays open until
//! the committed regular entry is checked against its inode without following links.
//! A durable SHA-256 publication intent permits recovery after the part moves but
//! before completion is recorded; a failed live identity check never adopts a path.
//! Internet downloads and redirects require HTTPS (literal loopback fixtures excepted).
//! Nearby uses an ephemeral selected-peer QUIC grant, never an HTTP policy exception.
//! Grants are released on completion, cancellation or row/file retirement; only
//! resumable ownership keeps them alive. Restored local rows must be browsed again.
//! Metalink size/hash metadata is authoritative over OPDS's sometimes rounded length.
//! `complete` means transport, size, ZIM magic and any supplied SHA-256 passed; the
//! caller must still open/integrity-check the archive before adding it to its library.
//! Terminal rows can be dismissed while a hidden ledger retains file ownership.
//! Explicit file deletion is separate, guarded, and never infers ownership from a name.
//! Native FAT/FAT32 per-file limits are checked before payload writes/truncation;
//! unknown-length bodies probe only when crossing the 4 GiB boundary.
//! Installed library paths stay reserved even when their archives are missing.
//! The Store mutation lock protects the final verified move from library changes;
//! recovery only inspects already-published files and never republishes a part.

use crate::catalog::{checked_url, http_client};
use crate::store::Store;
use crate::transfer_response::TransferResponse;
#[cfg(not(windows))]
use crate::fs_ext::FileExt;
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE,
    LAST_MODIFIED, RANGE,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio::sync::{watch, OnceCell, Semaphore};
use tokio::task::JoinSet;
use tokio_util::io::StreamReader;
use unicode_normalization::UnicodeNormalization;
use url::Url;
use uuid::Uuid;

mod persistence;
#[allow(unused_imports)]
pub(crate) use self::persistence::*;
mod manager;
#[allow(unused_imports)]
pub(crate) use self::manager::*;
mod segments;
#[allow(unused_imports)]
pub(crate) use self::segments::*;
mod transfer;
#[allow(unused_imports)]
pub(crate) use self::transfer::*;
mod finalize;
#[allow(unused_imports)]
pub(crate) use self::finalize::*;
mod http_meta;
#[allow(unused_imports)]
pub(crate) use self::http_meta::*;

const STATE_LIMIT: u64 = 16 * 1024 * 1024;
const MAX_JOBS: usize = 2000;
const DISK_RESERVE: u64 = 1024 * 1024;
const SPACE_CHECK_INTERVAL: u64 = 8 * 1024 * 1024;
const METALINK_LIMIT: usize = 4 * 1024 * 1024;
/// A live manager re-checks for fully durable rows without a worker this often.
const RECOVERY_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
/// A background transition may wait this long for a local finalize to finish;
/// it never aborts the pass, so a later foreground never restarts from zero.
const INTERRUPT_FINALIZE_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJob {
    pub id: String,
    /// The original user/catalog URL, not a temporary mirror URL.
    pub url: String,
    pub file_name: String,
    /// Absolute full path of the final archive (not the temporary part).
    pub destination: String,
    pub status: String,
    /// True only for running/queued work interrupted by shutdown, never a manual pause.
    #[serde(default)]
    pub interrupted: bool,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub speed: u64,
    pub eta_seconds: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
    pub sha256: Option<String>,
}

/// Verified publication identity of the finalized bytes. It is serialized with
/// the public snapshot only: the persisted row already owns this field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicationIdentity {
    pub size: u64,
    pub sha256: String,
}

/// Diagnostic provenance only; it never restores a selected-peer grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DownloadTransport {
    Http,
    QuicLibrary,
    Unknown,
}

impl DownloadTransport {
    fn resolve(recorded: Option<Self>, url: &str) -> Self {
        recorded.unwrap_or_else(|| {
            // The old whole-library HTTP path also used kiwix-library URLs.
            // Only ordinary HTTP(S) history has an unambiguous legacy label.
            if url.starts_with("http://") || url.starts_with("https://") {
                Self::Http
            } else {
                Self::Unknown
            }
        })
    }
}

/// One owned status observation, including provenance from the same journal row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DownloadSnapshot {
    #[serde(flatten)]
    pub job: DownloadJob,
    pub(crate) transport: DownloadTransport,
    /// Public receipt for the verified final bytes; absent until publication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicationIdentity>,
}

impl std::ops::Deref for DownloadSnapshot {
    type Target = DownloadJob;

    fn deref(&self) -> &Self::Target {
        &self.job
    }
}

/// Immutable publication authority copied into the import verifier. Its full
/// checksum runs on that worker, outside acquisition/import/Store locks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportPublication {
    File { destination: String, ownership: crate::torrent::CleanupEntry },
    LocalSwarm(crate::localswarm::ImportPublication),
}

impl ImportPublication {
    pub(crate) fn new(destination: String, ownership: crate::torrent::CleanupEntry) -> Self {
        Self::File { destination, ownership }
    }

    pub(crate) fn from_localswarm(publication: crate::localswarm::ImportPublication) -> Self {
        Self::LocalSwarm(publication)
    }

    pub(crate) fn destination(&self) -> &str {
        match self {
            Self::File { destination, .. } => destination,
            Self::LocalSwarm(publication) => publication.destination(),
        }
    }

    fn from_stored(stored: &StoredJob) -> Result<Self, String> {
        if stored.job.status != "complete" || stored.history_dismissed || stored.files_deleted
            || stored.part_ownership.as_ref().is_some_and(crate::torrent::CleanupEntry::cleanup_started)
            || stored.published_ownership.as_ref().is_some_and(crate::torrent::CleanupEntry::cleanup_started)
        {
            return Err("This download is no longer available for import".into());
        }
        let missing = "Kiwix cannot verify this download's original file. Add the archive to your library manually";
        let ownership = stored.published_ownership.clone().or_else(||
            stored.part_ownership.as_ref().map(|entry| entry.renamed(&stored.job.file_name)))
            .ok_or(missing)?;
        let publication = stored.publication.as_ref().ok_or(missing)?;
        let mut digest = [0u8; 32];
        hex::decode_to_slice(&publication.sha256, &mut digest).map_err(|error| error.to_string())?;
        Ok(Self::new(stored.job.destination.clone(), ownership.with_publication(publication.size, Some(digest))))
    }

    pub(crate) fn validate_source(&self, source: &crate::archive_source::RetainedSource, digest: bool) -> Result<(), String> {
        let (destination, ownership) = match self {
            Self::File { destination, ownership } => (destination, ownership),
            Self::LocalSwarm(publication) => return publication.validate_source(source, digest),
        };
        source.unchanged()?;
        if source.files().len() != 1 {
            return Err("A download must have exactly one published archive".into());
        }
        let file = &source.files()[0].1;
        let path = Path::new(destination);
        if !ownership.matches(path, file)?
            || (digest && !ownership.matches_retained(path, file)?)
        {
            return Err("The downloaded archive was replaced or changed. Kiwix left the file unchanged; add it to your library manually if needed".into());
        }
        source.unchanged()
    }

    pub(crate) fn verify(&self, source: &crate::archive_source::RetainedSource, cancelled: impl Fn() -> bool) -> Result<(), String> {
        self.validate_source(source, false)?;
        match self {
            Self::File { destination, ownership } => {
                if !ownership.matches_retained_controlled(Path::new(destination), &source.files()[0].1, cancelled)? {
                    return Err("The downloaded archive changed. Kiwix left the file unchanged; add it to your library manually if needed".into());
                }
            }
            Self::LocalSwarm(publication) => publication.verify(source, cancelled)?,
        }
        source.unchanged()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredJob {
    #[serde(flatten)]
    job: DownloadJob,
    /// Written at admission, independently of the synthetic library identity.
    /// Missing legacy provenance must not be inferred as QUIC from that URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport: Option<DownloadTransport>,
    expected_size: Option<u64>,
    transfer_url: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    #[serde(default)]
    publication: Option<PublicationIntent>,
    #[serde(default)]
    history_dismissed: bool,
    #[serde(default)]
    files_deleted: bool,
    #[serde(default)]
    part_ownership: Option<crate::torrent::CleanupEntry>,
    #[serde(default)]
    published_ownership: Option<crate::torrent::CleanupEntry>,
    /// Parallel range layout and its durable per-segment progress. `segmented`
    /// stays true even if the plan is discarded, because a preallocated part
    /// must never be mistaken for a complete file on the next resume.
    #[serde(default)]
    segments: Option<SegmentPlan>,
    #[serde(default)]
    segmented: bool,
    /// Durable proof of one completed checksum pass over the exact part inode.
    /// A restart reuses it only while every identity field still matches.
    #[serde(default)]
    verification: Option<VerificationReceipt>,
    /// Only the trusted official-catalog service operation creates this lease.
    /// Native owns transport staging; this journal owns its canonical part,
    /// durable adoption checkpoints, stop intent and eventual publication.
    #[serde(default)]
    native: Option<NativeAcquisition>,
    /// Consent belongs to one running admission, never to automatic restoration.
    #[serde(skip)]
    space_acknowledgement: Option<String>,
}

impl StoredJob {
    fn status_snapshot(&self) -> DownloadSnapshot {
        DownloadSnapshot {
            job: self.job.clone(),
            transport: DownloadTransport::resolve(self.transport, &self.job.url),
            publication: self.publication.as_ref().map(|published| PublicationIdentity {
                size: published.size,
                sha256: published.sha256.clone(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeAcquisition {
    generation: String,
    /// Bytes copied into the existing owned part and synchronized to disk.
    copied: u64,
    /// Set only after the native adapter supplies a complete durable response.
    total: Option<u64>,
    #[serde(default)]
    stage_identity: Option<PartIdentity>,
    #[serde(default)]
    retirement_error: Option<String>,
}

/// A start has exactly one authority: Internet policy or a scoped library grant.
enum AcquisitionSource {
    Internet(Option<NativeAcquisition>),
    SharedLibrary(crate::shared_library::ScopedDownloadGrant),
}

/// One byte interval of a segmented transfer. `done` is the number of bytes
/// from `start` that have been written and synced. `end` is inclusive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentRange {
    start: u64,
    end: u64,
    #[serde(default)]
    done: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentPlan {
    total: u64,
    /// The strong ETag or Last-Modified value every segment must match.
    validator: String,
    ranges: Vec<SegmentRange>,
}

impl SegmentPlan {
    fn durable(&self) -> u64 {
        self.ranges.iter().map(|range| range.done).sum()
    }

    #[cfg(test)]
    fn complete(&self) -> bool {
        self.ranges
            .iter()
            .all(|range| range.done == range.end - range.start + 1)
    }

    fn covers(&self, total: u64, validator: &str) -> bool {
        if self.total != total || self.validator != validator || self.ranges.is_empty() {
            return false;
        }
        let mut next = 0u64;
        for range in &self.ranges {
            let Some(exclusive_end) = range.end.checked_add(1) else {
                return false;
            };
            if range.start != next
                || range.end < range.start
                || range.done > exclusive_end - range.start
            {
                return false;
            }
            next = exclusive_end;
        }
        next == total
    }

    /// Split `total` bytes into at most `SEGMENT_COUNT` contiguous ranges.
    /// `prefix` bytes are already present in the part and recorded as done.
    fn build(total: u64, validator: String, prefix: u64) -> Option<Self> {
        if total < SEGMENT_MIN_TOTAL || prefix >= total {
            return None;
        }
        let remaining = total - prefix;
        let count = SEGMENT_COUNT
            .min((remaining / SEGMENT_MIN_LENGTH) as usize)
            .max(1);
        let length = remaining.div_ceil(count as u64);
        let mut ranges = Vec::with_capacity(count + 1);
        if prefix > 0 {
            ranges.push(SegmentRange {
                start: 0,
                end: prefix - 1,
                done: prefix,
            });
        }
        let mut start = prefix;
        while start < total {
            let end = start.saturating_add(length - 1).min(total - 1);
            ranges.push(SegmentRange {
                start,
                end,
                done: 0,
            });
            start = end + 1;
        }
        Some(Self {
            total,
            validator,
            ranges,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PublicationIntent {
    size: u64,
    sha256: String,
}

/// The exact part inode one checksum pass covered. `links` is part of the
/// receipt so a hard link created after the pass can never authorize reuse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartIdentity {
    size: u64,
    #[serde(default)]
    device: u64,
    #[serde(default)]
    inode: u64,
    #[serde(default)]
    mtime_seconds: i64,
    #[serde(default)]
    mtime_nanos: u32,
    #[serde(default)]
    links: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VerificationOutcome {
    Passed,
    Failed,
}

const VERIFICATION_RECEIPT_VERSION: u32 = 1;

/// A durable checksum receipt. Reuse is fail-closed: any unknown version,
/// malformed digest, changed identity or hard link forces a fresh hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationReceipt {
    #[serde(default)]
    version: u32,
    identity: PartIdentity,
    sha256: String,
    outcome: VerificationOutcome,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(unix)]
fn part_identity(metadata: &fs::Metadata) -> Option<PartIdentity> {
    use std::os::unix::fs::MetadataExt;
    Some(PartIdentity {
        size: metadata.len(),
        device: metadata.dev(),
        inode: metadata.ino(),
        mtime_seconds: metadata.mtime(),
        mtime_nanos: u32::try_from(metadata.mtime_nsec()).ok()?,
        links: metadata.nlink(),
    })
}

/// Without a stable device/inode pair a receipt cannot prove byte identity, so
/// no receipt is ever issued and every restart rehashes.
#[cfg(all(not(unix), not(windows)))]
fn part_identity(_metadata: &fs::Metadata) -> Option<PartIdentity> {
    None
}

/// Windows exposes the volume serial number, file index and link count only
/// through an open handle (`GetFileInformationByHandle`); the stable `std`
/// metadata accessors do not carry them. The volume serial and 64-bit file
/// index are the NTFS equivalent of `dev`/`ino`: both are stable across rename
/// and differ for any replacement file, so a receipt still proves "this exact
/// file" exactly as the Unix inode pair does.
#[cfg(windows)]
fn handle_identity(handle: std::os::windows::io::BorrowedHandle<'_>) -> std::io::Result<PartIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the borrowed handle stays open for this synchronous call and
    // `info` is a valid, writable output structure.
    unsafe { GetFileInformationByHandle(HANDLE(handle.as_raw_handle()), &mut info) }
        .map_err(crate::fs_ext::win32_error)?;
    // FILETIME counts 100 ns intervals since 1601-01-01 (UTC).
    let ticks = (u64::from(info.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(info.ftLastWriteTime.dwLowDateTime);
    let unix_ticks = i128::from(ticks) - 116_444_736_000_000_000;
    Ok(PartIdentity {
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
        device: u64::from(info.dwVolumeSerialNumber),
        inode: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        mtime_seconds: i64::try_from(unix_ticks.div_euclid(10_000_000))
            .map_err(|_| std::io::Error::other("File time is out of range"))?,
        mtime_nanos: (unix_ticks.rem_euclid(10_000_000) * 100) as u32,
        links: u64::from(info.nNumberOfLinks),
    })
}

/// Receipt identity of an open part. Unix reads it from `fstat`; Windows from
/// the handle itself, because path metadata carries no file index there.
fn file_identity(file: &File) -> Result<Option<PartIdentity>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        handle_identity(file.as_handle()).map(Some).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(part_identity(&file.metadata().map_err(|e| e.to_string())?))
    }
}

/// [`file_identity`] for the transfer worker's asynchronous handle.
async fn tokio_file_identity(file: &tokio::fs::File) -> Result<Option<PartIdentity>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        // A handle query can block on network volumes; keep it off the async
        // worker, as tokio's own metadata() does. The duplicate refers to the
        // same file object, so the identity is that of `file`.
        let duplicate = file.try_clone().await.map_err(|e| e.to_string())?.into_std().await;
        tokio::task::spawn_blocking(move || handle_identity(duplicate.as_handle()))
            .await
            .map_err(|e| e.to_string())?
            .map(Some)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(part_identity(&file.metadata().await.map_err(|e| e.to_string())?))
    }
}

/// Build a receipt for a completed pass. `None` means this platform cannot
/// prove identity and the caller must leave the receipt absent.
fn verification_receipt(
    identity: Option<PartIdentity>,
    digest: &str,
    outcome: VerificationOutcome,
    error: Option<String>,
) -> Option<VerificationReceipt> {
    let identity = identity?;
    hex::decode_to_slice(digest, &mut [0u8; 32]).ok()?;
    Some(VerificationReceipt {
        version: VERIFICATION_RECEIPT_VERSION,
        identity,
        sha256: digest.to_owned(),
        outcome,
        error,
    })
}

/// A receipt may only skip hashing while it proves this exact inode still holds
/// the same bytes. A changed size, mtime, device, inode or link count rehashes.
fn matching_receipt<'a>(
    stored: &'a StoredJob,
    identity: Option<&PartIdentity>,
) -> Option<&'a VerificationReceipt> {
    let receipt = stored.verification.as_ref()?;
    if receipt.version != VERIFICATION_RECEIPT_VERSION {
        return None;
    }
    hex::decode_to_slice(&receipt.sha256, &mut [0u8; 32]).ok()?;
    let identity = identity?;
    if identity.links != 1 || *identity != receipt.identity {
        return None;
    }
    Some(receipt)
}

/// Privacy and read-only revocation are hard stops even for a purely local
/// finalize. Any doubt about the policy fails closed.
fn finalize_forbidden(inner: &Inner) -> bool {
    match (inner.store.private_mode(), inner.store.read_only()) {
        (Ok(private), Ok(read_only)) => private || read_only,
        _ => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Signal {
    Run,
    Pause,
    Interrupt,
    Cancel,
}

#[derive(Clone)]
struct Running {
    command: watch::Sender<Signal>,
    finished: watch::Receiver<bool>,
}

#[cfg(test)]
struct AsyncGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(test)]
async fn pass_verify_gate(inner: &Inner, index: usize) {
    let gate = inner.verify_gates.lock().unwrap()[index].take();
    if let Some(gate) = gate {
        let _ = gate.entered.send(());
        let _ = gate.release.await;
    }
}

#[cfg(test)]
struct FinalizeGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

struct State {
    jobs: BTreeMap<String, StoredJob>,
    running: HashMap<String, Running>,
}

struct Inner {
    data_dir: PathBuf,
    store: Arc<Store>,
    admission: crate::update_admission::AdmissionGate,
    state: Mutex<State>,
    /// Monotonic order of staged journals. A snapshot staged later must never
    /// be replaced on disk by one staged earlier that happens to publish slower.
    journal_generation: AtomicU64,
    /// Highest generation whose bytes actually reached the disk.
    published_generation: Mutex<u64>,
    pending_starts: Mutex<HashMap<String, String>>,
    client: OnceCell<reqwest::Client>,
    shared_grants: Mutex<HashMap<String, crate::shared_library::ScopedDownloadGrant>>,
    slots: Arc<Semaphore>,
    download_limit: AtomicU64,
    bandwidth: Mutex<BandwidthBudget>,
    limit_changed: watch::Sender<()>,
    // Held for the manager's lifetime: independent application processes cannot race.
    _process_lock: File,
    /// Set only when an unreadable saved record was quarantined this launch.
    /// It is a quiet, one-time notice, never a startup failure.
    history_notice: Option<String>,
    /// Per-record skips from an otherwise readable history.
    #[cfg(test)]
    load_warnings: Vec<String>,
    /// Test observability: complete checksum passes actually executed.
    #[cfg(test)]
    hash_passes: AtomicU64,
    /// Test-only per-chunk delay so a foreground cycle can be interleaved.
    #[cfg(test)]
    hash_delay: AtomicU64,
    #[cfg(test)]
    finalize_gate: Mutex<Option<FinalizeGate>>,
    /// Test-only async gates around the transfer worker's `verifying`
    /// checkpoint: index 0 before it, index 1 right after it is persisted.
    #[cfg(test)]
    verify_gates: Mutex<[Option<AsyncGate>; 2]>,
    #[cfg(all(test, unix))]
    native_copy_gate: Mutex<Option<FinalizeGate>>,
    #[cfg(all(test, unix))]
    native_copy_bytes: AtomicU64,
    #[cfg(test)]
    warning_available: AtomicU64,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

/// Read-only exclusions for automatic discovery, never authority to import or
/// adopt the file at a claimed name. Lazy managers load this once per scan, and
/// every claim is reduced to the exact lexical key `monitored_path_matches`
/// compares, so a scan queries one key per discovered file instead of
/// re-normalizing every ledger row per file.
pub(crate) struct MonitoredPathClaims(HashSet<MonitoredPathKey>);

impl MonitoredPathClaims {
    pub(crate) fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        let mut keys = HashSet::with_capacity(paths.len());
        for path in &paths {
            keys.insert(monitored_path_key(monitored_path_text(path)?));
        }
        Ok(Self(keys))
    }

    pub(crate) fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        Ok(self.0.contains(&monitored_path_key(monitored_path_text(path)?)))
    }
}

/// The lexical identity `store::acquisition_paths_equal(path, other, "")`
/// compares: a root class plus the normalized component sequence, each folded
/// to its NFD full case fold. These helpers are deliberately private copies of
/// the `store.rs` normalization; `monitored_path_key_matches_the_store_comparison`
/// pins them to the authoritative comparison.
#[derive(PartialEq, Eq, Hash)]
struct MonitoredPathKey {
    root: MonitoredPathRoot,
    components: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MonitoredPathRoot {
    Relative,
    Unix,
    Drive(u8),
    Unc,
}

fn monitored_path_key(path: &str) -> MonitoredPathKey {
    let (root, tail, verbatim) = monitored_path_root(path);
    // Every comparison against a claim uses an empty working directory, so an
    // absolute path has no base components to prepend and a relative one only
    // keeps its root class.
    let win32 = cfg!(windows)
        || matches!(root, MonitoredPathRoot::Drive(_) | MonitoredPathRoot::Unc)
        || path.contains('\\');
    let components = monitored_path_components(tail, !verbatim && win32)
        .map(monitored_component_key)
        .collect();
    MonitoredPathKey { root, components }
}

/// Purely lexical, matching `store::acquisition_path_root` byte for byte.
fn monitored_path_root(path: &str) -> (MonitoredPathRoot, &str, bool) {
    let separator = |byte| byte == b'/' || byte == b'\\';
    let bytes = path.as_bytes();
    let verbatim = bytes.starts_with(br"\\?\");
    let device = bytes.len() >= 4
        && separator(bytes[0])
        && separator(bytes[1])
        && matches!(bytes[2], b'?' | b'.')
        && separator(bytes[3]);
    let path = if device {
        let path = &path[4..];
        if path.len() >= 4
            && path.as_bytes()[..3].eq_ignore_ascii_case(b"UNC")
            && separator(path.as_bytes()[3])
        {
            return (MonitoredPathRoot::Unc, &path[4..], verbatim);
        }
        path
    } else {
        path
    };
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && separator(bytes[2])
    {
        return (
            MonitoredPathRoot::Drive(bytes[0].to_ascii_lowercase()),
            &path[3..],
            verbatim,
        );
    }
    if bytes.len() >= 2
        && separator(bytes[0])
        && separator(bytes[1])
        && (cfg!(windows) || bytes[0] == b'\\' || bytes[1] == b'\\')
    {
        (MonitoredPathRoot::Unc, &path[2..], verbatim)
    } else if bytes.first().copied().is_some_and(separator) {
        (MonitoredPathRoot::Unix, &path[1..], verbatim)
    } else {
        (MonitoredPathRoot::Relative, path, verbatim)
    }
}

fn monitored_component(component: &str, trim: bool, final_component: bool) -> &str {
    if !trim || matches!(component, "." | "..") {
        component
    } else if final_component {
        component.trim_end_matches(['.', ' '])
    } else if !component.ends_with("..") {
        component.strip_suffix('.').unwrap_or(component)
    } else {
        component
    }
}

fn monitored_path_components(tail: &str, trim: bool) -> impl Iterator<Item = &str> {
    // The empty working directory contributes no base components, so only the
    // tail's reverse order matters for `.`/`..` folding.
    let mut parents = 0usize;
    tail.rsplit(['/', '\\'])
        .enumerate()
        .map(move |(index, component)| monitored_component(component, trim, index == 0))
        .filter(move |component| match *component {
            "" | "." => false,
            ".." => {
                parents = parents.saturating_add(1);
                false
            }
            _ if parents != 0 => {
                parents -= 1;
                false
            }
            _ => true,
        })
}

fn monitored_component_key(component: &str) -> String {
    // ASCII is NFD-stable and folds to its lowercase, exactly like the
    // both-ASCII branch of `store::folded_nfd_eq`.
    if component.is_ascii() {
        return component.to_ascii_lowercase();
    }
    let nfd: String = component.nfd().collect();
    crate::casefold::fold(&nfd).nfd().collect()
}

fn monitored_path_text(path: &Path) -> Result<&str, String> {
    if !path.is_absolute() {
        return Err("Acquisition monitor claim requires an absolute path".into());
    }
    path.to_str().filter(|path| !path.contains('\0'))
        .ok_or_else(|| "Acquisition monitor claim requires a Unicode path without NUL".into())
}

pub(crate) fn monitored_path_matches(path: &Path, claimed: &Path) -> Result<bool, String> {
    Ok(crate::store::acquisition_paths_equal(
        monitored_path_text(path)?, monitored_path_text(claimed)?, "",
    ))
}

/// Compare a durable directory/name pair without allocating a joined path.
/// Root/verbatim handling and alias folding are identical to full-path claims.
pub(crate) fn monitored_child_matches(path: &Path, directory: &Path, name: &Path) -> Result<bool, String> {
    let name = name.to_str().ok_or("Acquisition monitor claim has a non-Unicode filename")?;
    monitored_descendant_matches(path, directory, &[name])
}

pub(crate) fn monitored_descendant_matches(
    path: &Path, directory: &Path, names: &[&str],
) -> Result<bool, String> {
    for name in names {
        let component = Path::new(name);
        if name.contains('\0') || component.components().count() != 1
            || component.file_name() != Some(component.as_os_str())
        {
            return Err("Acquisition monitor claim has an invalid filename".into());
        }
    }
    Ok(crate::store::acquisition_path_parts_equal(
        monitored_path_text(path)?, monitored_path_text(directory)?, names,
    ))
}

/// Why a bounded, read-only history record could not supply ownership claims.
/// Only an absent record or invalid content means "no claims": a transient
/// profile failure must stay an error so a folder scan never reads an
/// unreadable journal as proof that our outputs are unclaimed.
pub(crate) enum SavedRecordError {
    NoClaims,
    Unavailable(String),
}

impl std::fmt::Display for SavedRecordError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoClaims => formatter.write_str(
                "Saved transfer history is unreadable and carries no ownership claims",
            ),
            Self::Unavailable(error) => formatter.write_str(error),
        }
    }
}

impl From<SavedRecordError> for String {
    fn from(error: SavedRecordError) -> Self {
        error.to_string()
    }
}

/// Capability-relative, bounded history read only: no manager construction,
/// recovery, directory creation, process lock, or network initialization.
pub(crate) fn saved_monitored_record<T: serde::de::DeserializeOwned>(
    store: &Store, components: &[&str], limit: u64, kind: crate::profile_paths::RecordKind,
) -> Result<Option<T>, SavedRecordError> {
    use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
    let (_, mut directory) = store
        .acquisition_profile_directory()
        .map_err(SavedRecordError::Unavailable)?;
    let (name, parents) = components
        .split_last()
        .ok_or_else(|| SavedRecordError::Unavailable("Missing acquisition history filename".into()))?;
    for component in components {
        if Path::new(component).components().count() != 1
            || Path::new(component).file_name() != Some(std::ffi::OsStr::new(component))
        {
            return Err(SavedRecordError::Unavailable("Invalid acquisition history component".into()));
        }
    }
    for parent in parents {
        directory = match directory.open_dir_nofollow(parent) {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SavedRecordError::Unavailable(error.to_string())),
        };
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = match directory.open_with(name, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SavedRecordError::Unavailable(error.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(|error| SavedRecordError::Unavailable(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(SavedRecordError::NoClaims);
    }
    let mut value: serde_json::Value = serde_json::from_reader(file.take(limit + 1))
        .map_err(|error| {
            // A truncated or malformed record is invalid content; a read that
            // failed for another reason is an unavailable profile, not an
            // empty claim list.
            if error.is_io()
                && !matches!(error.io_error_kind(), Some(std::io::ErrorKind::UnexpectedEof))
            {
                SavedRecordError::Unavailable(error.to_string())
            } else {
                SavedRecordError::NoClaims
            }
        })?;
    #[cfg(mobile)]
    store.mobile_rebase_saved_json(kind, &mut value).map_err(|_| SavedRecordError::NoClaims)?;
    #[cfg(desktop)]
    store.portable_rebase_saved_json(kind, &mut value).map_err(|_| SavedRecordError::NoClaims)?;
    serde_json::from_value(value).map(Some).map_err(|_| SavedRecordError::NoClaims)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn safe_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 240
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
        || !name.to_ascii_lowercase().ends_with(".zim")
    {
        return Err("Filename must be a single safe .zim filename, at most 240 UTF-8 bytes".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("Filename is reserved by the operating system".into());
    }
    Ok(())
}

pub(crate) fn shared_file_name(name: &str) -> Result<(), String> {
    safe_name(name)
}

fn decode_segment(value: &str) -> Result<String, String> {
    let mut output = Vec::with_capacity(value.len());
    let input = value.as_bytes();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            if i + 2 >= input.len() {
                return Err("Malformed percent-encoded filename".into());
            }
            let high = (input[i + 1] as char)
                .to_digit(16)
                .ok_or("Malformed percent-encoded filename")?;
            let low = (input[i + 2] as char)
                .to_digit(16)
                .ok_or("Malformed percent-encoded filename")?;
            output.push((high * 16 + low) as u8);
            i += 3;
        } else {
            output.push(input[i]);
            i += 1;
        }
    }
    String::from_utf8(output).map_err(|_| "URL filename is not UTF-8".into())
}

fn filename(url: &Url) -> Result<String, String> {
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|s| !s.is_empty())
        .ok_or("URL has no filename; specify one explicitly")?;
    let mut name = decode_segment(segment)?;
    for suffix in [".meta4", ".metalink"] {
        if name.ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    safe_name(&name)?;
    Ok(name)
}

/// Sum the remaining bytes of other active HTTP/native/shared-library jobs that
/// target the same volume. Torrent records and LocalSwarm multi-file owned-set
/// transfers have separate journals and retain their own capacity guards;
/// shared-library `quic-library` archive downloads use this ledger and are
/// included. Native staging for other jobs is not in this journal; their
/// canonical parts are counted, and each native admission separately reserves
/// its own stage. Callers must hold the state lock; `exclude_id` prevents
/// double-counting the request being admitted.
fn outstanding_space_from_state(
    state: &State,
    directory: &Path,
    exclude_id: &str,
) -> Result<u64, String> {
    let target = crate::disk_warning::volume_device(directory)?;
    let mut outstanding = 0u64;
    for (id, stored) in &state.jobs {
        if id == exclude_id || stored.history_dismissed || stored.files_deleted {
            continue;
        }
        if !matches!(stored.job.status.as_str(), "queued" | "downloading") {
            continue;
        }
        let Ok((_, part)) = paths(&stored.job) else { continue; };
        let Some(parent) = part.parent() else { continue; };
        let Ok(device) = crate::disk_warning::volume_device(parent) else { continue; };
        if device != target { continue; }
        let Some(total) = stored.expected_size.or(stored.job.total) else { continue; };
        let allocated = fs::symlink_metadata(&part)
            .ok()
            .filter(|metadata| metadata.file_type().is_file())
            .map_or(0, |metadata| metadata.len());
        outstanding = outstanding.saturating_add(space_remaining(
            total,
            stored.job.downloaded,
            allocated,
        ));
    }
    Ok(outstanding)
}

fn paths(job: &DownloadJob) -> Result<(PathBuf, PathBuf), String> {
    Uuid::parse_str(&job.id).map_err(|_| "Invalid persisted download ID")?;
    if Uuid::parse_str(&job.id)
        .map_err(|_| "Invalid persisted download ID")?
        .to_string()
        != job.id
    {
        return Err("Noncanonical persisted download ID".into());
    }
    safe_name(&job.file_name)?;
    let destination = PathBuf::from(&job.destination);
    let parent = destination
        .parent()
        .filter(|p| destination.is_absolute() && !p.as_os_str().is_empty())
        .ok_or("Download destination is not absolute")?;
    if destination.file_name().and_then(|n| n.to_str()) != Some(job.file_name.as_str()) {
        return Err("Persisted destination does not match filename".into());
    }
    if parent.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err("Persisted destination is not normalized".into());
    }
    let part = parent.join(format!(".kiwix-{}.part", job.id));
    Ok((destination, part))
}

fn regular_file(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => Err(format!("Refusing non-regular file: {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Inspecting {}: {e}", path.display())),
    }
}

/// Remove a cancelled UUID-owned part only when the journal proves that the
/// current inode is the one Kiwix created. A missing journal, a replaced inode,
/// a symlink or a directory leaves the bytes untouched and reports why.
fn remove_owned_part(
    part: &Path,
    ownership: Option<&crate::torrent::CleanupEntry>,
    checkpoint: impl FnMut(&crate::torrent::CleanupEntry) -> Result<(), String>,
) -> Result<(), String> {
    if regular_file(part)?.is_none() && !ownership.is_some_and(crate::torrent::CleanupEntry::cleanup_started) {
        return Ok(());
    }
    let Some(ownership) = ownership else {
        return Err(
            "Cancelled download file was left in place because Kiwix cannot prove it created it"
                .into(),
        );
    };
    ownership.validate_cleanup(part).map_err(|error| format!(
        "Cancelled download file was left in place because Kiwix cannot prove it created it: {error}"
    ))?;
    // A pathname check cannot authorize a later unlink: capture privately,
    // journal that capture, then check the captured inode before deleting it.
    let mut ownership = ownership.clone();
    crate::torrent::delete_owned_file(part, &mut ownership, checkpoint)
}

fn remove_job_part(inner: &Inner, state: &mut State, id: &str) -> Result<(), String> {
    let stored = state.jobs.get(id).ok_or("Unknown download ID")?;
    let (_, part) = paths(&stored.job)?;
    let ownership = stored.part_ownership.clone();
    inner.store.with_acquisition_destinations(&[&part], || {
        remove_owned_part(&part, ownership.as_ref(), |entry| {
            state.jobs.get_mut(id).ok_or("Unknown download ID")?.part_ownership = Some(entry.clone());
            inner.save(state)
        })
    })
}

fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino() && a.nlink() == 1
    }
    #[cfg(not(unix))]
    {
        a.is_file() && b.is_file() && a.len() == b.len()
    }
}

fn writable_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open a reparse point itself, never its target. Deny concurrent writers,
        // but retain FILE_SHARE_DELETE so verified publication need not close the
        // handle whose bytes were hashed before its committed identity is checked.
        options.custom_flags(0x0020_0000).share_mode(1 | 4);
    }
    options
}

/// The coordinator's part handle and every segment handle must be able to
/// coexist. Windows share modes are enforced against open handles, so the
/// transfer handle allows read/write/delete sharing; the exclusive advisory lock
/// still keeps other processes out. Unix has no equivalent restriction.
fn writable_options_shared() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = writable_options();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1 | 2 | 4);
    }
    options
}

/// Byte offset of the Windows ownership lock: far beyond any archive this
/// manager can write (and inside the signed range NTFS accepts for locks).
#[cfg(windows)]
const OWNERSHIP_LOCK_OFFSET: u64 = i64::MAX as u64 - 1;

/// Take the exclusive, cooperative ownership lock of a download part or of the
/// manager's process lock file.
///
/// Unix `flock` is advisory: it excludes other lockers but never blocks reads
/// or writes through other descriptors, which is what segment workers (one
/// handle per range) and readers of the lock file rely on. A whole-file
/// Windows `LockFileEx` range is *mandatory* instead: every other handle,
/// including this process's own segment handles, then fails its writes with
/// `ERROR_LOCK_VIOLATION` (33). Windows therefore locks one sentinel byte past
/// any real payload. Every other ownership lock (this sentinel, or a
/// whole-file `try_flock_*` from `fs_ext`) overlaps that byte, so exclusivity
/// between lockers is exactly as strong as before, while payload I/O keeps the
/// Unix advisory semantics. The lock is released when the handle closes.
fn try_lock_exclusive(file: &File) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        use windows::Win32::System::IO::OVERLAPPED;
        let mut overlapped = OVERLAPPED::default();
        overlapped.Anonymous.Anonymous.Offset = OWNERSHIP_LOCK_OFFSET as u32;
        overlapped.Anonymous.Anonymous.OffsetHigh = (OWNERSHIP_LOCK_OFFSET >> 32) as u32;
        // SAFETY: `file` owns the handle for the duration of this synchronous
        // request; the OVERLAPPED only carries the lock offset.
        unsafe {
            LockFileEx(
                HANDLE(file.as_raw_handle()),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                None,
                1,
                0,
                &mut overlapped,
            )
        }
        .map_err(crate::fs_ext::win32_error)
    }
    #[cfg(not(windows))]
    {
        file.try_flock_exclusive()
    }
}

fn open_part(path: &Path) -> Result<File, String> {
    let before = regular_file(path)?.ok_or("Temporary download file is missing")?;
    let file = writable_options_shared()
        .open(path)
        .map_err(|e| format!("Opening download part: {e}"))?;
    let opened = file
        .metadata()
        .map_err(|e| format!("Inspecting opened download part: {e}"))?;
    let after = regular_file(path)?.ok_or("Temporary download file disappeared")?;
    if !same_file(&before, &opened) || !same_file(&opened, &after) {
        return Err("Temporary download file changed identity or has hard links".into());
    }
    // Path metadata has no link count on Windows; read it from the handle so a
    // hard-linked part is refused exactly as `same_file` refuses it on Unix.
    #[cfg(windows)]
    if file_identity(&file)?.is_none_or(|identity| identity.links != 1) {
        return Err("Temporary download file changed identity or has hard links".into());
    }
    try_lock_exclusive(&file)
        .map_err(|e| format!("Download part is already in use: {e}"))?;
    Ok(file)
}

/// Open one segment's independent cursor on the already-locked part. Identity
/// is checked against the coordinator's handle before any byte is written.
/// `handle` is the coordinator's handle identity: on Windows path metadata
/// cannot carry a file index, so the volume/file-index pair is compared here.
fn open_segment_part(
    path: &Path,
    identity: &fs::Metadata,
    handle: Option<&PartIdentity>,
) -> Result<File, String> {
    let file = writable_options_shared()
        .open(path)
        .map_err(|e| format!("Opening download part segment: {e}"))?;
    let opened = file
        .metadata()
        .map_err(|e| format!("Inspecting download part segment: {e}"))?;
    if !same_file(identity, &opened) {
        return Err("Download part changed identity during a segmented transfer".into());
    }
    #[cfg(windows)]
    {
        let opened = file_identity(&file)?;
        let same = matches!((handle, opened.as_ref()), (Some(expected), Some(opened))
            if expected.device == opened.device && expected.inode == opened.inode && opened.links == 1);
        if !same {
            return Err("Download part changed identity during a segmented transfer".into());
        }
    }
    #[cfg(not(windows))]
    let _ = handle;
    Ok(file)
}

fn publication_digest(path: &Path, before: &fs::Metadata) -> Result<Option<(String, File)>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000).share_mode(1);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("Opening published archive: {e}"))?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || opened.len() != before.len() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Ok(None);
        }
    }
    let Some(digest) = read_publication_digest(&mut file, before.len())? else {
        return Ok(None);
    };
    let after = regular_file(path)?.ok_or("Published archive disappeared during recovery")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if after.dev() != opened.dev() || after.ino() != opened.ino() {
            return Ok(None);
        }
    }
    if after.len() != before.len() {
        return Ok(None);
    }
    Ok(Some((digest, file)))
}

// Bound reads to the admitted length plus one byte: a same-inode writer must
// not keep cleanup/recovery hashing forever by appending faster than we read.
fn read_publication_digest(source: &mut impl Read, size: u64) -> Result<Option<String>, String> {
    if size < 4 { return Ok(None); }
    let mut signature = [0u8; 4];
    source.read_exact(&mut signature)
        .map_err(|e| format!("Reading published ZIM signature: {e}"))?;
    if signature != *b"ZIM\x04" { return Ok(None); }
    let mut hasher = Sha256::new();
    hasher.update(signature);
    let mut buffer = [0u8; 64 * 1024];
    let mut length = 4u64;
    loop {
        let remaining = size.saturating_sub(length);
        let limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let count = source.read(&mut buffer[..limit])
            .map_err(|e| format!("Reading published archive checksum: {e}"))?;
        if count == 0 { break; }
        if count as u64 > remaining { return Ok(None); }
        length += count as u64;
        hasher.update(&buffer[..count]);
    }
    Ok((length == size).then(|| hex::encode(hasher.finalize())))
}

fn recover_publication(
    stored: &StoredJob,
    destination: &Path,
    part: &Path,
) -> Result<bool, String> {
    recover_publication_with_policy(stored, destination, part, |file, path| {
        crate::mobile_backup::archive_policy(file, path)
    })
}

fn recover_publication_with_policy(
    stored: &StoredJob,
    destination: &Path,
    part: &Path,
    policy: impl FnOnce(&File, &Path) -> Result<(), String>,
) -> Result<bool, String> {
    let Some(final_meta) = regular_file(destination)? else {
        return Ok(false);
    };
    if final_meta.len() < 80
        || stored.job.total != Some(final_meta.len())
        || stored.job.downloaded != final_meta.len()
    {
        return Ok(false);
    }
    // Current publishers atomically move the part. A remaining public name
    // (including an old hard-link pair) requires explicit cleanup; recovery
    // never deletes a pathname that can be replaced after inspection.
    if regular_file(part)?.is_some() || stored.publication.is_none() {
        return Ok(false);
    }
    let Some((digest, file)) = publication_digest(destination, &final_meta)? else {
        return Ok(false);
    };
    // A digest proves content, never that this acquisition created the entry.
    // Older move journals retained the part's inode but not its renamed proof.
    let ownership = stored.published_ownership.clone().or_else(||
        stored.part_ownership.as_ref().map(|entry| entry.renamed(&stored.job.file_name)))
        .ok_or("Kiwix cannot verify this download's original file. Add the archive to your library manually")?;
    if !ownership.matches(destination, &file)? {
        return Err("The downloaded archive was replaced. Kiwix left the replacement file unchanged; add it to your library manually if needed".into());
    }
    if stored
        .job
        .sha256
        .as_ref()
        .is_some_and(|expected| expected != &digest)
    {
        return Ok(false);
    }
    if let Some(intent) = &stored.publication {
        if intent.size != final_meta.len() || intent.sha256 != digest {
            return Ok(false);
        }
    }
    // Replay policy on the owned descriptor whose bytes matched the durable intent
    // before any recovery caller is allowed to acknowledge completion.
    policy(&file, destination)?;
    Ok(true)
}

#[cfg(target_os = "ios")]
pub(crate) fn backup_evidence(store: &Store) -> Result<Vec<(PathBuf, crate::library_files::BackupEvidence)>, String> {
    let saved: BTreeMap<String, StoredJob> = crate::library_files::backup_record(
        store, &["downloads", "downloads.json"], STATE_LIMIT,
        crate::mobile_rebase::RecordKind::Downloads,
    )?.unwrap_or_default();
    let mut candidates = Vec::new();
    for (id, stored) in saved {
        if id != stored.job.id { return Err("Persisted download key/ID mismatch".into()); }
        if stored.job.status != "complete" || stored.job.total != Some(stored.job.downloaded) { continue; }
        let expected = if let Some(intent) = &stored.publication {
            if intent.size != stored.job.downloaded { return Err("Invalid completed HTTP publication size".into()); }
            Some(intent.sha256.as_str())
        } else { stored.job.sha256.as_deref() };
        let sha256 = expected.map(|expected| {
            let mut digest = [0u8; 32];
            hex::decode_to_slice(expected, &mut digest).map_err(|error| format!("Invalid completed HTTP publication digest: {error}"))?;
            Ok::<_, String>(digest)
        }).transpose()?;
        candidates.push((self::paths(&stored.job)?.0, crate::library_files::BackupEvidence {
            size: stored.job.downloaded, identity: None, sha256,
        }));
    }
    Ok(candidates)
}

/// One network read. The old 16 KiB quantum paid a permit lock, a timer check
/// and a tokio blocking-pool write round trip 64 times per MiB.
const READ_BUDGET: usize = 256 * 1024;
/// Writes are batched to one syscall per this many bytes instead of per read.
const WRITE_BUDGET: usize = 1024 * 1024;
/// Durable checkpoints are at most this far apart. Progress shown in memory is
/// updated more often, but the persisted byte count never runs ahead of a sync.
#[cfg(test)]
const SYNC_INTERVAL: Duration = Duration::from_millis(300);
#[cfg(not(test))]
const SYNC_INTERVAL: Duration = Duration::from_secs(5);
/// A local checksum pass re-checks pause/privacy at most this far apart. The
/// first byte and the moment before publication are always checked.
const STOP_CHECK_INTERVAL: u64 = 8 * 1024 * 1024;
#[cfg(test)]
const SPEED_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const SPEED_INTERVAL: Duration = Duration::from_secs(1);
/// Parallel ranges are only attempted for objects that can repay the extra
/// request latency and are large enough to split without creating slivers.
const SEGMENT_COUNT: usize = 4;
#[cfg(test)]
const SEGMENT_MIN_TOTAL: u64 = 1024 * 1024;
#[cfg(not(test))]
const SEGMENT_MIN_TOTAL: u64 = 4 * 1024 * 1024;
const SEGMENT_MIN_LENGTH: u64 = 256 * 1024;
const SEGMENT_RETRIES: usize = 8;
/// Bounded 206 continuations: a short-range server must not spin the request
/// loop forever.
const SHORT_BODY_CONTINUES: u32 = 8;

struct BandwidthBudget {
    updated: Instant,
    credit: f64,
    generation: u64,
}

impl BandwidthBudget {
    fn take(&mut self, limit: u64, now: Instant) -> Result<usize, Duration> {
        let quantum = limit.div_ceil(10).min(READ_BUDGET as u64).max(1) as usize;
        self.credit = (self.credit
            + now.saturating_duration_since(self.updated).as_secs_f64() * limit as f64)
            .min(quantum as f64);
        self.updated = now;
        if self.credit >= quantum as f64 {
            self.credit -= quantum as f64;
            Ok(quantum)
        } else {
            Err(Duration::from_secs_f64(
                (quantum as f64 - self.credit) / limit as f64,
            ))
        }
    }
}

struct BandwidthPermit {
    bytes: usize,
    generation: u64,
    limited: bool,
}

async fn bandwidth_permit(
    inner: &Inner,
    signal: &mut watch::Receiver<Signal>,
) -> Result<Option<BandwidthPermit>, String> {
    if inner.download_limit.load(Ordering::Acquire) == 0 {
        return Ok(Some(BandwidthPermit {
            bytes: READ_BUDGET,
            generation: 0,
            limited: false,
        }));
    }
    let mut changed = inner.limit_changed.subscribe();
    loop {
        if *signal.borrow() != Signal::Run {
            return Ok(None);
        }
        let delay = {
            let mut budget = inner
                .bandwidth
                .lock()
                .map_err(|_| "Download bandwidth lock is poisoned")?;
            let limit = inner.download_limit.load(Ordering::Acquire);
            if limit == 0 {
                return Ok(Some(BandwidthPermit {
                    bytes: READ_BUDGET,
                    generation: budget.generation,
                    limited: false,
                }));
            }
            match budget.take(limit, Instant::now()) {
                Ok(bytes) => {
                    return Ok(Some(BandwidthPermit {
                        bytes,
                        generation: budget.generation,
                        limited: true,
                    }))
                }
                Err(delay) => delay,
            }
        };
        tokio::select! {
            biased;
            _ = signal.changed() => return Ok(None),
            _ = changed.changed() => {}
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

fn refund_bandwidth(inner: &Inner, permit: &BandwidthPermit, used: usize) -> Result<(), String> {
    if permit.limited && used < permit.bytes {
        let mut budget = inner
            .bandwidth
            .lock()
            .map_err(|_| "Download bandwidth lock is poisoned")?;
        if budget.generation == permit.generation {
            budget.credit += (permit.bytes - used) as f64;
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("Synchronizing directory {}: {e}", path.display()))?;
    let _ = path;
    Ok(())
}

/// Journal bytes serialized while the caller holds the state lock. The
/// temp-file write, fsync and rename happen in `write_journal` after the caller
/// releases that lock.
struct PendingJournal {
    generation: u64,
    bytes: Vec<u8>,
}

impl Inner {
    async fn client(&self) -> Result<&reqwest::Client, String> {
        self.client.get_or_try_init(|| async { http_client(true) }).await
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, String> {
        self.state
            .lock()
            .map_err(|_| "Download state lock is poisoned".into())
    }

    /// Durable, serialized save for callers that keep the state lock. Callers
    /// that can release it use `stage` + `publish` instead.
    fn save(&self, state: &State) -> Result<(), String> {
        let journal = self.stage(state)?;
        if self.write_journal(journal)? {
            self.retire_shared_grants(state);
        }
        Ok(())
    }

    /// Serialize the whole journal. The caller holds the state lock and must
    /// drop it before `publish`, so a full rewrite never blocks status readers.
    fn stage(&self, state: &State) -> Result<PendingJournal, String> {
        let bytes = serde_json::to_vec(&state.jobs)
            .map_err(|e| format!("Serializing download state: {e}"))?;
        if bytes.len() as u64 > STATE_LIMIT {
            return Err("Download state exceeds size limit".into());
        }
        Ok(PendingJournal {
            generation: self.journal_generation.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
            bytes,
        })
    }

    /// Write a staged journal in generation order. Returns whether these bytes
    /// were the ones published; a newer snapshot may already have replaced them.
    ///
    /// The temp file is written and fsynced without the publication gate, so a
    /// caller that still holds the state lock waits only for the rename and its
    /// directory fsync, never for another thread's whole-document write.
    fn write_journal(&self, journal: PendingJournal) -> Result<bool, String> {
        let temporary = self
            .data_dir
            .join(format!("downloads-{}.tmp", Uuid::new_v4()));
        let written = (|| {
            let mut file = writable_options()
                .create_new(true)
                .open(&temporary)
                .map_err(|e| format!("Creating download state: {e}"))?;
            file.write_all(&journal.bytes)
                .map_err(|e| format!("Serializing download state: {e}"))?;
            file.flush()
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("Synchronizing download state: {e}"))
        })();
        if let Err(error) = written {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        let result = (|| {
            let mut published = self
                .published_generation
                .lock()
                .map_err(|_| "Download journal lock is poisoned")?;
            if journal.generation <= *published {
                // Writing an older snapshot would resurrect rows a newer one
                // already removed or dismissed.
                let _ = fs::remove_file(&temporary);
                return Ok(false);
            }
            fs::rename(&temporary, self.data_dir.join("downloads.json"))
                .map_err(|e| format!("Publishing download state: {e}"))?;
            // The rename is the ordering point. Advance before the directory
            // sync so a sync failure cannot let an older staged journal win
            // and resurrect rows this snapshot removed.
            *published = journal.generation;
            sync_directory(&self.data_dir)?;
            Ok(true)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Publish a journal staged under the caller's released state lock. The
    /// atomic replace/fsync ordering is unchanged; only the lock is released.
    fn publish(&self, journal: PendingJournal) -> Result<(), String> {
        if self.write_journal(journal)? {
            if let Ok(state) = self.state() {
                self.retire_shared_grants(&state);
            }
        }
        Ok(())
    }

    /// Network capabilities belong only to live, resumable ownership. The
    /// hidden cleanup ledger retains file proof, never shared-library access.
    /// Callers already hold state before taking the grant lock.
    fn retire_shared_grants(&self, state: &State) {
        self.shared_grants.lock().unwrap_or_else(|error| error.into_inner())
            .retain(|id, _| state.jobs.get(id).is_some_and(|stored| {
                stored.job.url.starts_with("kiwix-library:")
                    && !stored.history_dismissed && !stored.files_deleted
                    && matches!(stored.job.status.as_str(), "queued" | "downloading" | "verifying" | "paused" | "failed")
                    && !stored.part_ownership.as_ref().is_some_and(crate::torrent::CleanupEntry::cleanup_started)
                    && !stored.published_ownership.as_ref().is_some_and(crate::torrent::CleanupEntry::cleanup_started)
            }));
    }

    fn update(&self, id: &str, update: impl FnOnce(&mut StoredJob)) -> Result<(), String> {
        let mut state = self.state()?;
        let job = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
        update(job);
        job.job.updated_at = now();
        let journal = self.stage(&state)?;
        drop(state);
        self.publish(journal)
    }

    /// Update the row the UI reads without a disk write. Durable checkpoints
    /// call `update` after syncing the part; this path only keeps speed and the
    /// displayed byte count current between syncs.
    fn update_memory(&self, id: &str, update: impl FnOnce(&mut StoredJob)) -> Result<(), String> {
        let mut state = self.state()?;
        let job = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
        update(job);
        job.job.updated_at = now();
        Ok(())
    }

    fn snapshot(&self, id: &str) -> Result<StoredJob, String> {
        self.state()?
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| "Unknown download ID".into())
    }

    fn outstanding_space(&self, directory: &Path, exclude_id: &str) -> Result<u64, String> {
        let state = self.state()?;
        outstanding_space_from_state(&state, directory, exclude_id)
    }
}

const SHARED_BROWSE_AGAIN: &str = "Browse this shared library again before resuming its download";
/// Stable prefix the Nearby UI matches to offer replacing the earlier download.
const SHARED_EARLIER_SESSION: &str = "An earlier Nearby download from a previous sharing session is still in Downloads";

/// `(session, peer, book)` of a Nearby download identity
/// (`kiwix-library://public/<session>/<peer>/<book>`, see
/// `shared_library::download_identity`); `None` for anything else.
fn library_identity_parts(identity: &str) -> Option<(String, String, String)> {
    let url = Url::parse(identity).ok()?;
    if url.scheme() != "kiwix-library" || url.host_str() != Some("public") { return None; }
    let mut segments = url.path_segments()?;
    let parts = (segments.next()?.to_owned(), segments.next()?.to_owned(), segments.next()?.to_owned());
    (segments.next().is_none() && !parts.0.is_empty() && !parts.1.is_empty() && !parts.2.is_empty()).then_some(parts)
}

fn shared_grant(inner: &Inner, id: &str, source: &str) -> Result<Option<crate::shared_library::ScopedDownloadGrant>, String> {
    let grant = inner.shared_grants.lock().map_err(|_| "Shared library grant lock is poisoned")?
        .get(id).cloned();
    if grant.is_none() && source.starts_with("kiwix-library:") {
        return Err(SHARED_BROWSE_AGAIN.into());
    }
    Ok(grant)
}

fn require_download_network(inner: &Inner, id: &str, source: &str) -> Result<(), String> {
    match shared_grant(inner, id, source)? {
        Some(grant) => grant.require(),
        None => crate::transfer_policy::require_internet(),
    }
}

fn download_url(inner: &Inner, id: &str, value: &str) -> Result<Url, String> {
    match shared_grant(inner, id, value)? {
        Some(grant) => grant.validate_url(value),
        None => checked_url(value, true),
    }
}

async fn download_denied(inner: &Inner, id: &str) {
    match inner.snapshot(id).and_then(|stored| shared_grant(inner, id, &stored.job.url)) {
        Ok(Some(grant)) => grant.denied().await,
        Ok(None) => crate::transfer_policy::internet_denied().await,
        Err(_) => {},
    }
}

async fn download_request(inner: &Inner, id: &str, url: Url, headers: http::HeaderMap) -> Result<TransferResponse, String> {
    let source = inner.snapshot(id)?.job.url;
    match shared_grant(inner, id, &source)? {
        Some(grant) => {
            grant.validate_url(url.as_str())?;
            grant.request(&headers).await
        }
        None => {
            checked_url(url.as_str(), true)?;
            inner.client().await?.get(url).headers(headers).send().await.map(TransferResponse::Http)
                .map_err(|e| format!("Requesting download: {e}"))
        }
    }
}

#[derive(Clone, Copy)]
enum WorkerMode {
    Transfer,
    DurableFinalize,
    #[cfg(any(target_os = "ios", all(test, unix)))]
    NativeAttach,
}

fn spawn_worker(inner: &Arc<Inner>, state: &mut State, id: &str) {
    spawn_download_worker(inner, state, id, WorkerMode::Transfer);
}

fn spawn_download_worker(inner: &Arc<Inner>, state: &mut State, id: &str, mode: WorkerMode) {
    let (command, receiver) = watch::channel(Signal::Run);
    let (finished, finish_receiver) = watch::channel(false);
    state.running.insert(
        id.to_string(),
        Running {
            command,
            finished: finish_receiver,
        },
    );
    let inner = Arc::clone(inner);
    let id = id.to_string();
    tokio::spawn(async move {
        let mut result = match mode {
            WorkerMode::Transfer => execute(Arc::clone(&inner), &id, receiver).await,
            #[cfg(any(target_os = "ios", all(test, unix)))]
            WorkerMode::NativeAttach => execute_native(&inner, &id, receiver, false).await,
            WorkerMode::DurableFinalize => {
                let task_inner = Arc::clone(&inner);
                let task_id = id.clone();
                tokio::task::spawn_blocking(move || {
                    finalize_durable_download(&task_inner, &task_id, FinalizeMode::Sweep)
                }).await.map_err(|error| format!("Download finalize task failed: {error}"))
                    .and_then(|result| result)
            }
        };
        let state = {
            #[cfg(any(target_os = "ios", all(test, unix)))]
            {
                if !matches!(&result, Ok(Outcome::Complete))
                    && inner.snapshot(&id).is_ok_and(|stored| stored.native.is_some())
                {
                    retire_native_worker(&inner, &id).await
                } else {
                    inner.state()
                }
            }
            #[cfg(not(any(target_os = "ios", all(test, unix))))]
            { inner.state() }
        };
        if let Ok(mut state) = state {
            // Publication already persisted complete jobs. Do not turn a durable
            // success into a failure because of a redundant second state write.
            let completed = matches!(&result, Ok(Outcome::Complete));
            if !completed {
                // Stop commands may race the worker's last await. Resolve the
                // latest intent while holding the same lock used by publication.
                let action = state
                    .running
                    .get(&id)
                    .map(|running| *running.command.borrow())
                    .unwrap_or(Signal::Pause);
                let mut cancel_cleanup = None;
                if action == Signal::Cancel {
                    if let Some(stored) = state.jobs.get_mut(&id) {
                        stored.job.status = "cancelled".into();
                        stored.job.interrupted = false;
                    }
                    cancel_cleanup = remove_job_part(&inner, &mut state, &id).err();
                    result = Ok(Outcome::Cancelled);
                } else {
                    result = settle_worker_result(result, action);
                }
                if let Some(stored) = state.jobs.get_mut(&id) {
                    stored.job.speed = 0;
                    stored.job.eta_seconds = None;
                    stored.job.updated_at = now();
                    match result {
                        Ok(Outcome::Complete) => {}
                        Ok(Outcome::Paused) => {
                            stored.job.status = "paused".into();
                            stored.job.interrupted = false;
                            stored.job.error = None;
                        }
                        Ok(Outcome::Interrupted) => {
                            stored.job.status = "paused".into();
                            stored.job.interrupted = true;
                            stored.job.error = None;
                        }
                        Ok(Outcome::Cancelled) => {
                            stored.job.status = "cancelled".into();
                            stored.job.interrupted = false;
                            stored.job.error = cancel_cleanup;
                            stored.job.downloaded = 0;
                            stored.segments = None;
                            stored.segmented = false;
                            stored.verification = None;
                            if let Some(native) = &mut stored.native {
                                native.copied = 0;
                                native.total = None;
                                native.stage_identity = None;
                            }
                        }
                        Err(error) => {
                            stored.job.status = "failed".into();
                            stored.job.interrupted = false;
                            stored.job.error = Some(if error.starts_with(crate::disk_warning::WARNING_PREFIX) {
                                error
                            } else {
                                error.chars().take(2000).collect()
                            });
                        }
                    }
                    if let Some(error) = stored.native.as_ref().and_then(|native| native.retirement_error.clone()) {
                        stored.job.error = Some(error);
                    }
                }
            }
            state.running.remove(&id);
            if !completed {
                match inner.stage(&state) {
                    Ok(journal) => {
                        drop(state);
                        if let Err(error) = inner.publish(journal) {
                            if let Ok(mut state) = inner.state() {
                                if let Some(stored) = state.jobs.get_mut(&id) {
                                    // A journal failure does not revoke an explicit stop.
                                    // In particular, Failed would make Cancel resumable.
                                    if !matches!(stored.job.status.as_str(), "cancelled" | "paused") {
                                        stored.job.status = "failed".into();
                                    }
                                    stored.job.error = Some(format!("Persisting download result: {error}"));
                                }
                                inner.retire_shared_grants(&state);
                            }
                        }
                    }
                    Err(error) => {
                        // Serialization failed before any write; the in-memory
                        // row still records why the result could not be saved.
                        if let Some(stored) = state.jobs.get_mut(&id) {
                            if !matches!(stored.job.status.as_str(), "cancelled" | "paused") {
                                stored.job.status = "failed".into();
                            }
                            stored.job.error = Some(format!("Persisting download result: {error}"));
                        }
                        inner.retire_shared_grants(&state);
                    }
                }
            } else {
                // Publication can succeed even when its final history write fails.
                // Retire committed/cancelled ownership before acknowledging exit.
                inner.retire_shared_grants(&state);
            }
        }
        finished.send_replace(true);
    });
}

/// Return the settlement lock only after retirement acknowledges its final
/// stop intent. Taking that lock fences Cancel against worker removal; a late
/// Cancel during stop(false) must first receive the stronger stop(true) ack.
#[cfg(any(target_os = "ios", all(test, unix)))]
async fn retire_native_worker<'a>(
    inner: &'a Inner,
    id: &str,
) -> Result<std::sync::MutexGuard<'a, State>, String> {
    loop {
        let (generation, cancel) = {
            let state = inner.state()?;
            let stored = state.jobs.get(id).ok_or("Unknown download ID")?;
            let native = stored.native.as_ref().ok_or("Download has no native transport lease")?;
            let cancel = stored.job.status == "cancelled"
                || state.running.get(id).is_some_and(|running| *running.command.borrow() == Signal::Cancel);
            (native.generation.clone(), cancel)
        };
        let retirement = native_transport::stop(id, &generation, cancel).await;
        let mut state = inner.state()?;
        let cancelled = state.running.get(id).is_some_and(|running| *running.command.borrow() == Signal::Cancel)
            || state.jobs.get(id).is_some_and(|stored| stored.job.status == "cancelled");
        if cancelled && !cancel {
            continue;
        }
        if let Some(native) = state.jobs.get_mut(id).and_then(|stored| stored.native.as_mut()) {
            native.retirement_error = retirement.err();
        }
        return Ok(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Complete,
    Paused,
    Interrupted,
    Cancelled,
}

fn signal_outcome(signal: Signal) -> Outcome {
    match signal {
        Signal::Cancel => Outcome::Cancelled,
        Signal::Interrupt => Outcome::Interrupted,
        Signal::Run | Signal::Pause => Outcome::Paused,
    }
}

async fn stop_file(
    inner: &Inner,
    id: &str,
    file: &mut tokio::fs::File,
    downloaded: u64,
    signal: Signal,
) -> Result<Outcome, String> {
    file.flush()
        .await
        .map_err(|e| format!("Flushing download: {e}"))?;
    file.sync_all()
        .await
        .map_err(|e| format!("Synchronizing download: {e}"))?;
    inner.update(id, |stored| {
        stored.job.downloaded = downloaded;
        stored.job.speed = 0;
        stored.job.eta_seconds = None;
    })?;
    Ok(signal_outcome(signal))
}

async fn stop_buffered(
    inner: &Inner,
    id: &str,
    writer: &mut tokio::io::BufWriter<tokio::fs::File>,
    downloaded: u64,
    signal: Signal,
) -> Result<Outcome, String> {
    writer
        .flush()
        .await
        .map_err(|e| format!("Flushing download: {e}"))?;
    writer
        .get_ref()
        .sync_all()
        .await
        .map_err(|e| format!("Synchronizing download: {e}"))?;
    inner.update(id, |stored| {
        stored.job.downloaded = downloaded;
        stored.job.speed = 0;
        stored.job.eta_seconds = None;
    })?;
    Ok(signal_outcome(signal))
}

fn http_space_identity(job: &DownloadJob) -> String {
    // Include the immutable job ID: removing history and starting an otherwise
    // identical request cannot reuse the previous job's consent.
    format!("http:{}:{}:{}", job.id, job.url, job.destination)
}

async fn execute(
    inner: Arc<Inner>,
    id: &str,
    mut signal: watch::Receiver<Signal>,
) -> Result<Outcome, String> {
    #[cfg(any(target_os = "ios", all(test, unix)))]
    if inner.snapshot(id)?.native.is_some() {
        return execute_native(&inner, id, signal, true).await;
    }
    execute_transfer(&inner, id, &mut signal).await
}

#[cfg(target_os = "ios")]
use crate::ios_background as native_transport;

#[cfg(all(test, unix, not(target_os = "ios")))]
use tests::native_transport;

/// Polling is a monotonic interval sample, never a lifetime average. An
/// attachment starts at the current counter so daemon work while detached does
/// not appear as an instantaneous burst. Regressing counters reset the sample.
#[cfg(any(target_os = "ios", all(test, unix)))]
struct NativeProgressSample {
    at: Instant,
    bytes: u64,
}

#[cfg(any(target_os = "ios", all(test, unix)))]
impl NativeProgressSample {
    fn new(at: Instant, bytes: u64) -> Self {
        Self { at, bytes }
    }

    fn update(&mut self, job: &mut DownloadJob, bytes: u64, total: Option<u64>, now: Instant) {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        let speed = if bytes >= self.bytes && elapsed > 0.0 {
            ((bytes - self.bytes) as f64 / elapsed) as u64
        } else {
            0
        };
        self.at = now;
        self.bytes = bytes;
        job.downloaded = bytes;
        job.total = total.or(job.total);
        job.speed = speed;
        job.eta_seconds = job.total.filter(|_| speed > 0)
            .map(|total| total.saturating_sub(bytes).div_ceil(speed));
    }
}

/// URLSession retains its stage until the canonical copy is durable. Admit
/// both files before starting payload transport, using the native-reported
/// staging volume rather than assuming it is the selected library volume.
#[cfg(any(target_os = "ios", all(test, unix)))]
fn admit_native_transport(inner: &Inner, id: &str, staging_directory: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let stored = inner.snapshot(id)?;
    let (destination, part) = paths(&stored.job)?;
    inner.store.with_http_acquisition_destinations(&[&destination], &stored.job.url, || Ok(()))?;
    let directory = part.parent().ok_or("Download has no destination directory")?;
    let Some(total) = stored.expected_size.or(stored.job.total) else { return Ok(()); };
    crate::storage::check_file_size(directory, total)?;
    crate::storage::check_file_size(staging_directory, total)?;
    let file = open_part(&part)?;
    if !stored.part_ownership.as_ref().is_some_and(|ownership| ownership.matches(&part, &file).unwrap_or(false)) {
        return Err("Owned native part was replaced; refusing to start transport".into());
    }
    let canonical_remaining = total.saturating_sub(file.metadata().map_err(|error| error.to_string())?.len());
    let staging_volume = staging_directory.ancestors().find(|path| path.is_dir())
        .ok_or("Native staging directory has no accessible filesystem ancestor")?;
    let canonical_volume = fs::metadata(directory).map_err(|error| error.to_string())?.dev();
    let stage_volume = fs::metadata(staging_volume).map_err(|error| error.to_string())?.dev();
    // Opaque resume data and progress counters are not proof of allocated
    // stage bytes: the daemon may restart its body. Reserve its whole object.
    let (warning_directory, remaining) = if canonical_volume == stage_volume {
        let remaining = total.checked_add(canonical_remaining)
            .ok_or("Native staging and adoption size overflow")?;
        check_space(&part, remaining)?;
        (directory, remaining)
    } else {
        check_space(&staging_volume.join("native-stage"), total)?;
        check_space(&part, canonical_remaining)?;
        let stage_free = crate::fs_ext::available_space(staging_volume).map_err(|error| error.to_string())?;
        let canonical_free = crate::fs_ext::available_space(directory).map_err(|error| error.to_string())?;
        // One acknowledgement covers the most constrained volume. Its scope
        // includes both paths/sizes; do not create two unacknowledgeable prompts.
        if stage_free.saturating_sub(total) < canonical_free.saturating_sub(canonical_remaining) {
            (staging_directory, total)
        } else {
            (directory, canonical_remaining)
        }
    };
    let outstanding = inner.outstanding_space(warning_directory, id)?;
    download_space_warning(
        inner, warning_directory, Some(remaining), outstanding,
        &format!("{}:native:{}:{total}:{canonical_remaining}", http_space_identity(&stored.job), staging_directory.display()),
        stored.space_acknowledgement.as_deref(),
    )
}

fn download_space_warning(
    inner: &Inner, directory: &Path, remaining: Option<u64>, outstanding: u64, identity: &str,
    acknowledgement: Option<&str>,
) -> Result<(), String> {
    #[cfg(test)]
    let available = match inner.warning_available.load(Ordering::SeqCst) {
        u64::MAX => None,
        available => Some(available),
    };
    #[cfg(not(test))]
    let available = { let _ = inner; None };
    crate::disk_warning::check_with_available_and_outstanding(
        directory, remaining, outstanding, identity, acknowledgement, available,
    )
}

#[cfg(any(target_os = "ios", all(test, unix)))]
struct NativeStage {
    path: PathBuf,
    total: u64,
    device: u64,
    inode: u64,
}

/// The native stage remains its immutable transport receipt until acknowledge.
/// Copy into the pre-existing locked part rather than replacing its ownership;
/// synchronized prefix checkpoints make pause/restart resume this local copy.
#[cfg(any(target_os = "ios", all(test, unix)))]
fn copy_native_completion(
    inner: &Inner,
    id: &str,
    stage: &NativeStage,
) -> Result<Option<Signal>, String> {
    let snapshot = inner.snapshot(id)?;
    let native = snapshot.native.as_ref().ok_or("Download has no native transport lease")?;
    let (_, part) = paths(&snapshot.job)?;
    let source = writable_options().write(false).open(&stage.path)
        .map_err(|error| format!("Opening completed native download: {error}"))?;
    let metadata = source.metadata().map_err(|error| error.to_string())?;
    let identity = part_identity(&metadata).ok_or("Native download has no stable inode receipt")?;
    if identity.device != stage.device || identity.inode != stage.inode
        || identity.size != stage.total || identity.links != 1 || stage.total < 80
    {
        return Err("Native download no longer matches its completed transport receipt".into());
    }
    if snapshot.expected_size.is_some_and(|expected| expected != stage.total) {
        return Err("Native download length differs from its authoritative archive size".into());
    }
    let verified = crate::publication::VerifiedFile::new(source, stage.total)
        .map_err(|error| format!("Retaining native download receipt: {error}"))?;
    if !verified.matches_path(&stage.path).map_err(|error| error.to_string())? {
        return Err("Native download stage changed before adoption".into());
    }
    let mut destination = open_part(&part)?;
    if !snapshot.part_ownership.as_ref()
        .is_some_and(|ownership| ownership.matches(&part, &destination).unwrap_or(false))
    {
        return Err("Native download cannot replace an unowned canonical part".into());
    }
    if same_file(&metadata, &destination.metadata().map_err(|error| error.to_string())?) {
        return Err("Native staging and canonical download part must be distinct files".into());
    }
    let mut copied = if native.stage_identity.as_ref() == Some(&identity)
        && native.total == Some(stage.total)
        && destination.metadata().map_err(|error| error.to_string())?.len() >= native.copied
    {
        native.copied
    } else {
        0
    };
    check_space(&part, stage.total.saturating_sub(copied))?;
    crate::storage::check_file_size(&part, stage.total)?;
    destination.set_len(copied).map_err(|error| format!("Preparing native download part: {error}"))?;
    destination.seek(std::io::SeekFrom::Start(copied)).map_err(|error| error.to_string())?;
    let mut source = verified.file();
    source.seek(std::io::SeekFrom::Start(copied)).map_err(|error| error.to_string())?;
    let checkpoint = |destination: &File, copied: u64| -> Result<(), String> {
        destination.sync_all().map_err(|error| format!("Synchronizing native download part: {error}"))?;
        inner.update(id, |stored| {
            let native = stored.native.as_mut().unwrap();
            native.copied = copied;
            native.total = Some(stage.total);
            native.stage_identity = Some(identity.clone());
            stored.job.downloaded = copied;
            stored.job.total = Some(stage.total);
            stored.verification = None;
        })
    };
    let mut buffer = vec![0u8; WRITE_BUDGET];
    let mut synced = copied;
    let mut last_sync = Instant::now();
    while copied < stage.total {
        if let Some(action) = local_stop_action(inner, id)? {
            checkpoint(&destination, copied)?;
            return Ok(Some(action));
        }
        let limit = (stage.total - copied).min(buffer.len() as u64) as usize;
        let count = source.read(&mut buffer[..limit])
            .map_err(|error| format!("Reading native download stage: {error}"))?;
        if count == 0 {
            return Err("Native download stage ended before its completed size".into());
        }
        destination.write_all(&buffer[..count])
            .map_err(|error| format!("Adopting native download bytes: {error}"))?;
        copied += count as u64;
        #[cfg(test)]
        {
            inner.native_copy_bytes.fetch_add(count as u64, Ordering::SeqCst);
            let gate = if copied < stage.total {
                inner.native_copy_gate.lock().unwrap().take()
            } else {
                None
            };
            if let Some(gate) = gate {
                checkpoint(&destination, copied)?;
                synced = copied;
                last_sync = Instant::now();
                let _ = gate.entered.send(());
                let _ = gate.release.recv();
            }
        }
        if copied < stage.total
            && (last_sync.elapsed() >= SYNC_INTERVAL || space_check_due(copied, synced))
        {
            checkpoint(&destination, copied)?;
            check_space(&part, stage.total - copied)?;
            synced = copied;
            last_sync = Instant::now();
        }
    }
    let after = verified.file().metadata().map_err(|error| error.to_string())?;
    if part_identity(&after).as_ref() != Some(&identity)
        || !verified.matches_path(&stage.path).map_err(|error| error.to_string())?
    {
        return Err("Native download stage changed while its bytes were being adopted".into());
    }
    checkpoint(&destination, copied)?;
    local_stop_action(inner, id)
}

#[cfg(any(target_os = "ios", all(test, unix)))]
async fn execute_native(
    inner: &Arc<Inner>,
    id: &str,
    mut signal: watch::Receiver<Signal>,
    allow_start: bool,
) -> Result<Outcome, String> {
    let generation = inner.snapshot(id)?.native.as_ref()
        .ok_or("Download has no native transport lease")?.generation.clone();
    execute_native_transfer(inner, id, &generation, &mut signal, allow_start).await
}

#[cfg(any(target_os = "ios", all(test, unix)))]
async fn execute_native_transfer(
    inner: &Arc<Inner>,
    id: &str,
    generation: &str,
    signal: &mut watch::Receiver<Signal>,
    allow_start: bool,
) -> Result<Outcome, String> {
    if finalize_forbidden(inner) {
        return Ok(Outcome::Interrupted);
    }
    let mut stored = inner.snapshot(id)?;
    let already_durable = durable_total(&stored).is_some();
    if !already_durable {
        let slots = Arc::clone(&inner.slots);
        let _permit = tokio::select! {
            biased;
            _ = signal.changed() => return Ok(signal_outcome(*signal.borrow())),
            permit = slots.acquire_owned() => permit.map_err(|_| "Download scheduler closed")?,
        };
        if *signal.borrow() != Signal::Run {
            return Ok(signal_outcome(*signal.borrow()));
        }
        let existing = native_transport::snapshot(id, generation).await?;
        if allow_start && existing.state != "complete" {
            crate::transfer_policy::require_internet()?;
            let original = checked_url(&stored.job.url, true)?;
            if stored.transfer_url.is_none() {
                if original.path().ends_with(".meta4") || original.path().ends_with(".metalink") {
                    let resolved = resolve_metalink(inner, &original, signal).await?;
                    inner.update(id, |stored| {
                        stored.transfer_url = Some(resolved.url);
                        stored.expected_size = Some(resolved.size);
                        stored.job.total = Some(resolved.size);
                        stored.job.sha256 = resolved.sha256;
                    })?;
                } else {
                    inner.update(id, |stored| stored.transfer_url = Some(original.to_string()))?;
                }
                stored = inner.snapshot(id)?;
            }
            if *signal.borrow() != Signal::Run {
                return Ok(signal_outcome(*signal.borrow()));
            }
            admit_native_transport(inner, id, Path::new(&existing.staging_directory))?;
            crate::transfer_policy::require_internet()?;
            native_transport::start(&crate::ios_background::NativeRequest {
                id: id.into(),
                generation: generation.into(),
                url: stored.transfer_url.clone().ok_or("Native download has no resolved URL")?,
                expected_size: stored.expected_size,
                allows_cellular: inner.store.settings()?.allow_metered_downloads,
            }).await?;
        }
        inner.update(id, |stored| {
            if stored.job.status == "queued" {
                stored.job.status = "downloading".into();
            }
            if let Some(native) = &mut stored.native {
                native.retirement_error = None;
            }
        })?;
        let mut progress = NativeProgressSample::new(Instant::now(), existing.copied);
        loop {
            let action = *signal.borrow();
            if matches!(action, Signal::Pause | Signal::Cancel) {
                return Ok(signal_outcome(action));
            }
            let native = native_transport::snapshot(id, generation).await?;
            if native.state == "complete" {
                if !matches!(native.response_status, Some(200 | 206))
                    || native.source_url.as_deref() != stored.transfer_url.as_deref()
                    || native.final_url.as_deref().and_then(|url| Url::parse(url).ok())
                        .is_none_or(|url| url.scheme() != "https")
                    || native.total.is_some_and(|total| total != native.copied)
                {
                    return Err("Native download did not provide a complete successful archive response".into());
                }
                let stage = NativeStage {
                    path: native.path.ok_or("Native completion has no durable stage")?.into(),
                    total: native.copied,
                    device: native.device.ok_or("Native completion has no device receipt")?,
                    inode: native.inode.ok_or("Native completion has no inode receipt")?,
                };
                {
                    let mut state = inner.state()?;
                    let action = state.running.get(id).map(|running| *running.command.borrow())
                        .unwrap_or(Signal::Pause);
                    if matches!(action, Signal::Pause | Signal::Cancel) {
                        return Ok(signal_outcome(action));
                    }
                    let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
                    stored.job.status = "verifying".into();
                    stored.job.interrupted = false;
                    stored.job.speed = 0;
                    stored.job.eta_seconds = None;
                    let journal = inner.stage(&state)?;
                    drop(state);
                    inner.publish(journal)?;
                }
                let copy_inner = Arc::clone(inner);
                let copy_id = id.to_owned();
                let copied = tokio::task::spawn_blocking(move || {
                    copy_native_completion(&copy_inner, &copy_id, &stage)
                }).await.map_err(|error| format!("Adopting native download: {error}"))??;
                if let Some(action) = copied {
                    return Ok(signal_outcome(action));
                }
                break;
            }
            match native.state.as_str() {
                "running" => {}
                "failed" => return Err(native.error.unwrap_or_else(|| "Native background download failed".into())),
                "paused" | "missing" => return Ok(Outcome::Interrupted),
                _ => return Err("Native download returned an unknown transport state".into()),
            }
            if action == Signal::Interrupt || finalize_forbidden(inner) {
                return Ok(Outcome::Interrupted);
            }
            inner.update_memory(id, |stored| {
                progress.update(&mut stored.job, native.copied, native.total, Instant::now());
            })?;
            tokio::select! {
                biased;
                _ = signal.changed() => {}
                _ = crate::transfer_policy::internet_denied() => return Ok(Outcome::Interrupted),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }
    native_transport::acknowledge(id, generation).await?;
    {
        let mut state = inner.state()?;
        if let Some(native) = state.jobs.get_mut(id).and_then(|stored| stored.native.as_mut()) {
            native.retirement_error = None;
        }
        let action = state.running.get(id).map(|running| *running.command.borrow())
            .unwrap_or(Signal::Pause);
        if matches!(action, Signal::Pause | Signal::Cancel) {
            return Ok(signal_outcome(action));
        }
        let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
        stored.job.status = "verifying".into();
        stored.job.interrupted = false;
        let journal = inner.stage(&state)?;
        drop(state);
        inner.publish(journal)?;
    }
    let finalize_inner = Arc::clone(inner);
    let finalize_id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        finalize_durable_download(&finalize_inner, &finalize_id, FinalizeMode::Sweep)
    }).await.map_err(|error| format!("Finalizing native download: {error}"))?
}


#[cfg(test)]
#[path = "acquisition_tests.rs"]
mod tests;
