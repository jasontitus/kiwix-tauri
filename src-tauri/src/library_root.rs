// SPDX-License-Identifier: GPL-3.0-or-later
//! App-owned library locations and verified copy migration.
//!
//! Archives are still served from paths this application owns. On Android the
//! removable-volume choice therefore points at this app's own directory on that
//! volume (`Android/data/<package>/files/archives`), never at a raw SD-card path.
//!
//! A switch copies every managed, available archive to a private staging
//! directory on the destination volume, verifies the staged archive with the
//! existing libzim integrity gate, publishes the exact part set without
//! replacing an existing name, and only then commits the library paths and the
//! selected root and edition rollback location expectations in one atomic Store
//! update. Relocation preserves the reader principal and retained original.
//! The previous root stays intact until that commit. Old copies are removed only
//! when the caller asks and the destination still matches its recorded SHA-256.
//!
//! The journal lives in the profile, not on the removable volume. An
//! interrupted copy/publish is rolled back on the next writable start while the
//! old root is still active; an interrupted cleanup is finished when the store
//! already points at the new root. No journal is written in private or
//! read-only mode.

use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;

/// Marker proving that an external directory was adopted as a library root.
/// Raw external paths are never accepted just because they exist.
pub const MARKER: &str = ".kiwix-library-root.json";
const JOURNAL_DIRECTORY: &str = "library-root-v1";
const JOURNAL_FILE: &str = "migration.json";
const JOURNAL_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 4096;
const COPY_BUFFER: usize = 1024 * 1024;
/// Empty stage identities are durable before copying any payload. Batch the
/// later digest checkpoints; the complete record is durable before publishing.
const JOURNAL_ENTRY_BATCH: usize = 8;
static OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeLocation {
    pub path: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub removable: bool,
    #[serde(default)]
    pub label: String,
}

/// Non-terminal transfer/map work that still owns files below the current
/// library root. The switch refuses rather than leaving a resumable job pointed
/// at a root the store no longer uses.
#[derive(Clone, Debug, Default)]
pub struct Activity {
    pub transfers: Vec<TransferTarget>,
    pub map_busy: bool,
}

#[derive(Clone, Debug)]
pub struct TransferTarget {
    pub destination: String,
    pub status: String,
}

pub const ACTIVE_TRANSFERS: &str =
    "Finish or cancel your downloads, then switch storage";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub path: String,
    pub label: String,
    pub kind: String,
    pub removable: bool,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub active: bool,
    pub available: bool,
}

#[derive(Clone, Debug)]
struct Root {
    path: PathBuf,
    label: String,
    kind: String,
    removable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    id: String,
    profile: String,
    old_root: String,
    new_root: String,
    remove_old: bool,
    // Version-one journals can predate marker identity tracking. Their boolean
    // is never authority to unlink an external directory entry during recovery.
    #[serde(default, rename = "markerCreated", skip_serializing)]
    _legacy_marker_created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    marker_directory: Option<FileIdentity>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    staging_directories: std::collections::BTreeMap<String, FileIdentity>,
    staging: String,
    phase: Phase,
    entries: Vec<JournalEntry>,
    #[serde(default)]
    selection: Option<Vec<SelectedSource>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectedSource {
    book: crate::store::Book,
    delete_receipt: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Copying,
    Publishing,
    Committed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalEntry {
    book_id: String,
    title: String,
    primary: String,
    relative: Vec<String>,
    parts: Vec<JournalPart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalPart {
    name: String,
    size: u64,
    sha256: Option<String>,
    #[serde(default)]
    staging_identity: Option<FileIdentity>,
    #[serde(default)]
    source_identity: Option<FileIdentity>,
    #[serde(default)]
    publication_identity: Option<FileIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

fn file_identity(path: &Path) -> Result<FileIdentity, String> {
    use cap_fs_ext::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Archive ownership requires a regular file".into());
    }
    let file = crate::acquisition::open_history(path)?.ok_or("Archive disappeared")?;
    let metadata = cap_std::fs::Metadata::from_file(&file).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err("Archive ownership requires a regular file without hard links".into());
    }
    Ok(FileIdentity { device: metadata.dev(), inode: metadata.ino() })
}

fn owned_part(path: &Path, identity: &Option<FileIdentity>) -> bool {
    identity.as_ref().is_some_and(|expected| file_identity(path).as_ref() == Ok(expected))
}

/// Full identity of a file this process copied: device, inode, size and mtime.
/// Renames preserve it, so a later match proves the file still holds the bytes
/// that were hashed during copying and passed libzim's integrity gate, without
/// reading those bytes again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified_ns: u128,
}

fn file_stamp(path: &Path) -> Result<FileStamp, String> {
    use cap_fs_ext::MetadataExt;
    let file = crate::acquisition::open_history(path)?.ok_or("Archive disappeared")?;
    let metadata = cap_std::fs::Metadata::from_file(&file).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err("Archive ownership requires a regular file without hard links".into());
    }
    let modified_ns = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .into_std()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(FileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified_ns,
    })
}

#[derive(Clone, Debug)]
struct PlannedEntry {
    book_id: String,
    title: String,
    primary: String,
    relative: Vec<String>,
    parts: Vec<PlannedPart>,
}

#[derive(Clone, Debug)]
struct PlannedPart {
    name: String,
    size: u64,
}

/// Validate the native enumeration before any path becomes a candidate.
#[cfg(mobile)]
pub fn parse_native(response: &Value) -> Result<Vec<NativeLocation>, String> {
    const MAX_LOCATIONS: usize = 16;
    let locations = response
        .get("locations")
        .and_then(Value::as_array)
        .ok_or("Native storage enumeration returned no locations")?;
    if locations.len() > MAX_LOCATIONS {
        return Err("Native storage enumeration returned too many locations".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(locations.len());
    for value in locations {
        let location: NativeLocation =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        let path = Path::new(&location.path);
        if !normalized_absolute(path) {
            return Err("Native storage location is not an absolute normalized path".into());
        }
        if !matches!(location.kind.as_str(), "internal" | "external") {
            return Err("Native storage location has an unknown kind".into());
        }
        if !seen.insert(location.path.clone()) {
            continue;
        }
        result.push(location);
    }
    Ok(result)
}

/// Whether `path` belongs to this app's library storage: the profile itself or
/// an external root carrying this profile's marker. Used by the archive open
/// gate; it never follows an unmarked external path.
pub fn is_managed_path(store: &Store, path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path.starts_with(store.data_dir()) {
        return true;
    }
    let Ok(profile) = profile_identity(store) else {
        return false;
    };
    for ancestor in path.ancestors() {
        // A file path is its own first ancestor; only directories can carry
        // the marker, and probing `<file>/.kiwix...` yields ENOTDIR rather than
        // NotFound on Unix.
        if !ancestor.is_dir() {
            continue;
        }
        let Ok(directory) = crate::storage::open_directory_nofollow(ancestor) else { return false };
        match read_marker(&directory, &profile) {
            Ok(Some(matches)) => return matches,
            Ok(None) => {},
            Err(_) => return false,
        }
    }
    false
}

/// Enumerate the locations the user may choose. The internal app directory is
/// always present; each native external app directory becomes a candidate even
/// before its `archives` child exists. The active root is included even when its
/// volume is currently unavailable, so the summary never silently pretends the
/// library moved back to internal storage.
pub fn locations(store: &Store, native: &[NativeLocation]) -> Result<Value, String> {
    let roots = allowed_roots(store, native)?;
    let active = configured_root(store)?;
    let mut result = Vec::with_capacity(roots.len() + 1);
    for root in roots {
        result.push(location_for(root, &active));
    }
    if !result
        .iter()
        .any(|location| Path::new(&location.path) == active)
    {
        result.push(Location {
            path: active.to_string_lossy().into_owned(),
            label: "Selected storage".into(),
            kind: "custom".into(),
            removable: false,
            free_bytes: crate::storage::volume_space(&active).map(|(free, _)| free),
            total_bytes: crate::storage::volume_space(&active).map(|(_, total)| total),
            active: true,
            available: active.is_dir(),
        });
    }
    Ok(json!({ "locations": result }))
}

/// Copy and verify the library into `target`, then commit the new root. The
/// caller must have obtained explicit confirmation. `remove_old` is applied
/// only after the destination is verified and the Store commit succeeded.
pub fn switch(
    store: &Store,
    target: &Path,
    remove_old: bool,
    native: &[NativeLocation],
    activity: &Activity,
) -> Result<Value, String> {
    let _operation = OPERATIONS
        .lock()
        .map_err(|_| "Library storage operation lock unavailable")?;
    if store.read_only()? {
        return Err("Read-only mode cannot change the library location".into());
    }
    if store.private_mode()? {
        return Err("Private mode cannot copy or move the library".into());
    }
    if has_pending_operation(store)? {
        return Err(
            "A previous storage move is still pending. Restart Kiwix to finish it before changing the location again."
                .into(),
        );
    }
    if !normalized_absolute(target) {
        return Err("Library location must be an absolute normalized path".into());
    }
    let roots = allowed_roots(store, native)?;
    if !roots.iter().any(|root| root.path == target) {
        return Err("Choose one of the storage locations this app exposes".into());
    }
    let old_root = configured_root(store)?;
    if !normalized_absolute(&old_root) {
        return Err("The current library location is not an absolute normalized path".into());
    }
    if old_root == target {
        return Ok(json!({
            "copied": 0, "parts": 0, "bytes": 0,
            "from": old_root.to_string_lossy(), "to": target.to_string_lossy(),
            "removedOld": false, "keptOld": true, "skipped": 0,
        }));
    }
    if target.starts_with(&old_root) || old_root.starts_with(target) {
        return Err("Library locations cannot contain one another".into());
    }
    if activity_blocks(activity, &old_root) {
        return Err(ACTIVE_TRANSFERS.into());
    }
    let planned = plan_entries(store, &old_root)?;
    let bytes = planned
        .iter()
        .flat_map(|entry| entry.parts.iter())
        .try_fold(0u64, |total, part| total.checked_add(part.size))
        .ok_or("Library size overflow")?;
    if let Some((free, _)) = crate::storage::volume_space(target) {
        ensure_space(Some(free), bytes)?;
    }
    let marker_directory = Some(prepare_target(store, target)?);
    let id = Uuid::new_v4().to_string();
    let staging_name = format!(".kiwix-library-root-{id}");
    let mut journal = Journal {
        version: JOURNAL_VERSION,
        id,
        profile: profile_identity(store)?,
        old_root: old_root.to_string_lossy().into_owned(),
        new_root: target.to_string_lossy().into_owned(),
        remove_old,
        _legacy_marker_created: false, marker_directory, staging_directories: Default::default(),
        staging: staging_name,
        phase: Phase::Copying,
        selection: None,
        entries: planned
            .iter()
            .map(|entry| JournalEntry {
                book_id: entry.book_id.clone(),
                title: entry.title.clone(),
                primary: entry.primary.clone(),
                relative: entry.relative.clone(),
                parts: entry
                    .parts
                    .iter()
                    .map(|part| JournalPart {
                        name: part.name.clone(),
                        size: part.size,
                        sha256: None,
                        staging_identity: None, source_identity: None,
                        publication_identity: None,
                    })
                    .collect(),
            })
            .collect(),
    };
    write_journal(store, &journal)?;
    let result = copy_and_commit(store, target, &old_root, &planned, &mut journal, remove_old);
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            // Store may adopt the new state before reporting a directory-sync
            // error. An unreadable or committed state never authorizes rollback.
            if !matches!(configured_root(store), Ok(current) if current == old_root)
                || !matches!(root_can_rollback(store, &journal), Ok(true)) {
                return Err(format!("{error}. Copies and the recovery journal were preserved; restart Kiwix to finish the storage move."));
            }
            if let Err(rollback) = rollback(store, &journal) {
                return Err(format!("{error}. Partial copies were preserved for recovery: {rollback}"));
            }
            Err(error)
        }
    }
}

