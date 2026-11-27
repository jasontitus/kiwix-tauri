// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal cleanup retains ownership until both deletion and its receipt commit.
#![forbid(unsafe_code)]
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CleanupIntent {
    published: bool,
    stamps: Vec<Option<Stamp>>,
    #[serde(default)]
    complete: bool,
}

pub(super) fn terminal(status: &str) -> bool {
    matches!(status, "complete" | "cancelled" | "failed")
}

pub(super) fn cleanup_name(saved: &StoredJob) -> Result<String, String> {
    let suffix = saved.job.id.strip_prefix("localswarm:").ok_or("Invalid LocalSwarm cleanup ID")?;
    let name = format!(".kiwix-localswarm-{suffix}.cleanup");
    storage::basename(&name)?;
    Ok(name)
}

impl CleanupIntent {
    pub(super) fn validate(&self, saved: &StoredJob) -> Result<(), String> {
        if self.stamps.len() != saved.manifest.files.len() || saved.owned.files.len() != saved.manifest.files.len()
            || (self.published && !saved.publishing) {
            return Err("Invalid LocalSwarm cleanup ownership receipt".into());
        }
        for ((stamp, identity), entry) in self.stamps.iter().zip(&saved.owned.files).zip(&saved.manifest.files) {
            if stamp.as_ref().is_some_and(|stamp| stamp.identity != *identity || stamp.size != entry.size_bytes) {
                return Err("LocalSwarm cleanup receipt does not identify the retained files".into());
            }
        }
        Ok(())
    }

    fn source<'a>(&self, saved: &'a StoredJob) -> &'a str {
        if self.published { &saved.owned.destination } else { &saved.owned.staging }
    }
}

fn parent(saved: &StoredJob) -> Result<Directory, String> {
    let parent = Directory::open(&saved.owned.parent)?;
    if parent.identity != saved.owned.parent_identity {
        return Err("LocalSwarm cleanup parent was replaced; retained files were not touched".into());
    }
    Ok(parent)
}

fn owned_directory(parent: &Directory, name: &str, saved: &StoredJob) -> Result<Directory, String> {
    let directory = parent.child(name, false)?;
    if directory.identity != saved.owned.identity {
        return Err("LocalSwarm cleanup directory ownership changed; all entries were preserved".into());
    }
    Ok(directory)
}

fn known_entries(directory: &Directory, saved: &StoredJob) -> Result<(), String> {
    directory.unchanged()?;
    for entry in directory.dir.entries().map_err(err)? {
        let name = entry.map_err(err)?.file_name();
        if !saved.manifest.files.iter().any(|entry| std::ffi::OsStr::new(&entry.path) == name) {
            return Err("Unexpected entries in LocalSwarm cleanup directory; all entries were preserved".into());
        }
    }
    Ok(())
}

fn owned_file(directory: &Directory, saved: &StoredJob, index: usize, writable: bool) -> Result<(File, Stamp), String> {
    let entry = &saved.manifest.files[index];
    let file = directory.file(&entry.path, writable, false)?;
    let stamp = Stamp::of(&Metadata::from_file(&file).map_err(err)?)?;
    if stamp.identity != saved.owned.files[index] || stamp.size != entry.size_bytes
        || Stamp::of(&directory.dir.symlink_metadata(&entry.path).map_err(err)?)? != stamp {
        return Err("LocalSwarm cleanup file ownership changed; all remaining files were preserved".into());
    }
    Ok((file, stamp))
}

// Legacy jobs retain inode ownership but no publication stamp. For a published
// member also require its manifest bytes, not merely a matching persisted name.
fn verify_published(file: &mut (impl Read + Seek), saved: &StoredJob, index: usize, bytes: &mut Vec<u8>, mut publication_digest: Option<&mut Sha256>, cancel: &CancellationToken) -> Result<(), String> {
    let entry = &saved.manifest.files[index];
    file.seek(SeekFrom::Start(0)).map_err(err)?;
    for chunk in entry.start_chunk_index..=entry.end_chunk_index {
        check_cancel(cancel)?;
        bytes.resize(saved.manifest.chunk_len(chunk)?, 0);
        file.read_exact(bytes).map_err(err)?;
        check_cancel(cancel)?;
        if hex::encode(Sha256::digest(&*bytes)) != saved.manifest.chunk_hashes[chunk as usize] {
            return Err("Published LocalSwarm archive changed; remove it explicitly from the library instead".into());
        }
        if let Some(digest) = &mut publication_digest { digest.update(&*bytes); }
    }
    Ok(())
}

type PreparedCleanup = (CleanupIntent, Vec<(usize, File)>, Vec<[u8; 32]>);

/// Publication digests already proven for one exact inode state: device,
/// inode, size, mtime and ctime. Cleanup preview and confirmation each require
/// the full content proof, and a digest is reused only when the kernel proves
/// the bytes cannot have changed; any write, truncate or replacement changes
/// the key and is re-hashed. Never populated where ctime is not authoritative.
type CleanupDigestKey = (String, usize, u64, u64, u64, i128, i128);
pub(super) type CleanupDigestCache = Mutex<HashMap<CleanupDigestKey, [u8; 32]>>;
#[cfg(unix)]
const CLEANUP_DIGEST_CACHE: usize = 256;
#[cfg(unix)]
const RACY_TIMESTAMP_MARGIN_NS: i128 = 1_000_000_000;
#[cfg(unix)]
fn unix_now_ns() -> Option<i128> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos() as i128)
}
#[cfg(unix)]
fn digest_key(saved: &StoredJob, index: usize, file: &File) -> Result<Option<CleanupDigestKey>, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(err)?;
    // Refuse caching where timestamps are missing or coarse (zero sub-second
    // part on FAT/exFAT/SMB/HFS+ mounts), or within the racy-timestamp margin
    // where a same-tick rewrite could preserve the tuple.
    if metadata.ctime() <= 0
        || metadata.mtime() <= 0
        || metadata.ctime_nsec() == 0
        || metadata.mtime_nsec() == 0
    {
        return Ok(None);
    }
    let mtime_ns = i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec());
    let ctime_ns = i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec());
    let Some(now_ns) = unix_now_ns() else {
        return Ok(None);
    };
    if mtime_ns.max(ctime_ns).saturating_add(RACY_TIMESTAMP_MARGIN_NS) > now_ns {
        return Ok(None);
    }
    Ok(Some((
        saved.manifest.swarm_id.clone(),
        index,
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        mtime_ns,
        ctime_ns,
    )))
}

