//! Native, opt-in BitTorrent acquisition. The reader never initializes this engine.
//!
//! Jobs own a private staging directory; published archives are separate, no-clobber
//! copies. Cancellation deliberately retains staging data for recovery and never
//! asks the engine to recursively remove files. Resume rechecks pieces from disk;
//! it does not trust a stale fast-resume bitmap after an unclean shutdown.
//! List dismissal keeps files and a hidden ownership ledger. Explicit terminal
//! deletion retires the worker/session handle and captures only journaled inodes;
//! installed library reservations and unknown staging entries fail closed.
//!
//! Automatic restoration is policy-gated independently of explicit user pauses.
//! Loading a held profile creates only the profile directory/lock, not an engine
//! or a rewritten database. Explicit resume opts that job back into acquisition.
//! Cancelled metadata workers cannot overwrite a later manual pause/cancellation
//! while exiting; lifecycle holds remain distinct from both user decisions.
//! Manual pause cancels and joins that job's publication/adapter worker before
//! acknowledging; resume rechecks retained pieces with a fresh worker.
//! Await shutdown before switching to read-only mode: it joins adapter workers
//! (including publication), fences engine storage mutations, then persists once.
//! rqbit itself exposes cancellation plus a grace period, not joined network tasks.
//! Publication copies the engine-opened inode, at its exact advertised length,
//! then persists a copied-inode/size/SHA-256 intent before an exclusive rename.
//! Interrupted acknowledgement recovers that identical, unaliased output before
//! opening staging or starting the engine. Linux volumes lacking exclusive rename
//! fail closed (no hardlink fallback). Native FAT/FAT32 limits apply per non-padding
//! staging file and published ZIM, never to the torrent's aggregate size.
//! Partial publication copies have a durable inode/directory journal before any
//! payload is written. Cleanup captures entries in mutation-protected directories
//! before unlinking them; cancelled jobs are reclaimed on startup without an engine.
//! Unix publication/capture directories must belong to the effective user and deny
//! group/other writes. Public read/search permissions (including FAT/exFAT 0755
//! mount modes) are safe; mounts permitting other users to mutate entries are not.
//! Fresh publication commits hold the library destination reservation through the
//! exclusive rename, so even missing installed paths cannot acquire another ZIM.
//! Existing committed-intent recovery only reads its previously published inode.
//! Torrent metadata uses the same private-inode protocol for downloads and
//! shares: no final `.torrent` name exists before its complete bytes are synced
//! and its size/SHA-256 ownership intent is checkpointed. Startup and resume
//! finish complete intents or reclaim interrupted partials without an engine.
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::SocketAddr,
    num::NonZeroU32,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt as CapMetadataExt, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
