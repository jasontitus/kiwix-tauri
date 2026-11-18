// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Mobile profile relocation, admitted only while Store holds its process lock.
//!
//! Persisted filesystem-path inventory (do not recursively rewrite arbitrary JSON):
//! * state.json: managed books[].path, settings.downloadDirectory/monitorDirectories.
//!   Bookmark/history/tab/mainPath/readerStorage paths are archive content, not files.
//! * downloads/downloads.json: each flattened StoredJob.destination. Part paths are
//!   derived from that destination; publication size/SHA-256 evidence is untouched.
//! * torrents/jobs.json: jobs[].{staging,directory,metadata}, job.destination,
//!   job.completedPaths and share.path. files and publicationIntents.fileName are
//!   relative names; share identity, publication inode/device, hashes and
//!   temporary ownership records are untouched.
//! * localswarm/jobs.json: each job.destination/completedPaths and owned.parent.
//!   OwnedSet parent/directory/file identities, manifest names and hashes are retained.
//! * maps/map-job.json: current/monitored paths, ownedPartial/recoveryRecords
//!   destinations, published.record and retainedPublications[].published.record.
//!   All directory/file/capture identities and publication names/sizes/hashes remain.
//! * library-migrations-v1/<UUID>.json: source_directory/destination_directory.
//!   Owner, file stamps, hashes and staging/cleanup/directory identities are retained.
//!
//! Imports have no Rust disk queue: consumed acquisition IDs live in state.json.
//! Torrent import receipts additionally embed SHA-256(full completed-path bytes).
//! Exact job/completed-path correlations migrate those consumedAcquisitions IDs;
//! HTTP/LocalSwarm/native IDs remain unchanged, including when a Book was removed.
//! Native incoming queues contain provider URIs, not Rust profile-owned paths.
//! Export/import staging and map-parts caches contain bytes or relative names only.
//! Torrent SDK session/fast-resume and DHT persistence are explicitly disabled in
//! torrent.rs; there is no additional owned SDK-session pathname store to migrate.
//!
//! The private marker binds its recorded root to the *same* profile and capability
//! directory identities. A missing marker never licenses guessing an earlier root.
//! Files are individually atomic and prefix rewriting is idempotent. The old root
//! marker remains authoritative until every rewritten record and directory is synced.
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fs::File, io::{Read, Write}, path::{Component, Path, PathBuf}};
use uuid::Uuid;

const CAPABILITY: &str = "mobile-root-v1";
const MARKER: &str = "root.json";
const MAX_RECORD: u64 = 128 * 1024 * 1024;
const MAX_JOURNALS: usize = 1024;
const MAX_NOTICES: usize = 8;

// Content-free notices for the Store's startup channel. They must never carry
// a path, title, or identifier; the detailed cause stays in the local log.
const MARKER_UNREADABLE: &str = "Kiwix had to create a new storage folder because the old one could not be read. Existing files were kept.";
const MARKER_INVALID: &str = "Kiwix could not check its storage folder and set it up again. Existing files were kept.";
#[cfg(unix)] // Only the Unix ownership check reports it; Windows hosts compile this for tests.
const MARKER_NOT_PRIVATE: &str = "Kiwix repaired its storage folder permissions. Existing files were kept.";
const MARKER_NOT_BOUND: &str = "Kiwix updated where it keeps its files after the app moved.";
const RELOCATION_SKIPPED: &str = "A saved transfer could not be moved after the app updated. Open Downloads and resume it.";
const PENDING_IGNORED: &str = "A saved storage move for a different library was ignored.";
const ROOT_OVERLAP: &str = "Kiwix ignored a saved storage location that pointed inside the current one. Your files were kept.";
const RECEIPTS_SKIPPED: &str = "Some saved transfer details could not be moved after the app updated. Downloads still work.";
pub const STATE_QUARANTINED: &str = "Kiwix could not read your saved settings, so it started with an empty library. The original file was kept.";

pub use crate::profile_paths::RecordKind;

/// Result of mobile profile initialization. `notices` is bounded and safe to
/// show in the shell's startup warning area.
#[derive(Default)]
pub struct Initialized {
    pub relocation: Option<Relocation>,
    pub notices: Vec<String>,
    /// True only when the new root marker was committed (or no relocation was
    /// needed). False means the runtime view may be rebased but the durable
    /// records/marker still need a retry.
    #[allow(dead_code, reason = "reported for callers; none reads it yet")]
    pub durable: bool,
}

fn notice(notices: &mut Vec<String>, text: &str) {
    if notices.len() < MAX_NOTICES && !notices.iter().any(|item| item == text) {
        notices.push(text.to_owned());
    }
}

#[derive(Clone, Debug)]
pub struct Relocation {
    pub old_root: PathBuf,
    pub new_root: PathBuf,
    // Targets durably authorized before an earlier interrupted migration. This
    // also handles a second OS container move before the first marker commits.
    previous_targets: Vec<PathBuf>,
    torrent_receipts: HashMap<String, String>,
}

impl Relocation {
    /// Pure runtime view: no probes, directory creation, repairs, or other writes.
    /// Only schema-designated, normalized absolute paths beneath old_root move.
    /// Foreign sources remain foreign; no ownership bit or identity is modified.
    pub fn rebase_json(&self, kind: RecordKind, value: &mut Value) -> Result<bool, String> {
        normalized_absolute(&self.old_root)?;
        normalized_absolute(&self.new_root)?;
        let mut changed = false;
        changed |= crate::profile_paths::rewrite(kind, value, false, |value, _| self.path(value))?;
        if matches!(kind, RecordKind::State) {
                if let Some(receipts) = value.get_mut("consumedAcquisitions") {
                    // The Store skips malformed receipt collections and entries
                    // with warnings, so relocation must not reject a state it
                    // accepts; only valid string receipts are rewritten.
                    if let Some(receipts) = receipts.as_array_mut() {
                        let mut receipts_changed = false;
                        for receipt in receipts.iter_mut() {
                            let Some(id) = receipt.as_str() else { continue; };
                            if let Some(replacement) = self.torrent_receipts.get(id) {
                                *receipt = Value::String(replacement.clone());
                                receipts_changed = true;
                            }
                        }
                        if receipts_changed {
                            // State stores a HashSet. Collapse only the duplicates
                            // created when both old and current receipts were saved.
                            receipts.sort_unstable_by(|left, right| left.as_str().cmp(&right.as_str()));
                            receipts.dedup();
                            changed = true;
                        }
                    }
                }
        }
        Ok(changed)
    }