/// Prove a published member against the manifest and return its publication
/// digest, reusing one already proven for the same manifest member and inode state.
fn published_digest(file: &mut File, saved: &StoredJob, index: usize, bytes: &mut Vec<u8>,
    cache: Option<&CleanupDigestCache>, cancel: &CancellationToken) -> Result<[u8; 32], String> {
    #[cfg(unix)]
    let key = if cache.is_some() { digest_key(saved, index, file)? } else { None };
    #[cfg(unix)]
    if let (Some(cache), Some(key)) = (cache, key.as_ref()) {
        if let Some(digest) = cache.lock().unwrap_or_else(|error| error.into_inner()).get(key).copied() {
            return Ok(digest);
        }
    }
    #[cfg(not(unix))]
    let _ = cache;
    let mut hash = Sha256::new();
    verify_published(file, saved, index, bytes, Some(&mut hash), cancel)?;
    let digest: [u8; 32] = hash.finalize().into();
    // Only a state that survived the read and satisfies the racy-timestamp
    // separation guard may authorize a later reuse.
    #[cfg(unix)]
    if let (Some(cache), Some(key)) = (cache, key) {
        if digest_key(saved, index, file)?.as_ref() == Some(&key) {
            let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
            if cache.len() >= CLEANUP_DIGEST_CACHE { cache.clear(); }
            cache.insert(key, digest);
        }
    }
    Ok(digest)
}

fn prepare(saved: &StoredJob, cancel: &CancellationToken, cache: Option<&CleanupDigestCache>) -> Result<CleanupIntent, String> {
    prepare_with_access(saved, true, cancel, cache).map(|(intent, _, _)| intent)
}

fn prepare_with_access(saved: &StoredJob, writable: bool, cancel: &CancellationToken, cache: Option<&CleanupDigestCache>) -> Result<PreparedCleanup, String> {
    check_cancel(cancel)?;
    let parent = parent(saved)?;
    let name = cleanup_name(saved)?;
    if !parent.absent(&name)? {
        return Err(format!("LocalSwarm cleanup has no ownership receipt for {}; all entries were preserved", parent.path.join(name).display()));
    }
    let published = if !parent.absent(&saved.owned.staging)? { false } else { saved.publishing };
    let source = if published { &saved.owned.destination } else { &saved.owned.staging };
    // A final directory without publication intent is not ours to delete.
    if !published && parent.absent(source)? && !parent.absent(&saved.owned.destination)? {
        return Err("LocalSwarm has no publication ownership proof for the existing destination".into());
    }
    let mut intent = CleanupIntent { published, stamps: vec![None; saved.manifest.files.len()], complete: false };
    if parent.absent(source)? { return Ok((intent, Vec::new(), Vec::new())); }
    let directory = owned_directory(&parent, source, saved)?;
    known_entries(&directory, saved)?;
    let mut bytes = Vec::new();
    let mut files = Vec::with_capacity(saved.manifest.files.len());
    let mut digests = if published && !writable { Vec::with_capacity(saved.manifest.files.len()) } else { Vec::new() };
    for (index, entry) in saved.manifest.files.iter().enumerate() {
        check_cancel(cancel)?;
        if directory.absent(&entry.path)? {
            if !published { return Err("LocalSwarm partial files disappeared before cleanup; ownership was retained".into()); }
            // Protected library removal may already have deleted split members.
            continue;
        }
        let (mut file, stamp) = owned_file(&directory, saved, index, writable)?;
        if published && writable {
            published_digest(&mut file, saved, index, &mut bytes, cache, cancel)?;
        } else if published {
            digests.push(published_digest(&mut file, saved, index, &mut bytes, cache, cancel)?);
        }
        if Stamp::of(&Metadata::from_file(&file).map_err(err)?)? != stamp
            || Stamp::of(&directory.dir.symlink_metadata(&entry.path).map_err(err)?)? != stamp {
            return Err("LocalSwarm retained bytes changed during cleanup admission".into());
        }
        intent.stamps[index] = Some(stamp);
        files.push((index, file));
    }
    known_entries(&directory, saved)?;
    parent.unchanged()?;
    Ok((intent, files, digests))
}

fn check_remaining(directory: &Directory, saved: &StoredJob, intent: &CleanupIntent, captured: bool, writable: bool) -> Result<Vec<(usize, File)>, String> {
    known_entries(directory, saved)?;
    let mut files = Vec::with_capacity(saved.manifest.files.len());
    for (index, (entry, expected)) in saved.manifest.files.iter().zip(&intent.stamps).enumerate() {
        if directory.absent(&entry.path)? {
            if expected.is_some() && !captured && !intent.published {
                return Err("LocalSwarm file disappeared before cleanup capture; ownership was retained".into());
            }
            continue;
        }
        let expected = expected.as_ref().ok_or("An unowned file appeared in LocalSwarm cleanup; all remaining files were preserved")?;
        let (file, stamp) = owned_file(directory, saved, index, writable)?;
        if stamp != *expected {
            return Err("LocalSwarm file changed after cleanup confirmation; all remaining files were preserved".into());
        }
        files.push((index, file));
    }
    Ok(files)
}

fn erase(saved: &StoredJob, intent: &CleanupIntent) -> Result<(), String> {
    erase_with_hook(saved, intent, || {})
}

fn erase_with_hook(saved: &StoredJob, intent: &CleanupIntent, before_capture: impl FnOnce()) -> Result<(), String> {
    intent.validate(saved)?;
    if intent.complete { return Ok(()); }
    let parent = parent(saved)?;
    let name = cleanup_name(saved)?;
    let source = intent.source(saved);
    if parent.absent(&name)? {
        if parent.absent(source)? { return Ok(()); }
        let directory = owned_directory(&parent, source, saved)?;
        let files = check_remaining(&directory, saved, intent, false, true)?;
        // Windows cannot rename a directory while this process holds its
        // capability (cap-std never shares delete) or any member handle (NTFS
        // refuses directory renames with open children). Release both:
        // capture_directory binds the rename to the journaled identity by
        // handle and re-checks it, and every member is re-verified against its
        // journaled stamp below before anything is unlinked.
        #[cfg(windows)]
        drop((directory, files));
        before_capture();
        parent.capture_directory(source, &name, &saved.owned.identity)?;
        #[cfg(not(windows))]
        drop(files);
    } else if !parent.absent(source)? {
        return Err("Both LocalSwarm cleanup and source names exist; all remaining files were preserved".into());
    }
    let directory = owned_directory(&parent, &name, saved)?;
    // Check the entire remaining set before deleting any member, including retry
    // after a crash between unlink and the final durable completion receipt.
    let files = check_remaining(&directory, saved, intent, true, true)?;
    for (index, file) in files {
        parent.unchanged()?;
        directory.unchanged()?;
        let entry = &saved.manifest.files[index];
        let expected = intent.stamps[index].as_ref().ok_or("Missing LocalSwarm cleanup file receipt")?;
        if Stamp::of(&Metadata::from_file(&file).map_err(err)?)? != *expected
            || Stamp::of(&directory.dir.symlink_metadata(&entry.path).map_err(err)?)? != *expected {
            return Err("LocalSwarm cleanup file was replaced; all remaining files were preserved".into());
        }
        directory.dir.remove_file(&entry.path).map_err(err)?;
        directory.sync()?;
    }
    directory.unchanged()?;
    if directory.dir.entries().map_err(err)?.next().is_some() {
        return Err("Unexpected entries appeared during LocalSwarm cleanup; directory was preserved".into());
    }
    parent.unchanged()?;
    #[cfg(not(windows))]
    parent.dir.remove_dir(&name).map_err(err)?;
    #[cfg(windows)]
    {
        // Close the cleanup capability (it never shares delete) and remove the
        // empty directory through an identity-checked DELETE handle instead.
        drop(directory);
        parent.remove_empty_directory(&name, &saved.owned.identity)?;
    }
    parent.sync()
}