use crate::fs_ext::FileExt;
use futures_util::StreamExt;
use librqbit::{
    api::TorrentIdOrHash,
    limits::LimitsConfig,
    storage::{
        filesystem::OurFileExt, BoxStorageFactory, StorageFactory, StorageFactoryExt,
        TorrentStorage,
    },
    AddTorrent, AddTorrentOptions, AddTorrentResponse, CreateTorrentOptions, DhtSessionConfig,
    ListenerOptions, ManagedTorrent, ManagedTorrentShared, Session, SessionOptions,
    TorrentMetadata, TorrentStatsState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::store::Store;

mod manager;
#[allow(unused_imports)]
pub(crate) use self::manager::*;
mod storage;
mod web_seed;
#[allow(unused_imports)]
pub(crate) use self::storage::*;
mod publication;
#[allow(unused_imports)]
pub(crate) use self::publication::*;

const MAX_METADATA: usize = 16 * 1024 * 1024;
const STATE_VERSION: u32 = 1;
/// Two mebibytes keeps metadata well under 16 MiB for multi-terabyte ZIMs and
/// matches librqbit's own deterministic default. Never derive this from a
/// clock or file order: the info hash must stay stable across recreations.
const SHARE_PIECE_LENGTH: u32 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentJob {
    pub id: String,
    pub url: String,
    pub file_name: String,
    pub destination: String,
    pub status: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub speed: f64,
    pub eta_seconds: Option<u64>,
    pub import_error: Option<String>,
    pub transport: String,
    /// Permission to continue uploading after download completion. Peers can
    /// receive verified pieces during an ordinary BitTorrent download as well.
    pub seeding: bool,
    pub upload_speed: f64,
    pub uploaded: u64,
    pub completed_paths: Vec<String>,
    /// True when this job shares an existing library archive instead of
    /// acquiring one. The library file is read-only input and is never copied.
    #[serde(default)]
    pub shared: bool,
    /// The magnet URI peers use to fetch a shared archive.
    #[serde(default)]
    pub magnet: Option<String>,
    /// Profile-owned `.torrent` metadata; saving it does not move the archive.
    #[serde(default)]
    pub torrent_path: Option<String>,
    /// When the job was created (RFC3339), so the Downloads list can show the
    /// most recent first. Older saved jobs get it from their metadata file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    job: TorrentJob,
    staging: PathBuf,
    directory: PathBuf,
    metadata: PathBuf,
    files: Vec<PathBuf>,
    paused: bool,
    /// A policy-held interruption is resumable on a later automatic restart;
    /// unlike `paused`, it does not record an explicit user pause.
    #[serde(default)]
    interrupted: bool,
    verified: bool,
    #[serde(default)]
    publication_intents: Vec<PublicationIntent>,
    #[serde(default)]
    publication_temporaries: Vec<PublicationTemporary>,
    /// Metadata-derived real files; padding never consumes staging capacity.
    #[serde(default)]
    space_files: Vec<SpaceFile>,
    /// Scoped confirmations are revalidated against the current remaining bytes.
    #[serde(default)]
    space_acknowledgements: Vec<String>,
    #[serde(default)]
    history_dismissed: bool,
    #[serde(default)]
    files_deleted: bool,
    #[serde(default)]
    staging_directories: Vec<(PathBuf, PublicationIdentity)>,
    #[serde(default)]
    staging_cleanup: Vec<(PathBuf, CleanupEntry)>,
    #[serde(default)]
    metadata_cleanup: Option<CleanupEntry>,
    /// Private metadata inode journaled before writing; the final name is only
    /// published after its complete bytes and ownership intent are durable.
    #[serde(default)]
    metadata_temporary: Option<PublicationTemporary>,
    #[serde(default)]
    published_cleanup: Vec<CleanupEntry>,
    /// Read-only library archive shared by this record. Absent for acquisitions.
    #[serde(default)]
    share: Option<ShareSource>,
    /// Test-only explicit peers used by the local end-to-end seeding test.
    #[serde(default)]
    initial_peers: Vec<SocketAddr>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified_ns: u128,
}

impl ShareIdentity {
    fn of(metadata: &cap_std::fs::Metadata) -> Result<Self, String> {
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err("This file cannot be shared because it is not a normal file. Copy it into your library and try again".into());
        }
        let modified_ns = metadata
            .modified()
            .map_err(display)?
            .into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(display)?
            .as_nanos();
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified_ns,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareSource {
    book_id: String,
    installation_id: String,
    path: PathBuf,
    identity: ShareIdentity,
    #[serde(default)]
    info_hash: String,
    #[serde(default)]
    piece_length: u32,
}

impl Record {
    fn cleanup_started(&self) -> bool {
        self.staging_cleanup.iter().any(|(_, entry)| entry.cleanup_started())
            || self.published_cleanup.iter().any(CleanupEntry::cleanup_started)
            || self.metadata_cleanup.as_ref().is_some_and(CleanupEntry::cleanup_started)
    }

    fn can_authorize_cleanup(&self) -> bool {
        matches!(self.job.status.as_str(), "completed" | "complete" | "failed" | "error" | "cancelled")
            && (!self.job.seeding || self.verified)
            || self.job.status == "seeding" && self.verified
    }

    /// Seed byte/rate counters are observational, not acquisition identity.
    fn same_cleanup_identity(&self, current: &Self) -> bool {
        self.job.id == current.job.id && self.job.url == current.job.url
            && self.job.destination == current.job.destination && self.job.status == current.job.status
            && self.job.seeding == current.job.seeding && self.job.completed_paths == current.job.completed_paths
            && self.staging == current.staging && self.directory == current.directory
            && self.metadata == current.metadata && self.files == current.files
            && self.paused == current.paused && self.interrupted == current.interrupted
            && self.verified == current.verified && self.history_dismissed == current.history_dismissed
            && self.files_deleted == current.files_deleted
            && self.publication_intents == current.publication_intents
            && self.publication_temporaries == current.publication_temporaries
            && self.staging_directories == current.staging_directories
            && self.staging_cleanup == current.staging_cleanup
            && self.metadata_cleanup == current.metadata_cleanup
            && self.metadata_temporary == current.metadata_temporary
            && self.published_cleanup == current.published_cleanup
            && self.share.as_ref().map(|share| (&share.path, &share.identity))
                == current.share.as_ref().map(|share| (&share.path, &share.identity))
    }

    fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        if let Some(share) = &self.share {
            if crate::acquisition::monitored_path_matches(path, &share.path)? {
                return Ok(true);
            }
        }
        // Publication flattens archive basenames into directory, not staging.
        // Metadata and then intents are checkpointed before any final rename;
        // neither is discarded when completion/import acknowledgement fails.
        for file in self.files.iter().filter(|file| is_zim(file)) {
            let name = file.file_name().ok_or("Missing torrent archive filename")?;
            if crate::acquisition::monitored_child_matches(path, &self.directory, Path::new(name))? {
                return Ok(true);
            }
        }
        for intent in &self.publication_intents {
            if crate::acquisition::monitored_child_matches(path, &self.directory, &intent.file_name)? {
                return Ok(true);
            }
        }
        for completed in &self.job.completed_paths {
            if crate::acquisition::monitored_path_matches(path, Path::new(completed))? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpaceFile {
    path: PathBuf,
    size: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationIntent {
    file_name: PathBuf,
    size: u64,
    sha256: [u8; 32],
    // Bind recovery to the copied inode, not an unrelated identical user file.
    device: u64,
    inode: u64,
    #[serde(default)]
    directory: Option<PublicationIdentity>,
}

impl PublicationIntent {
    fn cleanup_entry(&self) -> Result<CleanupEntry, String> {
        Ok(CleanupEntry {
            name: self.file_name.to_str().ok_or("Invalid published torrent filename")?.to_owned(),
            directory: self.directory.clone().ok_or("Legacy torrent publication lacks original directory ownership proof; library deletion was not authorized")?,
            file: PublicationIdentity { device: self.device, inode: self.inode },
            capture: None,
            expected_size: Some(self.size),
            sha256: Some(self.sha256),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationIdentity {
    device: u64,
    inode: u64,
}

impl PublicationIdentity {
    fn of(metadata: &cap_std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationTemporary {
    // Both UUID names and all three identities are durable before copying.
    name: String,
    capture_name: String,
    directory: PublicationIdentity,
    capture_directory: PublicationIdentity,
    file: PublicationIdentity,
}

/// Durable authority for exactly one inode, never for a pathname alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupEntry {
    name: String,
    directory: PublicationIdentity,
    file: PublicationIdentity,
    capture: Option<PublicationTemporary>,
    #[serde(default)]
    expected_size: Option<u64>,
    #[serde(default)]
    sha256: Option<[u8; 32]>,
}

impl CleanupEntry {
    pub(crate) fn retain(path: &Path, file: &File) -> Result<Self, String> {
        let parent = path.parent().ok_or("Cleanup file has no parent")?;
        let name = path.file_name().and_then(|name| name.to_str()).ok_or("Invalid cleanup filename")?;
        safe_relative(Path::new(name)).map_err(display)?;
        let directory = crate::storage::open_directory_nofollow(parent).map_err(display)?;
        let metadata = directory.symlink_metadata(name).map_err(display)?;
        let opened = cap_std::fs::File::from_std(file.try_clone().map_err(display)?);
        let opened = opened.metadata().map_err(display)?;
        if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1
            || PublicationIdentity::of(&metadata) != PublicationIdentity::of(&opened)
        {
            return Err("The downloaded file changed while Kiwix was checking it. Download it again".into());
        }
        Ok(Self {
            name: name.to_owned(),
            directory: PublicationIdentity::of(&directory.dir_metadata().map_err(display)?),
            file: PublicationIdentity::of(&opened),
            capture: None,
            expected_size: None,
            sha256: None,
        })
    }
    pub(crate) fn cleanup_started(&self) -> bool { self.capture.is_some() }

    /// Bind immutable publication bytes, never a still-writable partial.
    pub(crate) fn with_publication(&self, size: u64, sha256: Option<[u8; 32]>) -> Self {
        Self { expected_size: Some(size), sha256, ..self.clone() }
    }

    fn matches_contents(&self, file: &File) -> Result<bool, String> {
        self.matches_contents_controlled(file, || false)
    }

    fn matches_contents_controlled(&self, file: &File, cancelled: impl Fn() -> bool) -> Result<bool, String> {
        if cancelled() { return Err("Archive publication verification was cancelled".into()); }
        let size = file.metadata().map_err(display)?.len();
        if self.expected_size.is_some_and(|expected| expected != size) { return Ok(false); }
        let Some(expected) = self.sha256 else { return Ok(true); };
        let mut hash = Sha256::new();
        let mut offset = 0u64;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            if cancelled() { return Err("Archive publication verification was cancelled".into()); }
            let limit = size.saturating_sub(offset).saturating_add(1).min(buffer.len() as u64) as usize;
            #[cfg(unix)]
            let count = std::os::unix::fs::FileExt::read_at(file, &mut buffer[..limit], offset).map_err(display)?;
            #[cfg(windows)]
            let count = std::os::windows::fs::FileExt::seek_read(file, &mut buffer[..limit], offset).map_err(display)?;
            if count == 0 { break; }
            if count as u64 > size.saturating_sub(offset) { return Ok(false); }
            offset += count as u64;
            hash.update(&buffer[..count]);
        }
        if cancelled() { return Err("Archive publication verification was cancelled".into()); }
        Ok(offset == size && file.metadata().map_err(display)?.len() == size
            && <[u8; 32]>::from(hash.finalize()) == expected)
    }

    pub(crate) fn matches(&self, path: &Path, file: &File) -> Result<bool, String> {
        let current = Self::retain(path, file)?;
        Ok(self.name == current.name && self.directory == current.directory && self.file == current.file)
    }

    /// Match a file after library deletion has moved it into private staging.
    /// The original child need not exist; never adopt the staging directory.
    pub(crate) fn matches_retained(&self, path: &Path, file: &File) -> Result<bool, String> {
        self.matches_retained_controlled(path, file, || false)
    }

    pub(crate) fn matches_retained_controlled(&self, path: &Path, file: &File, cancelled: impl Fn() -> bool) -> Result<bool, String> {
        if cancelled() { return Err("Archive publication verification was cancelled".into()); }
        if path.file_name().and_then(|name| name.to_str()) != Some(self.name.as_str()) { return Ok(false); }
        let parent = path.parent().ok_or("Owned download file has no parent")?;
        let directory = crate::storage::open_directory_nofollow(parent).map_err(display)?;
        let retained = cap_std::fs::File::from_std(file.try_clone().map_err(display)?);
        let metadata = retained.metadata().map_err(display)?;
        Ok(PublicationIdentity::of(&directory.dir_metadata().map_err(display)?) == self.directory
            && metadata.is_file() && !metadata.is_symlink() && metadata.nlink() == 1
            && PublicationIdentity::of(&metadata) == self.file && self.matches_contents_controlled(file, cancelled)?
            && retained.metadata().map_err(display)?.nlink() == 1)
    }
    /// Read-only provenance check; the descriptor pins the original inode.
    pub(crate) fn validate_existing(&self, path: &Path) -> Result<Option<File>, String> {
        let parent = path.parent().ok_or("Owned download file has no parent")?;
        if path.file_name().and_then(|name| name.to_str()) != Some(self.name.as_str()) {
            return Err("Owned download filename does not match its journal".into());
        }
        let directory = crate::storage::open_directory_nofollow(parent).map_err(display)?;
        if PublicationIdentity::of(&directory.dir_metadata().map_err(display)?) != self.directory {
            return Err("Owned download directory was replaced; cleanup was not authorized".into());
        }
        let metadata = match directory.symlink_metadata(&self.name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(display(error)),
        };
        if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1
            || PublicationIdentity::of(&metadata) != self.file
        {
            return Err("Download file is not its originally owned inode; cleanup was not authorized".into());
        }
        let file = directory.open_with(&self.name, &publication_read_options()).map_err(display)?.into_std();
        if !self.matches(path, &file)? {
            return Err("Owned download file changed during provenance validation".into());
        }
        Ok(Some(file))
    }

    /// Validate original and captured entries without creating, moving or saving.
    pub(crate) fn validate_cleanup(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("Owned cleanup file has no parent")?;
        let directory = match crate::storage::open_directory_nofollow(parent) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(display(error)),
        };
        if PublicationIdentity::of(&directory.dir_metadata().map_err(display)?) != self.directory {
            return Err("Cleanup directory was replaced; no files were authorized for deletion".into());
        }
        let original_exists = match directory.symlink_metadata(&self.name) {
            Ok(_) => {
                self.validate_existing(path)?.ok_or("Owned cleanup entry disappeared during validation")?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(display(error)),
        };
        let Some(capture) = &self.capture else { return Ok(()); };
        let suffix = capture.capture_name.strip_prefix(".kiwix-publish-cleanup-").ok_or("Invalid cleanup capture name")?;
        if Uuid::parse_str(suffix).map_err(display)?.to_string() != suffix
            || capture.name != self.name || capture.directory != self.directory || capture.file != self.file
        {
            return Err("Invalid cleanup capture ownership journal".into());
        }
        let captured = match directory.open_dir_nofollow(&capture.capture_name) {
            Ok(captured) => captured,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !original_exists => return Ok(()),
            Err(error) => return Err(format!("Cannot validate owned cleanup capture: {error}")),
        };
        if PublicationIdentity::of(&captured.dir_metadata().map_err(display)?) != capture.capture_directory {
            return Err("Cleanup capture directory was replaced; files were preserved".into());
        }
        for entry in captured.entries().map_err(display)? {
            let entry = entry.map_err(display)?;
            if entry.file_name() != std::ffi::OsStr::new("payload") {
                return Err("An unexpected file appeared in a temporary folder. Kiwix left it in place; restart the app".into());
            }
            let metadata = captured.symlink_metadata("payload").map_err(display)?;
            if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1
                || PublicationIdentity::of(&metadata) != self.file
            {
                return Err("Cleanup capture contains a replaced payload".into());
            }
            let retained = captured.open_with("payload", &publication_read_options()).map_err(display)?.into_std();
            if !self.matches_retained(path, &retained)? {
                return Err("Captured publication no longer matches its original bytes".into());
            }
        }
        Ok(())
    }


    pub(crate) fn renamed(&self, name: &str) -> Self {
        Self { name: name.to_owned(), capture: None, ..self.clone() }
    }
}

/// Journal the private capture before moving an entry. Failed checkpoints never
/// unlink payload; failed identity checks restore foreign entries exclusively.
pub(crate) fn delete_owned_file(
    path: &Path,
    entry: &mut CleanupEntry,
    mut checkpoint: impl FnMut(&CleanupEntry) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path.parent().ok_or("Cleanup file has no parent")?;
    if path.file_name().and_then(|name| name.to_str()) != Some(entry.name.as_str()) {
        return Err("The downloaded file changed since it was saved. Remove it from the library and delete the file yourself".into());
    }
    safe_relative(Path::new(&entry.name)).map_err(display)?;
    if let Some(capture) = &entry.capture {
        let suffix = capture.capture_name.strip_prefix(".kiwix-publish-cleanup-")
            .ok_or("Invalid download cleanup capture")?;
        if Uuid::parse_str(suffix).map_err(display)?.to_string() != suffix
            || capture.name != entry.name || capture.directory != entry.directory || capture.file != entry.file
        {
            return Err("Invalid download cleanup ownership journal".into());
        }
    }
    let directory = open_publication_directory(parent)?;
    publication_directory_unchanged(parent, &directory, &entry.directory)?;
    let previous = entry.clone();
    let created_capture = entry.capture.is_none();
    if entry.capture.is_none() {
        if !publication_entry_exists(&directory, &entry.name)? { return Ok(()); }
        let capture_name = format!(".kiwix-publish-cleanup-{}", Uuid::new_v4());
        #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
        let mut builder = cap_std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use cap_std::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        directory.create_dir_with(&capture_name, &builder).map_err(display)?;
        let capture = open_publication_capture(&directory, &capture_name)?;
        sync_publication_directory(&capture)?;
        sync_publication_directory(&directory)?;
        let temporary = PublicationTemporary {
            name: entry.name.clone(),
            capture_name: capture_name.clone(),
            directory: entry.directory.clone(),
            capture_directory: PublicationIdentity::of(&capture.dir_metadata().map_err(display)?),
            file: entry.file.clone(),
        };
        entry.capture = Some(temporary);
    }
    // Replay the checkpoint even for an in-memory capture whose previous save
    // failed. No rename is allowed to outrun its durable ownership journal.
    if let Err(error) = checkpoint(entry) {
        if created_capture {
            let cleanup = (|| -> Result<(), String> {
                let temporary = entry.capture.as_ref().ok_or("Missing new cleanup capture")?;
                publication_directory_unchanged(parent, &directory, &entry.directory)?;
                let capture = open_publication_capture(&directory, &temporary.capture_name)?;
                if PublicationIdentity::of(&capture.dir_metadata().map_err(display)?) != temporary.capture_directory {
                    return Err("New cleanup capture was replaced; unrelated directory preserved".into());
                }
                // Windows cannot remove a directory whose capability is open;
                // the removal re-verifies this identity there.
                drop(capture);
                // Empty-only removal cannot erase an entry introduced by anyone else.
                remove_owned_empty_directory(&directory, &temporary.capture_name, &temporary.capture_directory)
                    .map_err(display)?;
                sync_publication_directory(&directory)
            })();
            *entry = previous;
            let rollback = checkpoint(entry);
            return Err(format!("{error}{}{}",
                cleanup.err().map(|error| format!("; removing empty owned capture: {error}")).unwrap_or_default(),
                rollback.err().map(|error| format!("; restoring ownership checkpoint: {error}")).unwrap_or_default()));
        }
        return Err(error);
    }
    reclaim_captured_file(parent, &directory, entry.capture.as_ref().ok_or("Missing cleanup capture")?, Some(entry), || {})
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    version: u32,
    download_limit: u64,
    upload_limit: u64,
    jobs: Vec<Record>,
}

struct Runtime {
    generation: Uuid,
    token: CancellationToken,
    retired: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    handle: Option<Arc<ManagedTorrent>>,
}

struct Inner {
    folder: PathBuf,
    store: Arc<Store>,
    state: Mutex<State>,
    runtimes: Mutex<HashMap<String, Runtime>>,
    retirement_failures: Mutex<HashMap<String, String>>,
    gate: AsyncMutex<()>,
    engine: Mutex<Arc<OnceCell<Arc<Session>>>>,
    tasks: Mutex<JoinSet<()>>,
    shutdown_gate: AsyncMutex<()>,
    /// One storage fence per job. Each job's staging tree is disjoint, so a
    /// single app-wide fence only serialized unrelated torrents (and terminal
    /// cleanup) behind the slowest one. A job's init/reads/writes and its
    /// terminal cleanup take the same fence, so no storage call is in flight
    /// while that job's files are unlinked.
    storage_gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    stop: CancellationToken,
    suspended: AtomicBool,
    /// Abandoned `add_torrent` calls whose session registration is still being
    /// undone, per job. Each token is cancelled when its reap finishes (or its
    /// reaper is dropped). A new worker for the job waits for them before it
    /// adds the same torrent again, so it never sees "already managed".
    reaps: Mutex<HashMap<String, Vec<CancellationToken>>>,
    #[cfg(test)]
    engine_hook: Mutex<Option<EngineHook>>,
    /// Set only when an unreadable saved database was quarantined this launch.
    /// It is a quiet, one-time notice, never a startup failure.
    history_notice: Option<String>,
    /// Per-record skips from an otherwise readable history.
    #[cfg(test)]
    load_warnings: Vec<String>,
    _lock: File,
}

#[derive(Clone)]
pub struct TorrentManager {
    inner: Arc<Inner>,
}

/// Repair paths that are fully derivable from durable identity instead of
/// rejecting the record. A moved app container changes every absolute path at
/// once; the job id still names its metadata/staging, and a share still names
/// the library installation it seeds.
fn reanchor_saved_paths(record: &mut Record, folder: &Path, store: &Store) -> Result<bool, String> {
    let suffix = record
        .job
        .id
        .strip_prefix("torrent:")
        .ok_or("Invalid torrent job identifier")?;
    Uuid::parse_str(suffix).map_err(display)?;
    let mut changed = false;
    let metadata = folder.join(format!("{suffix}.torrent"));
    if record.metadata != metadata {
        record.metadata = metadata;
        changed = true;
    }
    let staging = record.directory.join(format!(".kiwix-torrent-{suffix}"));
    if record.staging != staging {
        record.staging = staging;
        changed = true;
    }
    if let Some(share) = record.share.as_mut() {
        // The library entry, not the saved absolute path, is the authority for
        // where a shared archive lives now. Identity evidence is never refreshed.
        if let Ok(Some(book)) = store.book(&share.book_id) {
            let path = PathBuf::from(&book.path);
            if book.installation_id == share.installation_id
                && path.is_absolute()
                && share.path.file_name() == path.file_name()
            {
                let directory = path
                    .parent()
                    .ok_or("This shared file is no longer where Kiwix last saw it. Add it to the library again")?
                    .to_path_buf();
                if share.path != path {
                    share.path = path.clone();
                    changed = true;
                }
                if record.directory != directory {
                    record.directory = directory.clone();
                    changed = true;
                }
                let staging = directory.join(format!(".kiwix-torrent-{suffix}"));
                if record.staging != staging {
                    record.staging = staging;
                    changed = true;
                }
                let destination = path.to_string_lossy().into_owned();
                if record.job.destination != destination {
                    record.job.destination = destination;
                    changed = true;
                }
                if let Some(name) = path.file_name() {
                    let name = name.to_string_lossy().into_owned();
                    if record.job.file_name != name {
                        record.job.file_name = name;
                        changed = true;
                    }
                }
            }
        }
        let torrent_path = record.metadata.to_string_lossy().into_owned();
        if record.job.torrent_path.as_deref() != Some(torrent_path.as_str()) {
            record.job.torrent_path = Some(torrent_path);
            changed = true;
        }
    }
    Ok(changed)
}

/// Read, re-anchor and validate saved history. Per-record failures are skipped
/// and reported; only an unreadable file, unsupported version, or structurally
/// impossible envelope still returns `Err` so the caller can quarantine it.
fn load_state(
    path: &Path,
    folder: &Path,
    store: &Store,
) -> Result<(State, Vec<String>), String> {
    let file = File::open(path).map_err(display)?;
    if file.metadata().map_err(display)?.len() > MAX_METADATA as u64 {
        return Err("Saved transfer history is too large to read. Clear it from Troubleshooting or start with a new profile".into());
    }
    let mut value: serde_json::Value = serde_json::from_reader(file)
        .map_err(|error| format!("Invalid saved transfer history JSON: {error}"))?;
    #[cfg(desktop)]
    store
        .portable_rebase_saved_json(crate::profile_paths::RecordKind::Torrents, &mut value)
        .map_err(|error| format!("Cannot re-anchor saved transfer paths: {error}"))?;
    #[cfg(mobile)]
    store
        .mobile_rebase_saved_json(crate::mobile_rebase::RecordKind::Torrents, &mut value)
        .map_err(|error| format!("Cannot re-anchor saved transfer paths: {error}"))?;
    let Some(object) = value.as_object() else {
        return Err("Invalid saved transfer history: expected an object".into());
    };
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or("Invalid saved transfer history: missing version")? as u32;
    if version != STATE_VERSION {
        return Err(format!("Saved transfer history was written by a different version of Kiwix ({version}). Open it with that version, or start with a new profile"));
    }
    let mut warnings = Vec::new();
    let mut jobs = Vec::new();
    let mut ids = BTreeSet::new();
    if let Some(rows) = object.get("jobs").and_then(serde_json::Value::as_array) {
        for row in rows {
            let Ok(mut record) = serde_json::from_value::<Record>(row.clone()) else {
                warnings.push("A saved transfer could not be read and was skipped.".into());
                continue;
            };
            if reanchor_saved_paths(&mut record, folder, store).is_err()
                || validate_saved_record(&record, folder).is_err()
                || !ids.insert(record.job.id.clone())
            {
                warnings.push("A saved transfer could not be read and was skipped.".into());
                continue;
            }
            // Jobs saved before creation times were recorded: approximate with
            // the metadata file's modification time, which (unlike creation
            // time) survives copying or restoring a profile. A magnet job that
            // never received metadata has no file and stays unstamped.
            if record.job.created_at.is_none() {
                record.job.created_at = std::fs::metadata(&record.metadata)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339());
            }
            jobs.push(record);
        }
    } else if object.contains_key("jobs") {
        warnings.push("Some saved transfers could not be read and were skipped.".into());
    }
    let state = State {
        version,
        download_limit: object
            .get("downloadLimit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        upload_limit: object
            .get("uploadLimit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        jobs,
    };
    validate_saved_state(&state, folder)?;
    Ok((state, warnings))
}

/// Structural validation only. Paths that a moved container can explain were
/// already re-anchored; what remains here is genuinely inconsistent data.
fn validate_saved_state(state: &State, folder: &Path) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for record in &state.jobs {
        if !ids.insert(record.job.id.clone()) {
            return Err("The same transfer appears twice in saved history. Kiwix skipped the duplicate".into());
        }
        validate_saved_record(record, folder)?;
    }
    Ok(())
}

fn validate_saved_record(record: &Record, folder: &Path) -> Result<(), String> {
    let suffix = record
        .job
        .id
        .strip_prefix("torrent:")
        .ok_or("Invalid torrent job identifier")?;
    Uuid::parse_str(suffix).map_err(display)?;
    if record.metadata != folder.join(format!("{suffix}.torrent"))
        || record.staging != record.directory.join(format!(".kiwix-torrent-{suffix}"))
        || record.job.id != format!("torrent:{}", Uuid::parse_str(suffix).map_err(display)?)
    {
        return Err("A saved transfer is damaged. Kiwix set it aside; download the file again if it is missing".into());
    }
    match &record.share {
        Some(share) => {
            let destination = share.path.to_string_lossy();
            let torrent_path = record.metadata.to_string_lossy();
            if !record.job.shared
                || !record.verified
                || !share.path.is_absolute()
                || share.path.file_name().is_none()
                || record.job.destination != destination
                || record.job.torrent_path.as_deref() != Some(torrent_path.as_ref())
                || record.job.file_name != share.path.file_name().unwrap().to_string_lossy()
                || !record.files.is_empty()
                || !record.space_files.is_empty()
                || !record.publication_intents.is_empty()
                || !record.publication_temporaries.is_empty()
                || !record.staging_cleanup.is_empty()
                || !record.published_cleanup.is_empty()
                || !record.job.completed_paths.is_empty()
                || (share.piece_length != 0 && share.piece_length != SHARE_PIECE_LENGTH)
                || (!share.info_hash.is_empty()
                    && (share.info_hash.len() != 40
                        || !share.info_hash.bytes().all(|byte| byte.is_ascii_hexdigit())))
            {
                return Err("A saved shared file entry is damaged. Kiwix set it aside; share the file again if it is missing".into());
            }
        }
        None if record.job.shared
            || record.job.magnet.is_some()
            || record.job.torrent_path.is_some() =>
        {
            return Err("Invalid torrent share fields".into())
        }
        None => {}
    }
    for file in &record.files {
        safe_relative(file).map_err(display)?;
    }
    // Index once: per-entry linear scans made this quadratic in the file count.
    let file_paths: HashSet<&Path> = record.files.iter().map(PathBuf::as_path).collect();
    let file_names: HashSet<&std::ffi::OsStr> =
        record.files.iter().filter_map(|file| file.file_name()).collect();
    let mut space_paths = BTreeSet::new();
    for file in &record.space_files {
        safe_relative(&file.path).map_err(display)?;
        if !file_paths.contains(file.path.as_path()) || !space_paths.insert(&file.path) {
            return Err("A saved transfer detail is damaged. Kiwix set it aside; download the file again if it is missing".into());
        }
    }
    let mut publications = BTreeSet::new();
    for intent in &record.publication_intents {
        safe_relative(&intent.file_name).map_err(display)?;
        if intent.file_name.components().count() != 1
            || intent.size < 4
            || !publications.insert(&intent.file_name)
            || !file_names.contains(intent.file_name.as_os_str())
        {
            return Err("Invalid torrent publication intent".into());
        }
    }
    let mut temporary_names = BTreeSet::new();
    for temporary in &record.publication_temporaries {
        validate_publication_temporary(temporary)?;
        if !temporary_names.insert(&temporary.name) {
            return Err("The same temporary transfer appears twice in saved history. Kiwix skipped the duplicate".into());
        }
    }
    if let Some(temporary) = &record.metadata_temporary {
        validate_publication_temporary(temporary)?;
    }
    Ok(())
}

/// Move unreadable history aside without changing a byte. Rename is preferred;
/// a directory that forbids renames still gets a preserved copy, and the
/// original is never deleted here.
fn quarantine_history(path: &Path, folder: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    for attempt in 0..1000u32 {
        let target = folder.join(if attempt == 0 {
            format!("jobs.unreadable-{stamp}.json")
        } else {
            format!("jobs.unreadable-{stamp}-{attempt}.json")
        });
        if target.exists() {
            continue;
        }
        match fs::rename(path, &target) {
            Ok(()) => {
                #[cfg(unix)]
                if let Ok(directory) = File::open(folder) {
                    let _ = directory.sync_all();
                }
                return Ok(target);
            }
            Err(rename_error) => {
                return match fs::copy(path, &target) {
                    Ok(_) => Ok(target),
                    Err(_) => Err(format!(
                        "Cannot set aside unreadable transfer history {}: {rename_error}",
                        path.display()
                    )),
                };
            }
        }
    }
    Err("Kiwix could not move a damaged saved transfer file aside. Restart the app; if this keeps happening, free some storage".into())
}


// Staging is retained after publication, so a same-volume download needs both
// its remaining payload and the separate no-clobber archive copies at once.
fn space_requirements(record: &Record) -> Result<Vec<(PathBuf, u64, &'static str)>, String> {
    if record.verified {
        return Ok(Vec::new());
    }
    let Some(total) = record.job.total else {
        return Ok(Vec::new());
    };
    let destination = Dir::open_ambient_dir(&record.directory, ambient_authority())
        .map_err(display)?;
    let committed = |path: &Path| {
        record.publication_intents.iter().any(|intent| {
            path.file_name() == Some(intent.file_name.as_os_str())
                && destination.symlink_metadata(&intent.file_name).is_ok_and(|metadata| {
                    metadata.is_file()
                        && metadata.len() == intent.size
                        && metadata.dev() == intent.device
                        && metadata.ino() == intent.inode
                        && metadata.nlink() == 1
                })
        })
    };
    if record.files.iter().any(|path| is_zim(path))
        && record.files.iter().filter(|path| is_zim(path)).all(|path| committed(path))
    {
        // Durable publication recovery reads existing output, without payload.
        return Ok(Vec::new());
    }
    let mut staging_total = 0u64;
    let mut publication_remaining = 0u64;
    for file in &record.space_files {
        staging_total = staging_total.checked_add(file.size).ok_or("Torrent size overflow")?;
        if is_zim(&file.path) && !committed(&file.path) {
            publication_remaining = publication_remaining
                .checked_add(file.size)
                .ok_or("Torrent publication size overflow")?;
        }
    }
    if record.space_files.is_empty() {
        // Older records know their total but not their per-file lengths. Use a
        // conservative bound until cached metadata supplies the exact estimate.
        staging_total = total;
        publication_remaining = total;
    }
    // Progress includes virtual padding. Bounding by both totals never treats
    // a sparse file's advertised length as already allocated payload.
    let staging_remaining = total.saturating_sub(record.job.downloaded).min(staging_total);
    let staging_path = if record.staging.exists() {
        record.staging.clone()
    } else {
        record.directory.clone()
    };
    let staging = Dir::open_ambient_dir(&staging_path, ambient_authority()).map_err(display)?;
    if staging.dir_metadata().map_err(display)?.dev()
        == destination.dir_metadata().map_err(display)?.dev()
    {
        let remaining = staging_remaining
            .checked_add(publication_remaining)
            .ok_or("Torrent capacity requirement overflow")?;
        return Ok(if remaining == 0 {
            Vec::new()
        } else {
            vec![(record.directory.clone(), remaining, "staging-and-publication")]
        });
    }
    let mut requirements = Vec::new();
    if staging_remaining != 0 {
        requirements.push((staging_path, staging_remaining, "staging"));
    }
    if publication_remaining != 0 {
        requirements.push((record.directory.clone(), publication_remaining, "publication"));
    }
    Ok(requirements)
}

fn apply_worker_error(record: &mut Record, error: String, cancelled: bool) {
    if error == "MOBILE_NETWORK_BLOCKED" {
        // Manual pause/cancel and completed non-seeding payloads win even when
        // their intent was committed while this worker was retiring.
        if can_restore_record(record) {
            restore_record(record, false);
            record.job.error = None;
        } else {
            return;
        }
    } else if !cancelled {
        record.job.status = "error".into();
        record.job.error = Some(error);
    } else {
        return;
    }
    record.job.speed = 0.0;
    record.job.upload_speed = 0.0;
    record.job.eta_seconds = None;
}

fn can_restore_record(record: &Record) -> bool {
    !record.history_dismissed && !record.files_deleted && !record.cleanup_started() && !record.paused
        && !matches!(record.job.status.as_str(), "cancelled" | "error")
        && (!record.verified || record.job.seeding)
}

fn restore_record(record: &mut Record, auto_resume: bool) -> bool {
    if !can_restore_record(record) {
        return false;
    }
    // Automatic restoration never supplies an explicit user confirmation.
    record.space_acknowledgements.clear();
    record.interrupted = !auto_resume;
    record.job.status = if auto_resume {
        "queued"
    } else if record.verified {
        "completed"
    } else {
        "paused"
    }
    .into();
    // Policy retirement stops a share; unlike an ordinary completed download,
    // it must not keep advertising a seed permission while no worker serves it.
    if record.share.is_some() && !auto_resume {
        record.job.seeding = false;
    }
    auto_resume
}

fn resume_record(record: &mut Record) {
    record.paused = false;
    record.interrupted = false;
    record.job.error = None;
    record.job.status = if record.verified { "seeding" } else { "queued" }.into();
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The runtime librqbit's own tasks run on. librqbit starts its tasks (session,
/// DHT, tracker comms, peers) on whichever runtime calls into it, and its
/// HTTPS tracker client verifies certificates with blocking platform calls:
/// JNI on Android, where Android may fetch a CRL inside the call, and the
/// platform APIs on macOS and Windows. On Tauri's shared runtime those calls
/// could hold the workers that IPC commands need.
///
/// Only the librqbit calls that start tasks or poll trackers run here
/// (`Session::new_with_opts`, `add_torrent`, `unpause`). The manager's worker
/// loops, its gates and every retirement path stay on the caller's runtime and
/// never await one of these calls while holding a gate. So pause, cancel,
/// suspend, shutdown and privacy cutovers never wait for a free thread here,
/// even while every thread is blocked in a verification. (Retirement's
/// `delete` and the worker loop's `pause` still take librqbit's own locks,
/// which a routed add holds while it opens storage: a disk wait, not a
/// verification.) Two workers keep the footprint small; blocked verifications
/// can only delay torrent networking.
/// A failed build is kept, so BitTorrent reports it until the app restarts.
fn torrent_runtime() -> Result<&'static tokio::runtime::Handle, String> {
    static RUNTIME: std::sync::OnceLock<Result<tokio::runtime::Runtime, String>> =
        std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("kiwix-torrent")
                .enable_all()
                .build()
                .map_err(|error| format!("BitTorrent could not start: {error}"))
        })
        .as_ref()
        .map(tokio::runtime::Runtime::handle)
        .map_err(Clone::clone)
}

/// A call running on the torrent runtime. Dropping it aborts the call, so a
/// lost `select!` ends the work as it would have inline. Abort only takes
/// effect at the task's next poll; a call that can register state in the
/// session must be handed to a reaper with `into_task` instead.
struct TorrentCall<T>(Option<tokio::task::JoinHandle<T>>);

impl<T: Send + 'static> TorrentCall<T> {
    fn spawn(future: impl std::future::Future<Output = T> + Send + 'static) -> Result<Self, String> {
        Ok(Self(Some(torrent_runtime()?.spawn(future))))
    }

    /// Hands over the task without aborting it.
    fn into_task(mut self) -> tokio::task::JoinHandle<T> {
        self.0.take().expect("torrent call already taken")
    }
}

impl<T> Drop for TorrentCall<T> {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

impl<T> std::future::Future for TorrentCall<T> {
    type Output = Result<T, String>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        let task = self.0.as_mut().expect("torrent call polled after completion");
        std::pin::Pin::new(task).poll(cx).map(|result| {
            result.map_err(|error| match error.try_into_panic() {
                // A panic in the call reaches the caller, as it would inline.
                Ok(panic) => std::panic::resume_unwind(panic),
                Err(error) => display(error),
            })
        })
    }
}

/// Runs one librqbit call on the torrent runtime and awaits it from any
/// runtime. Use it only for calls that register nothing in the session if
/// abandoned; `TorrentManager::add_on_torrent_runtime` handles `Added` adds.
async fn on_torrent_runtime<F>(future: F) -> Result<F::Output, String>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    TorrentCall::spawn(future)?.await
}

/// Test-only interception point inside routed calls, on a torrent thread.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EngineCall {
    /// After `add_torrent` returned; `true` when the session registered it.
    Added(bool),
    /// Before `unpause` starts the torrent.
    Unpause,
}

#[cfg(test)]
type EngineHook = Arc<dyn Fn(EngineCall) + Send + Sync>;

/// Inspect the selected path without following symlinks. This is the identity
/// gate used before metadata creation and again before the engine opens it.
fn share_identity_at(path: &Path) -> Result<ShareIdentity, String> {
    let parent = path.parent().ok_or("This shared file is no longer where Kiwix last saw it. Add it to the library again")?;
    let name = path.file_name().ok_or("This file cannot be shared because it has no name. Rename it and try again")?;
    let directory = crate::storage::open_directory_nofollow(parent).map_err(display)?;
    let metadata = directory.symlink_metadata(name).map_err(display)?;
    if metadata.file_type().is_symlink() {
        return Err("This file is a shortcut, so it cannot be shared. Copy the real file into your library".into());
    }
    ShareIdentity::of(&metadata)
}

fn limits_config(download: u64, upload: u64) -> Result<LimitsConfig, String> {
    fn limit(n: u64) -> Result<Option<NonZeroU32>, String> {
        if n == 0 {
            return Ok(None);
        }
        if !(16_384..=u32::MAX as u64).contains(&n) {
            return Err(
                "Torrent bandwidth must be zero (unlimited) or 16384–4294967295 bytes/second"
                    .into(),
            );
        }
        Ok(NonZeroU32::new(n as u32))
    }
    Ok(LimitsConfig {
        download_bps: limit(download)?,
        upload_bps: limit(upload)?,
    })
}

fn validate_source(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(display)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Torrent URLs cannot contain credentials".into());
    }
    match url.scheme() {
        "magnet" => {
            if url.query_pairs().any(|(k, v)| k == "xt" && v.starts_with("urn:btih:")) { Ok(()) }
            else { Err("Magnet must contain a BitTorrent v1 info hash".into()) }
        }
        "https" if url.host().is_some() => Ok(()),
        "http" if url.host_str().and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback()) => Ok(()),
        _ => Err("Torrent metadata requires HTTPS (literal loopback HTTP is allowed for local testing), or a magnet URI".into()),
    }
}

async fn fetch_metadata(url: &str) -> Result<Vec<u8>, String> {
    crate::transfer_policy::require_internet()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if let Err(error) = crate::transfer_policy::require_internet() { return attempt.error(error); }
            if attempt.previous().len() >= 5 {
                return attempt.error("Too many torrent metadata redirects");
            }
            if validate_source(attempt.url().as_str()).is_ok() && attempt.url().scheme() != "magnet"
            {
                attempt.follow()
            } else {
                attempt.error("Insecure torrent metadata redirect rejected")
            }
        }))
        .build()
        .map_err(display)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(display)?
        .error_for_status()
        .map_err(display)?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_METADATA as u64)
    {
        return Err("Torrent metadata exceeds 16 MiB".into());
    }
    let mut result = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(display)?;
        if result.len().saturating_add(chunk.len()) > MAX_METADATA {
            return Err("Torrent metadata exceeds 16 MiB".into());
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}