    fn load_torrent_receipts(&mut self, profile: &Directory) -> Result<Option<(Directory, Record)>, String> {
        use sha2::Digest;
        let Some(directory) = profile.child("torrents")? else { return Ok(None); };
        let Some(record) = directory.read("jobs.json", MAX_RECORD)? else { return Ok(None); };
        let value = record.json()?;
        let rows = value.get("jobs").and_then(Value::as_array).ok_or("Torrent state has no jobs array")?;
        let roots: Vec<_> = std::iter::once(&self.old_root).chain(&self.previous_targets)
            .chain(std::iter::once(&self.new_root)).collect();
        for row in rows {
            let job = row.get("job").ok_or("Saved transfer history is damaged. Restore a backup")?;
            let id = job.get("id").and_then(Value::as_str).ok_or("Torrent job has no ID")?;
            let suffix = id.strip_prefix("torrent:").ok_or("Invalid torrent receipt job ID")?;
            if Uuid::parse_str(suffix).map_err(err)?.to_string() != suffix {
                return Err("Invalid torrent receipt job ID".into());
            }
            let paths = job.get("completedPaths").and_then(Value::as_array).ok_or("Torrent job has no completed paths array")?;
            for path in paths {
                let path = Path::new(path.as_str().ok_or("Torrent completed path is not a string")?);
                normalized_absolute(path)?;
                // No Book lookup: consumption remains authoritative after removal.
                // Already-rebased paths still reveal the exact old-root suffix,
                // including replay after a second interrupted container move.
                let Some(relative) = roots.iter().find_map(|root| path.strip_prefix(root).ok()) else { continue; };
                let destination = self.new_root.join(relative);
                let destination = destination.to_str().ok_or("Torrent completed path is not UTF-8")?;
                let replacement = format!("{id}:{}", hex::encode(sha2::Sha256::digest(destination.as_bytes())));
                for root in &roots {
                    if *root == &self.new_root { continue; }
                    let source = root.join(relative);
                    let source = source.to_str().ok_or("Torrent receipt source path is not UTF-8")?;
                    let receipt = format!("{id}:{}", hex::encode(sha2::Sha256::digest(source.as_bytes())));
                    if receipt == replacement { continue; }
                    if let Some(previous) = self.torrent_receipts.insert(receipt, replacement.clone()) {
                        if previous != replacement { return Err("Conflicting torrent receipt path correlations".into()); }
                    }
                }
            }
        }
        record.unchanged(&directory)?;
        Ok(Some((directory, record)))
    }

    fn path(&self, value: &mut Value) -> Result<bool, String> {
        if value.is_null() { return Ok(false); }
        let text = value.as_str().ok_or("Persisted filesystem path is not a string")?;
        if text.is_empty() { return Ok(false); }
        let path = Path::new(text);
        // The Store accepts nonempty relative book paths. Relocation owns only
        // absolute paths beneath the recorded root, so a legacy relative entry
        // is left exactly as saved instead of aborting the whole rewrite.
        if !path.is_absolute() { return Ok(false); }
        // Reject traversal even when lexical prefix matching would otherwise let
        // old_root/../foreign escape the private capability.
        normalized_absolute(path)?;
        if path.starts_with(&self.new_root) { return Ok(false); }
        let relative = std::iter::once(&self.old_root).chain(&self.previous_targets)
            .find_map(|root| path.strip_prefix(root).ok());
        let Some(relative) = relative else { return Ok(false); };
        let replacement = self.new_root.join(relative);
        let replacement = replacement.to_str().ok_or("Mobile path is not UTF-8")?;
        *value = Value::String(replacement.to_owned());
        Ok(true)
    }
}

