// SPDX-License-Identifier: GPL-3.0-or-later
//! Owned partial-file identity, resume verification and crash recovery for map composition.
#![cfg_attr(unix, forbid(unsafe_code))]
#[allow(unused_imports)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
}

impl FileIdentity {
    pub(super) fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OwnedPartial {
    pub(super) destination: PathBuf,
    pub(super) directory_identity: FileIdentity,
    pub(super) name: String,
    // None is a durable reservation, not ownership of whatever occupies its name.
    // No composition bytes may be written until the actual inode is checkpointed.
    pub(super) identity: Option<FileIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) capture: Option<PartialCapture>,
    // A publication checkpoint precedes rename. Recovery may reclaim an owned
    // inode still at its partial name, but never removes a committed output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) publication: Option<PublicationRecovery>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PartialCapture {
    pub(super) name: String,
    pub(super) identity: FileIdentity,
    pub(super) size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublicationRecovery {
    pub(super) name: String,
    pub(super) size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MapResume {
    pub(super) recipe: String,
    pub(super) written: u64,
    pub(super) prefix_sha256: String,
    pub(super) blocks: Vec<u8>,
    #[serde(default)]
    pub(super) invalid: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublishedMap {
    pub(super) record: OwnedPartial,
    pub(super) size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RetainedMap {
    pub(super) id: String,
    pub(super) region_id: String,
    pub(super) language: String,
    pub(super) published: PublishedMap,
}

/// Cleanup-only rows: never enqueue these predecessor paths for import.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetainedMapJob {
    pub id: String,
    pub region_id: String,
    pub language: String,
    pub status: &'static str,
    /// Map recipes have no single source URL.
    pub url: &'static str,
    pub file_name: String,
    pub destination: String,
    pub completed_paths: Vec<String>,
    pub total: u64,
    pub written: u64,
    pub downloaded: u64,
    pub error: Option<String>,
    pub speed: u64,
    pub eta_seconds: Option<u64>,
    pub transport: &'static str,
    pub can_remove_from_list: bool,
}

pub(super) struct MapDirectory {
    pub(super) path: PathBuf,
    pub(super) directory: Dir,
    pub(super) identity: FileIdentity,
}

impl MapDirectory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let directory = crate::storage::open_directory_nofollow(path)?;
        let identity = FileIdentity::of(&directory.dir_metadata()?);
        Ok(Self {
            path: path.to_path_buf(),
            directory,
            identity,
        })
    }

    pub(super) fn unchanged(&self) -> io::Result<()> {
        if Self::open(&self.path)?.identity != self.identity {
            return Err(io::Error::other(
                "Map destination directory changed identity",
            ));
        }
        Ok(())
    }
}

pub(super) fn partial_name(name: &str) -> bool {
    name.strip_prefix(".streetzim-")
        .and_then(|name| name.strip_suffix(".part"))
        .is_some_and(|id| {
            Uuid::parse_str(id).is_ok_and(|id| name == format!(".streetzim-{id}.part"))
        })
}

pub(super) fn open_map_checkpoint(record: &OwnedPartial, written: u64) -> Result<fs::File, String> {
    if !partial_name(&record.name) || record.capture.is_some() || record.publication.is_some() {
        return Err("Map checkpoint has no resumable private output".into());
    }
    let directory = MapDirectory::open(&record.destination).map_err(|error| error.to_string())?;
    if directory.identity != record.directory_identity {
        return Err("Map checkpoint directory changed; no bytes were modified".into());
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).follow(FollowSymlinks::No);
    let file = directory.directory.open_with(&record.name, &options).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() < written
        || record.identity.as_ref() != Some(&FileIdentity::of(&metadata)) {
        return Err("Map checkpoint file changed identity or length; no bytes were modified".into());
    }
    directory.unchanged().map_err(|error| error.to_string())?;
    Ok(file.into_std())
}

pub(super) struct ResumeVerificationFailure { pub(super) message: String, pub(super) invalid: bool }
impl ResumeVerificationFailure {
    pub(super) fn invalid(message: &str) -> Self { Self { message: message.into(), invalid: true } }
}
impl From<String> for ResumeVerificationFailure {
    fn from(message: String) -> Self { Self { message, invalid: false } }
}

/// Bytes to read next. `remaining as usize` truncates a 4 GiB multiple to zero
/// on 32-bit targets and would spin the read loop, so narrow after clamping.
fn next_count(remaining: u64, buffer: usize) -> usize {
    remaining.min(buffer as u64) as usize
}

pub(super) fn verify_map_prefix(
    mut file: fs::File, checkpoint: &MapResume, record: &OwnedPartial,
    store: &crate::store::Store, control: &Control,
) -> Result<(fs::File, md5::Context, Sha256), ResumeVerificationFailure> {
    use std::io::{Read, Seek};
    file.seek(io::SeekFrom::Start(0)).map_err(|error| error.to_string())?;
    let mut md5 = md5::Context::new();
    let mut sha256 = Sha256::new();
    let mut remaining = checkpoint.written;
    let mut buffer = [0u8; 64 * 1024];
    while remaining != 0 {
        control.check()?;
        let count = next_count(remaining, buffer.len());
        file.read_exact(&mut buffer[..count]).map_err(|error| error.to_string())?;
        md5.consume(&buffer[..count]);
        sha256.update(&buffer[..count]);
        remaining -= count as u64;
    }
    if hex::encode(sha256.clone().finalize()) != checkpoint.prefix_sha256 {
        return Err(ResumeVerificationFailure::invalid("Saved map prefix failed its checksum; delete the retained download or get the map again. No bytes were modified"));
    }
    control.check()?;
    // Only the proven checkpoint prefix is reusable. A power loss may leave
    // trailing writes which were never durably acknowledged by the journal.
    let pinned = cap_std::fs::File::from_std(file);
    let metadata = pinned.metadata().map_err(|error| error.to_string())?;
    let directory = MapDirectory::open(&record.destination).map_err(|error| error.to_string())?;
    let ambient = directory.directory.symlink_metadata(&record.name).map_err(|error| error.to_string())?;
    if directory.identity != record.directory_identity || !ambient.is_file() || ambient.is_symlink()
        || metadata.nlink() != 1 || ambient.nlink() != 1
        || record.identity.as_ref() != Some(&FileIdentity::of(&metadata))
        || record.identity.as_ref() != Some(&FileIdentity::of(&ambient)) {
        return Err(ResumeVerificationFailure::invalid("Map checkpoint moved or changed identity during verification; no bytes were modified"));
    }
    directory.unchanged().map_err(|error| error.to_string())?;
    let mut file = pinned.into_std();
    store.with_acquisition_destination(&directory.path.join(&record.name), || {
        file.set_len(checkpoint.written).map_err(|error| error.to_string())?;
        file.seek(io::SeekFrom::Start(checkpoint.written)).map_err(|error| error.to_string())?;
        Ok(())
    })?;
    Ok((file, md5, sha256))
}

pub(super) fn map_file_sha256(file: &fs::File, size: u64) -> Result<[u8; 32], String> {
    use std::io::{Read, Seek};
    let mut reader = file;
    reader.seek(io::SeekFrom::Start(0)).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut remaining = size;
    let mut buffer = [0u8; 64 * 1024];
    while remaining != 0 {
        let count = next_count(remaining, buffer.len());
        reader.read_exact(&mut buffer[..count]).map_err(|error| error.to_string())?;
        hash.update(&buffer[..count]);
        remaining -= count as u64;
    }
    let mut extra = [0u8; 1];
    if reader.read(&mut extra).map_err(|error| error.to_string())? != 0 {
        return Err("Map file length changed during ownership verification".into());
    }
    Ok(hash.finalize().into())
}

/// Resolve a publication intent without discarding an otherwise resumable
/// partial. A failed exclusive rename is not permission to erase its prefix.
pub(super) fn recover_resume_publication(record: &mut OwnedPartial, written: u64) -> Result<Option<PublishedMap>, String> {
    let Some(publication) = record.publication.clone() else { return Ok(None) };
    let name = Path::new(&publication.name);
    if name.file_name() != Some(name.as_os_str())
        || !matches!(name.components().next(), Some(std::path::Component::Normal(_))) {
        return Err("Map publication checkpoint has an invalid filename".into());
    }
    let directory = MapDirectory::open(&record.destination).map_err(|error| error.to_string())?;
    if directory.identity != record.directory_identity {
        return Err("Map publication directory changed; retained entries were not modified".into());
    }
    if let Ok(metadata) = directory.directory.symlink_metadata(name) {
        if metadata.is_file() && !metadata.is_symlink() && metadata.nlink() == 1
            && metadata.len() == publication.size
            && record.identity.as_ref() == Some(&FileIdentity::of(&metadata)) {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let file = directory.directory.open_with(name, &options).map_err(|error| error.to_string())?;
            let verified = crate::publication::VerifiedFile::new(file.into_std(), publication.size)
                .map_err(|error| error.to_string())?;
            if !verified.matches_in(&directory.directory, name).map_err(|error| error.to_string())? {
                return Err("Published map changed identity during recovery; no bytes were modified".into());
            }
            directory.unchanged().map_err(|error| error.to_string())?;
            crate::mobile_backup::archive_policy(verified.file(), &directory.path.join(name))?;
            let mut published = record.clone();
            published.name = publication.name;
            published.publication = None;
            return Ok(Some(PublishedMap { record: published, size: publication.size, sha256: publication.sha256 }));
        }
    }
    let mut partial = record.clone();
    partial.publication = None;
    open_map_checkpoint(&partial, written)?;
    record.publication = None;
    Ok(None)
}

#[derive(Debug)]
pub(super) enum PartialRecovery {
    Reclaimed,
    Published(PathBuf),
    Preserved,
    /// Declined on ownership grounds that no retry changes: the journal and
    /// the journaled directory disagree, so nothing was touched (or what was
    /// captured has been put back). Carries the detail for callers that report
    /// it and for the log; like `Preserved`, it means "keep the record".
    Refused(String),
}

/// Capture a public entry before inspecting it for deletion. The checkpoint
/// makes the private capability's identity recoverable across interruption.
pub(super) fn reclaim_partial(
    record: &mut OwnedPartial,
    checkpoint: impl FnMut(&OwnedPartial) -> Result<(), String>,
) -> Result<PartialRecovery, String> {
    reclaim_partial_with_capture(record, checkpoint, capture_partial)
}

pub(super) fn reclaim_partial_with_capture(
    record: &mut OwnedPartial,
    checkpoint: impl FnMut(&OwnedPartial) -> Result<(), String>,
    capture: impl FnMut(&Dir, &str, &Dir, &str) -> Result<(), String>,
) -> Result<PartialRecovery, String> {
    reclaim_owned_file(record, checkpoint, capture, false, None, None)
}

pub(super) fn reclaim_owned_file(
    record: &mut OwnedPartial,
    mut checkpoint: impl FnMut(&OwnedPartial) -> Result<(), String>,
    mut capture: impl FnMut(&Dir, &str, &Dir, &str) -> Result<(), String>,
    published: bool,
    expected_sha256: Option<[u8; 32]>,
    store: Option<&crate::store::Store>,
) -> Result<PartialRecovery, String> {
    let final_name = Path::new(&record.name);
    let allowed_final = published && record.name.ends_with(".zim") && !record.name.starts_with('.')
        && final_name.file_name() == Some(final_name.as_os_str())
        && matches!(final_name.components().next(), Some(std::path::Component::Normal(_)));
    if !partial_name(&record.name) && !allowed_final {
        return Ok(PartialRecovery::Refused("This partial map download has an unexpected name, so Kiwix left it alone. Delete the map download, then start it again".into()));
    }
    let directory = MapDirectory::open(&record.destination).map_err(|e| e.to_string())?;
    if directory.identity != record.directory_identity {
        return Err(
            "This partial map download was moved or changed, so Kiwix kept it. Delete the map download, then start it again".into()
        );
    }
    let Some(identity) = record.identity.as_ref() else {
        // Ownership was never proven. Such a record must not block map
        // downloads forever once every file it named is gone: record that
        // conclusion instead of keeping an unprovable reservation alive.
        for name in std::iter::once(record.name.as_str()).chain(
            record.publication.as_ref().map(|publication| publication.name.as_str()),
        ) {
            match directory.directory.symlink_metadata(name) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
                Ok(_) => return Ok(PartialRecovery::Preserved),
            }
        }
        return Ok(PartialRecovery::Reclaimed);
    };
    if let Some(publication) = &record.publication {
        let name = Path::new(&publication.name);
        if name.file_name() != Some(name.as_os_str())
            || !matches!(
                name.components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Ok(PartialRecovery::Refused("Invalid saved map publication filename; nothing was removed".into()));
        }
        let matches_owned = |name: &str| -> Result<bool, String> {
            match directory.directory.symlink_metadata(name) {
                Ok(metadata) => Ok(metadata.is_file()
                    && !metadata.is_symlink()
                    && metadata.nlink() == 1
                    && &FileIdentity::of(&metadata) == identity
                    && metadata.len() == publication.size),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.to_string()),
            }
        };
        if matches_owned(&publication.name)? {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let file = directory
                .directory
                .open_with(&publication.name, &options)
                .map_err(|e| e.to_string())?;
            if &FileIdentity::of(&file.metadata().map_err(|e| e.to_string())?) != identity {
                return Err("Recovered map output changed identity; entry was preserved".into());
            }
            let verified = crate::publication::VerifiedFile::new(file.into_std(), publication.size)
                .map_err(|e| e.to_string())?;
            if !verified
                .matches_in(&directory.directory, name)
                .map_err(|e| e.to_string())?
            {
                return Err("Recovered map output changed identity; entry was preserved".into());
            }
            directory.unchanged().map_err(|e| e.to_string())?;
            crate::mobile_backup::archive_policy(verified.file(), &directory.path.join(&publication.name))?;
            return Ok(PartialRecovery::Published(
                directory.path.join(&publication.name),
            ));
        }
        if !matches_owned(&record.name)? {
            return Ok(PartialRecovery::Refused(format!(
                "Map publication remains unresolved at {} (directory {:?}, file {:?}, size {}); all entries were preserved",
                directory.path.join(&publication.name).display(),
                record.directory_identity,
                record.identity,
                publication.size
            )));
        }
        // The verified inode still has its partial name: the exclusive rename
        // never committed. Persist that conclusion before ordinary owned cleanup.
        record.publication = None;
        checkpoint(record)?;
    }
    if record.capture.is_none() {
        let metadata = match directory.directory.symlink_metadata(&record.name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(PartialRecovery::Reclaimed)
            }
            Err(error) => return Err(error.to_string()),
        };
        // A second link means the bytes would outlive our name: not ours alone.
        if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1
            || &FileIdentity::of(&metadata) != identity
        {
            return Ok(PartialRecovery::Preserved);
        }
        let name = format!(".streetzim-cleanup-{}", Uuid::new_v4());
        #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
        let mut builder = cap_std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use cap_std::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        directory
            .directory
            .create_dir_with(&name, &builder)
            .map_err(|e| e.to_string())?;
        let staging = directory
            .directory
            .open_dir_nofollow(&name)
            .map_err(|e| e.to_string())?;
        record.capture = Some(PartialCapture {
            name,
            identity: FileIdentity::of(&staging.dir_metadata().map_err(|e| e.to_string())?),
            size: metadata.len(),
        });
        sync_map_directory(&directory.directory)?;
        checkpoint(record)?;
    }
    let saved = record
        .capture
        .as_ref()
        .ok_or("Missing map cleanup capability")?;
    // Validate the single exact journaled name, never enumerate matching prefixes.
    if !saved
        .name
        .strip_prefix(".streetzim-cleanup-")
        .is_some_and(|id| {
            Uuid::parse_str(id).is_ok_and(|id| saved.name == format!(".streetzim-cleanup-{id}"))
        })
    {
        return Ok(PartialRecovery::Refused("Invalid map cleanup capability name".into()));
    }
    let staging = match directory.directory.open_dir_nofollow(&saved.name) {
        Ok(staging) => staging,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return match directory.directory.symlink_metadata(&record.name) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Ok(PartialRecovery::Reclaimed)
                }
                _ => Ok(PartialRecovery::Refused(
                    "Map cleanup capability disappeared; remaining entries were preserved".into(),
                )),
            };
        }
        Err(error) => return Err(error.to_string()),
    };
    if FileIdentity::of(&staging.dir_metadata().map_err(|e| e.to_string())?) != saved.identity {
        return Ok(PartialRecovery::Refused("Map cleanup capability was replaced; nothing was removed".into()));
    }
    // Both successful paths end by removing the (now empty) private capability;
    // that removal runs below, after this closure, so Windows can release it.
    let mut refused = false;
    let result: Result<(), String> = (|| {
        // The exclusive move between the public directory and the capability,
        // in either direction, under the library guard when one is supplied.
        let mut exclusive_move = |from: &Dir, to: &Dir| -> Result<(), String> {
            if let Some(store) = store {
                let paths = [directory.path.join(&record.name), directory.path.join(&saved.name).join(&record.name)];
                store.with_acquisition_destinations(&paths, || capture(from, &record.name, to, &record.name))
            } else {
                capture(from, &record.name, to, &record.name)
            }
        };
        // On a filesystem without an exclusive rename the move links the new
        // name before it retires the old one. Interrupted in between, the
        // captured entry is one of two names for the file: the public name, or
        // a private name the move left beside it. Put it back through the same
        // primitive, which finishes such a move, and capture it afresh below.
        // That leaves one name, so the unlink further down frees the bytes
        // instead of leaving them under the other name.
        if captured_under_two_names(&staging, &directory.directory, &record.name)? {
            directory.unchanged().map_err(|e| e.to_string())?;
            exclusive_move(&staging, &directory.directory)?;
        }
        if !map_entry_exists(&staging, &record.name)? {
            if !map_entry_exists(&directory.directory, &record.name)? {
                return Ok(());
            }
            directory.unchanged().map_err(|e| e.to_string())?;
            exclusive_move(&directory.directory, &staging)?;
        }
        sync_map_directory(&staging)?;
        sync_map_directory(&directory.directory)?;
        let metadata = staging
            .symlink_metadata(&record.name)
            .map_err(|e| e.to_string())?;
        // Exactly one name, published or not: unlinking one of two would
        // report the bytes reclaimed while they stay reachable elsewhere.
        if !metadata.is_file()
            || metadata.is_symlink()
            || metadata.nlink() != 1
            || Some(&FileIdentity::of(&metadata)) != record.identity.as_ref()
            || metadata.len() != saved.size
        {
            refused = true;
            return Err(
                "Captured map partial changed identity or size; entry was preserved".into(),
            );
        }
        if let Some(expected) = expected_sha256 {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let captured = staging.open_with(&record.name, &options).map_err(|error| error.to_string())?;
            if map_file_sha256(&captured.into_std(), saved.size)? != expected {
                return Err("Captured map contents changed; no bytes were deleted".into());
            }
        }
        // Only the entry inside our recorded private capability may be unlinked.
        let unlink = || staging.remove_file(&record.name).map_err(|error| error.to_string());
        if let Some(store) = store {
            let paths = [directory.path.join(&record.name), directory.path.join(&saved.name).join(&record.name)];
            store.with_acquisition_destinations(&paths, unlink)?;
        } else { unlink()?; }
        sync_map_directory(&staging)?;
        Ok(())
    })();
    let mut staging = Some(staging);
    let result = result.and_then(|()| {
        remove_cleanup_capability(&directory.directory, &saved.name, &mut staging, &saved.identity)?;
        sync_map_directory(&directory.directory)?;
        Ok(PartialRecovery::Reclaimed)
    });
    if let Err(error) = result {
        // Only Windows releases the capability (to remove it). Reacquire it by
        // its exact journaled name and identity; a substitute is never touched.
        let staging = match staging {
            Some(staging) => Some(staging),
            None => reopen_cleanup_capability(&directory.directory, &saved.name, &saved.identity),
        };
        let restoration = match &staging {
            Some(staging) if map_entry_exists(staging, &record.name)? => {
                capture_partial(staging, &record.name, &directory.directory, &record.name)
                    .err()
                    .unwrap_or_default()
            }
            _ => String::new(),
        };
        if let Some(staging) = &staging {
            let _ = sync_map_directory(staging);
        }
        let _ = sync_map_directory(&directory.directory);
        // What was captured is not provably ours alone and is back at its
        // name: an ownership refusal, which trying again does not change.
        if refused && restoration.is_empty() {
            return Ok(PartialRecovery::Refused(error));
        }
        return Err(format!(
            "{error}. Retained entries are recoverable at {} (directory {:?}). {restoration}",
            directory.path.join(&saved.name).display(),
            saved.identity
        ));
    }
    result
}