fn read_torrent_metadata(record: &Record) -> Result<Vec<u8>, String> {
    let path = &record.metadata;
    let file = if let Some(ownership) = &record.metadata_cleanup {
        ownership.validate_existing(path)?.ok_or("Torrent metadata is missing")?
    } else {
        let parent = path.parent().ok_or("Torrent metadata has no parent directory")?;
        let directory = crate::storage::open_directory_nofollow(parent).map_err(display)?;
        directory.open_with(
            path.file_name().ok_or("Invalid torrent metadata filename")?,
            &publication_read_options(),
        ).map_err(display)?.into_std()
    };
    let metadata = file.metadata().map_err(display)?;
    if !metadata.is_file() || metadata.len() > MAX_METADATA as u64 {
        return Err("Torrent metadata is not a regular file of at most 16 MiB".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&file).take(MAX_METADATA as u64 + 1).read_to_end(&mut bytes).map_err(display)?;
    if bytes.len() > MAX_METADATA {
        return Err("Torrent metadata exceeds 16 MiB".into());
    }
    if let Some(ownership) = &record.metadata_cleanup {
        if !ownership.matches(path, &file)?
            || ownership.expected_size.is_some_and(|size| size != bytes.len() as u64)
            || ownership.sha256.is_some_and(|hash| hash != <[u8; 32]>::from(Sha256::digest(&bytes)))
        {
            return Err("Torrent metadata no longer matches its complete durable publication".into());
        }
    }
    Ok(bytes)
}

/// Metadata follows the archive publication protocol: journal an empty private
/// inode, sync all bytes, checkpoint their identity, then exclusively publish.
/// A failed write/checkpoint leaves only the journaled temporary for recovery.
fn publish_metadata(
    path: &Path,
    bytes: &[u8],
    mut checkpoint: impl FnMut(Option<PublicationTemporary>, Option<CleanupEntry>) -> Result<(), String>,
) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > MAX_METADATA {
        return Err("Torrent metadata is empty or exceeds 16 MiB".into());
    }
    let parent = path.parent().ok_or("Torrent metadata has no parent directory")?;
    let name = path.file_name().and_then(|name| name.to_str()).ok_or("Invalid torrent metadata filename")?;
    safe_relative(Path::new(name)).map_err(display)?;
    let directory = open_publication_directory(parent)?;
    let (temporary, mut output) = create_publication_temporary(
        parent, &directory, format!(".kiwix-publish-{}", Uuid::new_v4()),
    )?;
    if let Err(error) = checkpoint(Some(temporary.clone()), None) {
        // Windows cannot capture (rename) the temporary while this writer is
        // open; nothing else needs it on this path.
        drop(output);
        return Err(match reclaim_publication_temporary(parent, &directory, &temporary) {
            Ok(()) => error,
            Err(cleanup) => format!("{error}; metadata temporary cleanup: {cleanup}"),
        });
    }
    output.write_all(bytes).map_err(display)?;
    output.sync_all().map_err(display)?;
    sync_publication_directory(&directory)?;
    let ownership = CleanupEntry {
        name: name.to_owned(),
        directory: temporary.directory.clone(),
        file: temporary.file.clone(),
        capture: None,
        expected_size: Some(bytes.len() as u64),
        sha256: Some(Sha256::digest(bytes).into()),
    };
    checkpoint(Some(temporary.clone()), Some(ownership.clone()))?;
    publication_directory_unchanged(parent, &directory, &temporary.directory)?;
    let verified = crate::publication::VerifiedFile::new(output.into_std(), bytes.len() as u64)
        .map_err(display)?;
    crate::publication::publish_verified_noreplace_in(
        &directory, Path::new(&temporary.name), Path::new(name), &verified,
    ).map_err(display)?;
    sync_publication_directory(&directory)?;
    reclaim_publication_temporary(parent, &directory, &temporary)?;
    checkpoint(None, Some(ownership))
}