fn normalized_absolute(path: &Path) -> Result<(), String> {
    if !path.is_absolute() { return Err(format!("Mobile filesystem path is not absolute: {}", path.display())); }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => normalized.push(component.as_os_str()),
            _ => return Err(format!("Mobile filesystem path contains traversal: {}", path.display())),
        }
    }
    if normalized.as_os_str() != path.as_os_str() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(format!("Mobile filesystem path is not normalized: {}", path.display()));
    }
    Ok(())
}
fn err(error: impl std::fmt::Display) -> String { error.to_string() }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity { device: u64, inode: u64 }
impl Identity {
    fn of(metadata: &Metadata) -> Self { Self { device: metadata.dev(), inode: metadata.ino() } }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    version: u32,
    owner: String,
    root: PathBuf,
    profile: Identity,
    capability: Identity,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pending {
    version: u32,
    owner: String,
    source: PathBuf,
    profile: Identity,
    capability: Identity,
    targets: Vec<PathBuf>,
}

struct Directory { path: PathBuf, dir: Dir, identity: Identity }
impl Directory {
    fn open(path: &Path) -> Result<Self, String> {
        normalized_absolute(path)?;
        let dir = crate::storage::open_directory_nofollow(path).map_err(err)?;
        Self::from_dir(path.to_path_buf(), dir)
    }
    fn from_dir(path: PathBuf, dir: Dir) -> Result<Self, String> {
        let metadata = dir.dir_metadata().map_err(err)?;
        owned_file(&dir.try_clone().map_err(err)?.into_std_file(), false)?;
        Ok(Self { path, identity: Identity::of(&metadata), dir })
    }
    fn child(&self, name: &str) -> Result<Option<Self>, String> {
        match self.dir.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(err(error)),
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => return Err(format!("Relocation directory is not a real directory: {name}")),
            Ok(_) => (),
        }
        Self::from_dir(self.path.join(name), self.dir.open_dir_nofollow(name).map_err(err)?).map(Some)
    }
    fn unchanged(&self) -> Result<(), String> {
        if Self::open(&self.path)?.identity != self.identity { return Err("The storage folder changed while Kiwix was moving files. Restart the app".into()); }
        Ok(())
    }
    #[cfg(unix)]
    fn sync(&self) -> Result<(), String> {
        crate::storage::sync_cap_directory(&self.dir).map_err(|_| "Kiwix could not finish saving files for the move. Restart the app".to_string())
    }
    // Mobile rebasing only ships on Android and iOS (both Unix). This module is
    // also compiled into host unit tests, so Windows needs a definition; NTFS
    // has no directory fsync and those tests do not observe durability.
    #[cfg(not(unix))]
    fn sync(&self) -> Result<(), String> { Ok(()) }
    fn read(&self, name: &str, limit: u64) -> Result<Option<Record>, String> {
        let before = match self.dir.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(err(error)),
            Ok(metadata) => metadata,
        };
        regular(&before)?;
        if before.len() > limit { return Err("A saved file is too large to move. Restart the app".into()); }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = self.dir.open_with(name, &options).map_err(err)?.into_std();
        owned_file(&file, false)?;
        if Identity::of(&Metadata::from_file(&file).map_err(err)?) != Identity::of(&before) {
            return Err("A saved file changed while Kiwix was moving it. Restart the app".into());
        }
        let mut bytes = Vec::new();
        (&mut file).take(limit + 1).read_to_end(&mut bytes).map_err(err)?;
        if bytes.len() as u64 > limit { return Err("A saved file changed while Kiwix was moving it. Restart the app".into()); }
        let record = Record { name: name.to_owned(), file, before, bytes };
        record.unchanged(self)?;
        Ok(Some(record))
    }
    fn publish(&self, name: &str, bytes: &[u8], previous: Option<&Record>) -> Result<(), String> {
        self.unchanged()?;
        if let Some(previous) = previous { previous.unchanged(self)?; }
        let temporary = format!(".mobile-rebase-{}", Uuid::new_v4());
        let prepared = format!(".mobile-ready-{}", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).read(true).create_new(true).follow(FollowSymlinks::No);
        #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = self.dir.open_with(&temporary, &options).map_err(err)?.into_std();
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        let verified = crate::publication::VerifiedFile::new(file, bytes.len() as u64).map_err(err)?;
        self.unchanged()?;
        // Exclusively expose the complete temporary before touching an existing
        // record. Unknown temporary entries are never adopted or swept on replay.
        crate::publication::publish_verified_noreplace_in(&self.dir, Path::new(&temporary), Path::new(&prepared), &verified).map_err(err)?;
        self.sync()?;
        if let Some(previous) = previous {
            previous.unchanged(self)?;
            self.dir.rename(&prepared, &self.dir, name).map_err(err)?;
            if !verified.matches_in(&self.dir, Path::new(name)).map_err(err)? { return Err("Relocation publication identity changed".into()); }
        } else {
            crate::publication::publish_verified_noreplace_in(&self.dir, Path::new(&prepared), Path::new(name), &verified).map_err(err)?;
        }
        self.sync()?;
        self.unchanged()
    }
}
fn owned_file(file: &File, private: bool) -> Result<(), String> {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = file.metadata().map_err(err)?;
        let forbidden = if private { 0o077 } else { 0o022 };
        if metadata.uid() != crate::fs_ext::effective_uid() || metadata.mode() & forbidden != 0 {
            return Err("Relocation capability must be owned and not writable by other users".into());
        }
    }
    // Mobile targets are Unix; Windows only compiles this module for host tests.
    #[cfg(not(unix))]
    let _ = (file, private);
    Ok(())
}
fn private_directory(directory: &Directory) -> Result<(), String> {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = directory.dir.try_clone().map_err(err)?.into_std_file().metadata().map_err(err)?;
        if metadata.uid() != crate::fs_ext::effective_uid() || metadata.mode() & 0o077 != 0 {
            return Err("Mobile root capability must be owned and private (0700)".into());
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}
fn regular(metadata: &Metadata) -> Result<(), String> {
    if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1 {
        return Err("Relocation state must be a regular file without hard links".into());
    }
    Ok(())
}
struct Record { name: String, file: File, before: Metadata, bytes: Vec<u8> }
impl Record {
    fn unchanged(&self, directory: &Directory) -> Result<(), String> {
        owned_file(&self.file, false)?;
        directory.unchanged()?;
        for metadata in [directory.dir.symlink_metadata(&self.name).map_err(err)?, Metadata::from_file(&self.file).map_err(err)?] {
            regular(&metadata)?;
            if Identity::of(&metadata) != Identity::of(&self.before)
                || metadata.len() != self.before.len()
                || metadata.modified().map_err(err)? != self.before.modified().map_err(err)? {
                return Err("A saved file changed while Kiwix was moving it. Restart the app".to_string());
            }
        }
        Ok(())
    }
    fn json(&self) -> Result<Value, String> {
        serde_json::from_slice(&self.bytes).map_err(|_| "A saved file could not be read while moving it. Restart the app".to_string())
    }
}

struct Planned { directory: Directory, record: Record, bytes: Option<Vec<u8>> }

/// Call under the retained Store process lock, before loading state or admitting
/// any manager. Read-only initialization has no writing/repairing/creating branch.
///
/// A marker whose recorded identities no longer match is treated as absent for
/// authority: its root is never adopted as a filesystem root. When its recorded
/// root differs from the current container it is still the only durable
/// statement of where this profile lived, so saved paths beneath it are
/// re-anchored lexically to the current root — the container-restore case.
/// Missing, unreadable, and invalid markers never fail startup.
pub fn initialize(profile: &Path, read_only: bool) -> Result<Initialized, String> {
    let mut notices = Vec::new();
    let current = std::fs::canonicalize(profile).map_err(err)?;
    let profile = Directory::open(&current)?;
    let capability = profile.child(CAPABILITY)?;
    let mut unreadable_marker = false;
    let previous = match capability.as_ref() {
        Some(directory) => {
            private_directory(directory)?;
            match directory.read(MARKER, 4096) {
                Ok(record) => record,
                Err(error) => {
                    // An oversized, linked, or otherwise unreadable marker is
                    // metadata, not profile data: set it aside and rebind.
                    eprintln!("Kiwix: saved mobile root marker cannot be read: {error}");
                    unreadable_marker = true;
                    None
                }
            }
        }
        None => None,
    };
    if unreadable_marker {
        if let Some(directory) = capability.as_ref().filter(|_| !read_only) {
            let aside = format!("root.invalid-{}.json", Uuid::new_v4());
            directory.dir.rename(MARKER, &directory.dir, &aside).map_err(err)?;
            directory.sync()?;
        }
        notice(&mut notices, MARKER_UNREADABLE);
    }
    let mut recorded: Option<Marker> = match previous.as_ref() {
        Some(record) => match serde_json::from_slice(&record.bytes) {
            Ok(marker) => Some(marker),
            Err(error) => { eprintln!("Kiwix: mobile root marker is unreadable: {error}"); notice(&mut notices, MARKER_UNREADABLE); None }
        },
        None => None,
    };
    #[cfg(unix)]
    if let Some(record) = &previous {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = record.file.metadata().map_err(err)?;
        if metadata.uid() != crate::fs_ext::effective_uid() || metadata.mode() & 0o077 != 0 {
            // Never derive a relocation root from a marker another principal
            // (or the group/world) could have written.
            eprintln!("Kiwix: mobile root marker is not private to this user; the saved root is not used");
            notice(&mut notices, MARKER_NOT_PRIVATE);
            recorded = None;
        }
    }
    if let Some(marker) = &recorded {
        let valid = marker.version == 1
            && normalized_absolute(&marker.root).is_ok()
            && Uuid::parse_str(&marker.owner).is_ok_and(|owner| owner.to_string() == marker.owner);
        if !valid {
            eprintln!("Kiwix: mobile root marker is structurally invalid; it will be replaced");
            notice(&mut notices, MARKER_INVALID);
            recorded = None;
        }
    }
    let capability_identity = capability.as_ref().map(|directory| directory.identity.clone());
    let bound = recorded.as_ref().is_some_and(|marker| {
        capability_identity.as_ref() == Some(&marker.capability) && marker.profile == profile.identity
    });
    if recorded.is_some() && !bound {
        // Restore, reinstall, device transfer, or adoptable-storage moves change
        // the profile directory identities while the marker travels with the
        // data. Keep the recorded root as a lexical relocation source only.
        eprintln!("Kiwix: mobile root marker is not bound to this profile; re-anchoring saved paths");
        notice(&mut notices, MARKER_NOT_BOUND);
    }
    let mut pending_unreadable = false;
    let pending_record = match capability.as_ref().map(|directory| directory.read("pending.json", 64 * 1024)).transpose() {
        Ok(record) => record.flatten(),
        Err(error) => {
            // A damaged intent authorizes nothing; it must not deny startup.
            eprintln!("Kiwix: mobile relocation intent is unreadable: {error}");
            notice(&mut notices, PENDING_IGNORED);
            pending_unreadable = true;
            None
        }
    };
    let mut targets = Vec::new();
    if let (Some(record), Some(marker)) = (&pending_record, &recorded) {
        // An intent that cannot be validated authorizes nothing. It is ignored
        // rather than denying startup; the marker's source root stays
        // authoritative on its own.
        let accepted = owned_file(&record.file, true).is_ok()
            && serde_json::from_slice::<Pending>(&record.bytes).is_ok_and(|pending| {
                pending.version == 1
                    && pending.owner == marker.owner
                    && pending.profile == marker.profile
                    && pending.capability == marker.capability
                    && normalized_absolute(&pending.source).is_ok()
                    // A completed migration leaves an inert intent. Only an
                    // intent bound to the still-authoritative source root can
                    // authorize intermediate roots.
                    && pending.source == marker.root
                    && pending.targets.iter().all(|root| normalized_absolute(root).is_ok())
            });
        if accepted {
            let pending: Pending = serde_json::from_slice(&record.bytes).map_err(err)?;
            targets = pending.targets;
        } else {
            eprintln!("Kiwix: mobile relocation intent did not validate; ignored");
            notice(&mut notices, PENDING_IGNORED);
        }
    }
    let overlaps = |root: &PathBuf| root != &current && (root.starts_with(&current) || current.starts_with(root));
    let mut source_root = recorded.as_ref().map(|marker| marker.root.clone());
    if source_root.as_ref().is_some_and(overlaps) {
        eprintln!("Kiwix: mobile root overlaps the current container; no relocation source is used");
        notice(&mut notices, ROOT_OVERLAP);
        source_root = None;
    }
    targets.retain(|root| !overlaps(root));
    let mut relocation = source_root
        .filter(|root| root != &current || targets.iter().any(|target| target != &current))
        .map(|old_root| Relocation {
            old_root, new_root: current.clone(),
            previous_targets: targets.clone(), torrent_receipts: HashMap::new(),
        });
    // Build the same exact receipt view for writable planning and readonly State
    // loading, before the latter returns without touching any persistence.
    let receipt_source = match relocation.as_mut() {
        Some(relocation) => match relocation.load_torrent_receipts(&profile) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("Kiwix: torrent receipts cannot be relocated: {error}");
                notice(&mut notices, RECEIPTS_SKIPPED);
                relocation.torrent_receipts.clear();
                None
            }
        },
        None => None,
    };
    if read_only || (bound && relocation.is_none()) {
        return Ok(Initialized { relocation, notices, durable: false });
    }
    // The intent file exists but its identity could not be verified: it cannot
    // be replaced, and the relocation it authorized cannot be inspected. Stop
    // before publishing anything and keep the old marker authoritative so a
    // later start retries instead of aborting this one.
    if pending_unreadable && relocation.is_some() {
        eprintln!("Kiwix: mobile relocation intent is present but unreadable; durable relocation stopped");
        notice(&mut notices, PENDING_IGNORED);
        return Ok(Initialized { relocation, notices, durable: false });
    }

    // Read/parse/transform every known record before publishing the first one.
    // Missing marker: the marker is created below without reading any record;
    // those managers own their files and validate them on admission. Only an
    // actual relocation parses the full profile-owned path inventory.
    let mut plans = Vec::new();
    let mut complete = true;
    if let Some(relocation) = &relocation {
        complete &= plan_record(&profile, "state.json", RecordKind::State, relocation, &mut plans, &mut notices);
        for (directory, filename, kind) in [
            ("downloads", "downloads.json", RecordKind::Downloads),
            ("torrents", "jobs.json", RecordKind::Torrents),
            ("localswarm", "jobs.json", RecordKind::LocalSwarm),
            ("maps", "map-job.json", RecordKind::Maps),
        ] {
            if let Some(directory) = profile.child(directory)? {
                complete &= plan_record(&directory, filename, kind, relocation, &mut plans, &mut notices);
            }
        }
        if let Some(directory) = profile.child("library-migrations-v1")? {
            let mut count = 0;
            for entry in directory.dir.entries().map_err(err)? {
                let entry = entry.map_err(err)?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    eprintln!("Kiwix: non-UTF-8 migration journal name; journal ignored");
                    notice(&mut notices, RELOCATION_SKIPPED);
                    continue;
                };
                let Some(id) = name.strip_suffix(".json") else { continue; };
                if !Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id) {
                    eprintln!("Kiwix: invalid migration journal name; journal ignored");
                    notice(&mut notices, RELOCATION_SKIPPED);
                    continue;
                }
                count += 1;
                if count > MAX_JOURNALS {
                    eprintln!("Kiwix: too many relocation migration journals; remaining journals ignored");
                    notice(&mut notices, RELOCATION_SKIPPED);
                    break;
                }
                complete &= plan_record(&directory, name, RecordKind::Migration, relocation, &mut plans, &mut notices);
            }
        }
    }
    for planned in &plans { planned.record.unchanged(&planned.directory)?; }
    if let Some((directory, record)) = &receipt_source { record.unchanged(directory)?; }
    if let (Some(relocation), Some(marker), Some(directory)) = (&relocation, &recorded, &capability) {
        if !targets.contains(&relocation.new_root) { targets.push(relocation.new_root.clone()); }
        let pending = Pending {
            version: 1, owner: marker.owner.clone(), source: marker.root.clone(),
            profile: marker.profile.clone(), capability: marker.capability.clone(), targets,
        };
        let bytes = serde_json::to_vec(&pending).map_err(err)?;
        if bytes.len() > 64 * 1024 { return Err("Mobile relocation intent exceeds its size bound".into()); }
        directory.publish("pending.json", &bytes, pending_record.as_ref())?;
    }
    for planned in &plans {
        if let Some(bytes) = &planned.bytes {
            planned.directory.publish(&planned.record.name, bytes, Some(&planned.record))?;
        } else {
            // A previous attempt may have renamed this already-rebased record
            // but failed its parent fsync. Replay must establish that durability
            // before advancing the marker, even when no path changes this time.
            planned.record.unchanged(&planned.directory)?;
            planned.record.file.sync_all().map_err(err)?;
            planned.directory.sync()?;
        }
    }
    if !complete {
        // A record that could not be planned still holds old-root paths. Keep
        // the old marker authoritative so the next start revisits it; the
        // rewritten records above replay idempotently.
        eprintln!("Kiwix: relocation is incomplete; the saved root marker is retained");
        return Ok(Initialized { relocation, notices, durable: false });
    }
    // Each record's parent was synced above. Commit the trust root LAST, so an
    // interruption keeps authorizing exactly the same idempotent old->new replay.
    let capability = match capability {
        Some(directory) => directory,
        None => {
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = cap_std::fs::DirBuilder::new();
            #[cfg(unix)] { use cap_std::fs::DirBuilderExt; builder.mode(0o700); }
            profile.unchanged()?;
            profile.dir.create_dir_with(CAPABILITY, &builder).map_err(err)?;
            profile.sync()?;
            profile.child(CAPABILITY)?.ok_or("Created mobile root capability disappeared")?
        }
    };
    private_directory(&capability)?;
    let marker = Marker {
        version: 1,
        owner: recorded.map(|marker| marker.owner).unwrap_or_else(|| Uuid::new_v4().to_string()),
        root: current,
        profile: profile.identity.clone(),
        capability: capability.identity.clone(),
    };
    capability.publish(MARKER, &serde_json::to_vec(&marker).map_err(err)?, previous.as_ref())?;
    profile.sync()?;
    Ok(Initialized { relocation, notices, durable: true })
}