/// Copy or move only these installations, leaving acquisition storage unchanged.
#[cfg(any(target_os = "android", test))]
pub fn relocate_selected(
    store: &Store, target: &Path, book_ids: &[String], remove_old: bool,
    native: &[NativeLocation], activity: &Activity,
) -> Result<Value, String> {
    let _operation = OPERATIONS.lock().map_err(|_| "Library storage operation lock unavailable")?;
    if store.read_only()? || store.private_mode()? {
        return Err("Private or read-only mode cannot copy or move archives".into());
    }
    if has_pending_operation(store)? {
        return Err("A storage operation is pending. Restart Kiwix to finish recovery first".into());
    }
    if !normalized_absolute(target) || !allowed_roots(store, native)?.iter().any(|root| root.path == target) {
        return Err("Choose one of the storage locations this app exposes".into());
    }
    if book_ids.is_empty() || book_ids.len() > MAX_ENTRIES {
        return Err("Select at least one archive to copy or move".into());
    }
    let state = store.read()?;
    let mut seen = std::collections::HashSet::new();
    let mut selection = Vec::new();
    let mut planned = Vec::new();
    for id in book_ids {
        if !seen.insert(id) { return Err("An archive was selected more than once".into()); }
        let book = state.books.iter().find(|book| &book.metadata.id == id).ok_or("Selected archive is no longer in the library")?;
        if !book.available { return Err("Reconnect the selected archive before copying it".into()); }
        if book.source_reference.is_none() {
            if !book.managed || !is_managed_path(store, Path::new(&book.path)) {
                return Err("Only app-managed archives or native provider selections can be copied or moved".into());
            }
            let parent = Path::new(&book.path).parent().ok_or("Selected archive has no parent directory")?;
            if parent == target || parent == target.join(&book.installation_id).as_path() {
                return Err("A selected archive is already at that location".into());
            }
            if activity_blocks(activity, parent) { return Err(ACTIVE_TRANSFERS.into()); }
        }
        let retained = retain_relocation_source(book)?;
        let archive = retained.open()?;
        if archive.metadata()?.id != *id || !archive.check_integrity()? {
            return Err("A selected archive failed verification; no files were moved".into());
        }
        let mut parts = Vec::with_capacity(retained.files().len());
        for (name, file) in retained.files() {
            validate_part_name(name)?;
            let size = file.metadata().map_err(|error| error.to_string())?.len();
            if size == 0 { return Err("Archive parts must not be empty".into()); }
            parts.push(PlannedPart { name: name.clone(), size });
        }
        let primary = parts.first().ok_or("Selected archive has no parts")?.name.clone();
        // Installation subdirectories avoid display-name collisions without
        // renaming split parts or changing the installation's reader principal.
        let relative = vec![book.installation_id.clone()];
        validate_part_name(&book.installation_id)?;
        planned.push(PlannedEntry { book_id: id.clone(), title: book.metadata.title.clone(), primary, relative, parts });
        let delete_receipt = if remove_old { book.source_reference.as_ref().map(prepare_provider_move).transpose()? } else { None };
        selection.push(SelectedSource { book: book.clone(), delete_receipt });
    }
    let bytes = planned.iter().flat_map(|entry| &entry.parts).try_fold(0u64, |sum, part| sum.checked_add(part.size)).ok_or("Archive size overflow")?;
    ensure_space(crate::storage::volume_space(target).map(|space| space.0), bytes)?;
    let marker_directory = Some(prepare_target(store, target)?);
    let id = Uuid::new_v4().to_string();
    let old_root = configured_root(store)?;
    let mut journal = Journal {
        version: JOURNAL_VERSION, staging: format!(".kiwix-library-root-{id}"), id,
        profile: profile_identity(store)?, old_root: old_root.to_string_lossy().into_owned(),
        new_root: target.to_string_lossy().into_owned(), remove_old, _legacy_marker_created: false, marker_directory, staging_directories: Default::default(), phase: Phase::Copying,
        selection: Some(selection),
        entries: planned.iter().map(|entry| JournalEntry {
            book_id: entry.book_id.clone(), title: entry.title.clone(), primary: entry.primary.clone(), relative: entry.relative.clone(),
            parts: entry.parts.iter().map(|part| JournalPart {
                name: part.name.clone(), size: part.size, sha256: None, staging_identity: None, source_identity: None, publication_identity: None,
            }).collect(),
        }).collect(),
    };
    write_journal(store, &journal)?;
    match copy_and_commit(store, target, &old_root, &planned, &mut journal, remove_old) {
        Ok(value) => Ok(value),
        Err(error) => {
            if selection_state(store, &journal)? != Some(false) {
                return Err(format!("{error}. Copies and the recovery journal were preserved; restart Kiwix to finish recovery"));
            }
            rollback(store, &journal)?;
            Err(error)
        }
    }
}

fn selection_state(store: &Store, journal: &Journal) -> Result<Option<bool>, String> {
    let selection = journal.selection.as_ref().ok_or("Storage selection is missing")?;
    let state = store.read()?;
    let mut original = true;
    let mut committed = true;
    for source in selection {
        let book = state.books.iter().find(|book| book.metadata.id == source.book.metadata.id);
        original &= book.is_some_and(|book| book.installation_id == source.book.installation_id
            && book.path == source.book.path && book.source_reference == source.book.source_reference);
        let entry = journal.entries.iter().find(|entry| entry.book_id == source.book.metadata.id).ok_or("Storage selection entry is missing")?;
        let destination = Path::new(&journal.new_root).join(relative_path(&entry.relative)).join(&entry.primary);
        committed &= book.is_some_and(|book| book.installation_id == source.book.installation_id
            && Path::new(&book.path) == destination && book.managed && book.source_reference.is_none());
    }
    Ok(if committed { Some(true) } else if original { Some(false) } else { None })
}

fn root_can_rollback(store: &Store, journal: &Journal) -> Result<bool, String> {
    if journal.phase == Phase::Committed { return Ok(false); }
    let books = store.books()?;
    let installed: std::collections::HashSet<_> = books.iter().map(|book| Path::new(&book.path)).collect();
    Ok(journal.entries.iter().all(|entry| {
        let new = Path::new(&journal.new_root).join(relative_path(&entry.relative));
        !entry.parts.iter().any(|part| installed.contains(new.join(&part.name).as_path()))
    }))
}

fn retain_relocation_source(book: &crate::store::Book) -> Result<crate::archive_source::RetainedSource, String> {
    if book.source_reference.is_some() {
        crate::archive_source::retain(book)
    } else {
        crate::library_files::retain_archive_files(book).map(crate::archive_source::RetainedSource::Filesystem)
    }
}

#[cfg(any(target_os = "android", test))]
fn prepare_provider_move(reference: &crate::provider_reference::ProviderReference) -> Result<String, String> {
    #[cfg(target_os = "android")]
    return crate::provider_reference::prepare_move_registered(reference);
    #[cfg(not(target_os = "android"))]
    { let _ = reference; Err("This platform cannot delete files from that document provider. Choose Copy instead".into()) }
}

fn delete_provider_original(source: &SelectedSource, entry: &JournalEntry) -> Result<(), String> {
    let reference = source.book.source_reference.as_ref().ok_or("Provider source missing")?;
    let receipt = source.delete_receipt.as_deref().ok_or("Kiwix could not confirm permission to delete the original, so it was kept")?;
    let parts = entry.parts.iter().map(|part| {
        let sha256 = part.sha256.as_deref().filter(|digest| digest.len() == 64
            && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
            .ok_or("Copied provider digest is missing; original and journal were preserved")?;
        Ok(json!({"name": part.name, "size": part.size, "sha256": sha256}))
    }).collect::<Result<Vec<_>, String>>()?;
    #[cfg(target_os = "android")]
    return crate::provider_reference::delete_move_registered(reference, receipt, &parts);
    #[cfg(not(target_os = "android"))]
    { let _ = (reference, receipt, parts); Err("Provider deletion is unavailable on this platform; original and journal were preserved".into()) }
}

/// Startup recovery. Never writes in private or read-only mode.
pub fn recover(store: &Store) -> Result<Option<String>, String> {
    let _operation = OPERATIONS
        .lock()
        .map_err(|_| "Library storage operation lock unavailable")?;
    if store.read_only()? || store.private_mode()? {
        return Ok(None);
    }
    if !has_pending_operation(store)? {
        return Ok(None);
    }
    let journal = match read_journal(store).and_then(|journal| {
        validate_journal(store, &journal)?;
        Ok(journal)
    }) {
        Ok(journal) => journal,
        Err(error) => {
            // An unreadable journal must not block storage operations until a
            // human deletes it. Move its exact bytes aside instead of
            // discarding them, then continue as if no recovery were pending.
            let kept = quarantine_journal(store).map_err(|quarantine| {
                format!("{error}; the unreadable journal was left in place because it could not be set aside: {quarantine}")
            })?;
            let name = kept
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| kept.to_string_lossy().into_owned());
            return Ok(Some(format!(
                "An unreadable storage recovery journal was set aside as {name}: {error}"
            )));
        }
    };
    if journal.selection.is_some() {
        return match selection_state(store, &journal)? {
            Some(false) => { rollback(store, &journal)?; Ok(Some("An interrupted selected-file copy was rolled back; originals were kept.".into())) }
            Some(true) => { finish_cleanup(store, &journal, None)?; Ok(Some("An interrupted selected-file operation was completed.".into())) }
            None => Err("Selected archives changed during recovery; all copies and the journal were preserved".into()),
        };
    }
    let current = configured_root(store)?;
    let old = PathBuf::from(&journal.old_root);
    let new = PathBuf::from(&journal.new_root);
    if current == old {
        if !root_can_rollback(store, &journal)? {
            // Older versions allowed changing the preferred folder while a
            // committed move still needed cleanup. Keep the old copies too
            // if the user has since selected that folder again. Completion
            // still requires every registered destination and file to verify.
            let mut keep_originals = journal;
            keep_originals.remove_old = false;
            finish_cleanup(store, &keep_originals, None)?;
            return Ok(Some("A previous storage move was verified. The original copies and your current download folder were kept.".into()));
        }
        rollback(store, &journal)?;
        return Ok(Some(
            "A previous storage move was rolled back. Archives remain in the previous location.".into(),
        ));
    }
    if current == new {
        finish_cleanup(store, &journal, None)?;
        return Ok(Some("A previous storage move was completed.".into()));
    }
    // The store points at a third root. Never delete files there: the journal
    // may belong to an older library state. Discard only the journal and leave
    // every archive for manual inspection.
    remove_journal(store);
    Ok(Some(
        "A saved storage move for a different library was ignored. The current library was not changed."
            .into(),
    ))
}

fn copy_and_commit(
    store: &Store,
    target: &Path,
    old_root: &Path,
    planned: &[PlannedEntry],
    journal: &mut Journal,
    remove_old: bool,
) -> Result<Value, String> {
    let staging = target.join(&journal.staging);
    let (target_dir, opened) = reserve_staging(store, target, journal)?;
    let mut copied_parts = 0usize;
    // Stamps of the bytes written by this process, keyed by their published
    // path, so cleanup can prove the replacements without re-reading them.
    let mut published: std::collections::HashMap<PathBuf, FileStamp> =
        std::collections::HashMap::new();
    let mut uncheckpointed = 0usize;
    for (index, entry) in planned.iter().enumerate() {
        let parent = open_staging_parent(&opened, &entry.relative, &mut journal.staging_directories, false)?;
        let selected = journal.selection.as_ref().and_then(|sources| sources.iter().find(|source| source.book.metadata.id == entry.book_id));
        let mut retained = selected.map(|source| retain_relocation_source(&source.book)).transpose()?;
        for part in &entry.parts {
            let source = selected.map(|selected| Path::new(&selected.book.path).with_file_name(&part.name))
                .unwrap_or_else(|| old_root.join(relative_path(&entry.relative)).join(&part.name));
            let destination = staging.join(relative_path(&entry.relative)).join(&part.name);
            let source_identity = if selected.is_some_and(|source| source.book.source_reference.is_some()) {
                None
            } else { Some(file_identity(&source)?) };
            let recorded = journal.entries[index].parts.iter_mut()
                .find(|recorded| recorded.name == part.name).ok_or("Library migration journal is missing a part")?;
            let mut output = open_staged_output(&parent, recorded)?;
            let hash = if let Some(retained) = retained.as_mut() {
                let (_, file) = retained.files_mut().iter_mut().find(|(name, _)| name == &part.name)
                    .ok_or("Selected source part disappeared")?;
                copy_descriptor(file, &mut output, part.size, File::write_all)?
            } else {
                let mut input = crate::acquisition::open_history(&source)?.ok_or("The original archive disappeared")?;
                copy_descriptor(&mut input, &mut output, part.size, File::write_all)?
            };
            if source_identity.is_some() && !owned_part(&source, &source_identity) {
                return Err("Archive source was replaced while copying".into());
            }
            recorded.sha256 = Some(hash);
            recorded.source_identity = source_identity;
            recorded.publication_identity.clone_from(&recorded.staging_identity);
            if !owned_part(&destination, &recorded.publication_identity) { return Err("The staged archive was replaced".into()); }
            // A stamp that cannot be read only costs the identity shortcut in
            // cleanup; it never fails the copy, which keeps its full checksum.
            if let Ok(stamp) = file_stamp(&destination) {
                published.insert(
                    target.join(relative_path(&entry.relative)).join(&part.name),
                    stamp,
                );
            }
            copied_parts += 1;
        }
        if let Some(retained) = retained { retained.unchanged()?; }
        verify_archive(&parent, &staging.join(relative_path(&entry.relative)), &entry.book_id, &journal.entries[index].parts)?;
        #[cfg(unix)]
        crate::storage::sync_cap_directory(&parent).map_err(|e| e.to_string())?;
        uncheckpointed += 1;
        if uncheckpointed >= JOURNAL_ENTRY_BATCH || index + 1 == planned.len() {
            write_journal(store, journal)?;
            uncheckpointed = 0;
        }
    }
    #[cfg(unix)]
    crate::storage::sync_cap_directory(&opened).map_err(|e| e.to_string())?;
    journal.phase = Phase::Publishing;
    write_journal(store, journal)?;
    let mut destinations = Vec::with_capacity(journal.entries.len());
    for recorded in &journal.entries {
        let source = open_staging_parent(&opened, &recorded.relative, &mut journal.staging_directories, false)?;
        let destination = destination_parent(&target_dir, &recorded.relative, true)?;
        for part in &recorded.parts {
            check_published_part(&source, part)?;
            publish_exclusive(&source, &destination, &part.name)?;
            check_published_part(&destination, part)?;
        }
        #[cfg(unix)]
        crate::storage::sync_cap_directory(&destination).map_err(|e| e.to_string())?;
        destinations.push(directory_identity(&destination)?);
    }
    #[cfg(unix)]
    crate::storage::sync_cap_directory(&target_dir).map_err(|e| e.to_string())?;
    let current = crate::storage::open_directory_nofollow(target).map_err(|e| e.to_string())?;
    if directory_identity(&current)? != directory_identity(&target_dir)? { return Err("Library destination changed before adoption".into()); }
    for (entry, expected) in journal.entries.iter().zip(destinations) {
        let parent = destination_parent(&current, &entry.relative, false)?;
        if directory_identity(&parent)? != expected { return Err("Library destination changed before adoption".into()); }
        for part in &entry.parts { check_published_part(&parent, part)?; }
    }
    drop(opened);
    let entries = planned;
    let target_string = target
        .to_str()
        .ok_or("Library destination path is not UTF-8")?
        .to_owned();
    store.update(|state| {
        for entry in entries {
            let index = state
                .books
                .iter()
                .position(|book| book.metadata.id == entry.book_id)
                .ok_or("A library archive was removed during the storage move")?;
            let book = &state.books[index];
            if let Some(selection) = &journal.selection {
                let source = selection.iter().find(|source| source.book.metadata.id == entry.book_id)
                    .ok_or("Selected archive source is missing")?;
                if book.installation_id != source.book.installation_id || book.path != source.book.path
                    || book.source_reference != source.book.source_reference {
                    return Err("A selected archive changed during copying".into());
                }
            } else {
                let expected = old_root.join(relative_path(&entry.relative)).join(&entry.primary);
                if Path::new(&book.path) != expected {
                    return Err("A library archive changed during the storage move".into());
                }
            }
            let destination = target.join(relative_path(&entry.relative)).join(&entry.primary);
            let before = book.clone();
            let mut after = before.clone();
            after.path = destination
                .to_str()
                .ok_or("Library destination path is not UTF-8")?
                .into();
            after.available = true;
            after.managed = true;
            after.source_reference = None;
            // The verified location and durable rollback expectation must
            // become visible together without changing the reader principal.
            state.reconcile_edition_relocation(&before, &after)?;
            state.books[index] = after;
        }
        if journal.selection.is_none() { state.settings.download_directory = target_string.clone(); }
        Ok(())
    })?;
    journal.phase = Phase::Committed;
    // A failed journal write after the Store commit is recoverable: the startup
    // path sees a "publishing" journal whose store already points at the new
    // root and finishes cleanup. It must never be reported as a failed switch,
    // because rolling back would delete archives the Store already references.
    let _ = write_journal(store, journal);
    // Verify the whole admitted replacement cohort before deleting even one
    // original. A cleanup failure is committed work, never rollback authority.
    let cleanup = finish_cleanup(store, journal, Some(&published));
    Ok(json!({
        "copied": journal.entries.len(),
        "parts": copied_parts,
        "bytes": journal.entries.iter().flat_map(|entry| entry.parts.iter())
            .try_fold(0u64, |total, part| total.checked_add(part.size))
            .ok_or("Library size overflow")?,
        "from": journal.old_root,
        "to": journal.new_root,
        "removedOld": cleanup.is_ok() && remove_old,
        "keptOld": cleanup.is_err() || !remove_old,
        "cleanupPending": cleanup.err(),
        "skipped": 0,
    }))
}