/// Recovery never adopts metadata by filename/content alone. Complete intents
/// can finish their exclusive rename; partial writes are reclaimed by inode.
fn recover_metadata_publication(
    path: &Path,
    temporary: &PublicationTemporary,
    ownership: Option<&CleanupEntry>,
) -> Result<Option<CleanupEntry>, String> {
    validate_publication_temporary(temporary)?;
    let parent = path.parent().ok_or("Torrent metadata has no parent directory")?;
    let directory = open_publication_directory(parent)?;
    publication_directory_unchanged(parent, &directory, &temporary.directory)?;
    if let Some(ownership) = ownership {
        if ownership.file != temporary.file || ownership.directory != temporary.directory
            || ownership.expected_size.is_none() || ownership.sha256.is_none()
            || ownership.capture.is_some()
            || ownership.expected_size.is_some_and(|size| size == 0 || size > MAX_METADATA as u64)
        {
            return Err("Torrent metadata publication ownership does not match its temporary".into());
        }
        if let Some(file) = ownership.validate_existing(path)? {
            if !ownership.matches_contents(&file)? {
                return Err("Published torrent metadata no longer matches its complete durable bytes".into());
            }
        } else {
            let temporary_path = parent.join(&temporary.name);
            let temporary_ownership = ownership.renamed(&temporary.name);
            let file = temporary_ownership.validate_existing(&temporary_path)?
                .ok_or("Complete torrent metadata temporary is missing; its journal was preserved")?;
            if !ownership.matches_contents(&file)? {
                return Err("Torrent metadata temporary no longer matches its complete durable bytes".into());
            }
            let verified = crate::publication::VerifiedFile::new(
                file, ownership.expected_size.ok_or("Missing metadata size")?,
            ).map_err(display)?;
            publication_directory_unchanged(parent, &directory, &temporary.directory)?;
            crate::publication::publish_verified_noreplace_in(
                &directory, Path::new(&temporary.name), Path::new(&ownership.name), &verified,
            ).map_err(display)?;
            sync_publication_directory(&directory)?;
        }
        reclaim_publication_temporary(parent, &directory, temporary)?;
        Ok(Some(ownership.clone()))
    } else {
        reclaim_publication_temporary(parent, &directory, temporary)?;
        Ok(None)
    }
}