/// A record that cannot be parsed or validated is left untouched for its own
/// manager; it must never deny the whole profile. The detailed error stays in
/// the local log, never in a user-visible notice. Returns whether the record
/// was planned, so the caller can retain the old marker while any record still
/// holds old-root paths.
fn plan_record(
    directory: &Directory,
    name: &str,
    kind: RecordKind,
    relocation: &Relocation,
    plans: &mut Vec<Planned>,
    notices: &mut Vec<String>,
) -> bool {
    match plan(directory, name, kind, Some(relocation), plans) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("Kiwix: saved {name} cannot be relocated and is left unchanged: {error}");
            notice(notices, RELOCATION_SKIPPED);
            false
        }
    }
}

fn plan(directory: &Directory, name: &str, kind: RecordKind, relocation: Option<&Relocation>, plans: &mut Vec<Planned>) -> Result<(), String> {
    let Some(record) = directory.read(name, MAX_RECORD)? else { return Ok(()); };
    let mut value = record.json()?;
    if matches!(kind, RecordKind::State) {
        // Validate exactly what the Store will accept, including its tolerant
        // per-record decoding. A state the Store would quarantine must not be
        // rewritten here.
        crate::store::state_from_value(value.clone())
            .map_err(|error| format!("Saved state cannot be relocated: {error}"))?;
    }
    let mut bytes = None;
    if let Some(relocation) = relocation {
        let changed = relocation.rebase_json(kind, &mut value)?;
        validate_paths(kind, &value, &relocation.new_root)?;
        validate_evidence(kind, &value, relocation)?;
        if changed { bytes = Some(serde_json::to_vec(&value).map_err(err)?); }
    }
    plans.push(Planned { directory: Directory::open(&directory.path)?, bytes, record });
    Ok(())
}

