// SPDX-License-Identifier: GPL-3.0-or-later
//! Durable publication and crash recovery of completed torrent archives.
#![cfg_attr(unix, forbid(unsafe_code))]
#[allow(unused_imports)]
use super::*;

pub(super) fn import_publication(record: &Record, destination: &str) -> Result<crate::acquisition::ImportPublication, String> {
    if record.share.is_some() || record.job.shared || record.history_dismissed || record.files_deleted
        || !record.published_cleanup.is_empty()
        || !record.job.completed_paths.iter().any(|path| path == destination)
    {
        return Err("This torrent publication is no longer available for import".into());
    }
    let path = Path::new(destination);
    let name = path.file_name().ok_or("Torrent publication has no filename")?;
    safe_relative(Path::new(name)).map_err(display)?;
    if record.directory.join(name) != path
        || !record.files.iter().any(|file| is_zim(file) && file.file_name() == Some(name))
    {
        return Err("Torrent import path does not match its published archive".into());
    }
    let mut intents = record.publication_intents.iter().filter(|intent| intent.file_name == Path::new(name));
    let intent = intents.next().ok_or("Kiwix cannot verify this torrent's original publication. Add the archive to your library manually")?;
    if intents.next().is_some() { return Err("Torrent publication ownership is ambiguous".into()); }
    Ok(crate::acquisition::ImportPublication::new(destination.to_owned(), intent.cleanup_entry()?))
}

#[cfg(target_os = "ios")]
pub(crate) fn backup_evidence(store: &Store) -> Result<Vec<(PathBuf, crate::library_files::BackupEvidence)>, String> {
    let saved: Option<State> = crate::library_files::backup_record(
        store, &["torrents", "jobs.json"], MAX_METADATA as u64,
        crate::mobile_rebase::RecordKind::Torrents,
    )?;
    let mut candidates = Vec::new();
    if let Some(saved) = saved {
        if saved.version != STATE_VERSION { return Err("Unsupported torrent backup evidence version".into()); }
        for record in saved.jobs {
            for file in record.files.iter().filter(|file| is_zim(file)) {
                safe_relative(file).map_err(display)?;
                let name = file.file_name().ok_or("Missing completed torrent archive filename")?;
                let path = record.directory.join(name);
                if !record.job.completed_paths.iter().any(|completed| Path::new(completed) == path) { continue; }
                let intent = record.publication_intents.iter().find(|intent| intent.file_name == Path::new(name));
                candidates.push((path, crate::library_files::BackupEvidence {
                    size: intent.map(|intent| intent.size).unwrap_or(0),
                    identity: intent.map(|intent| (intent.device, intent.inode)),
                    sha256: intent.map(|intent| intent.sha256),
                }));
            }
        }
    }
    Ok(candidates)
}

// Recover committed outputs before touching staging, torrent metadata or peers.
// An absent destination still needs an engine-verified source; an existing
// mismatched destination is never silently adopted or overwritten.
pub(super) fn recover_archives(
    record: &Record,
    destination: &Dir,
    stop: &CancellationToken,
    committed: impl FnMut(String) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    recover_archives_with_policy(record, destination, stop, committed, |file, path| {
        crate::mobile_backup::archive_policy(file, path)
    })
}