fn safe_relative(path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        bail!("Torrent contains an absolute path or traversal");
    }
    for part in path.components() {
        let part = part
            .as_os_str()
            .to_str()
            .context("Torrent filename is not UTF-8")?;
        // A portable policy also prevents Windows alternate streams/device names
        // and ambiguous trailing-dot/space names on case-insensitive filesystems.
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.contains(['\\', ':', '\0'])
            || part.ends_with(['.', ' '])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            bail!("Torrent contains an unsafe portable filename");
        }
    }
    Ok(())
}

/// A readonly walk is limited to directories already in the ownership ledger.
/// Unknown files fail before any installed book can be removed.
fn validate_staging_provenance(record: &Record, directory: &Dir, relative: &Path) -> Result<(), String> {
    let expected = record.staging_directories.iter().find(|(path, _)| path == relative)
        .ok_or("Torrent staging lacks its original directory ownership proof")?;
    if PublicationIdentity::of(&directory.dir_metadata().map_err(display)?) != expected.1 {
        return Err("Torrent staging directory was replaced; cleanup was not authorized".into());
    }
    for entry in directory.entries().map_err(display)? {
        let entry = entry.map_err(display)?;
        let name = entry.file_name();
        let path = relative.join(&name);
        safe_relative(&path).map_err(display)?;
        let metadata = directory.symlink_metadata(&name).map_err(display)?;
        if metadata.is_dir() && !metadata.is_symlink() {
            if record.staging_directories.iter().any(|(known, _)| known == &path) {
                let child = directory.open_dir_nofollow(&name).map_err(display)?;
                validate_staging_provenance(record, &child, &path)?;
                continue;
            }
            let capture = record.staging_cleanup.iter().find_map(|(file, entry)| {
                entry.capture.as_ref().filter(|capture| {
                    file.parent().unwrap_or(Path::new("")).join(&capture.capture_name) == path
                })
            }).ok_or("Kiwix found an unexpected temporary folder and left it alone. Restart the app")?;
            let child = directory.open_dir_nofollow(&name).map_err(display)?;
            if PublicationIdentity::of(&child.dir_metadata().map_err(display)?) != capture.capture_directory {
                return Err("Torrent cleanup capture directory was replaced".into());
            }
            for entry in child.entries().map_err(display)? {
                let entry = entry.map_err(display)?;
                let metadata = child.symlink_metadata(entry.file_name()).map_err(display)?;
                if entry.file_name() != std::ffi::OsStr::new("payload")
                    || !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1
                    || PublicationIdentity::of(&metadata) != capture.file
                {
                    return Err("Torrent cleanup capture contains an unowned entry".into());
                }
            }
        } else {
            let entry = record.staging_cleanup.iter().find(|(known, _)| known == &path)
                .ok_or("Kiwix found an unexpected temporary file and left it alone. Restart the app")?;
            entry.1.validate_existing(&record.staging.join(&path))?
                .ok_or("Owned staging file disappeared during provenance validation")?;
        }
    }
    Ok(())
}