// Directory identities already used by destructive recovery are never refreshed
// from a new pathname. When present, the relocated directory must still identify
// the original capability. Missing entries remain missing for normal recovery.
fn validate_evidence(kind: RecordKind, value: &Value, relocation: &Relocation) -> Result<(), String> {
    match kind {
        RecordKind::Migration => {
            identity_at(value, "source_directory", "source_directory_identity", relocation)?;
            identity_at(value, "destination_directory", "destination_directory_identity", relocation)?;
        }
        RecordKind::LocalSwarm => {
            if let Some(rows) = value.as_object() {
                for row in rows.values() {
                    if let Some(owned) = row.get("owned") { identity_at(owned, "parent", "parent_identity", relocation)?; }
                }
            }
        }
        RecordKind::Maps => {
            if let Some(owned) = value.get("ownedPartial").filter(|v| !v.is_null()) { identity_at(owned, "destination", "directoryIdentity", relocation)?; }
            if let Some(published) = value.get("published").and_then(|value| value.get("record")) {
                identity_at(published, "destination", "directoryIdentity", relocation)?;
            }
            if let Some(rows) = value.get("retainedPublications").and_then(Value::as_array) {
                for saved in rows {
                    if let Some(record) = saved.get("published").and_then(|value| value.get("record")) {
                        identity_at(record, "destination", "directoryIdentity", relocation)?;
                    }
                }
            }
            if let Some(rows) = value.get("recoveryRecords").and_then(Value::as_array) {
                for owned in rows { identity_at(owned, "destination", "directoryIdentity", relocation)?; }
            }
        }
        _ => (),
    }
    Ok(())
}
fn identity_at(value: &Value, path_key: &str, identity_key: &str, relocation: &Relocation) -> Result<(), String> {
    let Some(path) = value.get(path_key).and_then(Value::as_str) else { return Ok(()); };
    let path = Path::new(path);
    if !path.starts_with(&relocation.new_root) { return Ok(()); }
    let expected: Identity = serde_json::from_value(value.get(identity_key).ok_or("Recovery directory has no identity evidence")?.clone()).map_err(err)?;
    // Walk no-follow and tolerate absent stale history, but never substitute an
    // unrelated live directory or invent replacement inode/device evidence.
    let mut directory = Directory::open(&relocation.new_root)?;
    for component in path.strip_prefix(&relocation.new_root).map_err(err)?.components() {
        let Component::Normal(name) = component else { return Err("Invalid recovery directory component".into()); };
        let name = name.to_str().ok_or("Non-UTF-8 recovery directory")?;
        let Some(child) = directory.child(name)? else { return Ok(()); };
        directory = child;
    }
    if directory.identity != expected { return Err(format!("Relocated recovery directory changed identity: {}", path.display())); }
    Ok(())
}