fn finish_cleanup(
    store: &Store,
    journal: &Journal,
    published: Option<&std::collections::HashMap<PathBuf, FileStamp>>,
) -> Result<(), String> {
    let old_root = PathBuf::from(&journal.old_root);
    let new_root = PathBuf::from(&journal.new_root);
    verify_replacements(store, journal, published)?;
    if let Some(expected) = &journal.marker_directory {
        adopt_committed_root(&new_root, expected, &journal.profile)?;
    }
    if journal.remove_old {
        for entry in &journal.entries {
            let relative = relative_path(&entry.relative);
            let selected = journal.selection.as_ref().and_then(|sources| sources.iter().find(|source| source.book.metadata.id == entry.book_id));
            if let Some(selected) = selected.filter(|source| source.book.source_reference.is_some()) {
                let reference = selected.book.source_reference.as_ref().ok_or("Provider source disappeared")?;
                if store.retains_provider_reference(&reference.reference)? {
                    return Err("The managed copy is ready, but its original is retained for edition rollback; keep the original archive and its provider permission".into());
                }
                delete_provider_original(selected, entry)?;
                continue;
            }
            for part in &entry.parts {
                let source = selected.map(|source| Path::new(&source.book.path).with_file_name(&part.name))
                    .unwrap_or_else(|| old_root.join(&relative).join(&part.name));
                if !remove_recorded_part(&source, &journal.id, part, Cleanup::Original)? {
                    return Err("An original archive changed; copies and recovery journal were preserved".into());
                }
            }
        }
        sync_directory(&old_root).ok();
    }
    remove_staging(&new_root, journal)?;
    #[cfg(mobile)]
    if let Some(selection) = &journal.selection {
        for source in selection {
            if let Some(reference) = &source.book.source_reference {
                // Native move receipts remain restart-idempotent even after
                // this no-longer-installed reference releases its grant.
                if !store.retains_provider_reference(&reference.reference)? {
                    let _ = crate::provider_reference::forget_registered(reference);
                }
            }
        }
    }
    remove_journal(store);
    Ok(())
}

fn rollback(store: &Store, journal: &Journal) -> Result<(), String> {
    let target = PathBuf::from(&journal.new_root);
    let mut errors = Vec::new();
    for entry in &journal.entries {
        let relative = relative_path(&entry.relative);
        for part in &entry.parts {
            let destination = target.join(&relative).join(&part.name);
            if let Err(error) = remove_recorded_part(&destination, &journal.id, part, Cleanup::Published) {
                errors.push(error);
            }
        }
    }
    if let Err(error) = remove_staging(&target, journal) { errors.push(error); }
    if errors.is_empty() {
        remove_journal(store);
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn verify_replacements(
    store: &Store,
    journal: &Journal,
    published: Option<&std::collections::HashMap<PathBuf, FileStamp>>,
) -> Result<(), String> {
    let state = store.read()?;
    let target = Path::new(&journal.new_root);
    for entry in &journal.entries {
        let primary = target.join(relative_path(&entry.relative)).join(&entry.primary);
        let admitted = state.books.iter().find(|book| book.metadata.id == entry.book_id
            && Path::new(&book.path) == primary && book.available && book.managed)
            .ok_or("Replacement archive is no longer admitted; originals and recovery journal were preserved")?;
        if let Some(selection) = &journal.selection {
            if !selection.iter().any(|source| source.book.metadata.id == entry.book_id
                && source.book.installation_id == admitted.installation_id) {
                return Err("Replacement installation changed; originals and recovery journal were preserved".into());
            }
        }
        // The publication rename preserves the stamp captured right after the
        // copy, so a match proves these are the exact bytes that were hashed
        // during copying and then passed libzim's integrity gate. Replay after a
        // restart has no stamp and keeps the full checksum.
        let mut cohort_proven = true;
        for part in &entry.parts {
            let path = target.join(relative_path(&entry.relative)).join(&part.name);
            let proven = published.is_some_and(|published| {
                published
                    .get(&path)
                    .is_some_and(|stamp| file_stamp(&path) == Ok(*stamp))
            });
            if !owned_part(&path, &part.publication_identity) || (!proven && !source_matches(&path, part)) {
                return Err("Replacement archive is missing or changed; originals and recovery journal were preserved".into());
            }
            cohort_proven &= proven;
        }
        let files = if cohort_proven {
            // Identity-only manifest check: the content gate already ran on
            // these same inodes in this process.
            crate::library_files::archive_files(admitted)?
        } else {
            crate::library_files::verified_archive_files(admitted)?
        };
        if files.len() != entry.parts.len() || !files.iter().all(|file| {
            entry.parts.iter().any(|part| file.file_name().and_then(|name| name.to_str()) == Some(part.name.as_str()))
        }) {
            return Err("Replacement archive part set changed; originals and recovery journal were preserved".into());
        }
    }
    Ok(())
}

fn destination_parent(root: &cap_std::fs::Dir, relative: &[String], create: bool) -> Result<cap_std::fs::Dir, String> {
    use cap_fs_ext::DirExt;
    let mut dir = root.try_clone().map_err(|e| e.to_string())?;
    for name in relative {
        dir = match dir.open_dir_nofollow(name) {
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                match dir.create_dir(name) {
                    Ok(()) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                    Err(error) => return Err(error.to_string()),
                }
                #[cfg(unix)]
                crate::storage::sync_cap_directory(&dir).map_err(|e| e.to_string())?;
                dir.open_dir_nofollow(name).map_err(|e| e.to_string())?
            },
            result => result.map_err(|e| e.to_string())?,
        };
    }
    Ok(dir)
}

fn matches_publication(meta: &cap_std::fs::Metadata, part: &JournalPart) -> bool {
    use cap_fs_ext::MetadataExt;
    meta.is_file() && !meta.is_symlink() && meta.nlink() == 1 && meta.len() == part.size
        && part.publication_identity.as_ref() == Some(&FileIdentity { device: meta.dev(), inode: meta.ino() })
}

fn check_published_part(parent: &cap_std::fs::Dir, part: &JournalPart) -> Result<(), String> {
    if !matches_publication(&parent.symlink_metadata(&part.name).map_err(|e| e.to_string())?, part) {
        return Err("The copied archive changed before adoption".into());
    }
    Ok(())
}

fn publish_exclusive(source: &cap_std::fs::Dir, destination: &cap_std::fs::Dir, name: &str) -> Result<(), String> {
    validate_part_name(name)?;
    #[cfg(unix)]
    let result = rustix::fs::renameat_with(source, name, destination, name, rustix::fs::RenameFlags::NOREPLACE)
        .map_err(std::io::Error::from);
    #[cfg(windows)]
    let result = {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        use cap_std::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        let mut options = cap_std::fs::OpenOptions::new();
        options.access_mode(0x0001_0000 | 0x0000_0080).follow(FollowSymlinks::No).custom_flags(0x0020_0000);
        let file = source.open_with(name, &options).map_err(|e| e.to_string())?;
        crate::storage::rename_into_directory_noreplace(file.as_raw_handle(), destination.as_raw_handle(), name.as_ref())
    };
    result.map_err(|error| if error.kind() == std::io::ErrorKind::AlreadyExists {
        format!("A file already exists at {name}; nothing was overwritten")
    } else { format!("Publishing archive without overwriting: {error}") })
}

fn activity_blocks(activity: &Activity, old_root: &Path) -> bool {
    if activity.map_busy {
        return true;
    }
    activity.transfers.iter().any(|transfer| {
        matches!(
            transfer.status.as_str(),
            "queued" | "downloading" | "verifying" | "paused"
        ) && Path::new(&transfer.destination).starts_with(old_root)
    })
}

fn plan_entries(store: &Store, old_root: &Path) -> Result<Vec<PlannedEntry>, String> {
    let state = store.read()?;
    let mut planned = Vec::new();
    for book in &state.books {
        let path = Path::new(&book.path);
        if !book.managed || !book.available || !path.starts_with(old_root) {
            continue;
        }
        if planned.len() >= MAX_ENTRIES {
            return Err("The library contains too many archives for one storage move".into());
        }
        // Existing exact-part manifest and libzim integrity gate. A failure here
        // aborts the whole switch before anything is copied.
        let files = crate::library_files::verified_archive_files(book).map_err(|error| {
            format!(
                "{} could not be verified, so the library location was not changed: {error}",
                book.metadata.title
            )
        })?;
        let relative = path
            .parent()
            .and_then(|parent| parent.strip_prefix(old_root).ok())
            .ok_or("Managed archive has no directory below the library root")?;
        let relative = relative_components(relative)?;
        let primary = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Archive filename must be valid Unicode")?;
        validate_part_name(primary)?;
        let mut parts = Vec::with_capacity(files.len());
        for file in files {
            if file.parent() != path.parent() {
                return Err("Archive parts are not in the same directory".into());
            }
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("Archive part filename must be valid Unicode")?;
            validate_part_name(name)?;
            let size = fs::metadata(&file)
                .map_err(|error| format!("Reading archive part size: {error}"))?
                .len();
            if size == 0 {
                return Err("Archive parts must not be empty".into());
            }
            parts.push(PlannedPart {
                name: name.to_owned(),
                size,
            });
        }
        planned.push(PlannedEntry {
            book_id: book.metadata.id.clone(),
            title: book.metadata.title.clone(),
            primary: primary.to_owned(),
            relative,
            parts,
        });
    }
    Ok(planned)
}

fn prepare_target(store: &Store, target: &Path) -> Result<FileIdentity, String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("The library location must not be a symbolic link".into())
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err("The library location is not a directory".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(target)
                .map_err(|error| format!("Creating library location: {error}"))?;
        }
        Err(error) => return Err(format!("Checking library location: {error}")),
    }
    let directory = crate::storage::open_directory_nofollow(target).map_err(|error| error.to_string())?;
    if read_marker(&directory, &profile_identity(store)?)? == Some(false) {
        return Err("That folder belongs to a different Kiwix profile".into());
    }
    directory_identity(&directory)
}

fn directory_identity(directory: &cap_std::fs::Dir) -> Result<FileIdentity, String> {
    use cap_fs_ext::MetadataExt;
    let metadata = directory.dir_metadata().map_err(|error| error.to_string())?;
    Ok(FileIdentity { device: metadata.dev(), inode: metadata.ino() })
}

