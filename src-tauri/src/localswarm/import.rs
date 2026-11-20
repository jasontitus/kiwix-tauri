// SPDX-License-Identifier: GPL-3.0-or-later
//! Original publication authority for the background archive importer.
#![forbid(unsafe_code)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportPublication {
    destination: String,
    owned: OwnedSet,
    manifest: Arc<Manifest>,
}

impl ImportPublication {
    fn from_saved(id: &str, destination: &str, saved: &StoredJob) -> Result<Self, String> {
        // Callers hold already-admitted manager state or a decoded snapshot.
        // start() and decode_jobs() validate the immutable manifest once; do not
        // rehash its potentially large chunk-hash list under the commit lock.
        let first = saved.manifest.files.first().ok_or("Nearby publication has no archive")?;
        if saved.job.id != id || saved.job.status != "complete" || !saved.publishing || saved.history_dismissed
            || saved.cleanup.is_some() || saved.verified.iter().any(|verified| !verified)
            || saved.verified.len() != saved.manifest.chunk_hashes.len()
            || saved.owned.files.len() != saved.manifest.files.len()
            || saved.job.destination != destination || saved.owned.final_path(&first.path) != Path::new(destination)
            || saved.job.completed_paths.len() != 1 || saved.job.completed_paths[0] != destination
        {
            return Err("This nearby download is no longer available for import".into());
        }
        Ok(Self { destination: destination.to_owned(), owned: saved.owned.clone(), manifest: saved.manifest.clone() })
    }

    pub(crate) fn destination(&self) -> &str { &self.destination }

    /// Check the retained descriptors and the exact split set, without reading
    /// payload bytes. This is also used under the short final commit lock.
    fn validate_identity(&self, source: &crate::archive_source::RetainedSource) -> Result<(), String> {
        source.unchanged()?;
        let parent = Directory::open(&self.owned.parent)?;
        if parent.identity != self.owned.parent_identity || !parent.absent(&self.owned.staging)? {
            return Err("Nearby download parent or publication changed; files were left unchanged".into());
        }
        let directory = parent.child(&self.owned.destination, false)?;
        if directory.identity != self.owned.identity || source.files().len() != self.manifest.files.len()
            || self.owned.files.len() != self.manifest.files.len()
        {
            return Err("Nearby download ownership or split parts changed; files were left unchanged".into());
        }
        let count = directory.dir.entries().map_err(err)?.try_fold(0usize, |count, entry| entry.map(|_| count + 1)).map_err(err)?;
        if count != self.manifest.files.len() { return Err("Nearby download contains unexpected files; files were left unchanged".into()); }
        for (((name, file), entry), identity) in source.files().iter().zip(&self.manifest.files).zip(&self.owned.files) {
            let retained = Stamp::of(&Metadata::from_file(file).map_err(err)?)?;
            if name != &entry.path || retained.identity != *identity || retained.size != entry.size_bytes
                || Stamp::of(&directory.dir.symlink_metadata(&entry.path).map_err(err)?)? != retained
            {
                return Err("Nearby download file was replaced or changed; files were left unchanged".into());
            }
        }
        directory.unchanged()?;
        parent.unchanged()?;
        source.unchanged()
    }

    pub(crate) fn validate_source(&self, source: &crate::archive_source::RetainedSource, digest: bool) -> Result<(), String> {
        if digest { self.verify(source, || false) } else { self.validate_identity(source) }
    }

    /// Hash the original manifest chunks on the import worker, with no manager
    /// or Store locks held. Positional reads preserve all consumer cursors.
    pub(crate) fn verify(&self, source: &crate::archive_source::RetainedSource, cancelled: impl Fn() -> bool) -> Result<(), String> {
        let check = || if cancelled() { Err("Archive publication verification was cancelled".to_string()) } else { Ok(()) };
        check()?;
        self.validate_identity(source)?;
        let mut buffer = [0u8; 128 * 1024];
        for ((_, file), entry) in source.files().iter().zip(&self.manifest.files) {
            check()?;
            let mut offset = 0u64;
            for chunk in entry.start_chunk_index..=entry.end_chunk_index {
                let mut remaining = self.manifest.chunk_len(chunk)?;
                let mut hash = Sha256::new();
                while remaining > 0 {
                    check()?;
                    let limit = remaining.min(buffer.len());
                    #[cfg(unix)]
                    let count = std::os::unix::fs::FileExt::read_at(file, &mut buffer[..limit], offset).map_err(err)?;
                    #[cfg(windows)]
                    let count = std::os::windows::fs::FileExt::seek_read(file, &mut buffer[..limit], offset).map_err(err)?;
                    if count == 0 { return Err("Nearby download ended before its verified size; files were left unchanged".into()); }
                    hash.update(&buffer[..count]);
                    offset += count as u64;
                    remaining -= count;
                }
                check()?;
                if hex::encode(hash.finalize()) != self.manifest.chunk_hashes[chunk as usize] {
                    return Err("Nearby download bytes changed after publication; files were left unchanged".into());
                }
            }
            if offset != entry.size_bytes { return Err("Nearby download size does not match its manifest".into()); }
        }
        check()?;
        self.validate_identity(source)
    }
}