fn validate_paths(kind: RecordKind, value: &Value, root: &Path) -> Result<(), String> {
    fn field(value: &Value, key: &str, root: &Path) -> Result<(), String> {
        if let Some(path) = value.get(key).and_then(Value::as_str) { owned_path(Path::new(path), root)?; }
        Ok(())
    }
    fn array(value: &Value, key: &str, root: &Path) -> Result<(), String> {
        if let Some(paths) = value.get(key).and_then(Value::as_array) {
            for path in paths { if let Some(path) = path.as_str() { owned_path(Path::new(path), root)?; } }
        }
        Ok(())
    }
    fn job(value: &Value, root: &Path) -> Result<(), String> {
        field(value, "destination", root)?;
        array(value, "completedPaths", root)
    }
    match kind {
        RecordKind::State => {
            if let Some(books) = value.get("books").and_then(Value::as_array) {
                for book in books {
                    if book.get("managed").and_then(Value::as_bool) == Some(true) { field(book, "path", root)?; }
                }
            }
            for collection in ["editionIntents", "editionTransactions"] {
                if let Some(records) = value.get(collection).and_then(Value::as_array) {
                    for record in records {
                        if collection == "editionIntents" {
                            field(record, "destination", root)?;
                            field(record, "downloadDirectory", root)?;
                        }
                        for key in ["target", "verified", "original", "replacement", "rollbackBook"] {
                            if let Some(book) = record.get(key).filter(|book| !book.is_null()) {
                                if book.get("managed").and_then(Value::as_bool) == Some(true)
                                    && !book.get("sourceReference").is_some_and(|source| !source.is_null()) {
                                    field(book, "path", root)?;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(settings) = value.get("settings") {
                field(settings, "downloadDirectory", root)?;
                array(settings, "monitorDirectories", root)?;
            }
        }
        RecordKind::Downloads => {
            if let Some(rows) = value.as_object() { for row in rows.values() { field(row, "destination", root)?; } }
        }
        RecordKind::Torrents => {
            if let Some(rows) = value.get("jobs").and_then(Value::as_array) {
                for row in rows {
                    for key in ["staging", "directory", "metadata"] { field(row, key, root)?; }
                    if let Some(value) = row.get("job") { job(value, root)?; }
                }
            }
        }
        RecordKind::LocalSwarm => {
            if let Some(rows) = value.as_object() {
                for row in rows.values() {
                    if let Some(value) = row.get("job") { job(value, root)?; }
                    if let Some(owned) = row.get("owned") { field(owned, "parent", root)?; }
                }
            }
        }
        RecordKind::Maps => {
            field(value, "path", root)?;
            if let Some(owned) = value.get("ownedPartial") { field(owned, "destination", root)?; }
            if let Some(decision) = value.get("decision").filter(|value| !value.is_null()) {
                field(decision, "destination", root)?;
                if let Some(target) = decision.get("target").filter(|value| !value.is_null()
                    && value.get("managed").and_then(Value::as_bool) == Some(true)
                    && !value.get("sourceReference").is_some_and(|source| !source.is_null())) {
                    field(target, "path", root)?;
                }
            }
            if let Some(published) = value.get("published").and_then(|value| value.get("record")) {
                field(published, "destination", root)?;
            }
            if let Some(rows) = value.get("retainedPublications").and_then(Value::as_array) {
                for saved in rows {
                    if let Some(record) = saved.get("published").and_then(|value| value.get("record")) {
                        field(record, "destination", root)?;
                    }
                }
            }
            if let Some(rows) = value.get("recoveryRecords").and_then(Value::as_array) {
                for owned in rows { field(owned, "destination", root)?; }
            }
        }
        RecordKind::Migration => {
            field(value, "source_directory", root)?;
            field(value, "destination_directory", root)?;
        }
    }
    Ok(())
}

/// A rebased name must not traverse a symlink into an external provider's files.
/// Missing suffixes remain absent; they confer no ownership of future entries.
fn owned_path(path: &Path, root: &Path) -> Result<(), String> {
    let Ok(relative) = path.strip_prefix(root) else { return Ok(()); };
    normalized_absolute(path)?;
    let mut directory = Directory::open(root)?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else { return Err("Invalid relocated path component".into()); };
        let metadata = match directory.dir.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(err(error)),
            Ok(metadata) => metadata,
        };
        if metadata.is_symlink() { return Err(format!("Relocated path crosses a symlink: {}", path.display())); }
        if metadata.is_dir() {
            let name = name.to_str().ok_or("Non-UTF-8 relocated path component")?;
            directory = directory.child(name)?.ok_or("Relocated directory disappeared")?;
        } else {
            if components.peek().is_some() { return Err("Relocated path crosses a non-directory".into()); }
            regular(&metadata)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test paths are built per component (a '/' inside one `join` is not
    // normalized on Windows) and foreign roots must be absolute on the host.
    const OUTSIDE_PROFILE: &str = if cfg!(windows) { r"C:\outside-profile" } else { "/outside-profile" };

    fn profile_dir(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mobile-rebase-{tag}-{}", Uuid::new_v4()));
        crate::fs_ext::create_private_dir_all(&root).unwrap();
        std::fs::canonicalize(root).unwrap()
    }

    #[cfg(unix)]
    fn private(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    fn private(_path: &Path, _mode: u32) {}

    fn write_state(root: &Path, download: &Path, book_path: Option<&Path>) {
        let mut value = serde_json::to_value(crate::store::State::default()).unwrap();
        value["settings"]["downloadDirectory"] = serde_json::Value::String(download.to_string_lossy().into_owned());
        if let Some(path) = book_path {
            value["books"] = serde_json::json!([{
                "id": "archive", "title": "Archive", "description": "", "language": "en",
                "creator": "", "publisher": "", "date": "", "name": "archive",
                "tags": [], "size": 1, "articleCount": 0, "mediaCount": 0,
                "hasFulltextIndex": false, "hasTitleIndex": false, "mainPath": "Main_Page",
                "installationId": Uuid::new_v4().to_string(),
                "path": path.to_string_lossy(), "addedAt": "2026-01-01T00:00:00Z",
                "available": true, "managed": true
            }]);
        }
        std::fs::write(root.join("state.json"), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        private(&root.join("state.json"), 0o600);
    }

    fn write_marker(root: &Path, marker: &serde_json::Value) {
        let capability = root.join(CAPABILITY);
        std::fs::create_dir_all(&capability).unwrap();
        private(&capability, 0o700);
        let path = capability.join(MARKER);
        std::fs::write(&path, serde_json::to_vec(marker).unwrap()).unwrap();
        private(&path, 0o600);
    }

    /// A container that is no longer current and is not nested inside it.
    fn old_container(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mobile-old-container-{tag}-{}", Uuid::new_v4()))
    }

    fn identities(root: &Path) -> (Identity, Identity) {
        let profile = Directory::open(root).unwrap();
        let capability = profile.child(CAPABILITY).unwrap().unwrap();
        (profile.identity, capability.identity)
    }

    /// A marker that travelled with a restored container is not bound to the
    /// new profile identities. Startup must succeed and re-anchor saved paths
    /// instead of rejecting the profile.
    #[test]
    fn unbound_marker_reanchors_saved_library_paths() {
        let root = profile_dir("unbound-reanchor");
        let old = old_container("unbound");
        let archive = root.join("archives").join("book.zim");
        crate::fs_ext::create_private_dir_all(archive.parent().unwrap()).unwrap();
        std::fs::write(&archive, b"archive").unwrap();
        write_state(&root, &root.join("archives"), Some(&old.join("archives").join("book.zim")));
        let stale = serde_json::json!({
            "version": 1, "owner": Uuid::new_v4().to_string(), "root": old,
            "profile": {"device": 1, "inode": 2}, "capability": {"device": 1, "inode": 3}
        });
        write_marker(&root, &stale);

        let initialized = initialize(&root, false).unwrap();
        assert!(initialized.notices.iter().any(|notice| notice == MARKER_NOT_BOUND));
        let relocation = initialized.relocation.expect("stale container re-anchors saved paths");
        assert_eq!(relocation.old_root, old);
        assert_eq!(relocation.new_root, root);

        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["books"][0]["path"], serde_json::json!(archive.to_string_lossy()));
        assert_eq!(
            state["settings"]["downloadDirectory"],
            serde_json::json!(root.join("archives").to_string_lossy())
        );
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(CAPABILITY).join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["root"], serde_json::json!(root.to_string_lossy()));
        let (profile, capability) = identities(&root);
        assert_eq!(
            serde_json::from_value::<Identity>(marker["profile"].clone()).unwrap(),
            profile
        );
        assert_eq!(
            serde_json::from_value::<Identity>(marker["capability"].clone()).unwrap(),
            capability
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Wrong identities at the same root mean no relocation is needed; the
    /// marker is rebound without touching state.
    #[test]
    fn unbound_marker_at_current_root_is_rebound_without_error() {
        let root = profile_dir("unbound-same-root");
        write_state(&root, &root.join("archives"), None);
        let before = std::fs::read(root.join("state.json")).unwrap();
        let stale = serde_json::json!({
            "version": 1, "owner": Uuid::new_v4().to_string(), "root": root,
            "profile": {"device": 9, "inode": 9}, "capability": {"device": 9, "inode": 9}
        });
        write_marker(&root, &stale);
        let initialized = initialize(&root, false).unwrap();
        assert!(initialized.relocation.is_none());
        assert!(initialized.notices.iter().any(|notice| notice == MARKER_NOT_BOUND));
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), before);
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(CAPABILITY).join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["root"], serde_json::json!(root.to_string_lossy()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// With no marker there is no relocation, so the managers stay the only
    /// readers of their records; corrupt files cannot deny startup.
    #[test]
    fn missing_marker_ignores_corrupt_auxiliary_records() {
        let root = profile_dir("missing-marker");
        write_state(&root, &root.join("downloads"), None);
        for (directory, name) in [
            ("downloads", "downloads.json"),
            ("torrents", "jobs.json"),
            ("localswarm", "jobs.json"),
            ("maps", "map-job.json"),
        ] {
            crate::fs_ext::create_private_dir_all(root.join(directory)).unwrap();
            std::fs::write(root.join(directory).join(name), b"{not json").unwrap();
        }
        let initialized = initialize(&root, false).unwrap();
        assert!(initialized.relocation.is_none());
        assert!(initialized.notices.is_empty());
        assert!(root.join(CAPABILITY).join(MARKER).is_file());
        assert_eq!(std::fs::read(root.join("downloads/downloads.json")).unwrap(), b"{not json");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Bound markers keep the original exact old-root -> new-root rewrite.
    #[test]
    fn bound_marker_rewrites_prefixes_exactly() {
        let root = profile_dir("bound");
        let old = old_container("bound");
        write_state(&root, &root.join("archives"), Some(&old.join("archives").join("book.zim")));
        crate::fs_ext::create_private_dir_all(root.join("archives")).unwrap();
        std::fs::write(root.join("archives").join("book.zim"), b"archive").unwrap();
        std::fs::create_dir_all(root.join(CAPABILITY)).unwrap();
        private(&root.join(CAPABILITY), 0o700);
        let (profile, capability) = identities(&root);
        let marker = serde_json::json!({
            "version": 1, "owner": Uuid::new_v4().to_string(), "root": old,
            "profile": profile, "capability": capability
        });
        let path = root.join(CAPABILITY).join(MARKER);
        std::fs::write(&path, serde_json::to_vec(&marker).unwrap()).unwrap();
        private(&path, 0o600);

        let initialized = initialize(&root, false).unwrap();
        assert!(initialized.notices.is_empty());
        let relocation = initialized.relocation.expect("bound marker relocates");
        assert_eq!(relocation.old_root, old);
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("state.json")).unwrap()).unwrap();
        assert_eq!(
            state["books"][0]["path"],
            serde_json::json!(root.join("archives").join("book.zim").to_string_lossy())
        );
        // The second pass is idempotent and leaves the bytes identical.
        let after = std::fs::read(root.join("state.json")).unwrap();
        assert!(initialize(&root, false).unwrap().relocation.is_none());
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), after);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn retained_map_relocation_preserves_directory_authority_and_rejects_substitution() {
        let root = std::env::temp_dir().join(format!("map-retained-rebase-{}", Uuid::new_v4()));
        crate::fs_ext::create_private_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let destination = root.join("archives");
        crate::fs_ext::create_private_dir_all(&destination).unwrap();
        let identity = Directory::open(&destination).unwrap().identity;
        let relocation = Relocation {
            old_root: root.join("old-container"), new_root: root.clone(),
            previous_targets: Vec::new(), torrent_receipts: HashMap::new(),
        };
        let mut value = serde_json::json!({"retainedPublications": [{
            "published": {"record": {
                "destination": destination,
                "directoryIdentity": identity,
                "name": "owned.zim",
                "identity": {"device": 1, "inode": 2}
            }, "size": 17, "sha256": [1, 2, 3]}
        }]});
        validate_paths(RecordKind::Maps, &value, &root).unwrap();
        validate_evidence(RecordKind::Maps, &value, &relocation).unwrap();
        value["retainedPublications"][0]["published"]["record"]["directoryIdentity"]["inode"] = 0.into();
        assert!(validate_evidence(RecordKind::Maps, &value, &relocation).is_err());
        value["retainedPublications"][0]["published"]["record"]["destination"] = OUTSIDE_PROFILE.into();
        // A foreign destination is left foreign by the schema inventory; it is
        // never rewritten into the profile. The identity evidence above is what
        // prevents substituting an unrelated live directory.
        validate_paths(RecordKind::Maps, &value, &root).unwrap();
        let mut rebased = value.clone();
        assert!(!relocation.rebase_json(RecordKind::Maps, &mut rebased).unwrap());
        assert_eq!(rebased["retainedPublications"][0]["published"]["record"]["destination"], OUTSIDE_PROFILE);
        std::fs::remove_dir(&destination).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn read_only_unreadable_marker_preserves_every_profile_entry() {
        let root = profile_dir("readonly-unreadable");
        write_state(&root, &root.join("archives"), None);
        write_marker(&root, &serde_json::json!({"padding": "x".repeat(5000)}));
        let marker = root.join(CAPABILITY).join(MARKER);
        let before_marker = std::fs::read(&marker).unwrap();
        let before_state = std::fs::read(root.join("state.json")).unwrap();
        let before_entries: std::collections::HashSet<_> = std::fs::read_dir(root.join(CAPABILITY))
            .unwrap().map(|entry| entry.unwrap().file_name()).collect();
        let initialized = initialize(&root, true).unwrap();
        assert!(initialized.relocation.is_none());
        assert!(!initialized.notices.is_empty());
        assert_eq!(std::fs::read(&marker).unwrap(), before_marker);
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), before_state);
        let after_entries: std::collections::HashSet<_> = std::fs::read_dir(root.join(CAPABILITY))
            .unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(after_entries, before_entries);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A marker bound to this profile's identities that records `old` as the
    /// authoritative source root.
    fn write_bound_marker(root: &Path, old: &Path) {
        std::fs::create_dir_all(root.join(CAPABILITY)).unwrap();
        private(&root.join(CAPABILITY), 0o700);
        let (profile, capability) = identities(root);
        let marker = serde_json::json!({
            "version": 1, "owner": Uuid::new_v4().to_string(), "root": old,
            "profile": profile, "capability": capability
        });
        let path = root.join(CAPABILITY).join(MARKER);
        std::fs::write(&path, serde_json::to_vec(&marker).unwrap()).unwrap();
        private(&path, 0o600);
    }

    /// The Store accepts relative book paths and skips malformed receipt
    /// entries; relocation must not reject a state the Store loads.
    #[test]
    fn relative_paths_and_malformed_receipts_are_skipped_not_rejected() {
        let root = profile_dir("skip-malformed");
        let relocation = Relocation {
            old_root: old_container("skip-malformed"), new_root: root.clone(),
            previous_targets: Vec::new(),
            torrent_receipts: HashMap::from([("old-receipt".to_string(), "new-receipt".to_string())]),
        };
        let mut value = serde_json::json!({
            "consumedAcquisitions": ["old-receipt", 7],
            "settings": {"downloadDirectory": "relative/downloads"},
        });
        assert!(relocation.rebase_json(RecordKind::State, &mut value).unwrap());
        assert_eq!(value["consumedAcquisitions"], serde_json::json!([7, "new-receipt"]));
        assert_eq!(value["settings"]["downloadDirectory"], "relative/downloads");
        // A non-array collection is skipped exactly as the Store skips it.
        let mut malformed = serde_json::json!({"consumedAcquisitions": "not-an-array"});
        assert!(!relocation.rebase_json(RecordKind::State, &mut malformed).unwrap());
        assert_eq!(malformed["consumedAcquisitions"], "not-an-array");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A present but unreadable relocation intent cannot be replaced, so the
    /// durable relocation stops before publishing and the old marker stays
    /// authoritative for the next start.
    #[test]
    fn unreadable_pending_intent_stops_durable_relocation() {
        let root = profile_dir("unreadable-pending");
        let old = old_container("unreadable-pending");
        write_state(&root, &root.join("archives"), Some(&old.join("archives").join("book.zim")));
        crate::fs_ext::create_private_dir_all(root.join("archives")).unwrap();
        std::fs::write(root.join("archives").join("book.zim"), b"archive").unwrap();
        write_bound_marker(&root, &old);
        let pending = root.join(CAPABILITY).join("pending.json");
        std::fs::write(&pending, vec![b'x'; 64 * 1024 + 1]).unwrap();
        let before_state = std::fs::read(root.join("state.json")).unwrap();
        let before_marker = std::fs::read(root.join(CAPABILITY).join(MARKER)).unwrap();
        let before_pending = std::fs::read(&pending).unwrap();

        let initialized = initialize(&root, false).unwrap();
        assert!(initialized.relocation.is_some());
        assert!(initialized.notices.iter().any(|notice| notice == PENDING_IGNORED));
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), before_state);
        assert_eq!(std::fs::read(root.join(CAPABILITY).join(MARKER)).unwrap(), before_marker);
        assert_eq!(std::fs::read(&pending).unwrap(), before_pending);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// One record that cannot be planned leaves old-root paths behind, so the
    /// old marker is retained and the next start revisits the record.
    #[test]
    #[cfg_attr(windows, ignore = "the second initialize fails with os error 5 on Windows; mobile_rebase only runs on mobile (cfg(any(mobile, test)))")]
    fn unplannable_record_retains_the_old_marker() {
        let root = profile_dir("incomplete");
        let old = old_container("incomplete");
        write_state(&root, &root.join("archives"), Some(&old.join("archives").join("book.zim")));
        crate::fs_ext::create_private_dir_all(root.join("archives")).unwrap();
        std::fs::write(root.join("archives").join("book.zim"), b"archive").unwrap();
        crate::fs_ext::create_private_dir_all(root.join("downloads")).unwrap();
        std::fs::write(root.join("downloads/downloads.json"), b"{not json").unwrap();
        write_bound_marker(&root, &old);

        let initialized = initialize(&root, false).unwrap();
        assert!(initialized.relocation.is_some());
        assert!(initialized.notices.iter().any(|notice| notice == RELOCATION_SKIPPED));
        // The plannable state was rewritten, but the marker still records the
        // old root so the next start replays and retries the unplanned record.
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["books"][0]["path"], serde_json::json!(root.join("archives").join("book.zim").to_string_lossy()));
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(CAPABILITY).join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["root"], serde_json::json!(old.to_string_lossy()));
        assert!(initialize(&root, false).unwrap().relocation.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