/// True when the captured entry has a second link and the public name is
/// either free or already that same file: the two states an interrupted
/// link-then-unlink move can leave, and the only ones a put-back can settle.
fn captured_under_two_names(staging: &Dir, public: &Dir, name: &str) -> Result<bool, String> {
    let captured = match staging.symlink_metadata(name) {
        Ok(captured) => captured,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if !captured.is_file() || captured.nlink() < 2 {
        return Ok(false);
    }
    match public.symlink_metadata(name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Ok(public) => Ok(FileIdentity::of(&public) == FileIdentity::of(&captured)),
        Err(error) => Err(error.to_string()),
    }
}

/// Reuse the exact owned-publication capture journal for current and retained
/// downloads. Library paths remain guarded at capture and again before unlink.
pub(super) fn reclaim_published_map(
    published: &mut PublishedMap,
    store: &crate::store::Store,
    checkpoint: impl FnMut(&OwnedPartial) -> Result<(), String>,
) -> Result<(), String> {
    let path = published.record.destination.join(&published.record.name);
    store.with_acquisition_destination(&path, || Ok(()))?;
    let directory = MapDirectory::open(&published.record.destination).map_err(|error| error.to_string())?;
    if directory.identity != published.record.directory_identity {
        return Err("Published map directory changed; no bytes were deleted".into());
    }
    if published.record.capture.is_none() {
        match directory.directory.symlink_metadata(&published.record.name) {
            Ok(metadata) if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1
                || metadata.len() != published.size
                || published.record.identity.as_ref() != Some(&FileIdentity::of(&metadata)) => {
                return Err("Published map file changed; no bytes were deleted".into());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
    }
    if published.sha256.is_none() {
        return Err("This older map cannot be checked for damage. Remove the map from your library, then download it again".into());
    }
    match reclaim_owned_file(
        &mut published.record, checkpoint, capture_partial, true, published.sha256, Some(store),
    )? {
        PartialRecovery::Reclaimed => Ok(()),
        PartialRecovery::Refused(reason) => Err(reason),
        _ => Err("Published map ownership changed; no bytes were deleted".into()),
    }
}

/// True when the journaled destination directory itself holds none of the
/// entries the record names: its partial, its publication name and its cleanup
/// capability. Only then is nothing left to report.
///
/// The directory must be the journaled one, as for every other conclusion
/// drawn here. A different directory at the same path (a folder the user
/// recreated, or a mount point whose volume is absent) says nothing about the
/// entries, and neither does an unreachable destination or any other error.
pub(super) fn record_vanished(record: &OwnedPartial) -> bool {
    let Ok(directory) = MapDirectory::open(&record.destination) else { return false };
    if directory.identity != record.directory_identity { return false; }
    std::iter::once(record.name.as_str())
        .chain(record.publication.as_ref().map(|publication| publication.name.as_str()))
        .chain(record.capture.as_ref().map(|capture| capture.name.as_str()))
        .all(|name| matches!(directory.directory.symlink_metadata(name),
            Err(error) if error.kind() == io::ErrorKind::NotFound))
}

/// Why explicit cleanup kept an entry, in terms the person can act on. The
/// reclaim's own error (operating-system text, device and inode numbers) is
/// for the log; none of it belongs on the download's card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kept {
    /// The destination cannot be opened: typically its drive is not connected.
    Unavailable,
    /// The path now leads to a different directory than the journaled one.
    FolderReplaced,
    /// The journaled directory is there, but what it holds is not provably
    /// ours alone, or the journal no longer describes it.
    Unconfirmed,
    /// Removing it did not complete this time, for a reason a retry can change.
    Busy,
}

impl Kept {
    /// Classify a reclaim that kept the record, by the same checks in the same
    /// order as the reclaim: the destination must open, then be the journaled
    /// directory; after that the reclaim either declined the entry or failed.
    pub(super) fn of(record: &OwnedPartial, declined: bool) -> Self {
        match MapDirectory::open(&record.destination) {
            Err(_) => Self::Unavailable,
            Ok(directory) if directory.identity != record.directory_identity => Self::FolderReplaced,
            Ok(_) if declined => Self::Unconfirmed,
            Ok(_) => Self::Busy,
        }
    }

    /// Whether pressing Delete again can succeed once the cause is gone. The
    /// other outcomes are only ever reported: nothing there is ours to remove.
    pub(super) fn retryable(self) -> bool {
        matches!(self, Self::Unavailable | Self::Busy)
    }

    /// `again` is true when the message answers a Delete the person pressed.
    pub(super) fn sentence(self, again: bool) -> String {
        match self {
            Self::Unavailable => format!(
                "Its folder cannot be opened right now. If it is on a drive that is not connected, reconnect it and {}. If the folder is gone, dismiss this warning",
                if again { "press Delete again" } else { "delete this download" }),
            Self::FolderReplaced =>
                "Its folder was moved or replaced, so Kiwix left the file alone".into(),
            Self::Unconfirmed =>
                "It could not be confirmed as the file this download wrote, so Kiwix left it alone".into(),
            Self::Busy => format!(
                "It could not be removed just now. {}; if it keeps failing, free some space or close programs using that folder",
                if again { "Press Delete again" } else { "Delete this download to try again" }),
        }
    }
}

pub(super) fn map_entry_exists(directory: &Dir, name: &str) -> Result<bool, String> {
    match directory.symlink_metadata(name) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// Remove the empty private cleanup capability `name` from `parent`.
///
/// Unix removes it by name while `staging` stays open, exactly as before.
#[cfg(not(windows))]
pub(super) fn remove_cleanup_capability(
    parent: &Dir, name: &str, _staging: &mut Option<Dir>, _identity: &FileIdentity,
) -> Result<(), String> {
    parent.remove_dir(name).map_err(|e| e.to_string())
}

/// Windows: cap-std directory handles never share delete, so holding
/// `staging` makes any removal of it fail with ERROR_SHARING_VIOLATION. Release
/// it, reopen the exact journaled name (no-follow, delete access), prove the
/// journaled identity on that handle, and delete through the same handle. A
/// directory substituted at that name is never removed, and a non-empty one
/// fails with ERROR_DIR_NOT_EMPTY. On failure `staging` stays released; the
/// caller reacquires it with `reopen_cleanup_capability`.
#[cfg(windows)]
pub(super) fn remove_cleanup_capability(
    parent: &Dir, name: &str, staging: &mut Option<Dir>, identity: &FileIdentity,
) -> Result<(), String> {
    drop(staging.take());
    crate::storage::remove_empty_directory_verified(
        parent,
        std::path::Path::new(name),
        |metadata| &FileIdentity::of(metadata) == identity,
        "Map cleanup capability was replaced; nothing was removed",
    )
    .map_err(|e| e.to_string())
}

/// Reacquire a released cleanup capability only at its exact journaled identity.
pub(super) fn reopen_cleanup_capability(parent: &Dir, name: &str, identity: &FileIdentity) -> Option<Dir> {
    let staging = parent.open_dir_nofollow(name).ok()?;
    let metadata = staging.dir_metadata().ok()?;
    (&FileIdentity::of(&metadata) == identity).then_some(staging)
}

pub(super) fn sync_map_directory(directory: &Dir) -> Result<(), String> {
    #[cfg(unix)]
    crate::storage::sync_cap_directory(directory).map_err(|e| e.to_string())?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

// Like library migration cleanup, capture and restoration use exclusive
// renames between capabilities, never an ambient-path fallback. Where the
// filesystem has no RENAME_NOREPLACE (NFS, FUSE) the shared primitive moves
// the entry by an exclusive link and a captured unlink through the same two
// capabilities; an interruption there can leave the file a second name
// (`.kiwix-renamed-<device>-<inode>` in the source directory).
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
pub(super) fn capture_partial(source: &Dir, from: &str, destination: &Dir, to: &str) -> Result<(), String> {
    let from = std::ffi::CString::new(from).map_err(|e| e.to_string())?;
    let to = std::ffi::CString::new(to).map_err(|e| e.to_string())?;
    crate::publication::rename_noreplace_at(source, &from, destination, &to)
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
pub(super) fn capture_partial(source: &Dir, from: &str, destination: &Dir, to: &str) -> Result<(), String> {
    use cap_std::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .access_mode(0x0001_0000 | 0x0000_0080)
        .follow(FollowSymlinks::No)
        .custom_flags(0x0020_0000)
        .share_mode(1 | 4);
    let file = source
        .open_with(from, &options)
        .map_err(|e| e.to_string())?;
    crate::storage::rename_into_directory_noreplace(
        file.as_raw_handle(),
        destination.as_raw_handle(),
        std::ffi::OsStr::new(to),
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_counts_never_truncate_to_zero_on_32_bit_targets() {
        let buffer = 64 * 1024;
        assert_eq!(super::next_count(7, buffer), 7);
        assert_eq!(super::next_count(buffer as u64, buffer), buffer);
        assert_eq!(super::next_count(buffer as u64 + 1, buffer), buffer);
        // 2^32 as usize is 0 on 32-bit targets; the loop must still make progress.
        assert_eq!(super::next_count(1 << 32, buffer), buffer);
        assert_eq!(super::next_count(u64::MAX, buffer), buffer);
    }
}