fn reserved_paths(saved: &StoredJob) -> Result<Vec<PathBuf>, String> {
    let name = cleanup_name(saved)?;
    let mut paths = Vec::with_capacity(saved.manifest.files.len() * 3);
    for entry in &saved.manifest.files {
        paths.push(saved.owned.parent.join(&saved.owned.staging).join(&entry.path));
        paths.push(saved.owned.final_path(&entry.path));
        paths.push(saved.owned.parent.join(&name).join(&entry.path));
    }
    Ok(paths)
}

fn retain_publications(saved: &StoredJob, intent: &CleanupIntent, files: &[(usize, File)], digests: &[[u8; 32]]) -> Result<Vec<(PathBuf, crate::torrent::CleanupEntry)>, String> {
    let parent = parent(saved)?;
    if files.len() != digests.len() { return Err("Missing LocalSwarm original publication digests".into()); }
    if files.is_empty() { return Ok(Vec::new()); }
    let directory = owned_directory(&parent, &saved.owned.destination, saved)?;
    let mut publications = Vec::with_capacity(files.len());
    for ((index, file), digest) in files.iter().zip(digests) {
        let expected = intent.stamps[*index].as_ref().ok_or("Missing LocalSwarm publication ownership stamp")?;
        if Stamp::of(&Metadata::from_file(file).map_err(err)?)? != *expected {
            return Err("LocalSwarm publication changed before library-removal proof was retained".into());
        }
        directory.unchanged()?;
        let path = saved.owned.final_path(&saved.manifest.files[*index].path);
        let proof = crate::torrent::CleanupEntry::retain(&path, file)?.with_publication(expected.size, Some(*digest));
        if Stamp::of(&Metadata::from_file(file).map_err(err)?)? != *expected {
            return Err("LocalSwarm publication changed while library-removal proof was retained".into());
        }
        directory.unchanged()?;
        publications.push((path, proof));
    }
    parent.unchanged()?;
    Ok(publications)
}

impl LocalSwarmManager {
    /// Test-only path projection of the protected library-removal evidence.
    /// Paths are evidence only for the exact currently validated job, not an
    /// authorization token: the service must recheck after confirmation while
    /// binding the captured book ID, installation ID, and path under its gates.
    #[cfg(test)]
    pub fn validate_terminal_owned_paths(&self, id: &str) -> Result<Vec<PathBuf>, String> {
        Ok(self.terminal_owned_publications(id)?.into_iter().map(|(path, _)| path).collect())
    }

    /// Original-inode evidence for protected removal. Retain it from the
    /// descriptor already proven against this job, never from a fresh pathname.
    pub(crate) fn terminal_owned_publications(&self, id: &str) -> Result<Vec<(PathBuf, crate::torrent::CleanupEntry)>, String> {
        let admission = self.cancel_token();
        self.allowed()?;
        let _operation = self.inner.operation.try_lock()
            .map_err(|_| "A LocalSwarm operation is still using this download; try cleanup after it stops")?;
        check_cancel(&admission)?;
        let saved = {
            let state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
            let saved = state.jobs.get(id).cloned().ok_or("LocalSwarm job not found")?;
            validate_stored(id, &saved)?;
            if !terminal(&saved.job.status) || state.running.get(id).is_some_and(|running| !running.task.is_finished()) {
                return Err("Stop this LocalSwarm worker before preparing file cleanup".into());
            }
            saved
        };
        let Some(intent) = &saved.cleanup else {
            let (intent, files, digests) = prepare_with_access(&saved, false, &admission, Some(&self.inner.cleanup_digests))?;
            return if intent.published { retain_publications(&saved, &intent, &files, &digests) } else { Ok(Vec::new()) };
        };
        let parent = parent(&saved)?;
        let cleanup = cleanup_name(&saved)?;
        if intent.complete {
            if !parent.absent(&saved.owned.staging)? || !parent.absent(&saved.owned.destination)? || !parent.absent(&cleanup)? {
                return Err("Files appeared after this LocalSwarm cleanup completed; they cannot authorize library removal".into());
            }
            return Ok(Vec::new());
        }
        let source = intent.source(&saved);
        let captured = !parent.absent(&cleanup)?;
        if captured && !parent.absent(source)? {
            return Err("Both LocalSwarm cleanup and source names exist; cleanup ownership must be resolved first".into());
        }
        let name = if captured { cleanup.as_str() } else { source };
        if parent.absent(name)? { return Ok(Vec::new()); }
        let directory = owned_directory(&parent, name, &saved)?;
        let mut files = check_remaining(&directory, &saved, intent, captured, false)?;
        parent.unchanged()?;
        if intent.published && !captured {
            // Service refetch must repeat the content proof even when a prior
            // cleanup intent already retained inode/stamp ownership; only an
            // inode state that provably did not change may reuse its digest.
            let mut bytes = Vec::new();
            let mut digests = Vec::with_capacity(files.len());
            for (index, file) in &mut files {
                digests.push(published_digest(file, &saved, *index, &mut bytes, Some(&self.inner.cleanup_digests), &admission)?);
            }
            retain_publications(&saved, intent, &files, &digests)
        } else { Ok(Vec::new()) }
    }

    /// Rows dismissed with "keep files" remain durable cleanup principals.
    pub fn retained_jobs(&self) -> Result<Vec<DownloadJob>, String> {
        Ok(self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?.jobs.values()
            .filter(|saved| saved.history_dismissed).map(|saved| saved.job.clone()).collect())
    }

    /// Lazy counterpart: never starts discovery, listeners, workers, or offers.
    pub fn saved_retained_jobs(store: &Store) -> Result<Vec<DownloadJob>, String> {
        let (path, dir) = store.acquisition_profile_directory()?;
        let identity = storage::Identity::of(&dir.dir_metadata().map_err(err)?);
        let profile = Directory { path, dir, identity };
        if profile.absent("localswarm")? { return Ok(Vec::new()); }
        let directory = profile.child("localswarm", false)?;
        Ok(load(&directory, store)?.into_values().filter(|saved| saved.history_dismissed).map(|saved| saved.job).collect())
    }

    /// Dismissal never forfeits inode ownership of retained bytes. Only a
    /// durably completed cleanup receipt permits forgetting the principal.
    pub fn remove_terminal(&self, id: &str) -> Result<(), String> {
        self.allowed()?;
        let mut state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        let saved = state.jobs.get(id).cloned().ok_or("LocalSwarm job not found")?;
        validate_stored(id, &saved)?;
        if !terminal(&saved.job.status) || state.running.get(id).is_some_and(|running| !running.task.is_finished()) {
            return Err("Stop this LocalSwarm transfer before removing its history".into());
        }
        let running = state.running.remove(id);
        if saved.cleanup.as_ref().is_some_and(|intent| intent.complete) {
            state.jobs.remove(id);
        } else {
            state.jobs.get_mut(id).unwrap().history_dismissed = true;
        }
        if let Err(error) = self.persist(&state) {
            if !error.committed {
                state.jobs.insert(id.to_string(), saved);
                if let Some(running) = running { state.running.insert(id.to_string(), running); }
            }
            return Err(error.message);
        }
        Ok(())
    }