fn read_marker(directory: &cap_std::fs::Dir, profile: &str) -> Result<Option<bool>, String> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
    let file = match directory.open_with(MARKER, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Opening library location marker: {error}")),
    };
    if !file.metadata().map_err(|error| error.to_string())?.is_file() {
        return Err("Library location marker must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > 4096 { return Err("Library location marker is oversized".into()); }
    let saved: Value = serde_json::from_slice(&bytes).map_err(|_| "Library location marker is invalid")?;
    Ok(Some(saved.get("profile").and_then(Value::as_str) == Some(profile)))
}

/// Publish only after the store commit and complete replacement verification.
/// A failed pre-commit move never changes the marker or needs to unlink it.
fn adopt_committed_root(target: &Path, expected: &FileIdentity, profile: &str) -> Result<(), String> {
    let directory = crate::storage::open_directory_nofollow(target).map_err(|error| error.to_string())?;
    if directory_identity(&directory)? != *expected { return Err("The selected storage folder was replaced".into()); }
    match read_marker(&directory, profile)? {
        Some(true) => {},
        Some(false) => return Err("That folder belongs to a different Kiwix profile".into()),
        None => {
            let bytes = serde_json::to_vec(&json!({"version": 1, "profile": profile})).map_err(|e| e.to_string())?;
            let temp = format!(".kiwix-library-marker-{}", Uuid::new_v4());
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            { use cap_std::fs::OpenOptionsExt; options.mode(0o600); }
            let mut file = directory.open_with(&temp, &options).map_err(|e| e.to_string())?.into_std();
            file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(|e| e.to_string())?;
            let verified = crate::publication::VerifiedFile::new(file, bytes.len() as u64).map_err(|e| e.to_string())?;
            crate::publication::publish_verified_noreplace_in(&directory, Path::new(&temp), Path::new(MARKER), &verified)
                .map_err(|e| format!("Publishing storage marker: {e}"))?;
        },
    }
    #[cfg(unix)]
    crate::storage::sync_cap_directory(&directory).map_err(|error| error.to_string())?;
    let current = crate::storage::open_directory_nofollow(target).map_err(|error| error.to_string())?;
    if directory_identity(&current)? != *expected { return Err("The selected storage folder was replaced".into()); }
    Ok(())
}

fn location_for(root: Root, active: &Path) -> Location {
    let (free_bytes, total_bytes) = match crate::storage::volume_space(&root.path) {
        Some((free, total)) => (Some(free), Some(total)),
        None => (None, None),
    };
    Location {
        available: if root.kind == "internal" {
            root.path.is_dir()
        } else {
            root.path.parent().is_some_and(Path::is_dir)
        },
        active: root.path == active,
        path: root.path.to_string_lossy().into_owned(),
        label: root.label,
        kind: root.kind,
        removable: root.removable,
        free_bytes,
        total_bytes,
    }
}

fn allowed_roots(store: &Store, native: &[NativeLocation]) -> Result<Vec<Root>, String> {
    let mut roots = vec![Root {
        path: internal_root(store),
        label: "Internal app storage".into(),
        kind: "internal".into(),
        removable: false,
    }];
    for location in native {
        if location.kind != "external" {
            continue;
        }
        let path = Path::new(&location.path);
        if !normalized_absolute(path) {
            return Err("Native external storage location is not absolute".into());
        }
        let root = path.join("archives");
        if roots.iter().any(|existing| existing.path == root) {
            continue;
        }
        roots.push(Root {
            path: root,
            label: if location.label.trim().is_empty() {
                "External app storage".into()
            } else {
                location.label.chars().take(128).collect()
            },
            kind: "external".into(),
            removable: location.removable,
        });
    }
    Ok(roots)
}

fn internal_root(store: &Store) -> PathBuf {
    store.data_dir().join("archives")
}

fn configured_root(store: &Store) -> Result<PathBuf, String> {
    let settings = store.settings()?;
    if settings.download_directory.is_empty() {
        Ok(internal_root(store))
    } else {
        Ok(PathBuf::from(settings.download_directory))
    }
}

fn profile_identity(store: &Store) -> Result<String, String> {
    fs::canonicalize(store.data_dir())
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| format!("Resolving profile directory: {error}"))
}

fn ensure_space(free: Option<u64>, needed: u64) -> Result<(), String> {
    if let Some(free) = free {
        if free < needed {
            return Err(format!(
                "Not enough space at the new location: {needed} bytes are needed but {free} bytes are free"
            ));
        }
    }
    Ok(())
}

fn reserve_staging(store: &Store, target: &Path, journal: &mut Journal) -> Result<(cap_std::fs::Dir, cap_std::fs::Dir), String> {
    use cap_fs_ext::{DirExt, MetadataExt};
    let root = crate::storage::open_directory_nofollow(target).map_err(|e| e.to_string())?;
    if journal.marker_directory.as_ref() != Some(&directory_identity(&root)?) { return Err("Library destination changed".into()); }
    root.create_dir(&journal.staging).map_err(|e| format!("Creating library staging directory: {e}"))?;
    let staging = root.open_dir_nofollow(&journal.staging).map_err(|e| e.to_string())?;
    journal.staging_directories.insert(String::new(), directory_identity(&staging)?);
    for entry in &mut journal.entries {
        let parent = open_staging_parent(&staging, &entry.relative, &mut journal.staging_directories, true)?;
        for part in &mut entry.parts {
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            { use cap_std::fs::OpenOptionsExt; options.mode(0o600); }
            let file = parent.open_with(&part.name, &options).map_err(|e| format!("Creating staged archive: {e}"))?;
            let meta = file.metadata().map_err(|e| e.to_string())?;
            part.staging_identity = Some(FileIdentity { device: meta.dev(), inode: meta.ino() });
        }
    }
    // Even a full volume or sudden power loss during the first large payload
    // now leaves a bounded identity ledger for the partially written files.
    write_journal(store, journal)?;
    Ok((root, staging))
}

fn open_staging_parent(root: &cap_std::fs::Dir, relative: &[String], directories: &mut std::collections::BTreeMap<String, FileIdentity>, create: bool) -> Result<cap_std::fs::Dir, String> {
    use cap_fs_ext::DirExt;
    let mut parent = root.try_clone().map_err(|e| e.to_string())?;
    for end in 1..=relative.len() {
        let name = &relative[end - 1];
        let key = relative[..end].join("/");
        if !directories.contains_key(&key) {
            if !create { return Err("Missing staged directory identity".into()); }
            parent.create_dir(name).map_err(|e| e.to_string())?;
        }
        parent = parent.open_dir_nofollow(name).map_err(|e| e.to_string())?;
        let identity = directory_identity(&parent)?;
        if *directories.entry(key).or_insert_with(|| identity.clone()) != identity { return Err("Library staging directory changed".into()); }
    }
    Ok(parent)
}

fn open_staged_output(parent: &cap_std::fs::Dir, part: &JournalPart) -> Result<File, String> {
    use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
    let file = parent.open_with(&part.name, &options).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.is_symlink() || meta.nlink() != 1 || meta.len() != 0
        || part.staging_identity.as_ref() != Some(&FileIdentity { device: meta.dev(), inode: meta.ino() }) {
        return Err("Reserved staged archive changed before copying".into());
    }
    Ok(file.into_std())
}

fn copy_descriptor(input: &mut File, output: &mut File, expected: u64,
    mut write: impl FnMut(&mut File, &[u8]) -> std::io::Result<()>) -> Result<String, String> {
    use std::io::{Seek, SeekFrom};
    input.seek(SeekFrom::Start(0)).map_err(|error| format!("Seeking archive source: {error}"))?;
    let metadata = input
        .metadata()
        .map_err(|error| format!("Reading archive part metadata: {error}"))?;
    if !metadata.is_file() || metadata.len() != expected {
        return Err("Archive part changed before it could be copied".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut copied = 0u64;
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("Reading archive part: {error}"))?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .ok_or("Archive part length overflow")?;
        if copied > expected {
            return Err("Archive part changed size while copying".into());
        }
        digest.update(&buffer[..count]);
        write(output, &buffer[..count])
            .map_err(|error| format!("Writing staged archive part: {error}"))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("Synchronizing staged archive part: {error}"))?;
    if copied != expected {
        return Err("Archive part changed size while copying".into());
    }
    Ok(hex::encode(digest.finalize()))
}

fn verify_archive(parent: &cap_std::fs::Dir, directory: &Path, book_id: &str, parts: &[JournalPart]) -> Result<(), String> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
    let files = parts.iter().map(|part| {
        let file = parent.open_with(&part.name, &options).map_err(|e| e.to_string())?;
        if !matches_publication(&file.metadata().map_err(|e| e.to_string())?, part) { return Err("Staged archive changed before verification".to_string()); }
        Ok((part.name.clone(), file.into_std()))
    }).collect::<Result<Vec<_>, _>>()?;
    let archive = crate::archive::Archive::open_files(directory, &files)
        .map_err(|error| format!("Opening copied archive for verification: {error}"))?;
    let metadata = archive
        .metadata()
        .map_err(|error| format!("Reading copied archive metadata: {error}"))?;
    if metadata.id != book_id || !archive.check_integrity()? {
        return Err("A copied archive did not pass verification; the previous library was kept".into());
    }
    Ok(())
}

fn source_matches(path: &Path, part: &JournalPart) -> bool {
    let Some(expected) = part.sha256.as_deref() else {
        return false;
    };
    let Ok(Some(file)) = crate::acquisition::open_history(path) else { return false };
    if !file.metadata().is_ok_and(|meta| meta.len() == part.size) { return false; }
    let mut file = file.take(part.size.saturating_add(1));
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => digest.update(&buffer[..count]),
            Err(_) => return false,
        }
    }
    hex::encode(digest.finalize()) == expected
}

fn journal_directory(store: &Store) -> PathBuf {
    store.data_dir().join(JOURNAL_DIRECTORY)
}

fn journal_path(store: &Store) -> PathBuf {
    journal_directory(store).join(JOURNAL_FILE)
}

pub(crate) fn has_pending_operation(store: &Store) -> Result<bool, String> {
    match fs::symlink_metadata(journal_path(store)) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("Checking for a pending storage move: {err}")),
    }
}

fn write_journal(store: &Store, journal: &Journal) -> Result<(), String> {
    let directory = journal_directory(store);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Creating storage journal directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Securing storage journal directory: {error}"))?;
    }
    let bytes = serde_json::to_vec(journal).map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Select fewer archives: the storage recovery journal would be too large".into());
    }
    write_atomic(&directory.join(JOURNAL_FILE), &bytes)?;
    sync_directory(&directory)
}

fn read_journal(store: &Store) -> Result<Journal, String> {
    let file = crate::acquisition::open_history(&journal_path(store))?
        .ok_or("The storage migration journal disappeared")?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)
        .map_err(|error| format!("Reading storage migration journal: {error}"))?;
    if bytes.len() > 1024 * 1024 { return Err("Storage migration journal is oversized".into()); }
    serde_json::from_slice(&bytes).map_err(|error| format!("Reading storage migration journal: {error}"))
}

fn validate_journal(store: &Store, journal: &Journal) -> Result<(), String> {
    if journal.version != JOURNAL_VERSION {
        return Err("Storage migration journal has an unsupported version".into());
    }
    let canonical_id = Uuid::parse_str(&journal.id)
        .map(|id| id.to_string())
        .map_err(|_| "Storage migration journal has an invalid ID".to_string())?;
    if canonical_id != journal.id {
        return Err("Storage migration journal has an invalid ID".into());
    }
    if journal.profile != profile_identity(store)? {
        return Err("Storage migration journal belongs to a different profile".into());
    }
    let old = Path::new(&journal.old_root);
    let new = Path::new(&journal.new_root);
    if !normalized_absolute(old) || !normalized_absolute(new) || (old == new && journal.selection.is_none()) {
        return Err("Storage migration journal has invalid roots".into());
    }
    if journal.selection.is_none() && (new.starts_with(old) || old.starts_with(new)) {
        return Err("Storage migration journal has overlapping roots".into());
    }
    if journal.staging != format!(".kiwix-library-root-{}", journal.id) {
        return Err("Storage migration journal has an invalid staging name".into());
    }
    for key in journal.staging_directories.keys().filter(|key| !key.is_empty()) {
        for name in key.split('/') { validate_part_name(name)?; }
    }
    if journal.entries.len() > MAX_ENTRIES {
        return Err("Storage migration journal has too many entries".into());
    }
    if let Some(selection) = &journal.selection {
        if selection.is_empty() || selection.len() != journal.entries.len() {
            return Err("Storage selection has an invalid source count".into());
        }
        let mut ids = std::collections::HashSet::new();
        for source in selection {
            let book = &source.book;
            if !ids.insert(&book.metadata.id) || !journal.entries.iter().any(|entry| entry.book_id == book.metadata.id) {
                return Err("Storage selection has duplicate or missing sources".into());
            }
            Uuid::parse_str(&book.installation_id).map_err(|_| "Storage selection has an invalid installation")?;
            if let Some(reference) = &book.source_reference {
                reference.validate_book(book)?;
                if journal.remove_old {
                    let receipt = source.delete_receipt.as_deref().ok_or("Provider move receipt is missing")?;
                    Uuid::parse_str(receipt).map_err(|_| "Provider move receipt is invalid")?;
                }
            } else if !book.managed || !normalized_absolute(Path::new(&book.path))
                || !is_managed_path(store, Path::new(&book.path)) {
                return Err("Kiwix could not confirm which original files belong to this move. The files were kept".into());
            }
        }
    }
    for entry in &journal.entries {
        if entry.book_id.is_empty() || entry.book_id.len() > 128 {
            return Err("Storage migration journal has an invalid archive ID".into());
        }
        validate_part_name(&entry.primary)?;
        for component in &entry.relative {
            validate_part_name(component)?;
        }
        if entry.parts.is_empty() || entry.parts.len() > 676 {
            return Err("Storage migration journal has an invalid part count".into());
        }
        for part in &entry.parts {
            validate_part_name(&part.name)?;
            if part.size == 0 {
                return Err("Storage migration journal has an empty part".into());
            }
        }
    }
    Ok(())
}

fn remove_journal(store: &Store) {
    let _ = fs::remove_file(journal_path(store));
}