pub(super) fn recover_archives_with_policy(
    record: &Record,
    destination: &Dir,
    stop: &CancellationToken,
    mut committed: impl FnMut(String) -> Result<(), String>,
    mut policy: impl FnMut(&File, &Path) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let mut recovered = Vec::new();
    for path in record.files.iter().filter(|p| is_zim(p)) {
        if stop.is_cancelled() {
            return Err("Torrent publication cancelled; staging was retained".into());
        }
        safe_relative(path).map_err(display)?;
        let name = path.file_name().ok_or("Missing torrent archive filename")?;
        let final_path = record.directory.join(name).to_string_lossy().into_owned();
        if let Some(intent) = record
            .publication_intents
            .iter()
            .find(|intent| intent.file_name == Path::new(name))
        {
            if let Some(verified) = recover_publication(destination, intent, stop)? {
                policy(verified.file(), &record.directory.join(name))?;
                sync_publication_directory(destination)?;
                committed(final_path.clone())?;
                recovered.push(final_path);
            }
        } else if record.job.completed_paths.contains(&final_path) {
            // Preserve acknowledgement of completed jobs predating intents, but
            // only for a destination that still exists and passes the same
            // policy as an intent-backed recovery. An absent or replaced
            // destination is left unrecovered so publication runs again instead
            // of reporting a file that is not there.
            let options = publication_read_options();
            match destination.open_with(Path::new(name), &options) {
                Ok(file) => {
                    let file = file.into_std();
                    let metadata = file.metadata().map_err(display)?;
                    if !metadata.is_file() {
                        continue;
                    }
                    policy(&file, &record.directory.join(name))?;
                    recovered.push(final_path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(display(error)),
            }
        }
    }
    Ok(recovered)
}

// Persist the copied file's intent before exposing its final name. A crash or
// failed acknowledgement after rename can then be recovered without adopting an
// unrelated file, even one with identical bytes. No staging inode is published.
pub(super) fn publish_archives(
    record: &Record,
    store: &Store,
    sources: &HashMap<PathBuf, EngineSource>,
    stop: &CancellationToken,
    mut temporary_changed: impl FnMut(PublicationTemporary, bool) -> Result<(), String>,
    mut intended: impl FnMut(PublicationIntent) -> Result<(), String>,
    mut committed: impl FnMut(String) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let destination = open_publication_directory(&record.directory)?;
    let mut published = recover_archives(record, &destination, stop, &mut committed)?;
    for path in record.files.iter().filter(|p| is_zim(p)) {
        if stop.is_cancelled() {
            return Err("Torrent publication cancelled; staging was retained".into());
        }
        safe_relative(path).map_err(display)?;
        let name = path.file_name().ok_or("Missing torrent archive filename")?;
        let final_path = record.directory.join(name).to_string_lossy().into_owned();
        if published.contains(&final_path) {
            continue;
        }
        match destination.symlink_metadata(Path::new(name)) {
            Ok(_) => {
                return Err(format!(
                    "Cannot publish {final_path}: an unowned destination already exists"
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(display(error)),
        }
        let retained = sources
            .get(path)
            .ok_or("Missing engine-verified torrent publication source")?;
        // The engine's verdict covers the inode it hashed, not bytes a
        // same-user rewrite can substitute afterwards. Every copied byte must
        // still match the piece hashes that verdict was based on.
        let receipt = retained
            .receipt
            .as_deref()
            .ok_or("Missing engine-verified torrent piece receipt")?;
        let verified_source = crate::publication::VerifiedFile::new(
            retained.file.try_clone().map_err(display)?,
            retained.size,
        )
        .map_err(|e| {
            format!(
                "Torrent publication source is no longer the exact singly linked regular file: {e}"
            )
        })?;
        let mut source = verified_source.file().try_clone().map_err(display)?;
        use std::io::{Seek, SeekFrom};
        source.seek(SeekFrom::Start(0)).map_err(display)?;
        crate::storage::check_file_size(&record.directory, retained.size)
            .map_err(|e| format!("Cannot publish {}: {e}", path.display()))?;
        let mut magic = [0u8; 4];
        source.read_exact(&mut magic).map_err(display)?;
        if magic != [b'Z', b'I', b'M', 4] {
            return Err(format!("{} is not a ZIM archive", path.display()));
        }
        source.seek(SeekFrom::Start(0)).map_err(display)?;
        if !verified_source.matches_file(&source).map_err(display)? {
            return Err(format!(
                "Torrent publication source changed before copying: {}",
                path.display()
            ));
        }
        let temp = format!(".kiwix-publish-{}", Uuid::new_v4());
        let (temporary, mut output) =
            create_publication_temporary(&record.directory, &destination, temp)?;
        // No payload write may precede this durable ownership acknowledgement.
        if let Err(error) = temporary_changed(temporary.clone(), false) {
            // Windows cannot capture (rename) the temporary while this writer
            // is open; nothing else needs it on this path.
            drop(output);
            let cleanup =
                reclaim_publication_temporary(&record.directory, &destination, &temporary);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; temporary cleanup: {cleanup}"),
            });
        }
        let result = (|| -> Result<(), String> {
            publication_directory_unchanged(&record.directory, &destination, &temporary.directory)?;
            let mut checked = ReceiptReader::new(&mut source, receipt);
            let sha256 = publication_digest(&mut checked, &mut output, retained.size, stop)?;
            checked.finish()?;
            drop(checked);
            // The digest records copied-output identity for crash recovery.
            // The receipt above already bound these bytes to the engine's piece
            // verdict; re-bind the source handle too, so a same-inode rewrite
            // during the read cannot publish foreign bytes on platforms whose
            // change clock is weaker than the receipt.
            if !verified_source.matches_file(&source).map_err(display)? {
                return Err(format!(
                    "Torrent publication source changed while copying: {}",
                    path.display()
                ));
            }
            // Capture the copied output's identity before persisting its intent.
            // On Unix the change stamp is taken before the sync so a same-inode
            // rewrite during a long flush is detected; on Windows the sync runs
            // first because flushing can touch the last-write timestamp there.
            let copied = output.metadata().map_err(display)?;
            #[cfg(unix)]
            let verified_output = {
                let verified =
                    crate::publication::VerifiedFile::new(output.into_std(), retained.size)
                        .map_err(display)?;
                verified.file().sync_all().map_err(display)?;
                verified
            };
            #[cfg(not(unix))]
            let verified_output = {
                output.sync_all().map_err(display)?;
                crate::publication::VerifiedFile::new(output.into_std(), retained.size)
                    .map_err(display)?
            };
            // Hash the copied bytes too. A writer can modify the temporary
            // after the source digest pass; the output identity alone would
            // bind those foreign bytes.
            if output_digest(verified_output.file(), retained.size)? != sha256 {
                return Err(format!(
                    "Torrent publication output changed while copying: {}",
                    path.display()
                ));
            }
            // Sync the temporary directory entry as well as its contents before
            // recording the inode that recovery may later recognize.
            sync_publication_directory(&destination)?;
            intended(PublicationIntent {
                file_name: PathBuf::from(name),
                size: copied.len(),
                sha256,
                device: copied.dev(),
                inode: copied.ino(),
                directory: Some(temporary.directory.clone()),
            })?;
            if stop.is_cancelled() {
                return Err("Torrent publication cancelled".into());
            }
            publication_directory_unchanged(&record.directory, &destination, &temporary.directory)?;
            store.with_acquisition_destination(&record.directory.join(name), || {
                crate::publication::publish_verified_noreplace_bound_in(
                    &destination,
                    Path::new(&temporary.name),
                    Path::new(name),
                    &verified_output,
                )
                .map_err(|e| format!(
                    "Cannot publish {final_path} without changing its verified identity or overwriting an existing file: {e}; verified staging retained"))?;
                crate::mobile_backup::archive_policy(verified_output.file(), &record.directory.join(name))
            })?;
            publication_directory_unchanged(&record.directory, &destination, &temporary.directory)?;
            sync_publication_directory(&destination)?;
            Ok(())
        })();
        // Capture first, inspect second, unlink only inside our protected capability.
        // This runs on cancellation, disk/write failure and a successful rename.
        // Failed cleanup keeps its durable journal for startup/resume recovery.
        let cleanup = reclaim_publication_temporary(&record.directory, &destination, &temporary);
        match cleanup {
            Ok(()) => temporary_changed(temporary, true)?,
            Err(cleanup) => {
                return Err(match result {
                    Ok(()) => cleanup,
                    Err(error) => format!("{error}; temporary cleanup: {cleanup}"),
                });
            }
        }
        result?;
        committed(final_path.clone())?;
        published.push(final_path);
    }
    Ok(published)
}

pub(super) fn validate_publication_temporary(temporary: &PublicationTemporary) -> Result<(), String> {
    for (name, prefix) in [
        (&temporary.name, ".kiwix-publish-"),
        (&temporary.capture_name, ".kiwix-publish-cleanup-"),
    ] {
        let suffix = name
            .strip_prefix(prefix)
            .ok_or("Invalid torrent temporary name")?;
        let id = Uuid::parse_str(suffix).map_err(display)?;
        if id.to_string() != suffix {
            return Err("A saved transfer is damaged. Download the file again".into());
        }
    }
    Ok(())
}

pub(super) fn open_publication_directory(path: &Path) -> Result<Dir, String> {
    let directory = crate::storage::open_directory_nofollow(path).map_err(display)?;
    validate_publication_directory(&directory)?;
    Ok(directory)
}

pub(super) fn publication_directory_unchanged(
    path: &Path,
    directory: &Dir,
    identity: &PublicationIdentity,
) -> Result<(), String> {
    let current = open_publication_directory(path)?;
    if PublicationIdentity::of(&current.dir_metadata().map_err(display)?) != *identity
        || PublicationIdentity::of(&directory.dir_metadata().map_err(display)?) != *identity
    {
        return Err("Torrent publication directory was replaced; files were preserved".into());
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn publication_directory_mutation_is_owned(uid: u32, mode: u32) -> bool {
    uid == crate::fs_ext::effective_uid() && mode & 0o022 == 0
}

pub(super) fn validate_publication_directory(directory: &Dir) -> Result<(), String> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        let metadata = directory.dir_metadata().map_err(display)?;
        // Capture needs exclusive mutation, not confidentiality. FAT/exFAT can
        // report mount-derived 0755 even when creation requested 0700.
        if !publication_directory_mutation_is_owned(metadata.uid(), metadata.mode()) {
            return Err("Kiwix needs a folder owned by your account that other users cannot write to. Choose a private download folder. If it is already private, check the permissions on the torrents folder inside your Kiwix data folder.".into());
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

pub(super) fn open_publication_capture(destination: &Dir, name: &str) -> Result<Dir, String> {
    validate_publication_directory(destination)?;
    let directory = destination.open_dir_nofollow(name).map_err(display)?;
    validate_publication_directory(&directory)?;
    Ok(directory)
}

pub(super) fn create_publication_temporary(
    path: &Path,
    destination: &Dir,
    name: String,
) -> Result<(PublicationTemporary, cap_std::fs::File), String> {
    let directory = PublicationIdentity::of(&destination.dir_metadata().map_err(display)?);
    publication_directory_unchanged(path, destination, &directory)?;
    let capture_name = format!(".kiwix-publish-cleanup-{}", Uuid::new_v4());
    #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
    let mut builder = cap_std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    destination
        .create_dir_with(&capture_name, &builder)
        .map_err(display)?;
    let capture = open_publication_capture(destination, &capture_name)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.share_mode(1 | 4);
    }
    let output = destination.open_with(&name, &options).map_err(display)?;
    crate::mobile_backup::exclude_payload(&output, &path.join(&name))?;
    output.sync_all().map_err(display)?;
    sync_publication_directory(&capture)?;
    sync_publication_directory(destination)?;
    Ok((
        PublicationTemporary {
            name,
            capture_name,
            directory,
            capture_directory: PublicationIdentity::of(&capture.dir_metadata().map_err(display)?),
            file: PublicationIdentity::of(&output.metadata().map_err(display)?),
        },
        output,
    ))
}

pub(super) fn publication_entry_exists(directory: &Dir, name: &str) -> Result<bool, String> {
    match directory.symlink_metadata(name) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(display(error)),
    }
}

pub(super) fn reclaim_publication_temporary(
    path: &Path,
    destination: &Dir,
    temporary: &PublicationTemporary,
) -> Result<(), String> {
    reclaim_publication_temporary_with_hook(path, destination, temporary, || {})
}

pub(super) fn reclaim_publication_temporary_with_hook(
    path: &Path,
    destination: &Dir,
    temporary: &PublicationTemporary,
    before_capture: impl FnOnce(),
) -> Result<(), String> {
    validate_publication_temporary(temporary)?;
    reclaim_captured_file(path, destination, temporary, None, before_capture)
}

pub(super) fn reclaim_captured_file(
    path: &Path,
    destination: &Dir,
    temporary: &PublicationTemporary,
    ownership: Option<&CleanupEntry>,
    before_capture: impl FnOnce(),
) -> Result<(), String> {
    publication_directory_unchanged(path, destination, &temporary.directory)?;
    if !publication_entry_exists(destination, &temporary.capture_name)? {
        // An earlier cleanup may have removed the empty directory before its
        // journal acknowledgement became durable. Never adopt a new temp here.
        if !publication_entry_exists(destination, &temporary.name)? {
            return Ok(());
        }
        return Err("Torrent temporary capture directory is missing; file preserved".into());
    }
    let capture = open_publication_capture(destination, &temporary.capture_name)?;
    if PublicationIdentity::of(&capture.dir_metadata().map_err(display)?)
        != temporary.capture_directory
    {
        return Err("Torrent temporary capture directory was replaced; files preserved".into());
    }
    if !publication_entry_exists(&capture, "payload")?
        && publication_entry_exists(destination, &temporary.name)?
    {
        before_capture();
        publication_directory_unchanged(path, destination, &temporary.directory)?;
        move_publication_entry(destination, &temporary.name, &capture, "payload")?;
        sync_publication_directory(&capture)?;
        sync_publication_directory(destination)?;
    }
    if publication_entry_exists(&capture, "payload")? {
        // Other principals cannot mutate this captured name, unlike a public temp.
        // Identity deliberately excludes length: interrupted copies are partial.
        let metadata = capture.symlink_metadata("payload").map_err(display)?;
        if !metadata.is_file()
            || metadata.is_symlink()
            || PublicationIdentity::of(&metadata) != temporary.file
            || metadata.nlink() != 1
        {
            // Restore foreign entries exclusively, never overwrite a new public
            // collision. A failed restoration leaves both sentinel and journal.
            let restoration =
                move_publication_entry(&capture, "payload", destination, &temporary.name);
            sync_publication_directory(&capture)?;
            sync_publication_directory(destination)?;
            return Err(format!(
                "Torrent temporary was replaced; unrelated entry preserved{}",
                restoration
                    .err()
                    .map(|e| format!(" in {}: {e}", temporary.capture_name))
                    .unwrap_or_default()
            ));
        }
        publication_directory_unchanged(path, destination, &temporary.directory)?;
        let current_capture = open_publication_capture(destination, &temporary.capture_name)?;
        if PublicationIdentity::of(&current_capture.dir_metadata().map_err(display)?)
            != temporary.capture_directory
        {
            return Err("Torrent temporary capture directory changed; files preserved".into());
        }
        if let Some(ownership) = ownership {
            let retained = capture.open_with("payload", &publication_read_options()).map_err(display)?.into_std();
            if !ownership.matches_retained(&path.join(&temporary.name), &retained)? {
                let restoration = move_publication_entry(&capture, "payload", destination, &temporary.name);
                sync_publication_directory(&capture)?;
                sync_publication_directory(destination)?;
                return Err(format!("Captured download no longer matches its original publication bytes; file preserved{}",
                    restoration.err().map(|error| format!(" in {}: {error}", temporary.capture_name)).unwrap_or_default()));
            }
        }
        capture.remove_file("payload").map_err(display)?;
        sync_publication_directory(&capture)?;
    }
    // Windows refuses to remove a directory while any capability for it is
    // open (cap-std never shares delete on directory handles). The capture
    // capability is no longer needed; the removal below re-binds to the
    // journaled capture identity on Windows. On Unix this only closes it early.
    drop(capture);
    // Empty-only removal cannot recursively sweep a sentinel or an unknown file.
    remove_owned_empty_directory(destination, &temporary.capture_name, &temporary.capture_directory)
        .map_err(display)?;
    sync_publication_directory(destination)
}

/// Remove the empty directory `name` in `parent`, whose identity the caller
/// verified as `identity`. Removal is empty-only, so an unknown entry is never
/// swept.
///
/// Unix keeps the original name-based `rmdir` through the parent capability.
/// Windows cannot remove a directory while a capability for it is open, so
/// callers release theirs first; this then opens the entry itself (never
/// following a reparse point, denying other renamers/deleters while held),
/// re-verifies the volume serial and file index, and marks exactly that handle
/// for deletion. A replaced directory is preserved, and a non-empty one fails
/// with ERROR_DIR_NOT_EMPTY like `rmdir` fails with ENOTEMPTY.
pub(super) fn remove_owned_empty_directory(
    parent: &Dir,
    name: impl AsRef<Path>,
    identity: &PublicationIdentity,
) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        let _ = identity;
        parent.remove_dir(name)
    }
    #[cfg(windows)]
    {
        crate::storage::remove_empty_directory_verified(
            parent,
            name.as_ref(),
            |metadata| PublicationIdentity::of(metadata) == *identity,
            "Owned temporary folder was replaced; it was preserved",
        )
    }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
pub(super) fn move_publication_entry(
    source: &Dir,
    from: &str,
    destination: &Dir,
    to: &str,
) -> Result<(), String> {
    let from = std::ffi::CString::new(from).map_err(display)?;
    let to = std::ffi::CString::new(to).map_err(display)?;
    crate::publication::rename_noreplace_at(source, &from, destination, &to).map_err(display)
}

#[cfg(windows)]
pub(super) fn move_publication_entry(
    source: &Dir,
    from: &str,
    destination: &Dir,
    to: &str,
) -> Result<(), String> {
    use cap_std::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .access_mode(0x0001_0000 | 0x0000_0080)
        .follow(FollowSymlinks::No)
        .custom_flags(0x0020_0000)
        .share_mode(1 | 4);
    let file = source.open_with(from, &options).map_err(display)?;
    crate::storage::rename_into_directory_noreplace(
        file.as_raw_handle(),
        destination.as_raw_handle(),
        std::ffi::OsStr::new(to),
    )
    .map_err(display)
}

pub(super) fn sync_publication_directory(directory: &Dir) -> Result<(), String> {
    #[cfg(unix)]
    crate::storage::sync_cap_directory(directory).map_err(display)?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

pub(super) fn recover_publication(
    directory: &Dir,
    intent: &PublicationIntent,
    stop: &CancellationToken,
) -> Result<Option<crate::publication::VerifiedFile>, String> {
    let options = publication_read_options();
    #[cfg(windows)]
    let options = {
        use cap_std::fs::OpenOptionsExt;
        let mut options = options;
        options.share_mode(1); // Deny mutation/replacement while hashing final output.
        options
    };
    let mut file = match directory.open_with(&intent.file_name, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Cannot safely recover torrent publication: {error}"
            ))
        }
    };
    let metadata = file.metadata().map_err(display)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() != intent.size
        || metadata.dev() != intent.device
        || metadata.ino() != intent.inode
    {
        return Err(
            "Torrent publication destination is not the intended copied file; it was not changed"
                .into(),
        );
    }
    let digest = publication_digest(&mut file, &mut std::io::sink(), intent.size, stop)?;
    let after = file.metadata().map_err(display)?;
    if digest != intent.sha256 || after.len() != intent.size || after.nlink() != 1 {
        return Err("Torrent publication destination does not match its durable SHA-256 intent; it was not changed".into());
    }
    let verified =
        crate::publication::VerifiedFile::new(file.into_std(), intent.size).map_err(display)?;
    if !verified
        .matches_in(directory, &intent.file_name)
        .map_err(display)?
    {
        return Err(
            "Torrent publication destination was replaced during recovery; it was not changed"
                .into(),
        );
    }
    Ok(Some(verified))
}

// Constant-memory, cancellation-aware copying/hashing, bounded by the recorded
// length plus one byte to detect growth. Never hash an unbounded destination.
pub(super) fn publication_digest(
    source: &mut impl Read,
    output: &mut impl Write,
    size: u64,
    stop: &CancellationToken,
) -> Result<[u8; 32], String> {
    let mut remaining = size;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        if stop.is_cancelled() {
            return Err("Torrent publication cancelled".into());
        }
        let limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let n = source.read(&mut buffer[..limit]).map_err(display)?;
        if n == 0 {
            break;
        }
        if n as u64 > remaining {
            return Err("The downloaded file changed while Kiwix was checking it. Download it again".into());
        }
        remaining -= n as u64;
        hasher.update(&buffer[..n]);
        output.write_all(&buffer[..n]).map_err(display)?;
    }
    if remaining != 0 {
        return Err("Torrent publication file was truncated".into());
    }
    Ok(hasher.finalize().into())
}

fn output_digest(file: &File, size: u64) -> Result<[u8; 32], String> {
    let mut file = file.try_clone().map_err(display)?;
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).map_err(display)?;
    let mut remaining = size;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    while remaining != 0 {
        let limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let n = file.read(&mut buffer[..limit]).map_err(display)?;
        if n == 0 || n as u64 > remaining {
            return Err("Torrent publication output changed while copying".into());
        }
        remaining -= n as u64;
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().into())
}

pub(super) fn publication_read_options() -> cap_std::fs::OpenOptions {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options
}