fn validate_terminal_publications(record: &Record) -> Result<Vec<(PathBuf, CleanupEntry)>, String> {
    match fs::symlink_metadata(&record.staging) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(display(error)),
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            validate_staging_provenance(record, &open_staging(&record.staging)?, Path::new(""))?;
        }
        Ok(_) => return Err("Torrent staging is not its owned directory; cleanup was not authorized".into()),
    }
    for (relative, ownership) in &record.staging_cleanup {
        safe_relative(relative).map_err(display)?;
        ownership.validate_cleanup(&record.staging.join(relative))?;
    }
    if let Some(ownership) = &record.metadata_cleanup {
        ownership.validate_cleanup(&record.metadata)?;
    }
    if let Some(temporary) = &record.metadata_temporary {
        validate_publication_temporary(temporary)?;
        CleanupEntry {
            name: temporary.name.clone(), directory: temporary.directory.clone(),
            file: temporary.file.clone(), capture: Some(temporary.clone()),
            expected_size: None, sha256: None,
        }.validate_cleanup(&record.metadata.with_file_name(&temporary.name))?;
    }
    for ownership in &record.published_cleanup {
        ownership.validate_cleanup(&record.directory.join(&ownership.name))?;
    }
    for temporary in &record.publication_temporaries {
        validate_publication_temporary(temporary)?;
        CleanupEntry {
            name: temporary.name.clone(), directory: temporary.directory.clone(),
            file: temporary.file.clone(), capture: Some(temporary.clone()),
            expected_size: None, sha256: None,
        }.validate_cleanup(&record.directory.join(&temporary.name))?;
    }
    match fs::symlink_metadata(&record.metadata) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(display(error)),
        Ok(_) => {
            record.metadata_cleanup.as_ref().ok_or("Legacy torrent metadata lacks original file ownership proof")?
                .validate_existing(&record.metadata)?.ok_or("Torrent metadata disappeared during provenance validation")?;
        }
    }
    let mut proven = Vec::new();
    let destination = match crate::storage::open_directory_nofollow(&record.directory) {
        Ok(directory) => Some(directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(display(error)),
    };
    for file in record.files.iter().filter(|file| is_zim(file)) {
        let name = file.file_name().ok_or("Missing published torrent filename")?;
        let Some(destination) = &destination else { continue; };
        match destination.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(display(error)),
            Ok(_) => {}
        }
        let intent = record.publication_intents.iter().find(|intent| intent.file_name == Path::new(name))
            .ok_or("Published torrent archive lacks its original ownership proof; library deletion was not authorized")?;
        let ownership = intent.cleanup_entry()?;
        let path = record.directory.join(name);
        ownership.validate_existing(&path)?.ok_or("Published torrent archive disappeared during authorization")?;
        let verified = recover_publication(destination, intent, &CancellationToken::new())?
            .ok_or("Published torrent archive disappeared during checksum verification")?;
        if !ownership.matches(&path, verified.file())? {
            return Err("Published torrent archive was replaced; library deletion was not authorized".into());
        }
        proven.push((path, ownership));
    }
    for completed in &record.job.completed_paths {
        let path = Path::new(completed);
        if !proven.iter().any(|(known, _)| known == path) {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(display(error)),
                Ok(_) => return Err("Completed torrent path has no original publication proof; library deletion was not authorized".into()),
            }
        }
    }
    Ok(proven)
}

fn open_staging(path: &Path) -> Result<Dir, String> {
    let parent = path.parent().ok_or("Torrent staging has no parent")?;
    let name = path.file_name().ok_or("Torrent staging has no filename")?;
    let parent = Dir::open_ambient_dir(parent, ambient_authority()).map_err(display)?;
    parent
        .open_dir_nofollow(name)
        .map_err(|e| format!("Cannot safely open torrent staging: {e}"))
}

fn is_zim(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zim"))
}

fn save_state(folder: &Path, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(display)?;
    atomic_bytes(&folder.join("jobs.json"), &bytes).map_err(display)
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let temp = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        File::open(path.parent().context("No metadata parent")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}



#[cfg(test)]
#[path = "torrent_tests.rs"]
mod tests;