    /// Drop a retained row without touching any file. The kept path is left
    /// exactly as it is; only this list entry disappears.
    pub fn forget_terminal(&self, id: &str) -> Result<(), String> {
        self.allowed()?;
        let mut state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        let saved = state.jobs.get(id).cloned().ok_or("LocalSwarm job not found")?;
        validate_stored(id, &saved)?;
        if !terminal(&saved.job.status) || state.running.get(id).is_some_and(|running| !running.task.is_finished()) {
            return Err("Stop this LocalSwarm transfer before forgetting its history".into());
        }
        state.jobs.remove(id);
        // A finished worker slot must not outlive the row it belongs to:
        // stop() iterates `running`, so a stale entry would fail with
        // "LocalSwarm job not found" and block the next privacy switch.
        let running = state.running.remove(id);
        if let Err(error) = self.persist(&state) {
            if !error.committed {
                state.jobs.insert(id.to_string(), saved);
                if let Some(running) = running { state.running.insert(id.to_string(), running); }
            }
            return Err(error.message);
        }
        Ok(())
    }

    /// Delete only this terminal job's capability/inode-bound payload. The
    /// service must hold its privacy-generation/mobile-transfer admission gates.
    /// Installed destinations must first pass the protected library removal flow.
    /// History is kept until the caller separately commits remove_terminal.
    pub async fn delete_terminal(&self, id: &str) -> Result<(), String> {
        self.allowed()?;
        let manager = self.clone();
        let id = id.to_string();
        let admission = self.cancel_token();
        // Keep the operation barrier alive even when an invoking webview/task
        // retires its future. Manager stop/privacy retirement must still join it.
        tokio::spawn(async move { manager.delete_terminal_joined(&id, &admission).await }).await
            .map_err(|error| format!("Joining LocalSwarm file cleanup: {error}"))?
    }

    async fn delete_terminal_joined(&self, id: &str, admission: &CancellationToken) -> Result<(), String> {
        let _operation = self.inner.operation.lock().await;
        self.allowed()?;
        check_cancel(admission)?;
        let running = {
            let mut state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
            let saved = state.jobs.get(id).ok_or("LocalSwarm job not found")?;
            validate_stored(id, saved)?;
            if !terminal(&saved.job.status) {
                return Err("Stop this LocalSwarm transfer before deleting its files".into());
            }
            if state.running.get(id).is_some_and(|running| !running.task.is_finished() && !running.cancel.is_cancelled()) {
                return Err("A LocalSwarm worker is still using this download; stop it before deleting files".into());
            }
            state.running.remove(id)
        };
        if let Some(running) = running {
            running.cancel.cancel();
            running.task.await.map_err(|error| format!("Joining LocalSwarm cleanup worker: {error}"))?;
        }
        check_cancel(admission)?;
        let manager = self.clone();
        let id = id.to_string();
        let admission = admission.clone();
        blocking(move || manager.delete_terminal_files(&id, &admission)).await
    }

    fn delete_terminal_files(&self, id: &str, admission: &CancellationToken) -> Result<(), String> {
        check_cancel(admission)?;
        let saved = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?.jobs.get(id)
            .cloned().ok_or("LocalSwarm job not found")?;
        validate_stored(id, &saved)?;
        if !terminal(&saved.job.status) { return Err("LocalSwarm transfer is no longer terminal; files were preserved".into()); }
        let paths = reserved_paths(&saved)?;
        // Admission is checked before expensive hashing, and again atomically
        // with mutation so a newly installed/unavailable book cannot be deleted.
        self.inner.store.with_acquisition_destinations(&paths, || Ok(()))?;
        let intent = match &saved.cleanup {
            Some(intent) => intent.clone(),
            None => {
                let intent = prepare(&saved, admission, Some(&self.inner.cleanup_digests))?;
                check_cancel(admission)?;
                self.save_cleanup(id, intent.clone())?;
                intent
            }
        };
        check_cancel(admission)?;
        self.inner.store.with_acquisition_destinations(&paths, || erase(&saved, &intent))
            .map_err(|error| format!("{error}; LocalSwarm history and cleanup ownership were retained"))?;
        let mut complete = intent;
        complete.complete = true;
        self.save_cleanup(id, complete)
    }

    fn save_cleanup(&self, id: &str, intent: CleanupIntent) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        let saved = state.jobs.get_mut(id).ok_or("LocalSwarm job not found")?;
        let previous = saved.cleanup.replace(intent);
        if let Err(error) = self.persist(&state) {
            if !error.committed { state.jobs.get_mut(id).unwrap().cleanup = previous; }
            return Err(format!("Cannot commit LocalSwarm cleanup receipt: {}; history was retained", error.message));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kiwix-swarm-cleanup-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    fn fixture(root: &Path, chunks: usize, published: bool, status: &str) -> (LocalSwarmManager, String) {
        fixture_files(root, chunks, published, status, false)
    }

    fn fixture_files(root: &Path, chunks: usize, published: bool, status: &str, split: bool) -> (LocalSwarmManager, String) {
        let directory = root.join("downloads");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("user.zim"), b"unrelated user archive").unwrap();
        let mut manifest = Manifest { zim: None,
            protocol_version: 1, swarm_id: String::new(), name: "Retained failed archive".into(),
            total_bytes: chunks as u64 * 4, chunk_size_bytes: 4, chunk_hash_algo: "sha256".into(),
            files: if split {
                assert_eq!(chunks, 2);
                vec![SwarmFile { path: "book.zimaa".into(), size_bytes: 4, start_chunk_index: 0, end_chunk_index: 0 },
                    SwarmFile { path: "book.zimab".into(), size_bytes: 4, start_chunk_index: 1, end_chunk_index: 1 }]
            } else { vec![SwarmFile { path: "book.zim".into(), size_bytes: chunks as u64 * 4,
                start_chunk_index: 0, end_chunk_index: chunks as u32 - 1 }] },
            chunk_hashes: vec![hex::encode(Sha256::digest(b"data")); chunks],
        };
        manifest.swarm_id = manifest.content_address();
        supported(&manifest).unwrap();
        let manifest = Arc::new(manifest);
        let suffix = Uuid::new_v4().to_string();
        let id = format!("localswarm:{suffix}");
        let (owned, mut open) = OwnedSet::create(Directory::open(&directory).unwrap(), &manifest, &suffix).unwrap();
        for index in 0..if published { chunks } else { chunks.min(3) } {
            open.write_chunk(&manifest, index as u32, b"data").unwrap();
        }
        if published {
            open.seal(&manifest, &owned).unwrap();
            let files = manifest.files.iter().zip(&open.files).map(|(entry, file)| (
                PathBuf::from(&entry.path),
                crate::publication::VerifiedFile::new(file.try_clone().unwrap(), entry.size_bytes).unwrap(),
            )).collect::<Vec<_>>();
            open.publish(&manifest, &owned, files).unwrap();
        }
        drop(open);
        let destination = owned.final_path(&manifest.files[0].path).to_str().unwrap().to_string();
        let job = DownloadJob {
            id: id.clone(), url: format!("localswarm:{}", manifest.swarm_id), file_name: manifest.files[0].path.clone(),
            destination: destination.clone(), status: status.into(), downloaded: if published { manifest.total_bytes } else { 12.min(manifest.total_bytes) },
            total: Some(manifest.total_bytes), error: (status == "failed").then(|| "Peer disconnected".into()),
            speed: 0, eta_seconds: None, created_at: now(), updated_at: now(),
            completed_paths: if status == "complete" { vec![destination] } else { Vec::new() },
            requires_pin: false, transport: "localswarm".into(), route: None,
        };
        let saved = StoredJob {
            job, manifest, host: "127.0.0.1".into(), port: 12345, owned,
            verified: vec![published; chunks], publishing: published, direct: false,
            history_dismissed: false, cleanup: None,
        };
        validate_stored(&id, &saved).unwrap();
        let store = Arc::new(Store::new(root.join("profile")).unwrap());
        let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
        {
            let mut state = manager.inner.state.lock().unwrap();
            state.jobs.insert(id.clone(), saved);
            manager.persist(&state).unwrap();
        }
        (manager, id)
    }