fn authority(id: &str, destination: &str, saved: &StoredJob) -> Result<crate::acquisition::ImportPublication, String> {
    Ok(crate::acquisition::ImportPublication::from_localswarm(ImportPublication::from_saved(id, destination, saved)?))
}

/// One bounded journal decode for a synchronous listing or registration batch.
/// The shared manager lock is released before background payload verification.
pub(crate) struct SavedImportSnapshot {
    directory: Directory,
    lock: File,
    lock_identity: crate::integrity::ContentIdentity,
    journal: File,
    journal_identity: crate::integrity::ContentIdentity,
    jobs: BTreeMap<String, StoredJob>,
}

fn content_identity(file: &File) -> Result<crate::integrity::ContentIdentity, String> {
    crate::integrity::ContentIdentity::from_files(&[(String::new(), file.try_clone().map_err(err)?)])
}

impl SavedImportSnapshot {
    fn unchanged(&self) -> Result<(), String> {
        self.directory.unchanged()?;
        let current_lock = self.directory.file("manager.lock", false, false)?;
        let current_journal = self.directory.read_file("jobs.json")?;
        if content_identity(&self.lock)? != self.lock_identity || content_identity(&current_lock)? != self.lock_identity
            || content_identity(&self.journal)? != self.journal_identity || content_identity(&current_journal)? != self.journal_identity
        {
            return Err("Nearby download history changed during import; retry the import".into());
        }
        Ok(())
    }

    pub(crate) fn jobs(&self) -> Vec<DownloadJob> {
        self.jobs.values().filter(|saved| !saved.history_dismissed).map(|saved| saved.job.clone()).collect()
    }

    pub(crate) fn import_publication(&self, id: &str, destination: &str) -> Result<crate::acquisition::ImportPublication, String> {
        self.unchanged()?;
        authority(id, destination, self.jobs.get(id).ok_or("Nearby download no longer exists")?)
    }

    pub(crate) fn with_import_publication<T>(
        &self, id: &str, destination: &str, source: &crate::archive_source::RetainedSource,
        verified: &crate::acquisition::ImportPublication, commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let current = self.import_publication(id, destination)?;
        if &current != verified { return Err("Nearby publication changed during verification; retry the import".into()); }
        current.validate_source(source, false)?;
        commit()
    }
}

impl LocalSwarmManager {
    pub(crate) fn import_publication(&self, id: &str, destination: &str) -> Result<crate::acquisition::ImportPublication, String> {
        self.allowed()?;
        let state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        authority(id, destination, state.jobs.get(id).ok_or("Nearby download no longer exists")?)
    }

    pub(crate) fn with_import_publication<T>(
        &self, id: &str, destination: &str, source: &crate::archive_source::RetainedSource,
        verified: &crate::acquisition::ImportPublication, commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.allowed()?;
        let state = self.inner.state.lock().map_err(|_| "LocalSwarm state unavailable")?;
        let current = authority(id, destination, state.jobs.get(id).ok_or("Nearby download no longer exists")?)?;
        if &current != verified { return Err("Nearby publication changed during verification; retry the import".into()); }
        current.validate_source(source, false)?;
        commit()
    }

    /// Acquire outside the importer mutex, once per batch. Neither loading the
    /// journal nor looking up per-row proofs initializes discovery or a manager.
    pub(crate) fn saved_import_snapshot(store: &Store) -> Result<Option<SavedImportSnapshot>, String> {
        if store.private_mode()? || store.read_only()? { return Err("LocalSwarm is disabled in private/read-only mode".into()); }
        let (path, dir) = store.acquisition_profile_directory()?;
        let profile = Directory { path, identity: storage::Identity::of(&dir.dir_metadata().map_err(err)?), dir };
        if profile.absent("localswarm")? { return Ok(None); }
        let directory = profile.child("localswarm", false)?;
        if directory.absent("jobs.json")? { return Ok(None); }
        let lock = directory.file("manager.lock", false, false)?;
        let lock_identity = content_identity(&lock)?;
        let journal = directory.read_file("jobs.json")?;
        let journal_identity = content_identity(&journal)?;
        let mut bytes = Vec::new();
        journal.try_clone().map_err(err)?.take(STATE_LIMIT + 1).read_to_end(&mut bytes).map_err(err)?;
        if bytes.len() as u64 > STATE_LIMIT { return Err("LocalSwarm state exceeds its size limit".into()); }
        let jobs = decode_jobs(&bytes, store)?;
        let snapshot = SavedImportSnapshot { directory, lock, lock_identity, journal, journal_identity, jobs };
        snapshot.unchanged()?;
        Ok(Some(snapshot))
    }

    pub(crate) fn saved_import_publication(store: &Store, id: &str, destination: &str) -> Result<crate::acquisition::ImportPublication, String> {
        Self::saved_import_snapshot(store)?.ok_or("Nearby download no longer exists")?.import_publication(id, destination)
    }

}