/// Moves an unreadable journal aside under a unique name so it neither blocks
/// storage operations nor loses its exact bytes. A rename within the journal
/// directory keeps the original file; only its name changes.
fn quarantine_journal(store: &Store) -> Result<PathBuf, String> {
    let directory = journal_directory(store);
    let destination = directory.join(format!("{JOURNAL_FILE}.invalid-{}", Uuid::new_v4()));
    fs::rename(journal_path(store), &destination)
        .map_err(|error| format!("Moving the unreadable journal to {}: {error}", destination.display()))?;
    // A failed sync only risks losing the rename to a power loss, in which case
    // the original journal is still present and is quarantined again next start.
    let _ = sync_directory(&directory);
    Ok(destination)
}

enum Cleanup { Original, Published, Staged }

/// Capture before deleting. The deterministic private filename also lets a
/// restarted cleanup finish or restore an interrupted capture.
fn remove_recorded_part(path: &Path, id: &str, part: &JournalPart, purpose: Cleanup) -> Result<bool, String> {
    use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
    let dir = match crate::storage::open_directory_nofollow(path.parent().ok_or("Missing archive parent")?) {
        Ok(dir) => dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.to_string()),
    };
    let name = Path::new(&part.name);
    let expected = match purpose {
        Cleanup::Original => part.source_identity.as_ref(),
        Cleanup::Published => part.publication_identity.as_ref(),
        Cleanup::Staged => part.staging_identity.as_ref(),
    };
    let incomplete = matches!(purpose, Cleanup::Staged) && part.sha256.is_none();
    let Some(owned) = expected else {
        return match dir.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.to_string()),
            Ok(_) => Ok(false),
        };
    };
    let captured = format!(".kiwix-retired-{id}-{:x}-{:x}", owned.device, owned.inode);
    let captured = Path::new(&captured);
    let matches = |meta: &cap_std::fs::Metadata| meta.is_file() && !meta.is_symlink() && meta.nlink() == 1
        && meta.dev() == owned.device && meta.ino() == owned.inode
        && (meta.len() == part.size || (incomplete && meta.len() < part.size));
    let captured_here = match dir.symlink_metadata(captured) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match dir.symlink_metadata(name) {
                Ok(meta) if matches(&meta) => {},
                Ok(_) => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(error) => return Err(error.to_string()),
            }
            crate::publication::rename_entry_noreplace(&dir, name, captured).map_err(|e| e.to_string())?;
            true
        },
        Ok(_) => false,
        Err(error) => return Err(error.to_string()),
    };
    // Keep a changed or unrecognized object. Restore its old name only if it
    // is still free. Once captured, neither a stale waiter nor a restarted
    // operation may delete a new arrival at the original pathname.
    let check = (|| -> Result<bool, String> {
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
        if !matches(&dir.symlink_metadata(captured).map_err(|e| e.to_string())?) { return Ok(false); }
        let file = dir.open_with(captured, &options).map_err(|e| e.to_string())?;
        if !matches(&file.metadata().map_err(|e| e.to_string())?) { return Ok(false); }
        if let Some(digest) = &part.sha256 {
            let mut hash = Sha256::new();
            let size = std::io::copy(&mut file.take(part.size.saturating_add(1)), &mut hash).map_err(|e| e.to_string())?;
            if size != part.size || hex::encode(hash.finalize()) != *digest { return Ok(false); }
        }
        Ok(matches(&dir.symlink_metadata(captured).map_err(|e| e.to_string())?))
    })();
    if !matches!(check, Ok(true)) {
        if !captured_here { return Err(format!("An unrecognized file was preserved at {}", path.with_file_name(captured).display())); }
        crate::publication::rename_entry_noreplace(&dir, captured, name)
            .map_err(|e| format!("Changed file preserved at {}: {e}", path.with_file_name(captured).display()))?;
        return check;
    }
    dir.remove_file(captured).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    crate::storage::sync_cap_directory(&dir).map_err(|e| e.to_string())?;
    Ok(true)
}

fn remove_staging(root: &Path, journal: &Journal) -> Result<(), String> {
    let staging = root.join(&journal.staging);
    let Some(expected) = journal.staging_directories.get("") else { return Ok(()) };
    let directory = match crate::storage::open_directory_nofollow(&staging) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if directory_identity(&directory)? != *expected { return Ok(()); }
    drop(directory);
    for entry in &journal.entries {
        let parent = staging.join(relative_path(&entry.relative));
        for part in &entry.parts {
            remove_recorded_part(&parent.join(&part.name), &journal.id, part, Cleanup::Staged)?;
        }
    }
    // A crash may leave copied bytes before their first ownership checkpoint.
    // Only remove proven empty directories, deepest first. Unknown contents,
    // complete replacements and staging from older journals stay untouched.
    for (relative, expected) in journal.staging_directories.iter().rev() {
        let path = if relative.is_empty() { staging.clone() } else {
            for name in relative.split('/') { validate_part_name(name)?; }
            staging.join(relative)
        };
        let directory = match crate::storage::open_directory_nofollow(&path) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if directory_identity(&directory)? != *expected { continue; }
        match directory.remove_open_dir() {
            Ok(()) => {},
            Err(error) if matches!(error.kind(), std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound) => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn relative_path(components: &[String]) -> PathBuf {
    components.iter().collect()
}

fn relative_components(path: &Path) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err("Archive directory contains an unsafe component".into());
        };
        let name = name
            .to_str()
            .ok_or("Archive directory must be valid Unicode")?;
        validate_part_name(name)?;
        result.push(name.to_owned());
    }
    Ok(result)
}

fn validate_part_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 240
        || name == "."
        || name == ".."
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        || name.ends_with(['.', ' '])
        || Path::new(name).components().count() != 1
        || !matches!(Path::new(name).components().next(), Some(Component::Normal(_)))
    {
        return Err("Invalid library filename".into());
    }
    Ok(())
}

fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Library marker has no filename")?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("Creating temporary file: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("Securing temporary file: {error}"))?;
        }
        file.write_all(bytes)
            .map_err(|error| format!("Writing temporary file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Synchronizing temporary file: {error}"))?;
        fs::rename(&temporary, path).map_err(|error| format!("Replacing file: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let file = File::open(path).map_err(|error| format!("Opening directory for sync: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Synchronizing directory: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Book;

    struct Temp(PathBuf);
    impl Temp {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("kiwix-library-root-{label}-{}", Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn book_at(path: &Path) -> Book {
        let archive = crate::archive::Archive::open(path).unwrap();
        Book {
            metadata: archive.metadata().unwrap(),
            installation_id: crate::store::new_installation_id(),
            path: path.to_string_lossy().into_owned(),
            added_at: crate::store::now(),
            available: true,
            managed: true,
            source_reference: None,
            integrity: None,
        }
    }

    fn profile_with_book() -> (Temp, std::sync::Arc<Store>, PathBuf) {
        let profile = Temp::new("profile");
        let data = profile.0.join("data");
        fs::create_dir_all(&data).unwrap();
        let store = std::sync::Arc::new(Store::new(data.clone()).unwrap());
        let internal = data.join("archives");
        fs::create_dir_all(&internal).unwrap();
        let source = internal.join("small.zim");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
            &source,
        )
        .unwrap();
        let book = book_at(&source);
        store
            .update(|state| {
                state.books.push(book);
                state.settings.download_directory = internal.to_string_lossy().into_owned();
                Ok(())
            })
            .unwrap();
        let target = profile.0.join("removable");
        fs::create_dir_all(&target).unwrap();
        (profile, store, target)
    }

    fn native_for(target: &Path) -> Vec<NativeLocation> {
        vec![NativeLocation {
            path: target.to_string_lossy().into_owned(),
            kind: "external".into(),
            removable: true,
            label: "SD card".into(),
        }]
    }

    #[test]
    fn locations_report_internal_and_external_roots_with_active_selection() {
        let (_profile, store, target) = profile_with_book();
        let value = locations(&store, &native_for(&target)).unwrap();
        let entries = value.get("locations").unwrap().as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].get("kind").unwrap(), "internal");
        assert_eq!(entries[0].get("active").unwrap(), true);
        assert_eq!(entries[1].get("kind").unwrap(), "external");
        assert_eq!(entries[1].get("removable").unwrap(), true);
        assert_eq!(entries[1].get("active").unwrap(), false);
    }

    #[test]
    fn switch_copies_verifies_and_persists_the_selected_root() {
        let (_profile, store, target) = profile_with_book();
        let result = switch(&store, &target.join("archives"), false, &native_for(&target), &Activity::default()).unwrap();
        assert_eq!(result.get("copied").unwrap(), 1);
        let new_root = target.join("archives");
        assert!(new_root.join("small.zim").is_file());
        assert!(Path::new(&store.settings().unwrap().download_directory) == new_root);
        let state = store.read().unwrap();
        assert_eq!(Path::new(&state.books[0].path), new_root.join("small.zim"));
        // Keeping the old copy is the default and leaves the previous root usable.
        assert!(store.data_dir().join("archives/small.zim").is_file());
        assert!(new_root.join(MARKER).is_file());
        assert!(is_managed_path(&store, &new_root.join("small.zim")));
        assert!(!is_managed_path(&store, &target.join("unrelated.zim")));
    }

    #[test]
    fn switch_removes_old_copies_only_when_requested() {
        let (_profile, store, target) = profile_with_book();
        let old = store.data_dir().join("archives/small.zim");
        switch(&store, &target.join("archives"), true, &native_for(&target), &Activity::default()).unwrap();
        assert!(!old.exists());
        assert!(target.join("archives/small.zim").is_file());
    }

    #[test]
    fn verified_edition_relocations_preserve_explicit_rollback_and_security_boundaries() {
        use crate::{
            catalog::CatalogEntry,
            edition_updates::{EditionIntent, EditionManager, EditionSelection},
            store::{Bookmark, HistoryEntry, Tab, TabLocation, WindowSession},
        };


        // Both mobile storage actions share copy_and_commit; desktop moves use
        // library_files::commit_path, including its journal recovery ordering.
        for (operation, remove_old) in [(0, false), (0, true), (1, false), (1, true), (2, true)] {
            let (_profile, store, target) = profile_with_book();
            let original = store.read().unwrap().books[0].clone();
            let original_bytes = fs::read(&original.path).unwrap();
            let original_identity = crate::archive_source::retain(&original).unwrap().identity().unwrap();
            let replacement_path = store.data_dir().join("archives/edition-output.zim");
            // The media fixture has no catalog Name. Keep real named metadata
            // while producing a distinct, checksum-valid native archive UUID.
            let mut replacement_bytes = original_bytes.clone();
            replacement_bytes[8..24].copy_from_slice(Uuid::new_v4().as_bytes());
            let checksum = u64::from_le_bytes(replacement_bytes[72..80].try_into().unwrap()) as usize;
            let digest = md5::compute(&replacement_bytes[..checksum]).0;
            replacement_bytes[checksum..checksum + 16].copy_from_slice(&digest);
            fs::write(&replacement_path, replacement_bytes).unwrap();
            let verified = book_at(&replacement_path);
            crate::library_files::verified_archive_files(&verified).unwrap();
            assert!(!verified.metadata.name.is_empty(), "catalog edition fixtures require a native series name");
            assert_ne!(verified.metadata.id, original.metadata.id);
            let replacement_identity = crate::archive_source::retain(&verified).unwrap().identity().unwrap();
            let replacement_bytes = fs::read(&replacement_path).unwrap();
            let metadata = &verified.metadata;
            let intent = EditionIntent {
                id: Uuid::new_v4().to_string(),
                selection: EditionSelection {
                    source_id: "relocation-catalog".into(),
                    source_url: "https://example.org/catalog".into(),
                    entry_id: "relocation-edition".into(),
                    catalog_generation: 1,
                    entry: CatalogEntry {
                        id: "relocation-edition".into(), title: metadata.title.clone(),
                        description: metadata.description.clone(), language: metadata.language.clone(),
                        creator: metadata.creator.clone(), publisher: metadata.publisher.clone(),
                        date: metadata.date.clone(), name: metadata.name.clone(), tags: metadata.tags.clone(),
                        size: metadata.size, article_count: metadata.article_count, media_count: metadata.media_count,
                        url: "https://example.org/edition.zim".into(), thumbnail_url: None,
                        torrent_url: None, torrent_resolver_url: None,
                    },
                },
                target: original.clone(), original_identity: original_identity.clone(),
                lineage_associated: true,
                destination: verified.path.clone(), download_directory: store.settings().unwrap().download_directory,
                download_id: Some("relocation-download".into()), receipt: Some("relocation-download".into()),
                status: "ready".into(), verified: Some(verified.clone()),
                verified_identity: Some(replacement_identity), transaction_id: None,
            };
            let location = TabLocation {
                book_id: original.metadata.id.clone(), path: "A/Previous?q=1#part".into(),
                title: "Previous".into(), scroll: 8.0,
            };
            let tab = Tab {
                restricted: false,
                id: "relocation-tab".into(), book_id: Some(original.metadata.id.clone()),
                path: "A/Current#fragment".into(), title: "Current".into(), scroll: 42.0,
                back: vec![location.clone()], forward: vec![location],
            };
            let committed = store.update(|state| {
                state.bookmarks.push(Bookmark {
                    id: Uuid::new_v4().to_string(), book_id: original.metadata.id.clone(),
                    path: "A/Current#fragment".into(), title: "Bookmark".into(), note: "Original note".into(),
                    tags: vec!["personal".into()], highlights: Vec::new(),
                    created_at: crate::store::now(), legacy_book: None,
                });
                state.history.push(HistoryEntry {
                    id: "relocation-history".into(), book_id: original.metadata.id.clone(),
                    path: "A/Current".into(), title: "History".into(), visited_at: crate::store::now(), scroll: 10.0,
                });
                state.tabs.push(tab.clone());
                state.active_tab = Some(tab.id.clone());
                state.windows.insert("main".into(), WindowSession {
                    tabs: vec![tab.clone()], active_tab: Some(tab.id.clone()), ..WindowSession::default()
                });
                state.settings.book_zoom.insert(original.metadata.id.clone(), 1.5);
                state.settings.sharing_book_ids.push(original.metadata.id.clone());
                state.reader_storage.insert(original.installation_id.clone(), json!({"cookie": "original-secret"}));
                state.edition_intents.push(intent.clone());
                state.replace_edition(&intent, &verified, "relocation-download")
            }).unwrap();
            assert_ne!(committed.replacement.installation_id, original.installation_id);
            assert_ne!(committed.replacement.installation_id, verified.installation_id);
            let manager = EditionManager::new();
            let stale = manager.prepare_rollback(&store, &committed.id,
                &committed.replacement.installation_id, 7, "main").unwrap();
            store.update(|state| {
                // Newer reading state must survive both relocation and rollback;
                // the current edition's authority survives relocation only.
                state.bookmarks[0].note = "Edited after edition commit".into();
                state.tabs[0].scroll = 73.0;
                state.reader_storage.insert(committed.replacement.installation_id.clone(), json!({"cookie": "replacement-secret"}));
                state.settings.sharing_book_ids.push(committed.replacement.metadata.id.clone());
                Ok(())
            }).unwrap();
            let before = store.read().unwrap();
            let destination_root = target.join("archives");
            match operation {
                0 => { switch(&store, &destination_root, remove_old, &native_for(&target), &Activity::default()).unwrap(); }
                1 => { relocate_selected(&store, &destination_root, std::slice::from_ref(&committed.replacement.metadata.id),
                    remove_old, &native_for(&target), &Activity::default()).unwrap(); }
                2 => {
                    let from = PathBuf::from(&before.settings.download_directory);
                    store.update(|state| {
                        state.settings.download_directory = destination_root.to_string_lossy().into_owned();
                        Ok(())
                    }).unwrap();
                    let result = crate::library_files::move_archives(store.clone(), from, destination_root.clone()).unwrap();
                    assert_eq!(result["moved"].as_array().unwrap().len(), 1);
                    assert!(result["errors"].as_array().unwrap().is_empty());
                }
                _ => unreachable!(),
            }
            let relocated = store.read().unwrap();
            let current = &relocated.books[0];
            assert_eq!(current.installation_id, committed.replacement.installation_id);
            assert_ne!(current.path, committed.replacement.path);
            assert_eq!(relocated.edition_transactions[0].replacement, *current);
            assert_eq!(relocated.edition_transactions[0].original, original);
            assert_eq!(relocated.edition_transactions[0].original_identity, original_identity);
            assert_eq!(relocated.reference_revision, before.reference_revision);
            assert_eq!(relocated.reference_transitions, before.reference_transitions);
            assert_eq!(relocated.edition_bindings, before.edition_bindings);
            assert_eq!(relocated.reader_storage, before.reader_storage);
            assert!(!relocated.reader_storage.contains_key(&original.installation_id));
            assert_eq!(relocated.settings.sharing_book_ids, before.settings.sharing_book_ids);
            assert_eq!(relocated.settings.book_zoom, before.settings.book_zoom);
            assert_eq!(serde_json::to_value((&relocated.bookmarks, &relocated.history, &relocated.tabs, &relocated.windows)).unwrap(),
                serde_json::to_value((&before.bookmarks, &before.history, &before.tabs, &before.windows)).unwrap());
            assert_eq!(fs::read(&original.path).unwrap(), original_bytes);
            assert_eq!(fs::read(&current.path).unwrap(), replacement_bytes);
            assert_eq!(replacement_path.exists(), !remove_old);
            assert!(manager.rollback(&store, &stale, 7, "main").is_err());
            let prepared = manager.prepare_rollback(&store, &committed.id, &current.installation_id, 7, "main").unwrap();
            assert!(manager.rollback(&store, &prepared, 8, "main").is_err());
            assert!(manager.rollback(&store, &prepared, 7, "other-window").is_err());
            let rolled_back = manager.rollback(&store, &prepared, 7, "main").unwrap();
            let restored_state = store.read().unwrap();
            let restored = &restored_state.books[0];
            assert_eq!(restored.path, original.path);
            assert_eq!(serde_json::to_value(&restored.metadata).unwrap(), serde_json::to_value(&original.metadata).unwrap());
            assert_ne!(restored.installation_id, original.installation_id);
            assert_ne!(restored.installation_id, current.installation_id);
            assert_eq!(rolled_back.rollback_book.as_ref(), Some(restored));
            assert!(restored_state.reader_storage.is_empty());
            assert!(restored_state.settings.sharing_book_ids.is_empty());
            assert_eq!(restored_state.bookmarks[0].book_id, original.metadata.id);
            assert_eq!(restored_state.bookmarks[0].note, "Edited after edition commit");
            assert_eq!(restored_state.history[0].book_id, original.metadata.id);
            assert_eq!(restored_state.tabs[0].book_id.as_deref(), Some(original.metadata.id.as_str()));
            assert_eq!(restored_state.tabs[0].scroll, 73.0);
            assert_eq!(restored_state.tabs[0].back[0].book_id, original.metadata.id);
            assert_eq!(restored_state.tabs[0].forward[0].book_id, original.metadata.id);
            assert_eq!(restored_state.windows["main"].tabs[0].book_id.as_deref(), Some(original.metadata.id.as_str()));
            assert_eq!(restored_state.settings.book_zoom.get(&original.metadata.id), Some(&1.5));
            assert_eq!(restored_state.reference_revision, before.reference_revision + 1);
            assert_eq!(fs::read(&original.path).unwrap(), original_bytes);
            assert_eq!(fs::read(&current.path).unwrap(), replacement_bytes);
            assert_eq!(rolled_back.original, original);
            assert_eq!(rolled_back.original_identity, original_identity);
        }
    }

    #[test]
    fn failed_source_verification_aborts_before_copying_and_keeps_the_old_root() {
        let (_profile, store, target) = profile_with_book();
        let old = store.data_dir().join("archives/small.zim");
        // Replace the archive bytes after admission. The existing integrity gate
        // must reject the source before the destination is touched.
        fs::write(&old, b"not a ZIM archive").unwrap();
        let result = switch(&store, &target.join("archives"), false, &native_for(&target), &Activity::default());
        assert!(result.is_err());
        assert!(!target.join("archives/small.zim").exists());
        assert!(Path::new(&store.settings().unwrap().download_directory)
            == store.data_dir().join("archives"));
        let state = store.read().unwrap();
        assert_eq!(state.books[0].path, old.to_string_lossy());
    }

    #[test]
    fn interrupted_publish_rolls_back_on_next_start() {
        let (_profile, store, target) = profile_with_book();
        let new_root = target.join("archives");
        fs::create_dir_all(&new_root).unwrap();
        let id = Uuid::new_v4().to_string();
        let staging = new_root.join(format!(".kiwix-library-root-{id}"));
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("partial.zim"), b"partial").unwrap();
        fs::write(new_root.join("small.zim"), b"partial").unwrap();
        let journal = Journal {
            version: JOURNAL_VERSION,
            id: id.clone(),
            profile: profile_identity(&store).unwrap(),
            old_root: store.data_dir().join("archives").to_string_lossy().into_owned(),
            new_root: new_root.to_string_lossy().into_owned(),
            remove_old: true,
            _legacy_marker_created: true, marker_directory: None, staging_directories: Default::default(),
            staging: format!(".kiwix-library-root-{id}"),
            phase: Phase::Publishing,
            selection: None,
            entries: vec![JournalEntry {
                book_id: store.read().unwrap().books[0].metadata.id.clone(),
                title: "small".into(),
                primary: "small.zim".into(),
                relative: vec![],
                parts: vec![JournalPart {
                    name: "small.zim".into(),
                    size: 7,
                    sha256: None,
                    staging_identity: None, source_identity: None,
                    publication_identity: Some(file_identity(&new_root.join("small.zim")).unwrap()),
                }],
            }],
        };
        write_journal(&store, &journal).unwrap();
        let warning = recover(&store).unwrap().unwrap();
        assert!(warning.contains("rolled back"));
        assert!(!new_root.join("small.zim").exists());
        assert_eq!(fs::read(staging.join("partial.zim")).unwrap(), b"partial");
        assert!(!journal_path(&store).exists());
        assert!(store.data_dir().join("archives/small.zim").is_file());
    }

    #[test]
    fn interrupted_cleanup_finishes_when_store_already_points_at_new_root() {
        let (_profile, store, target) = profile_with_book();
        let old = store.data_dir().join("archives/small.zim");
        let new_root = target.join("archives");
        fs::create_dir_all(&new_root).unwrap();
        let source_bytes = fs::read(&old).unwrap();
        fs::write(new_root.join("small.zim"), &source_bytes).unwrap();
        let hash = hex::encode(Sha256::digest(&source_bytes));
        store
            .update(|state| {
                state.books[0].path = new_root.join("small.zim").to_string_lossy().into_owned();
                state.settings.download_directory = new_root.to_string_lossy().into_owned();
                Ok(())
            })
            .unwrap();
        let id = Uuid::new_v4().to_string();
        let journal = Journal {
            version: JOURNAL_VERSION,
            id: id.clone(),
            profile: profile_identity(&store).unwrap(),
            old_root: store.data_dir().join("archives").to_string_lossy().into_owned(),
            new_root: new_root.to_string_lossy().into_owned(),
            remove_old: true,
            _legacy_marker_created: false, marker_directory: None, staging_directories: Default::default(),
            staging: format!(".kiwix-library-root-{id}"),
            phase: Phase::Committed,
            selection: None,
            entries: vec![JournalEntry {
                book_id: store.read().unwrap().books[0].metadata.id.clone(),
                title: "small".into(),
                primary: "small.zim".into(),
                relative: vec![],
                parts: vec![JournalPart {
                    name: "small.zim".into(),
                    size: source_bytes.len() as u64,
                    sha256: Some(hash),
                    staging_identity: None, source_identity: Some(file_identity(&old).unwrap()),
                    publication_identity: Some(file_identity(&new_root.join("small.zim")).unwrap()),
                }],
            }],
        };
        write_journal(&store, &journal).unwrap();
        let warning = recover(&store).unwrap().unwrap();
        assert!(warning.contains("completed"));
        assert!(!old.exists());
        assert!(new_root.join("small.zim").is_file());
        assert!(!journal_path(&store).exists());
    }

    #[test]
    fn free_space_check_blocks_an_oversized_move() {
        assert!(ensure_space(Some(100), 100).is_ok());
        let error = ensure_space(Some(99), 100).unwrap_err();
        assert!(error.contains("Not enough space"));
        // An unreported capacity is not treated as zero.
        assert!(ensure_space(None, u64::MAX).is_ok());
    }

    #[test]
    fn switch_refuses_while_a_parked_transfer_targets_the_old_root() {
        let (_profile, store, target) = profile_with_book();
        let old = store.data_dir().join("archives");
        let activity = Activity {
            transfers: vec![TransferTarget {
                destination: old.join("parked.zim").to_string_lossy().into_owned(),
                status: "paused".into(),
            }],
            map_busy: false,
        };
        let error = switch(
            &store,
            &target.join("archives"),
            false,
            &native_for(&target),
            &activity,
        )
        .unwrap_err();
        assert!(error.contains("Finish or cancel your downloads"));
        assert!(!target.join("archives/small.zim").exists());
        assert!(old.join("small.zim").is_file());
        assert_eq!(
            Path::new(&store.settings().unwrap().download_directory),
            old
        );
    }

    #[test]
    fn switch_refuses_while_a_map_job_is_active() {
        let (_profile, store, target) = profile_with_book();
        let activity = Activity {
            transfers: Vec::new(),
            map_busy: true,
        };
        let error = switch(
            &store,
            &target.join("archives"),
            false,
            &native_for(&target),
            &activity,
        )
        .unwrap_err();
        assert!(error.contains("Finish or cancel your downloads"));
        assert!(!target.join("archives/small.zim").exists());
    }

    #[test]
    fn private_and_read_only_modes_refuse_a_switch() {
        let (_profile, store, target) = profile_with_book();
        let ids = vec![store.read().unwrap().books[0].metadata.id.clone()];
        store
            .update(|state| {
                state.settings.private_mode = true;
                Ok(())
            })
            .unwrap();
        assert!(switch(&store, &target.join("archives"), false, &native_for(&target), &Activity::default()).is_err());
        assert!(relocate_selected(&store, &target.join("archives"), &ids, false, &native_for(&target), &Activity::default()).is_err());
        store
            .update(|state| {
                state.settings.private_mode = false;
                state.settings.read_only = true;
                Ok(())
            })
            .unwrap();
        assert!(switch(&store, &target.join("archives"), false, &native_for(&target), &Activity::default()).is_err());
        assert!(relocate_selected(&store, &target.join("archives"), &ids, true, &native_for(&target), &Activity::default()).is_err());
        assert!(!target.join("archives/small.zim").exists());
    }

    #[test]
    fn foreign_marker_is_refused() {
        let (_profile, store, target) = profile_with_book();
        fs::create_dir_all(target.join("archives")).unwrap();
        fs::write(
            target.join("archives").join(MARKER),
            br#"{"version":1,"profile":"/different/profile"}"#,
        )
        .unwrap();
        assert!(switch(&store, &target.join("archives"), false, &native_for(&target), &Activity::default()).is_err());
        assert!(!target.join("archives/small.zim").exists());
    }

    #[test]
    fn marker_is_bounded_and_created_only_on_the_committed_directory() {
        let (_profile, store, target) = profile_with_book();
        let root = target.join("archives");
        let original = prepare_target(&store, &root).unwrap();
        let profile = profile_identity(&store).unwrap();
        let marker = root.join(MARKER);
        assert!(!marker.exists());
        fs::rename(&root, target.join("previous")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(adopt_committed_root(&root, &original, &profile).is_err());
        assert!(!marker.exists());
        fs::write(&marker, vec![b'x'; 4097]).unwrap();
        assert!(prepare_target(&store, &root).unwrap_err().contains("oversized"));
        assert!(!is_managed_path(&store, &root));
        let mut journal = committed_cleanup_journal(&store, &root);
        journal._legacy_marker_created = true;
        rollback(&store, &journal).unwrap();
        assert_eq!(fs::read(&marker).unwrap(), vec![b'x'; 4097]);
    }

    #[test]
    fn unknown_target_is_refused() {
        let (_profile, store, target) = profile_with_book();
        let elsewhere = Temp::new("elsewhere");
        assert!(switch(&store, &elsewhere.0, false, &native_for(&target), &Activity::default()).is_err());
    }

    #[test]
    fn journal_rejects_unsafe_relative_components() {
        let (_profile, store, target) = profile_with_book();
        let journal = Journal {
            version: JOURNAL_VERSION,
            id: Uuid::new_v4().to_string(),
            profile: profile_identity(&store).unwrap(),
            old_root: store.data_dir().join("archives").to_string_lossy().into_owned(),
            new_root: target.join("archives").to_string_lossy().into_owned(),
            remove_old: false,
            _legacy_marker_created: false, marker_directory: None, staging_directories: Default::default(),
            staging: format!(".kiwix-library-root-{}", Uuid::new_v4()),
            phase: Phase::Copying,
            selection: None,
            entries: vec![JournalEntry {
                book_id: "id".into(),
                title: "title".into(),
                primary: "small.zim".into(),
                relative: vec!["..".into()],
                parts: vec![JournalPart {
                    name: "small.zim".into(),
                    size: 1,
                    sha256: None,
                    staging_identity: None, source_identity: None,
                    publication_identity: None,
                }],
            }],
        };
        assert!(validate_journal(&store, &journal).is_err());
    }

    #[test]
    fn collision_rollback_preserves_preexisting_same_size_destination() {
        let (_profile, store, target) = profile_with_book();
        let source = store.data_dir().join("archives/small.zim");
        let new_root = target.join("archives");
        fs::create_dir(&new_root).unwrap();
        let sentinel = vec![b'x'; fs::metadata(&source).unwrap().len() as usize];
        fs::write(new_root.join("small.zim"), &sentinel).unwrap();
        assert!(switch(&store, &new_root, true, &native_for(&target), &Activity::default()).is_err());
        assert_eq!(fs::read(new_root.join("small.zim")).unwrap(), sentinel);
        assert!(source.exists());
    }

    fn committed_cleanup_journal(store: &Store, new_root: &Path) -> Journal {
        let old = store.data_dir().join("archives/small.zim");
        let bytes = fs::read(&old).unwrap();
        fs::create_dir_all(new_root).unwrap();
        fs::write(new_root.join("small.zim"), &bytes).unwrap();
        store.update(|state| {
            state.books[0].path = new_root.join("small.zim").to_string_lossy().into_owned();
            state.settings.download_directory = new_root.to_string_lossy().into_owned();
            Ok(())
        }).unwrap();
        let id = Uuid::new_v4().to_string();
        Journal {
            version: JOURNAL_VERSION, staging: format!(".kiwix-library-root-{id}"), id,
            profile: profile_identity(store).unwrap(),
            old_root: old.parent().unwrap().to_string_lossy().into_owned(),
            new_root: new_root.to_string_lossy().into_owned(),
            remove_old: true, _legacy_marker_created: false, marker_directory: None, staging_directories: Default::default(), phase: Phase::Committed, selection: None,
            entries: vec![JournalEntry {
                book_id: store.read().unwrap().books[0].metadata.id.clone(), title: "Small".into(),
                primary: "small.zim".into(), relative: vec![],
                parts: vec![JournalPart {
                    name: "small.zim".into(), size: bytes.len() as u64,
                    sha256: Some(hex::encode(Sha256::digest(&bytes))),
                    staging_identity: None, source_identity: Some(file_identity(&old).unwrap()),
                    publication_identity: Some(file_identity(&new_root.join("small.zim")).unwrap()),
                }],
            }],
        }
    }

    #[test]
    fn root_recovery_keeps_adopted_archives_when_the_preferred_folder_was_changed_back() {
        for phase in [Phase::Publishing, Phase::Committed] {
            let (_profile, store, target) = profile_with_book();
            let new_root = target.join("archives");
            let mut journal = committed_cleanup_journal(&store, &new_root);
            journal.phase = phase;
            write_journal(&store, &journal).unwrap();
            store.update(|state| {
                state.settings.download_directory.clone_from(&journal.old_root);
                Ok(())
            }).unwrap();
            let registered = store.books().unwrap();
            assert!(recover(&store).unwrap().unwrap().contains("verified"));
            assert!(Path::new(&registered[0].path).is_file());
            assert!(Path::new(&journal.old_root).join("small.zim").is_file());
            assert_eq!(store.books().unwrap(), registered);
            assert_eq!(store.settings().unwrap().download_directory, journal.old_root);
            assert!(!has_pending_operation(&store).unwrap());
        }
    }

    #[test]
    fn recovery_rolls_back_a_copy_when_the_planned_book_was_removed() {
        let (_profile, store, target) = profile_with_book();
        let new_root = target.join("archives");
        let mut journal = committed_cleanup_journal(&store, &new_root);
        journal.phase = Phase::Copying;
        write_journal(&store, &journal).unwrap();
        store.update(|state| {
            state.books.clear();
            state.settings.download_directory.clone_from(&journal.old_root);
            Ok(())
        }).unwrap();
        let notice = recover(&store).unwrap().unwrap();
        assert!(notice.contains("rolled back"), "{notice}");
        assert!(!new_root.join("small.zim").exists());
        assert!(Path::new(&journal.old_root).join("small.zim").exists());
        assert!(!journal_path(&store).exists());
        assert!(!has_pending_operation(&store).unwrap());
    }

    #[test]
    fn recovery_preserves_original_and_journal_when_replacement_missing_or_corrupt() {
        for corrupt in [false, true] {
            let (_profile, store, target) = profile_with_book();
            let new_root = target.join("archives");
            let journal = committed_cleanup_journal(&store, &new_root);
            write_journal(&store, &journal).unwrap();
            if corrupt {
                let mut bytes = fs::read(new_root.join("small.zim")).unwrap();
                bytes[0] ^= 0xff;
                fs::write(new_root.join("small.zim"), bytes).unwrap();
            } else { fs::remove_file(new_root.join("small.zim")).unwrap(); }
            assert!(recover(&store).is_err());
            assert!(store.data_dir().join("archives/small.zim").exists());
            assert!(journal_path(&store).exists());
        }
    }

    #[test]
    fn unreadable_journal_is_set_aside_with_its_bytes_and_does_not_block_recovery() {
        for (name, bytes) in [
            ("not json", b"{\"version\":".to_vec()),
            // Valid JSON, invalid journal: rejected by validation instead.
            ("invalid journal", br#"{"version":99,"id":"x"}"#.to_vec()),
        ] {
            let (_profile, store, _target) = profile_with_book();
            let directory = journal_directory(&store);
            fs::create_dir_all(&directory).unwrap();
            fs::write(journal_path(&store), &bytes).unwrap();
            let notice = recover(&store).unwrap().unwrap_or_default();
            assert!(notice.contains("set aside"), "{name}: {notice}");
            assert!(!journal_path(&store).exists(), "{name}");
            let kept = fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .contains(".invalid-")
                })
                .unwrap_or_else(|| panic!("{name}: quarantined journal missing"));
            assert_eq!(fs::read(&kept).unwrap(), bytes, "{name}");
            // A second start finds nothing left to recover.
            assert!(recover(&store).unwrap().is_none(), "{name}");
        }
    }

    #[test]
    fn rollback_does_not_delete_replacement_object_even_when_bytes_match() {
        let (_profile, store, target) = profile_with_book();
        let new_root = target.join("archives");
        let journal = committed_cleanup_journal(&store, &new_root);
        let path = new_root.join("small.zim");
        fs::rename(&path, new_root.join("original-publication.zim")).unwrap();
        fs::copy(store.data_dir().join("archives/small.zim"), &path).unwrap();
        rollback(&store, &journal).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn staging_cleanup_removes_recorded_bytes_and_preserves_every_other_entry() {
        let (_profile, store, target) = profile_with_book();
        let root = target.join("archives");
        let mut journal = committed_cleanup_journal(&store, &root);
        let staging = root.join(&journal.staging);
        fs::create_dir(&staging).unwrap();
        let dir = crate::storage::open_directory_nofollow(&staging).unwrap();
        journal.staging_directories.insert(String::new(), directory_identity(&dir).unwrap());
        drop(dir);
        let part = &mut journal.entries[0].parts[0];
        fs::copy(root.join(&part.name), staging.join(&part.name)).unwrap();
        part.staging_identity = Some(file_identity(&staging.join(&part.name)).unwrap());
        fs::write(staging.join("unrecorded.zim"), b"keep my file").unwrap();
        remove_staging(&root, &journal).unwrap();
        assert!(!staging.join("small.zim").exists());
        assert_eq!(fs::read(staging.join("unrecorded.zim")).unwrap(), b"keep my file");
        fs::rename(&staging, root.join("previous-stage")).unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("small.zim"), b"replacement directory").unwrap();
        remove_staging(&root, &journal).unwrap();
        assert_eq!(fs::read(staging.join("small.zim")).unwrap(), b"replacement directory");
    }

    #[test]
    fn removal_resumes_only_a_captured_original_and_preserves_a_new_arrival() {
        let (_profile, store, target) = profile_with_book();
        let root = target.join("archives");
        let journal = committed_cleanup_journal(&store, &root);
        let part = &journal.entries[0].parts[0];
        let owned = part.source_identity.as_ref().unwrap();
        let original = Path::new(&journal.old_root).join(&part.name);
        let captured = original.with_file_name(format!(".kiwix-retired-{}-{:x}-{:x}", journal.id, owned.device, owned.inode));
        fs::write(&captured, b"unrelated file").unwrap();
        assert!(remove_recorded_part(&original, &journal.id, part, Cleanup::Original).is_err());
        assert_eq!(fs::read(&captured).unwrap(), b"unrelated file");
        fs::remove_file(&captured).unwrap();
        fs::rename(&original, &captured).unwrap();
        fs::write(&original, b"new arrival").unwrap();
        assert!(remove_recorded_part(&original, &journal.id, part, Cleanup::Original).unwrap());
        assert!(!captured.exists());
        assert_eq!(fs::read(&original).unwrap(), b"new arrival");
    }

    #[test]
    fn full_volume_during_copy_reclaims_only_the_reserved_partial_after_restart() {
        let (_profile, store, target) = profile_with_book();
        let original_book = store.read().unwrap().books[0].clone();
        let original_root = configured_root(&store).unwrap();
        let root = target.join("archives");
        let mut journal = committed_cleanup_journal(&store, &root);
        store.update(|state| {
            state.books[0] = original_book.clone();
            state.settings.download_directory = original_root.to_string_lossy().into_owned();
            Ok(())
        }).unwrap();
        journal.phase = Phase::Copying;
        journal.marker_directory = Some(prepare_target(&store, &root).unwrap());
        let part = &mut journal.entries[0].parts[0];
        part.sha256 = None;
        part.publication_identity = None;
        fs::write(root.join(&part.name), b"unrelated destination").unwrap();
        let (_, staged) = reserve_staging(&store, &root, &mut journal).unwrap();
        let part = &journal.entries[0].parts[0];
        let source = fs::read(&original_book.path).unwrap();
        let mut input = File::open(&original_book.path).unwrap();
        let mut output = open_staged_output(&staged, part).unwrap();
        let error = copy_descriptor(&mut input, &mut output, part.size, |file, bytes| {
            file.write_all(&bytes[..8])?;
            Err(std::io::ErrorKind::StorageFull.into())
        }).unwrap_err();
        assert!(error.contains("Writing staged archive part"), "{error}");
        drop(output);
        let partial = root.join(&journal.staging).join(&part.name);
        assert_eq!(fs::read(&partial).unwrap(), source[..8]);
        drop(staged);
        drop(journal);
        assert!(recover(&store).unwrap().unwrap().contains("rolled back"));
        assert!(!partial.exists());
        assert_eq!(fs::read(root.join("small.zim")).unwrap(), b"unrelated destination");
        assert_eq!(fs::read(&original_book.path).unwrap(), source);
        assert_eq!(store.read().unwrap().books[0], original_book);
    }

    #[test]
    fn selected_copy_and_move_preserve_identity_and_leave_other_archives_and_root_unchanged() {
        for remove_old in [false, true] {
            let (_profile, store, target) = profile_with_book();
            let source = store.read().unwrap().books[0].clone();
            let other_path = store.data_dir().join("archives/other.zim");
            fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media.zim"), &other_path).unwrap();
            let other = book_at(&other_path);
            store.update(|state| {
                state.books.push(other.clone());
                state.reader_storage.insert(source.installation_id.clone(), json!({"saved": "reader state"}));
                Ok(())
            }).unwrap();
            let root = store.settings().unwrap().download_directory;
            let result = relocate_selected(&store, &target.join("archives"), std::slice::from_ref(&source.metadata.id),
                remove_old, &native_for(&target), &Activity::default()).unwrap();
            assert_eq!(result["copied"], 1);
            assert_eq!(store.settings().unwrap().download_directory, root);
            let copied = store.book(&source.metadata.id).unwrap().unwrap();
            assert_eq!(copied.installation_id, source.installation_id);
            assert_eq!(store.read().unwrap().reader_storage.get(&source.installation_id), Some(&json!({"saved": "reader state"})));
            assert_eq!(store.book(&other.metadata.id).unwrap().unwrap(), other);
            assert!(Path::new(&copied.path).starts_with(target.join("archives")));
            assert!(Path::new(&copied.path).exists());
            assert_eq!(Path::new(&source.path).exists(), !remove_old);
            assert!(!journal_path(&store).exists());
        }
    }

    #[test]
    fn selected_recovery_uses_installation_state_not_the_unchanged_download_root() {
        let (_profile, store, target) = profile_with_book();
        let original = store.read().unwrap().books[0].clone();
        let root = store.settings().unwrap().download_directory;
        let mut journal = committed_cleanup_journal(&store, &target.join("archives"));
        journal.selection = Some(vec![SelectedSource { book: original.clone(), delete_receipt: None }]);
        store.update(|state| { state.settings.download_directory = root.clone(); Ok(()) }).unwrap();
        write_journal(&store, &journal).unwrap();
        recover(&store).unwrap();
        assert!(!Path::new(&original.path).exists());
        assert!(target.join("archives/small.zim").exists());
        assert_eq!(store.settings().unwrap().download_directory, root);
    }

    #[test]
    fn cleanup_validates_every_replacement_before_removing_any_source() {
        let (_profile, store, target) = profile_with_book();
        let new_root = target.join("archives");
        let mut journal = committed_cleanup_journal(&store, &new_root);
        let other_path = store.data_dir().join("archives/other.zim");
        fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media.zim"), &other_path).unwrap();
        let mut other = book_at(&other_path);
        other.path = new_root.join("other.zim").to_string_lossy().into_owned();
        store.update(|state| { state.books.push(other.clone()); Ok(()) }).unwrap();
        journal.entries.push(JournalEntry {
            book_id: other.metadata.id, title: "Other".into(), primary: "other.zim".into(), relative: vec![],
            parts: vec![JournalPart {
                name: "other.zim".into(), size: fs::metadata(&other_path).unwrap().len(), sha256: None,
                staging_identity: None, source_identity: Some(file_identity(&other_path).unwrap()), publication_identity: None,
            }],
        });
        write_journal(&store, &journal).unwrap();
        assert!(recover(&store).is_err());
        assert!(store.data_dir().join("archives/small.zim").exists());
        assert!(other_path.exists());
        assert!(journal_path(&store).exists());
    }

    #[test]
    fn cleanup_preserves_replaced_source_even_with_identical_content() {
        let (_profile, store, target) = profile_with_book();
        let journal = committed_cleanup_journal(&store, &target.join("archives"));
        let old = store.data_dir().join("archives/small.zim");
        fs::rename(&old, old.with_extension("saved")).unwrap();
        fs::copy(target.join("archives/small.zim"), &old).unwrap();
        write_journal(&store, &journal).unwrap();
        assert!(recover(&store).is_err());
        assert!(old.exists());
        assert!(journal_path(&store).exists());
    }

    #[cfg(not(windows))]
    #[test]
    fn root_postcommit_sync_error_never_rolls_back_published_archives() {
        let (_profile, store, target) = profile_with_book();
        let data = store.data_dir().to_path_buf();
        let source = data.join("archives/small.zim");
        let destination = target.join("archives/small.zim");
        crate::store::fail_next_directory_sync_for_test();
        assert!(switch(&store, &target.join("archives"), true, &native_for(&target), &Activity::default()).is_err());
        assert!(source.exists());
        assert!(destination.exists());
        assert!(journal_path(&store).exists());
        assert_eq!(store.read().unwrap().books[0].path, destination.to_string_lossy());
        drop(store);
        let reopened = Store::new(data).unwrap();
        recover(&reopened).unwrap();
        assert!(!source.exists());
        assert!(destination.exists());
        assert!(!journal_path(&reopened).exists());
    }

    #[cfg(not(windows))]
    #[test]
    fn selected_postcommit_sync_error_retains_both_copies_and_installation_until_recovery() {
        let (_profile, store, target) = profile_with_book();
        let data = store.data_dir().to_path_buf();
        let source = store.read().unwrap().books[0].clone();
        let download_root = store.settings().unwrap().download_directory;
        crate::store::fail_next_directory_sync_for_test();
        assert!(relocate_selected(&store, &target.join("archives"), std::slice::from_ref(&source.metadata.id),
            true, &native_for(&target), &Activity::default()).is_err());
        let replacement = store.book(&source.metadata.id).unwrap().unwrap();
        assert_eq!(replacement.installation_id, source.installation_id);
        assert_ne!(replacement.path, source.path);
        assert!(Path::new(&source.path).exists());
        assert!(Path::new(&replacement.path).exists());
        assert!(journal_path(&store).exists());
        assert_eq!(store.settings().unwrap().download_directory, download_root);
        drop(store);
        let reopened = Store::new(data).unwrap();
        recover(&reopened).unwrap();
        assert!(!Path::new(&source.path).exists());
        assert!(Path::new(&replacement.path).exists());
        assert_eq!(reopened.settings().unwrap().download_directory, download_root);
        assert!(!journal_path(&reopened).exists());
    }
    #[cfg(unix)]
    #[test]
    fn hardlinked_selected_sources_refuse_relocation_without_changing_either_link() {
        let (_profile, store, target) = profile_with_book();
        let source = store.read().unwrap().books[0].clone();
        let saved = Path::new(&source.path).with_extension("saved");
        fs::hard_link(&source.path, &saved).unwrap();
        let original = fs::read(&source.path).unwrap();
        for remove_old in [false, true] {
            assert!(relocate_selected(&store, &target.join("archives"), std::slice::from_ref(&source.metadata.id),
                remove_old, &native_for(&target), &Activity::default()).is_err());
            assert_eq!(store.book(&source.metadata.id).unwrap().unwrap(), source);
            assert_eq!(fs::read(&source.path).unwrap(), original);
            assert_eq!(fs::read(&saved).unwrap(), original);
            assert!(!journal_path(&store).exists());
        }
    }

    #[test]
    fn publication_primitive_never_overwrites_a_late_collision() {
        let temp = Temp::new("exclusive-publication");
        let staging = temp.0.join("staged");
        fs::create_dir(&staging).unwrap();
        let source = staging.join("existing.zim");
        let destination = temp.0.join("existing.zim");
        fs::write(&source, b"staged").unwrap();
        fs::write(&destination, b"existing").unwrap();
        let from = crate::storage::open_directory_nofollow(&staging).unwrap();
        let to = crate::storage::open_directory_nofollow(&temp.0).unwrap();
        assert!(publish_exclusive(&from, &to, "existing.zim").is_err());
        assert_eq!(fs::read(&source).unwrap(), b"staged");
        assert_eq!(fs::read(&destination).unwrap(), b"existing");
    }

    #[cfg(unix)]
    #[test]
    fn publication_remains_in_the_open_folder_when_the_visible_parent_is_replaced() {
        let temp = Temp::new("moved-publication");
        let staging = temp.0.join("stage");
        let target = temp.0.join("target");
        let outside = temp.0.join("other-storage");
        for directory in [&staging, &target, &outside] { fs::create_dir(directory).unwrap(); }
        let source = crate::storage::open_directory_nofollow(&staging).unwrap();
        let root = crate::storage::open_directory_nofollow(&target).unwrap();
        let relative = vec!["installation".to_owned()];
        let parent = destination_parent(&root, &relative, true).unwrap();
        fs::write(staging.join("small.zim"), b"staged contents").unwrap();
        fs::rename(target.join("installation"), target.join("previous")).unwrap();
        std::os::unix::fs::symlink(&outside, target.join("installation")).unwrap();
        publish_exclusive(&source, &parent, "small.zim").unwrap();
        assert_eq!(fs::read(target.join("previous/small.zim")).unwrap(), b"staged contents");
        assert!(!outside.join("small.zim").exists());
        assert!(destination_parent(&root, &relative, false).is_err());
    }

    fn write_named_fixture(directory: &Path, names: &[&str]) -> Vec<PlannedPart> {
        fs::create_dir_all(directory).unwrap();
        let bytes = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
        let mut parts = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let start = index * bytes.len() / names.len();
            let end = (index + 1) * bytes.len() / names.len();
            fs::write(directory.join(name), &bytes[start..end]).unwrap();
            parts.push(PlannedPart { name: (*name).into(), size: (end - start) as u64 });
        }
        parts
    }

    #[test]
    fn selected_copy_preserves_uppercase_and_mixed_case_archive_cohorts() {
        for names in [vec!["BOOK.ZIM"], vec!["BOOK.ZIMAA", "BOOK.ZIMAB"], vec!["Book.ZimAa", "book.zImAb"]] {
            let (_profile, store, target) = profile_with_book();
            let mut source = store.read().unwrap().books[0].clone();
            let directory = store.data_dir().join("archives");
            write_named_fixture(&directory, &names);
            source.path = directory.join(names[0]).to_string_lossy().into_owned();
            store.update(|state| { state.books[0] = source.clone(); Ok(()) }).unwrap();
            let result = relocate_selected(&store, &target.join("archives"), &[source.metadata.id.clone()],
                false, &native_for(&target), &Activity::default()).unwrap();
            assert!(result["cleanupPending"].is_null());
            assert!(!journal_path(&store).exists());
            let replacement = store.book(&source.metadata.id).unwrap().unwrap();
            assert_eq!(replacement.installation_id, source.installation_id);
            let retained = crate::archive_source::retain(&replacement).unwrap();
            assert_eq!(retained.files().iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), names);
            assert!(retained.open().unwrap().check_integrity().unwrap());
            for name in names { assert!(directory.join(name).exists()); }
        }
    }

    #[test]
    fn committed_provider_copy_recovery_completes_with_uppercase_and_mixed_case_parts() {
        for names in [vec!["BOOK.ZIM"], vec!["BOOK.ZIMAA", "BOOK.ZIMAB"], vec!["Book.ZimAa", "book.zImAb"]] {
            let (profile, store, target) = profile_with_book();
            let mut provider = store.read().unwrap().books[0].clone();
            let provider_directory = profile.0.join("provider-originals");
            let parts = write_named_fixture(&provider_directory, &names);
            let reference = crate::provider_reference::ProviderReference {
                reference: Uuid::new_v4().to_string(), display_name: names[0].into(), persistent: true,
            };
            provider.path = reference.path_marker();
            provider.managed = false;
            provider.source_reference = Some(reference);
            let new_root = target.join("archives");
            let destination_id = prepare_target(&store, &new_root).unwrap();
            adopt_committed_root(&new_root, &destination_id, &profile_identity(&store).unwrap()).unwrap();
            let relative = vec![provider.installation_id.clone()];
            let destination = new_root.join(relative_path(&relative));
            let mut recorded = Vec::new();
            for part in &parts {
                let mut input = File::open(provider_directory.join(&part.name)).unwrap();
                let path = destination.join(&part.name);
                fs::create_dir_all(&destination).unwrap();
                let mut output = File::create_new(&path).unwrap();
                let hash = copy_descriptor(&mut input, &mut output, part.size, File::write_all).unwrap();
                let identity = file_identity(&path).unwrap();
                recorded.push(JournalPart {
                    name: part.name.clone(), size: part.size, sha256: Some(hash),
                    staging_identity: None, source_identity: None, publication_identity: Some(identity),
                });
            }
            let dir = crate::storage::open_directory_nofollow(&destination).unwrap();
            verify_archive(&dir, &destination, &provider.metadata.id, &recorded).unwrap();
            let mut replacement = provider.clone();
            replacement.path = destination.join(names[0]).to_string_lossy().into_owned();
            replacement.managed = true;
            replacement.source_reference = None;
            store.update(|state| { state.books[0] = replacement.clone(); Ok(()) }).unwrap();
            let id = Uuid::new_v4().to_string();
            let journal = Journal {
                version: JOURNAL_VERSION, staging: format!(".kiwix-library-root-{id}"), id,
                profile: profile_identity(&store).unwrap(), old_root: configured_root(&store).unwrap().to_string_lossy().into_owned(),
                new_root: new_root.to_string_lossy().into_owned(), remove_old: false,
                _legacy_marker_created: false, marker_directory: None, staging_directories: Default::default(), phase: Phase::Publishing,
                selection: Some(vec![SelectedSource { book: provider.clone(), delete_receipt: None }]),
                entries: vec![JournalEntry {
                    book_id: provider.metadata.id.clone(), title: provider.metadata.title.clone(),
                    primary: names[0].into(), relative, parts: recorded,
                }],
            };
            write_journal(&store, &journal).unwrap();
            recover(&store).unwrap();
            assert!(!journal_path(&store).exists());
            assert_eq!(store.book(&provider.metadata.id).unwrap().unwrap(), replacement);
            assert!(crate::archive_source::retain(&replacement).unwrap().open().unwrap().check_integrity().unwrap());
            for name in names { assert!(provider_directory.join(name).exists()); }
        }
    }
}