    fn saved(manager: &LocalSwarmManager, id: &str) -> StoredJob {
        manager.inner.state.lock().unwrap().jobs[id].clone()
    }

    fn retained_import(saved: &StoredJob) -> crate::archive_source::RetainedSource {
        let path = saved.owned.parent.join(&saved.owned.destination);
        let files: Vec<_> = saved.manifest.files.iter().map(|entry| (
            entry.path.clone(), File::open(path.join(&entry.path)).unwrap(),
        )).collect();
        let stamps = files.iter().map(|(_, file)| Metadata::from_file(file).unwrap()).collect();
        crate::archive_source::RetainedSource::Filesystem(crate::library_files::PinnedArchive {
            directory: crate::storage::open_directory_nofollow(&path).unwrap(), path, files, stamps,
        })
    }

    #[test]
    fn import_publication_hashes_without_manager_lock_and_fences_registration() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, true, "complete");
        let saved = saved(&manager, &id);
        let destination = &saved.job.destination;
        let proof = manager.import_publication(&id, destination).unwrap();
        let source = retained_import(&saved);
        proof.verify(&source, || {
            assert!(manager.inner.state.try_lock().is_ok());
            false
        }).unwrap();
        manager.with_import_publication(&id, destination, &source, &proof, || {
            assert!(manager.inner.state.try_lock().is_err());
            Ok(())
        }).unwrap();
        let checks = std::cell::Cell::new(0);
        assert!(proof.verify(&source, || { checks.set(checks.get() + 1); checks.get() > 5 }).is_err());
        assert_eq!(checks.get(), 6, "retirement must stop before reading the remaining chunks");
    }

    #[test]
    fn import_publication_rejects_replaced_inode_and_changed_original_bytes() {
        for replacement in [true, false] {
            let root = Temp::new();
            let (manager, id) = fixture(&root.0, 2, true, "complete");
            let saved = saved(&manager, &id);
            let proof = manager.import_publication(&id, &saved.job.destination).unwrap();
            let path = saved.owned.final_path("book.zim");
            let bytes = if replacement {
                fs::rename(&path, root.0.join("retained-original.zim")).unwrap();
                b"datadata"
            } else { b"useredit" };
            fs::write(&path, bytes).unwrap();
            let source = retained_import(&saved);
            assert!(proof.verify(&source, || false).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(fs::read(root.0.join("downloads/user.zim")).unwrap(), b"unrelated user archive");
        }
    }

    #[test]
    fn import_publication_rejects_missing_parts_and_retired_jobs_before_commit() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let saved = saved(&manager, &id);
        let proof = manager.import_publication(&id, &saved.job.destination).unwrap();
        let mut source = retained_import(&saved);
        proof.verify(&source, || false).unwrap();
        manager.inner.state.lock().unwrap().jobs.get_mut(&id).unwrap().history_dismissed = true;
        let committed = std::cell::Cell::new(false);
        assert!(manager.with_import_publication(&id, &saved.job.destination, &source, &proof, || {
            committed.set(true); Ok(())
        }).is_err());
        assert!(!committed.get());
        match &mut source {
            crate::archive_source::RetainedSource::Filesystem(retained) => {
                retained.files.clear();
                retained.stamps.clear();
            }
            #[cfg(mobile)]
            _ => unreachable!(),
        }
        assert!(proof.verify(&source, || false).is_err());
        assert_eq!(fs::read(saved.owned.final_path("book.zim")).unwrap(), b"datadata");
    }

    #[test]
    fn split_import_requires_every_original_manifest_member_in_order() {
        let root = Temp::new();
        let (manager, id) = fixture_files(&root.0, 2, true, "complete", true);
        let saved = saved(&manager, &id);
        let proof = manager.import_publication(&id, &saved.job.destination).unwrap();
        let mut source = retained_import(&saved);
        proof.verify(&source, || false).unwrap();
        match &mut source {
            crate::archive_source::RetainedSource::Filesystem(retained) => {
                retained.files.swap(0, 1);
                retained.stamps.swap(0, 1);
            }
            #[cfg(mobile)]
            _ => unreachable!(),
        }
        assert!(proof.verify(&source, || false).is_err());
        let second = saved.owned.final_path("book.zimab");
        drop(source);
        fs::write(&second, b"edit").unwrap();
        let source = retained_import(&saved);
        assert!(proof.verify(&source, || false).is_err(), "same-size rewrite of a later part must fail");
        assert_eq!(fs::read(second).unwrap(), b"edit");
        assert_eq!(fs::read(saved.owned.final_path("book.zimaa")).unwrap(), b"data");
    }

    #[test]
    fn saved_import_publication_remains_lazy_and_excludes_live_manager_mutation() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let saved = saved(&manager, &id);
        let source = retained_import(&saved);
        let store = manager.inner.store.clone();
        assert!(LocalSwarmManager::saved_import_publication(&store, &id, &saved.job.destination).is_err(),
            "the live manager owns the mutation lock");
        drop(manager);
        let before = fs::read_dir(store.data_dir().join("localswarm")).unwrap().count();
        let proof = LocalSwarmManager::saved_import_publication(&store, &id, &saved.job.destination).unwrap();
        proof.verify(&source, || false).unwrap();
        let snapshot = LocalSwarmManager::saved_import_snapshot(&store).unwrap().unwrap();
        snapshot.with_import_publication(&id, &saved.job.destination, &source, &proof, || {
            assert!(LocalSwarmManager::new(store.clone(), CancellationToken::new()).is_err(),
                "manager initialization must not race saved-history registration");
            Ok(())
        }).unwrap();
        assert_eq!(fs::read_dir(store.data_dir().join("localswarm")).unwrap().count(), before);
    }

    #[test]
    fn saved_import_batches_decode_history_once_and_release_locks_before_verification() {
        let root = Temp::new();
        let first = root.0.join("first");
        fs::create_dir(&first).unwrap();
        let (manager, first_id) = fixture(&first, 2, true, "complete");
        let mut rows = vec![saved(&manager, &first_id)];
        for index in 1..4 {
            let directory = root.0.join(format!("row-{index}"));
            fs::create_dir(&directory).unwrap();
            let (other, id) = fixture(&directory, 2, true, "complete");
            let row = saved(&other, &id);
            manager.inner.state.lock().unwrap().jobs.insert(id, row.clone());
            rows.push(row);
        }
        manager.persist(&manager.inner.state.lock().unwrap()).unwrap();
        let store = manager.inner.store.clone();
        drop(manager);
        HISTORY_DECODES.with(|count| count.set(0));
        let snapshot = LocalSwarmManager::saved_import_snapshot(&store).unwrap().unwrap();
        assert_eq!(snapshot.jobs().len(), 4);
        let proofs = rows.iter().map(|row| snapshot.import_publication(&row.job.id, &row.job.destination).unwrap()).collect::<Vec<_>>();
        HISTORY_DECODES.with(|count| assert_eq!(count.get(), 1));
        drop(snapshot);
        let sources = rows.iter().map(retained_import).collect::<Vec<_>>();
        for (proof, source) in proofs.iter().zip(&sources) { proof.verify(source, || false).unwrap(); }
        let snapshot = LocalSwarmManager::saved_import_snapshot(&store).unwrap().unwrap();
        let committed = std::cell::Cell::new(0);
        for ((row, proof), source) in rows.iter().zip(&proofs).zip(&sources) {
            snapshot.with_import_publication(&row.job.id, &row.job.destination, source, proof, || {
                committed.set(committed.get() + 1); Ok(())
            }).unwrap();
        }
        assert_eq!(committed.get(), 4);
        HISTORY_DECODES.with(|count| assert_eq!(count.get(), 2, "one decode per batch, never one per row or commit"));
    }

    #[test]
    fn saved_import_batch_refuses_replaced_history_without_redecoding_it() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let row = saved(&manager, &id);
        let store = manager.inner.store.clone();
        drop(manager);
        HISTORY_DECODES.with(|count| count.set(0));
        let snapshot = LocalSwarmManager::saved_import_snapshot(&store).unwrap().unwrap();
        let proof = snapshot.import_publication(&id, &row.job.destination).unwrap();
        let journal = store.data_dir().join("localswarm/jobs.json");
        let original = fs::read(&journal).unwrap();
        fs::rename(&journal, journal.with_extension("retained")).unwrap();
        fs::write(&journal, &original).unwrap();
        let source = retained_import(&row);
        let committed = std::cell::Cell::new(false);
        assert!(snapshot.with_import_publication(&id, &row.job.destination, &source, &proof, || {
            committed.set(true); Ok(())
        }).is_err());
        assert!(!committed.get());
        HISTORY_DECODES.with(|count| assert_eq!(count.get(), 1));
        assert_eq!(fs::read(journal).unwrap(), original);
    }

    #[test]
    fn saved_import_snapshot_does_not_create_nearby_history_in_a_new_profile() {
        let root = Temp::new();
        let store = Store::new(root.0.join("profile")).unwrap();
        assert!(LocalSwarmManager::saved_import_snapshot(&store).unwrap().is_none());
        assert!(!store.data_dir().join("localswarm").exists());
    }

    #[tokio::test]
    async fn publication_cleanup_verification_stops_between_chunks_without_losing_retry_ownership() {
        struct StopAfterRead {
            input: std::io::Cursor<Vec<u8>>,
            cancel: CancellationToken,
        }
        impl Read for StopAfterRead {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let count = self.input.read(buffer)?;
                if count != 0 { self.cancel.cancel(); }
                Ok(count)
            }
        }
        impl Seek for StopAfterRead {
            fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
                self.input.seek(position)
            }
        }
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, true, "complete");
        let receipt = saved(&manager, &id);
        let path = receipt.owned.final_path("book.zim");
        let bytes = fs::read(&path).unwrap();
        let cancel = CancellationToken::new();
        let mut source = StopAfterRead {
            input: std::io::Cursor::new(bytes.clone()), cancel: cancel.clone(),
        };
        assert!(verify_published(&mut source, &receipt, 0, &mut Vec::new(), None, &cancel).is_err());
        assert_eq!(source.input.position(), 4, "retirement must not read the remaining 238 chunks");
        assert!(manager.delete_terminal_files(&id, &cancel).is_err());
        assert!(saved(&manager, &id).cleanup.is_none());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(manager.list().unwrap().len(), 1);

        // Retirement is not a partial deletion or forfeiture of ownership.
        manager.delete_terminal(&id).await.unwrap();
        assert!(!path.exists());
        assert_eq!(manager.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failed_239_chunk_stage_is_reclaimed_but_unrelated_user_files_survive() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, false, "failed");
        let receipt = saved(&manager, &id);
        let staging = receipt.owned.parent.join(&receipt.owned.staging);
        assert_eq!(fs::metadata(staging.join("book.zim")).unwrap().len(), 239 * 4);
        assert!(manager.validate_terminal_owned_paths(&id).unwrap().is_empty());
        assert!(saved(&manager, &id).cleanup.is_none(), "read-only proof must not journal");
        manager.delete_terminal(&id).await.unwrap();
        assert!(!staging.exists());
        assert!(!receipt.owned.parent.join(cleanup_name(&receipt).unwrap()).exists());
        assert_eq!(fs::read(root.0.join("downloads/user.zim")).unwrap(), b"unrelated user archive");
        assert_eq!(manager.list().unwrap().len(), 1, "disk cleanup is not history dismissal");
        manager.remove_terminal(&id).unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert!(manager.retained_jobs().unwrap().is_empty());
        assert!(load(&manager.inner.directory, &manager.inner.store).unwrap().is_empty());
    }

    #[tokio::test]
    async fn dismissal_preserves_durable_cleanup_authority_without_restart_activity() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, false, "failed");
        let receipt = saved(&manager, &id);
        let file = receipt.owned.parent.join(&receipt.owned.staging).join("book.zim");
        let bytes = fs::read(&file).unwrap();
        manager.remove_terminal(&id).unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert_eq!(manager.retained_jobs().unwrap()[0].id, id);
        assert!(LocalSwarmManager::saved_jobs(&manager.inner.store).unwrap().is_empty());
        assert_eq!(LocalSwarmManager::saved_retained_jobs(&manager.inner.store).unwrap()[0].id, id);
        let store = manager.inner.store.clone();
        drop(manager);
        let restored = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
        assert!(restored.inner.state.lock().unwrap().running.is_empty());
        assert!(restored.list().unwrap().is_empty());
        assert!(restored.resume(&id, None, None).await.is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        restored.delete_terminal(&id).await.unwrap();
        restored.remove_terminal(&id).unwrap();
        assert!(!file.exists());
        assert!(restored.retained_jobs().unwrap().is_empty());
    }

    #[tokio::test]
    async fn published_archive_requires_owned_bytes_and_no_installed_reservation() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let receipt = saved(&manager, &id);
        let owned_file = receipt.owned.final_path("book.zim");
        let imported = root.0.join("library-fixture.zim");
        fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &imported).unwrap();
        let mut book = crate::imports::load_book(imported.to_str().unwrap(), false, false).unwrap();
        book.path = owned_file.to_str().unwrap().into();
        book.available = false;
        manager.inner.store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
        let before_history = fs::read(manager.inner.directory.path.join("jobs.json")).unwrap();
        assert_eq!(manager.validate_terminal_owned_paths(&id).unwrap(), vec![owned_file.clone()]);
        assert_eq!(fs::read(manager.inner.directory.path.join("jobs.json")).unwrap(), before_history);
        assert!(manager.delete_terminal(&id).await.unwrap_err().contains("reserved"));
        assert_eq!(fs::read(&owned_file).unwrap(), b"datadata");
        assert!(saved(&manager, &id).cleanup.is_none());
        manager.inner.store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
        manager.delete_terminal(&id).await.unwrap();
        assert!(!owned_file.exists());
        assert!(root.0.join("downloads/user.zim").exists());
    }

    #[tokio::test]
    async fn modified_published_bytes_are_not_adopted_as_job_owned_content() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let file = saved(&manager, &id).owned.final_path("book.zim");
        fs::write(&file, b"useredit").unwrap();
        assert!(manager.validate_terminal_owned_paths(&id).unwrap_err().contains("changed"));
        assert!(manager.delete_terminal(&id).await.unwrap_err().contains("changed"));
        assert_eq!(fs::read(&file).unwrap(), b"useredit");
        assert_eq!(manager.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn replaced_file_or_directory_inode_is_preserved_with_history() {
        for replace_directory in [false, true] {
            let root = Temp::new();
            let (manager, id) = fixture(&root.0, 2, false, "failed");
            let receipt = saved(&manager, &id);
            let staging = receipt.owned.parent.join(&receipt.owned.staging);
            let file = staging.join("book.zim");
            if replace_directory {
                fs::rename(&staging, root.0.join("retained-stage")).unwrap();
                fs::create_dir(&staging).unwrap();
                #[cfg(unix)] {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).unwrap();
                }
            } else {
                fs::rename(&file, root.0.join("retained-book.zim")).unwrap();
            }
            fs::write(&file, b"foreign!").unwrap();
            assert!(manager.validate_terminal_owned_paths(&id).is_err());
            assert!(manager.delete_terminal(&id).await.is_err());
            assert_eq!(fs::read(&file).unwrap(), b"foreign!");
            assert_eq!(manager.list().unwrap().len(), 1);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_and_hardlinked_partial_members_are_never_deleted() {
        use std::os::unix::fs::symlink;
        for hardlink in [false, true] {
            let root = Temp::new();
            let (manager, id) = fixture(&root.0, 2, false, "failed");
            let receipt = saved(&manager, &id);
            let file = receipt.owned.parent.join(&receipt.owned.staging).join("book.zim");
            let user = root.0.join("foreign.zim");
            if hardlink {
                fs::hard_link(&file, &user).unwrap();
            } else {
                fs::rename(&file, root.0.join("retained-book.zim")).unwrap();
                fs::write(&user, b"foreign!").unwrap();
                symlink(&user, &file).unwrap();
            }
            let before = fs::read(&user).unwrap();
            assert!(manager.validate_terminal_owned_paths(&id).is_err());
            assert!(manager.delete_terminal(&id).await.is_err());
            assert_eq!(fs::read(&user).unwrap(), before);
            assert!(fs::symlink_metadata(&file).is_ok());
            assert_eq!(manager.list().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn failed_cleanup_checkpoint_preserves_payload_and_visible_history() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, false, "failed");
        let receipt = saved(&manager, &id);
        let staging = receipt.owned.parent.join(&receipt.owned.staging);
        let history = manager.inner.directory.path.join("jobs.json");
        fs::rename(&history, root.0.join("retained-history.json")).unwrap();
        fs::create_dir(&history).unwrap();
        assert!(manager.delete_terminal(&id).await.unwrap_err().contains("receipt"));
        assert!(staging.join("book.zim").exists());
        assert!(saved(&manager, &id).cleanup.is_none());
        assert_eq!(manager.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn active_worker_denies_cleanup_and_cancelled_worker_is_joined_before_mutation() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, false, "cancelled");
        let receipt = saved(&manager, &id);
        let staging = receipt.owned.parent.join(&receipt.owned.staging);
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let (exited, done) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let worker_staging = staging.clone();
        let task = tokio::spawn(async move {
            worker_cancel.cancelled().await;
            released.await.unwrap();
            assert!(worker_staging.join("book.zim").exists());
            exited.send(()).unwrap();
        });
        manager.inner.state.lock().unwrap().running.insert(id.clone(), Running { cancel: cancel.clone(), task, progress: None });
        assert!(manager.delete_terminal(&id).await.unwrap_err().contains("worker"));
        assert!(staging.exists());
        cancel.cancel();
        let deleting = manager.clone();
        let deleting_id = id.clone();
        let deletion = tokio::spawn(async move { deleting.delete_terminal(&deleting_id).await });
        // The gated worker still owns the stage. Cleanup may not detach it.
        assert!(staging.join("book.zim").exists());
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), done).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(5), deletion).await.unwrap().unwrap().unwrap();
        assert!(!staging.exists());
        assert!(manager.inner.state.lock().unwrap().running.is_empty());
    }

    #[tokio::test]
    async fn forgetting_a_terminal_row_releases_its_finished_worker_slot() {
        async fn finished(manager: &LocalSwarmManager, id: &str) {
            let task = tokio::spawn(async {});
            manager.inner.state.lock().unwrap().running.insert(
                id.to_string(),
                Running { cancel: CancellationToken::new(), task, progress: None },
            );
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if manager.inner.state.lock().unwrap().running.get(id)
                        .is_some_and(|running| running.task.is_finished()) { break; }
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
        }
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, false, "failed");
        finished(&manager, &id).await;
        manager.forget_terminal(&id).unwrap();
        assert!(manager.inner.state.lock().unwrap().running.is_empty());
        // stop() iterates `running`; a stale row would fail here.
        manager.stop().await.unwrap();

        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        manager.delete_terminal(&id).await.unwrap();
        finished(&manager, &id).await;
        manager.remove_terminal(&id).unwrap();
        assert!(manager.inner.state.lock().unwrap().running.is_empty());
        manager.stop().await.unwrap();
    }

    #[test]
    fn capture_boundary_substitution_retains_every_foreign_entry() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, false, "failed");
        let receipt = saved(&manager, &id);
        let intent = prepare(&receipt, &manager.cancel_token(), None).unwrap();
        manager.save_cleanup(&id, intent.clone()).unwrap();
        let staging = receipt.owned.parent.join(&receipt.owned.staging);
        let retained = root.0.join("retained-stage");
        let result = erase_with_hook(&receipt, &intent, || {
            fs::rename(&staging, &retained).unwrap();
            fs::create_dir(&staging).unwrap();
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).unwrap();
            }
            fs::write(staging.join("book.zim"), b"foreign!").unwrap();
        });
        assert!(result.is_err());
        assert_eq!(fs::read(staging.join("book.zim")).unwrap(), b"foreign!");
        assert!(retained.join("book.zim").exists());
        assert!(saved(&manager, &id).cleanup.is_some());
    }

    #[tokio::test]
    async fn captured_cleanup_recovery_uses_durable_identity_not_original_names() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, false, "failed");
        let receipt = saved(&manager, &id);
        let intent = prepare(&receipt, &manager.cancel_token(), None).unwrap();
        manager.save_cleanup(&id, intent).unwrap();
        let parent = parent(&receipt).unwrap();
        let name = cleanup_name(&receipt).unwrap();
        parent.capture_directory(&receipt.owned.staging, &name, &receipt.owned.identity).unwrap();
        // Simulate interruption after a journaled unlink but before completion.
        fs::remove_file(parent.path.join(&name).join("book.zim")).unwrap();
        parent.sync().unwrap();
        let store = manager.inner.store.clone();
        drop(manager);
        let restored = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
        restored.delete_terminal(&id).await.unwrap();
        assert!(!parent.path.join(name).exists());
        assert_eq!(fs::read(root.0.join("downloads/user.zim")).unwrap(), b"unrelated user archive");
        restored.remove_terminal(&id).unwrap();
        assert!(restored.list().unwrap().is_empty());
    }
    #[tokio::test]
    async fn dropped_cleanup_caller_cannot_release_stop_barrier_while_joining_worker() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 239, false, "cancelled");
        let receipt = saved(&manager, &id);
        let staging = receipt.owned.parent.join(&receipt.owned.staging);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (release, released) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move { released.await.unwrap(); });
        manager.inner.state.lock().unwrap().running.insert(id.clone(), Running { cancel, task, progress: None });
        let deleting = manager.clone();
        let deleting_id = id.clone();
        let caller = tokio::spawn(async move { deleting.delete_terminal(&deleting_id).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !manager.inner.state.lock().unwrap().running.contains_key(&id) { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        caller.abort();
        let retirement = manager.cancel_token();
        let stopping = manager.clone();
        let stop = tokio::spawn(async move { stopping.stop().await });
        tokio::time::timeout(Duration::from_secs(5), retirement.cancelled()).await.unwrap();
        assert!(!stop.is_finished(), "retired caller must not detach the worker join barrier");
        assert!(staging.join("book.zim").exists());
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), stop).await.unwrap().unwrap().unwrap();
        assert!(staging.join("book.zim").exists(), "retirement before mutation preserves the payload");
        assert_eq!(manager.list().unwrap().len(), 1);
    }

    #[test]
    fn protected_removal_proof_identifies_original_descriptor_not_replacement_path() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let publications = manager.terminal_owned_publications(&id).unwrap();
        assert_eq!(publications.len(), 1);
        let (path, proof) = &publications[0];
        let original = File::open(path).unwrap();
        fs::rename(path, root.0.join("retained-original.zim")).unwrap();
        fs::write(path, b"datadata").unwrap();
        let replacement = File::open(path).unwrap();
        assert!(!proof.matches_retained(path, &replacement).unwrap());
        assert!(proof.matches_retained(path, &original).unwrap());
        assert!(manager.terminal_owned_publications(&id).is_err());
        assert_eq!(fs::read(path).unwrap(), b"datadata");
        assert_eq!(manager.list().unwrap().len(), 1);
    }

    #[test]
    fn retained_publication_proof_rejects_sibling_name_and_replaced_parent() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let publications = manager.terminal_owned_publications(&id).unwrap();
        let (path, proof) = &publications[0];
        let original = File::open(path).unwrap();
        let sibling = path.with_file_name("other.zim");
        assert!(!proof.matches_retained(&sibling, &original).unwrap());
        let directory = path.parent().unwrap();
        // NTFS refuses to rename a directory while any member is open, so on
        // Windows reopen the same original member from its displaced parent.
        #[cfg(windows)]
        drop(original);
        fs::rename(directory, root.0.join("retained-publication")).unwrap();
        #[cfg(windows)]
        let original = File::open(root.0.join("retained-publication").join(path.file_name().unwrap())).unwrap();
        fs::create_dir(directory).unwrap();
        fs::write(path, b"datadata").unwrap();
        assert!(!proof.matches_retained(path, &original).unwrap());
        assert!(manager.terminal_owned_publications(&id).is_err());
        assert_eq!(fs::read(path).unwrap(), b"datadata");
    }

    #[test]
    fn journaled_publication_refetch_still_requires_original_manifest_bytes() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let receipt = saved(&manager, &id);
        let intent = prepare(&receipt, &manager.cancel_token(), None).unwrap();
        manager.save_cleanup(&id, intent).unwrap();
        let file = receipt.owned.final_path("book.zim");
        fs::write(&file, b"useredit").unwrap();
        assert!(manager.terminal_owned_publications(&id).is_err());
        assert_eq!(fs::read(file).unwrap(), b"useredit");
        assert_eq!(manager.list().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_digest_cache_reuses_unchanged_file_and_invalidates_on_write() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let receipt = saved(&manager, &id);
        let cache = CleanupDigestCache::default();
        // Immediately after creation (<1s), the racy-timestamp guard refuses
        // caching so a same-tick rewrite can never reuse a cached proof.
        let _ = prepare(&receipt, &manager.cancel_token(), Some(&cache)).unwrap();
        assert_eq!(cache.lock().unwrap().len(), 0);
        // Once past the 1s racy-timestamp separation margin, sub-second Unix
        // mounts populate and reuse the digest for the unchanged inode.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let _ = prepare(&receipt, &manager.cancel_token(), Some(&cache)).unwrap();
        let cached = cache.lock().unwrap().len();
        let _ = prepare(&receipt, &manager.cancel_token(), Some(&cache)).unwrap();
        assert_eq!(cache.lock().unwrap().len(), cached);
        // Mutating the file changes its inode mtime/ctime so cached proof is NOT reused.
        let file = receipt.owned.final_path("book.zim");
        fs::write(&file, b"tampered").unwrap();
        assert!(prepare(&receipt, &manager.cancel_token(), Some(&cache)).is_err());
    }

    #[test]
    fn protected_removal_proof_rejects_same_inode_same_size_content_rewrite() {
        let root = Temp::new();
        let (manager, id) = fixture(&root.0, 2, true, "complete");
        let publications = manager.terminal_owned_publications(&id).unwrap();
        let (path, proof) = &publications[0];
        let retained = File::open(path).unwrap();
        fs::write(path, b"useredit").unwrap();
        assert!(!proof.matches_retained(path, &retained).unwrap());
        assert!(manager.terminal_owned_publications(&id).is_err());
        assert_eq!(fs::read(path).unwrap(), b"useredit");
        assert_eq!(manager.list().unwrap().len(), 1);
    }
}

