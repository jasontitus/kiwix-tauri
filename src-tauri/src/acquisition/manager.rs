// SPDX-License-Identifier: GPL-3.0-or-later
//! The public `DownloadManager` API: admission, control, listing and import handoff.
#[allow(unused_imports)]
use super::*;

impl DownloadManager {
    /// Retained ownership ledgers, including dismissed rows, remain excluded
    /// from automatic imports until their files are explicitly managed. A scan
    /// snapshots this once instead of re-normalizing every row per discovered
    /// file.
    pub(crate) fn monitored_paths(&self) -> Result<MonitoredPathClaims, String> {
        let destinations: Vec<PathBuf> = self
            .inner
            .state()?
            .jobs
            .values()
            .map(|stored| PathBuf::from(&stored.job.destination))
            .collect();
        MonitoredPathClaims::new(destinations)
    }

    pub(crate) fn saved_monitored_paths(store: &Store) -> Result<MonitoredPathClaims, String> {
        // An absent or invalid root carries no knowable claims; the manager
        // preserves and quarantines invalid content quietly on first use. A
        // transient read failure is different: treating it as "nothing is
        // claimed" would let a monitored scan adopt a completed download as an
        // external book without the HTTP integrity gate.
        let jobs: BTreeMap<String, StoredJob> = match saved_monitored_record(
            store, &["downloads", "downloads.json"], STATE_LIMIT,
            crate::profile_paths::RecordKind::Downloads,
        ) {
            Ok(jobs) => jobs.unwrap_or_default(),
            Err(SavedRecordError::NoClaims) => BTreeMap::new(),
            Err(SavedRecordError::Unavailable(error)) => return Err(error),
        };
        let mut claims = Vec::with_capacity(jobs.len().min(MAX_JOBS));
        if jobs.len() <= MAX_JOBS {
            for (id, stored) in jobs {
                if id != stored.job.id {
                    continue;
                }
                if let Ok((destination, _)) = paths(&stored.job) {
                    claims.push(destination);
                }
            }
        }
        MonitoredPathClaims::new(claims)
    }

    pub(crate) fn new(
        data_dir: PathBuf,
        store: Arc<Store>,
        admission: crate::update_admission::AdmissionGate,
    ) -> Result<Self, String> {
        fs::create_dir_all(&data_dir).map_err(|e| format!("Creating downloads directory: {e}"))?;
        let data_dir = fs::canonicalize(data_dir)
            .map_err(|e| format!("Resolving downloads directory: {e}"))?;
        let lock_path = data_dir.join("downloads.lock");
        if fs::symlink_metadata(&lock_path).is_ok() {
            regular_file(&lock_path)?;
        }
        // The lock file carries no data; exclusion comes from the lock below.
        // Share it on Windows so a second manager reaches that lock and reports
        // "owned by another manager" instead of a sharing violation on open.
        let process_lock = writable_options_shared()
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|e| format!("Opening download manager lock: {e}"))?;
        try_lock_exclusive(&process_lock)
            .map_err(|e| format!("Another download manager owns this data directory: {e}"))?;
        let state_path = data_dir.join("downloads.json");
        let mut history_notice = None;
        let (mut jobs, mut load_warnings): (BTreeMap<String, StoredJob>, Vec<String>) =
            match regular_file(&state_path)? {
                Some(meta) => match load_saved_downloads(&state_path, meta.len(), &store) {
                    Ok(loaded) => loaded,
                    Err(reason) => {
                        // Preserve the unreadable record byte-for-byte and start
                        // empty. A bad record must never block the profile:
                        // download history is recoverable, the app is not
                        // optional. This is a quiet notice, not an error.
                        let preserved = quarantine_state(&state_path, &data_dir)?;
                        crate::diagnostics::record(&store, "downloads", Some(&reason));
                        eprintln!(
                            "Saved download history was unreadable ({reason}); preserved at {}",
                            preserved.display()
                        );
                        history_notice = Some(preserved.to_string_lossy().into_owned());
                        (BTreeMap::new(), Vec::new())
                    }
                },
                None => (BTreeMap::new(), Vec::new()),
            };
        let mut unrestored = Vec::new();
        for (id, stored) in &mut jobs {
            if let Err(reason) = validate_loaded_download(id, stored) {
                // Keep the row instead of pruning it. Its destination and
                // ownership claims are the only durable proof of the bytes on
                // disk; deleting the row would orphan a multi-gigabyte part
                // and make it impossible to remove from the list.
                stored.job.status = "failed".into();
                stored.job.interrupted = false;
                stored.job.error = Some(
                    "Kiwix could not restore this saved download. Remove it from the list if you no longer need it"
                        .into(),
                );
                unrestored.push(format!("{id}: {reason}"));
            }
        }
        if !unrestored.is_empty() {
            load_warnings.push(
                "Some saved downloads could not be restored and are shown as failed.".into(),
            );
            // The specific reason stays in diagnostics; the row itself carries
            // the plain-language message a person can act on.
            crate::diagnostics::record(
                &store,
                "downloads",
                Some(&format!(
                    "Some saved downloads could not be restored and are shown as failed: {}",
                    unrestored.join("; ")
                )),
            );
        }
        let (limit_changed, _) = watch::channel(());
        #[cfg(not(test))]
        drop(load_warnings);
        let inner = Arc::new(Inner {
            data_dir,
            store,
            admission,
            state: Mutex::new(State {
                jobs,
                running: HashMap::new(),
            }),
            journal_generation: AtomicU64::new(0),
            published_generation: Mutex::new(0),
            pending_starts: Mutex::new(HashMap::new()),
            client: OnceCell::new(),
            shared_grants: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(3)),
            download_limit: AtomicU64::new(0),
            bandwidth: Mutex::new(BandwidthBudget {
                updated: Instant::now(),
                credit: 0.0,
                generation: 0,
            }),
            limit_changed,
            _process_lock: process_lock,
            history_notice,
            #[cfg(test)]
            load_warnings,
            #[cfg(test)]
            hash_passes: AtomicU64::new(0),
            #[cfg(test)]
            hash_delay: AtomicU64::new(0),
            #[cfg(test)]
            finalize_gate: Mutex::new(None),
            #[cfg(test)]
            verify_gates: Mutex::new([None, None]),
            #[cfg(all(test, unix))]
            native_copy_gate: Mutex::new(None),
            #[cfg(all(test, unix))]
            native_copy_bytes: AtomicU64::new(0),
            #[cfg(test)]
            warning_available: AtomicU64::new(u64::MAX),
        });
        {
            let mut state = inner.state()?;
            inner.save(&state)?;
            // Cancelled rows can retain a crash-interrupted private capture.
            // Recovery needs the live journal writer before it may move/unlink.
            let cancelled: Vec<_> = state.jobs.iter()
                .filter(|(_, stored)| stored.job.status == "cancelled"
                    && !stored.history_dismissed && !stored.files_deleted)
                .map(|(id, _)| id.clone()).collect();
            for id in &cancelled {
                let cleanup = remove_job_part(&inner, &mut state, id);
                if let Some(stored) = state.jobs.get_mut(id) {
                    stored.job.error = cleanup.err();
                }
            }
            if !cancelled.is_empty() { inner.save(&state)?; }
        }
        // A loaded transfer whose journal already records every byte as
        // durable is finished here instead of waiting for a resume: the worker
        // that would have published it may have died at 100%, and an offline
        // device must not need network admission to finish local work.
        let complete: Vec<String> = {
            let state = inner.state()?;
            state
                .jobs
                .iter()
                .filter(|(_, stored)| {
                    (stored.job.status == "verifying"
                        || (stored.job.status == "paused" && stored.job.interrupted))
                        && durable_finalize_candidate(stored)
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in complete {
            match finalize_durable_download(&inner, &id, FinalizeMode::Startup) {
                Ok(Outcome::Complete) => {}
                Ok(_) => {
                    // A local finalize has no worker to stop it; only a policy
                    // revocation could reach here, and its own row state wins.
                }
                Err(error) => {
                    // Keep the row actionable: a failed download can be resumed
                    // or cancelled, unlike a permanent 100% "downloading" entry.
                    let _ = inner.update(&id, |stored| {
                        stored.job.status = "failed".into();
                        stored.job.interrupted = false;
                        stored.job.speed = 0;
                        stored.job.eta_seconds = None;
                        stored.job.error = Some(error.chars().take(2000).collect());
                    });
                }
            }
        }
        start_recovery_sweep(&inner);
        Ok(Self { inner })
    }

    #[cfg(test)]
    pub(crate) fn load_warnings(&self) -> &[String] {
        &self.inner.load_warnings
    }

    /// Quarantine receipt for the downloads panel, or `None` on a clean launch.
    pub fn history_notice(&self) -> Option<String> {
        self.inner.history_notice.clone()
    }

    pub fn list(&self) -> Result<Vec<DownloadJob>, String> {
        Ok(self
            .inner
            .state()?
            .jobs
            .values()
            .filter(|stored| !stored.history_dismissed)
            .map(|j| j.job.clone())
            .collect())
    }

    pub(crate) fn list_snapshots(&self) -> Result<Vec<DownloadSnapshot>, String> {
        Ok(self.inner.state()?.jobs.values()
            .filter(|stored| !stored.history_dismissed)
            .map(StoredJob::status_snapshot).collect())
    }

    pub(crate) fn retained_snapshots(&self) -> Result<Vec<DownloadSnapshot>, String> {
        Ok(self.inner.state()?.jobs.values()
            .filter(|stored| stored.history_dismissed)
            .map(StoredJob::status_snapshot).collect())
    }

    /// Read-only/private history cannot construct a mutable manager.
    pub(crate) fn saved_transport(row: &serde_json::Value) -> DownloadTransport {
        let recorded = match row.get("transport").and_then(serde_json::Value::as_str) {
            Some("http") => Some(DownloadTransport::Http),
            Some("quic-library") => Some(DownloadTransport::QuicLibrary),
            Some("unknown") => Some(DownloadTransport::Unknown),
            _ => None,
        };
        DownloadTransport::resolve(recorded, row.get("url").and_then(serde_json::Value::as_str).unwrap_or(""))
    }

    /// A durable paused/cancelled status records intent, not acknowledgement:
    /// its retiring worker may still own a file if the stopping caller left.
    #[cfg(any(not(mobile), test))]
    pub(crate) fn has_running_work(&self) -> Result<bool, String> {
        Ok(self.inner.state()?.running.values().any(|running| {
            !*running.finished.borrow() && running.finished.has_changed().is_ok()
        }))
    }

    /// Worker ownership, not a visible downloading row, keeps local reception
    /// active through scheduler waits, verification and publication.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) fn has_active_shared_library_work(&self) -> Result<bool, String> {
        let state = self.inner.state()?;
        Ok(state.running.keys().any(|id| state.jobs.get(id)
            .is_some_and(|stored| stored.job.url.starts_with("kiwix-library:"))))
    }

    /// Hide terminal history without discarding files or their cleanup authority.
    pub fn remove_terminal(&self, id: &str) -> Result<(), String> {
        let mut state = self.inner.state()?;
        let key = state_key(&state, id).ok_or("Unknown download ID")?;
        let stored = state.jobs.get(&key).ok_or("Unknown download ID")?;
        if state.running.contains_key(&key)
            || !matches!(
                stored.job.status.as_str(),
                "complete" | "failed" | "cancelled"
            )
        {
            return Err(
                "Only finished complete, failed or cancelled downloads can be removed from history"
                    .into(),
            );
        }
        let previous = stored.clone();
        if stored.files_deleted { state.jobs.remove(&key); }
        else { state.jobs.get_mut(&key).ok_or("Unknown download ID")?.history_dismissed = true; }
        if let Err(error) = self.inner.save(&state) {
            state.jobs.insert(key, previous);
            return Err(error);
        }
        Ok(())
    }

    /// Drop a retained row without touching any file. A retained row may have
    /// no provable ownership, so no deletion is ever implied: the path stays
    /// exactly as it is and the row leaves the Kept download files list.
    pub fn forget_terminal(&self, id: &str) -> Result<(), String> {
        let mut state = self.inner.state()?;
        let key = state_key(&state, id).ok_or("Unknown download ID")?;
        let stored = state.jobs.get(&key).ok_or("Unknown download ID")?;
        if state.running.contains_key(&key)
            || !matches!(
                stored.job.status.as_str(),
                "complete" | "failed" | "cancelled"
            )
        {
            return Err(
                "Only finished complete, failed or cancelled downloads can be forgotten".into(),
            );
        }
        let previous = state.jobs.remove(&key).ok_or("Unknown download ID")?;
        if let Err(error) = self.inner.save(&state) {
            state.jobs.insert(key, previous);
            return Err(error);
        }
        Ok(())
    }

    /// Read-only proof before any installed-library deletion. Missing outputs
    /// are omitted: callers must never remove an installation for an omitted path.
    pub async fn terminal_owned_publications(&self, id: &str) -> Result<Vec<(PathBuf, crate::torrent::CleanupEntry)>, String> {
        let running = {
            let state = self.inner.state()?;
            let stored = state.jobs.get(id).ok_or("Unknown download ID")?;
            if !matches!(stored.job.status.as_str(), "complete" | "failed" | "cancelled") {
                return Err("Only terminal downloads can authorize cleanup".into());
            }
            state.running.get(id).cloned()
        };
        if let Some(mut running) = running {
            while !*running.finished.borrow() {
                running.finished.changed().await.map_err(|_| "Download worker did not acknowledge retirement")?;
            }
        }
        let stored = {
            let state = self.inner.state()?;
            let stored = state.jobs.get(id).cloned().ok_or("Unknown download ID")?;
            if state.running.contains_key(id)
                || !matches!(stored.job.status.as_str(), "complete" | "failed" | "cancelled")
            {
                return Err("Download changed during cleanup authorization".into());
            }
            stored
        };
        let (stored, proofs) = tokio::task::spawn_blocking(move || {
            let proofs = Self::validate_terminal_snapshot(&stored)?;
            Ok::<_, String>((stored, proofs))
        }).await.map_err(|error| format!("Validating HTTP cleanup ownership: {error}"))??;
        let state = self.inner.state()?;
        let current = state.jobs.get(id).ok_or("Download was removed during cleanup authorization")?;
        if state.running.contains_key(id) || current != &stored {
            return Err("Download changed during cleanup authorization; retry with its current state".into());
        }
        Ok(proofs)
    }

    pub(super) fn validate_terminal_snapshot(stored: &StoredJob) -> Result<Vec<(PathBuf, crate::torrent::CleanupEntry)>, String> {
        let (destination, part) = paths(&stored.job)?;
        for (path, ownership) in [(&part, &stored.part_ownership), (&destination, &stored.published_ownership)] {
            if let Some(ownership) = ownership { ownership.validate_cleanup(path)?; }
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
                Ok(_) => {}
            }
            ownership.as_ref().ok_or_else(|| format!("Cannot prove original download ownership of {}; no installed archive was authorized for deletion", path.display()))?
                .validate_existing(path)?.ok_or("Owned download disappeared during cleanup authorization")?;
        }
        let Some(metadata) = regular_file(&destination)? else { return Ok(Vec::new()); };
        let ownership = stored.published_ownership.as_ref().ok_or("Published download has no original inode proof")?;
        let intent = stored.publication.as_ref().ok_or("Published download has no durable checksum proof")?;
        let (digest, file) = publication_digest(&destination, &metadata)?.ok_or("Published download could not be verified")?;
        if metadata.len() != intent.size || digest != intent.sha256 || !ownership.matches(&destination, &file)? {
            return Err("Published archive no longer belongs to this download; library deletion was not authorized".into());
        }
        let mut expected = [0u8; 32];
        hex::decode_to_slice(&intent.sha256, &mut expected).map_err(|error| error.to_string())?;
        Ok(vec![(destination, ownership.with_publication(intent.size, Some(expected)))])
    }

    /// The edition verifier must hold its pinned import while comparing this
    /// durable publication receipt. Path names and archive UUIDs alone confer
    /// neither ownership nor replacement authority.
    pub(crate) fn validate_edition_publication(
        &self, id: &str, destination: &str,
        source: &crate::archive_source::RetainedSource,
    ) -> Result<(), String> {
        let publication = self.import_publication(id)?;
        if publication.destination() != destination {
            return Err("Downloaded archive path changed before import".into());
        }
        publication.verify(source, || false)?;
        self.with_import_publication(id, source, &publication, || Ok(()))
    }

    pub(crate) fn import_publication(&self, id: &str) -> Result<ImportPublication, String> {
        let state = self.inner.state()?;
        ImportPublication::from_stored(state.jobs.get(id).ok_or("Downloaded archive is no longer available")?)
    }

    /// The import worker verified the exact original publication digest without
    /// holding manager locks. Revalidate its unchanged source stamp and immutable
    /// authority here, then hold acquisition ownership through the Store commit.
    pub(crate) fn with_import_publication<T>(
        &self, id: &str, source: &crate::archive_source::RetainedSource,
        verified: &ImportPublication,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let state = self.inner.state()?;
        let current = ImportPublication::from_stored(
            state.jobs.get(id).ok_or("Downloaded archive is no longer available")?,
        )?;
        if &current != verified {
            return Err("Download ownership changed during verification; retry the import".into());
        }
        current.validate_source(source, false)?;
        commit()
    }

    /// Explicit disk deletion is separate from history dismissal. The terminal
    /// worker must finish first. Ownership is proved outside the state lock and
    /// the row is re-verified under it before any unlink, so a resume or
    /// ownership change always wins over a stale deletion.
    pub async fn delete_terminal(&self, id: &str) -> Result<(), String> {
        let running = {
            let state = self.inner.state()?;
            let stored = state.jobs.get(id).ok_or("Unknown download ID")?;
            if !matches!(stored.job.status.as_str(), "complete" | "failed" | "cancelled") {
                return Err("Only finished downloads can have their files deleted".into());
            }
            state.running.get(id).cloned()
        };
        if let Some(mut running) = running {
            while !*running.finished.borrow() {
                running.finished.changed().await.map_err(|_| "Download worker exited without acknowledging retirement")?;
            }
        }
        #[cfg(target_os = "ios")]
        if let Some(native) = self.inner.snapshot(id)?.native {
            crate::ios_background::stop(id, &native.generation, true).await?;
        }
        let manager = self.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || manager.delete_terminal_files(&id))
            .await.map_err(|error| format!("Deleting owned HTTP download files: {error}"))?
    }

    pub(super) fn delete_terminal_files(&self, id: &str) -> Result<(), String> {
        // Snapshot the row and prove the journal is writable, then release the
        // lock. The ownership proof and the full SHA-256 below must never block
        // download-status queries.
        let stored = {
            let state = self.inner.state()?;
            let stored = state.jobs.get(id).cloned().ok_or("Unknown download ID")?;
            if state.running.contains_key(id)
                || !matches!(stored.job.status.as_str(), "complete" | "failed" | "cancelled")
            {
                return Err("Download changed while its files were being deleted".into());
            }
            // A failed checkpoint must be discovered before payload mutation.
            let journal = self.inner.stage(&state)?;
            drop(state);
            self.inner.publish(journal)?;
            stored
        };
        let (destination, part) = paths(&stored.job)?;
        for (path, ownership) in [(&part, &stored.part_ownership), (&destination, &stored.published_ownership)] {
            if let Some(ownership) = ownership { ownership.validate_cleanup(path)?; }
            else {
                match fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                    Ok(_) => return Err(format!("Cannot prove download ownership of {}; files and history were preserved", path.display())),
                }
            }
        }
        let published_exists = regular_file(&destination)?.is_some();
        let published_proof = match (&stored.published_ownership, &stored.publication) {
            (Some(_), Some(intent)) => {
                let mut expected = [0u8; 32];
                hex::decode_to_slice(&intent.sha256, &mut expected).map_err(|error| error.to_string())?;
                Some((intent.size, expected))
            }
            (Some(_), None) if published_exists => {
                return Err("Published download has no durable checksum proof".into());
            }
            _ => None,
        };
        // Prove the published bytes while no lock is held. The identity checks
        // below and inside delete_owned_file pin the same inode, so the unlink
        // still requires proof that this acquisition wrote the file.
        let mut published_stamp = None;
        if published_exists {
            if let (Some(ownership), Some((size, expected))) = (&stored.published_ownership, published_proof) {
                let mut file = ownership.validate_existing(&destination)?
                    .ok_or("Published download disappeared before cleanup")?;
                let mut signature = [0u8; 4];
                file.read_exact(&mut signature).map_err(|error| error.to_string())?;
                let before = file.metadata().map_err(|error| error.to_string())?;
                if before.len() != size || signature != *b"ZIM\x04" {
                    return Err("Published download changed; file and history were preserved".into());
                }
                if !ownership.with_publication(size, Some(expected))
                    .matches_retained_controlled(&destination, &file, || false)?
                {
                    return Err("Published download changed; file and history were preserved".into());
                }
                let after = file.metadata().map_err(|error| error.to_string())?;
                if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
                    return Err("Published download changed; file and history were preserved".into());
                }
                published_stamp = Some((after.len(), after.modified().ok()));
            }
        }
        // Re-lock only after the digest. A download that changed in the
        // meantime is never deleted.
        let mut state = self.inner.state()?;
        {
            let current = state.jobs.get(id).ok_or("Download changed while its files were being deleted")?;
            if state.running.contains_key(id)
                || current.job.status != stored.job.status
                || current.files_deleted != stored.files_deleted
                || current.part_ownership != stored.part_ownership
                || current.published_ownership != stored.published_ownership
                || current.publication != stored.publication
            {
                return Err("Download changed while its files were being deleted".into());
            }
        }
        // Pin the exact inode whose bytes were hashed immediately before the
        // unlink. Replacing the entry cannot pass; a same-inode rewrite that
        // keeps size and mtime is the only change this ordering cannot observe.
        if let (Some((size, modified)), Some(ownership)) = (published_stamp, &stored.published_ownership) {
            let file = ownership.validate_existing(&destination)?
                .ok_or("Published download disappeared before cleanup")?;
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            if metadata.len() != size || metadata.modified().ok() != modified {
                return Err("Published download changed; file and history were preserved".into());
            }
        }
        self.inner.store.with_acquisition_destinations(&[&destination, &part], || {
            // The published member is first. Its complete digest was verified
            // above (public name) or by `validate_cleanup` (captured payload),
            // so delete_owned_file only has to pin the inode it moves and never
            // re-reads gigabytes while the state lock is held.
            for (path, published) in [(&destination, true), (&part, false)] {
                let ownership = if published { &stored.published_ownership } else { &stored.part_ownership };
                if let Some(mut ownership) = ownership.clone() {
                    if published {
                        if let Some((size, digest)) = published_proof {
                            // If the public name was absent when observed, a
                            // file may reappear before the capture move. Keep
                            // the intent digest then, but an existing capture
                            // was already verified by validate_cleanup above;
                            // re-attaching it would re-read the payload under
                            // the state lock.
                            ownership = ownership.with_publication(
                                size,
                                (!published_exists && !ownership.cleanup_started())
                                    .then_some(digest),
                            );
                        }
                    }
                    crate::torrent::delete_owned_file(path, &mut ownership, |entry| {
                        // Keep the durable digest in the journal even though the
                        // delete path no longer needs to read the bytes.
                        let persisted = match published_proof {
                            Some((size, digest)) if published => entry.with_publication(size, Some(digest)),
                            _ => entry.clone(),
                        };
                        let job = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
                        if published { job.published_ownership = Some(persisted); }
                        else { job.part_ownership = Some(persisted); }
                        self.inner.save(&state)
                    })?;
                } else if fs::symlink_metadata(path).is_ok() {
                    return Err(format!("Cannot prove ownership of {}; files and download history were preserved. Remove installed archives through the library", path.display()));
                } else if let Err(error) = fs::symlink_metadata(path) {
                    if error.kind() != std::io::ErrorKind::NotFound { return Err(error.to_string()); }
                }
            }
            Ok(())
        })?;
        // The store lock is released; finish the history update. A dismissed
        // row stays dismissed with its files deleted, exactly as before, so a
        // later `remove_terminal` retires it.
        let Some(previous) = state.jobs.get(id).cloned() else {
            // Forgotten during the deletion: the files are gone and no row is
            // left to record it.
            return Ok(());
        };
        state.jobs.get_mut(id).ok_or("Unknown download ID")?.files_deleted = true;
        let journal = match self.inner.stage(&state) {
            Ok(journal) => journal,
            Err(error) => {
                restore_deleted_row(&mut state, id, previous);
                return Err(error);
            }
        };
        drop(state);
        if let Err(error) = self.inner.publish(journal) {
            if let Ok(mut state) = self.inner.state() {
                restore_deleted_row(&mut state, id, previous);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Shared across HTTP archive and Metalink bodies; zero means unlimited.
    pub fn set_download_limit(&self, limit: u64) -> Result<(), String> {
        let mut budget = self
            .inner
            .bandwidth
            .lock()
            .map_err(|_| "Download bandwidth lock is poisoned")?;
        if self.inner.download_limit.load(Ordering::Acquire) == limit {
            return Ok(());
        }
        budget.credit = 0.0;
        budget.updated = Instant::now();
        budget.generation = budget.generation.wrapping_add(1);
        self.inner.download_limit.store(limit, Ordering::Release);
        self.inner.limit_changed.send_replace(());
        Ok(())
    }

    pub async fn start(
        &self,
        url: String,
        destination_dir: PathBuf,
        file_name: Option<String>,
        expected_size: Option<u64>,
        space_acknowledgement: Option<String>,
    ) -> Result<DownloadSnapshot, String> {
        self.start_transport(url, destination_dir, file_name, expected_size, space_acknowledgement, AcquisitionSource::Internet(None)).await
    }

    /// Reauthorization reuses the durable row, never a second part or destination.
    /// `replace_earlier` cancels (and discards the partial file of) a retained
    /// Nearby download of the same file from an earlier sharing session, which
    /// otherwise blocks this one; the UI asks the user before setting it.
    pub(crate) async fn start_shared_library(
        &self, grant: crate::shared_library::ScopedDownloadGrant, destination_dir: PathBuf,
        space_acknowledgement: Option<String>, replace_earlier: bool,
    ) -> Result<DownloadSnapshot, String> {
        grant.check().await?;
        let book = grant.book()?.clone();
        let identity = grant.identity()?;
        let existing = {
            let state = self.inner.state()?;
            let stored = state.jobs.values()
                .find(|stored| !stored.history_dismissed && !stored.files_deleted && stored.job.url == identity
                    && matches!(stored.job.status.as_str(), "paused" | "failed"));
            if let Some(stored) = stored {
                if stored.job.file_name != book.file_name || stored.expected_size != Some(book.size) {
                    return Err("The shared archive changed; cancel the previous download before downloading it again".into());
                }
                let id = stored.job.id.clone();
                self.inner.shared_grants.lock().map_err(|_| "Shared library grant lock is poisoned")?.insert(id.clone(), grant.clone());
                Some(id)
            } else {
                None
            }
        };
        if let Some(id) = existing {
            return self.resume_project(&id, space_acknowledgement, StoredJob::status_snapshot).await;
        }
        // Identity is session/peer/book, so after the same sender restarts
        // sharing (a new session) its earlier paused or failed download of this
        // book cannot be rebound above (by design), yet it still owns the
        // destination and its Retry cannot work. Only that exact case -- same
        // peer and book, different session, same destination -- is offered for
        // replacement; anything else keeps the generic ownership refusal.
        let destination = fs::canonicalize(&destination_dir).ok()
            .map(|directory| directory.join(&book.file_name))
            .and_then(|path| path.to_str().map(str::to_owned));
        let earlier: Vec<String> = match (library_identity_parts(&identity), destination.as_deref()) {
            (Some((session, peer, book_id)), Some(destination)) => self.inner.state()?.jobs.values()
                .filter(|stored| {
                    !stored.history_dismissed
                        && matches!(stored.job.status.as_str(), "paused" | "failed")
                        && stored.job.destination == destination
                        && library_identity_parts(&stored.job.url).is_some_and(|(earlier_session, earlier_peer, earlier_book)| {
                            earlier_peer == peer && earlier_book == book_id && earlier_session != session
                        })
                })
                .map(|stored| stored.job.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        if !earlier.is_empty() {
            if !replace_earlier {
                return Err(format!("{SHARED_EARLIER_SESSION}: {}", book.file_name));
            }
            // Preflight what the new start can still refuse before discarding
            // the earlier partial download, so a refusal never leaves neither.
            grant.check().await?;
            crate::storage::check_file_size(&destination_dir, book.size)?;
            if destination.as_deref().is_some_and(|destination| fs::symlink_metadata(destination).is_ok()) {
                return Err("Destination already exists; existing files are never overwritten".into());
            }
            for id in &earlier {
                self.cancel(id).await?;
            }
        }
        self.start_transport(identity, destination_dir, Some(book.file_name), Some(book.size), space_acknowledgement, AcquisitionSource::SharedLibrary(grant)).await
    }

    // Mobile resume admission distinguishes scoped library grants from Internet.
    #[cfg(mobile)]
    pub(crate) fn is_shared_library(&self, id: &str) -> Result<bool, String> {
        Ok(self.inner.snapshot(id)?.job.url.starts_with("kiwix-library:"))
    }

    /// Caller must have independently verified this exact acquisition URL
    /// against the official catalog; a frontend-provided boolean is not proof.
    #[cfg(target_os = "ios")]
    pub(crate) async fn start_official_catalog(
        &self,
        url: String,
        destination_dir: PathBuf,
        file_name: Option<String>,
        expected_size: Option<u64>,
        space_acknowledgement: Option<String>,
    ) -> Result<DownloadSnapshot, String> {
        self.start_transport(
            url, destination_dir, file_name, expected_size, space_acknowledgement,
            AcquisitionSource::Internet(Some(NativeAcquisition {
                generation: Uuid::new_v4().to_string(), copied: 0, total: None,
                stage_identity: None, retirement_error: None,
            })),
        ).await
    }

    pub(super) async fn start_transport(
        &self,
        url: String,
        destination_dir: PathBuf,
        file_name: Option<String>,
        expected_size: Option<u64>,
        space_acknowledgement: Option<String>,
        source: AcquisitionSource,
    ) -> Result<DownloadSnapshot, String> {
        let (native, shared) = match source {
            AcquisitionSource::Internet(native) => (native, None),
            AcquisitionSource::SharedLibrary(grant) => (None, Some(grant)),
        };
        let parsed = match &shared {
            Some(grant) => { grant.check().await?; grant.validate_url(&url)? }
            None if url.starts_with("kiwix-library:") => return Err(SHARED_BROWSE_AGAIN.into()),
            None => { crate::transfer_policy::require_internet()?; checked_url(&url, true)? }
        };
        let url = parsed.to_string();
        let file_name = match file_name {
            Some(name) => {
                safe_name(&name)?;
                name
            }
            None => filename(&parsed)?,
        };
        if expected_size == Some(0) {
            return Err("Expected size must be positive when supplied".into());
        }
        // Metalink's exact size, resolved by the worker, supersedes catalog estimates.
        let metalink = shared.is_none() && (parsed.path().ends_with(".meta4") || parsed.path().ends_with(".metalink"));
        let admission_size = if metalink { None } else { expected_size };
        if !metalink {
            if let Some(size) = expected_size {
                crate::storage::check_file_size(&destination_dir, size)?;
            }
        }
        fs::create_dir_all(&destination_dir)
            .map_err(|e| format!("Creating destination directory: {e}"))?;
        let directory = fs::canonicalize(destination_dir)
            .map_err(|e| format!("Resolving download directory: {e}"))?;
        let destination = directory.join(&file_name);
        let destination = destination
            .to_str()
            .ok_or("Destination is not valid UTF-8")?
            .to_string();
        let request_key = serde_json::to_string(&(&url, &destination, expected_size))
            .map_err(|e| format!("Encoding download request: {e}"))?;
        let mut pending = self.inner.pending_starts.lock()
            .map_err(|_| "Download confirmation lock is poisoned")?;
        let id = pending.get(&request_key).cloned().unwrap_or_else(|| Uuid::new_v4().to_string());
        let timestamp = now();
        let job = DownloadJob {
            id: id.clone(),
            url,
            file_name,
            destination,
            status: "queued".into(),
            interrupted: false,
            downloaded: 0,
            total: admission_size,
            error: None,
            speed: 0,
            eta_seconds: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            sha256: None,
        };
        let (destination, part) = paths(&job)?;
        self.inner
            .store
            .with_http_acquisition_destinations(&[&destination], &job.url, || Ok(()))?;
        let mut state = self.inner.state()?;
        if state.jobs.len() >= MAX_JOBS {
            return Err("Download ownership ledger is full; explicitly delete retained downloads and files before adding jobs".into());
        }
        if state.jobs.values().any(|j| {
            !j.history_dismissed && (state.running.contains_key(&j.job.id)
                || !matches!(j.job.status.as_str(), "complete" | "cancelled"))
                && (j.job.destination == job.destination || j.job.url == job.url)
        }) {
            return Err(
                "A download already owns this URL or destination; resume or cancel it instead"
                    .into(),
            );
        }
        if fs::symlink_metadata(&destination).is_ok() {
            return Err("Destination already exists; existing files are never overwritten".into());
        }
        if let Some(size) = admission_size {
            check_space(&part, size)?;
        }
        // Native receipts coexist with the canonical copy, possibly on another
        // volume. Only their authoritative combined preflight owns advisory
        // consent; a generic single-copy warning here would require it twice.
        if native.is_none() {
            let outstanding = outstanding_space_from_state(&state, &directory, &job.id)?;
            if let Err(error) = download_space_warning(
                &self.inner, &directory, admission_size, outstanding, &http_space_identity(&job), space_acknowledgement.as_deref(),
            ) {
                if error.starts_with(crate::disk_warning::WARNING_PREFIX) {
                    if pending.len() >= MAX_JOBS { pending.clear(); }
                    pending.insert(request_key, id);
                }
                return Err(error);
            }
        }
        pending.remove(&request_key);
        drop(pending);
        let file = writable_options()
            .create_new(true)
            .open(&part)
            .map_err(|e| format!("Reserving temporary download file: {e}"))?;
        let reservation = crate::mobile_backup::exclude_payload(&file, &part).and_then(|()| {
            file.sync_all().map_err(|e| format!("Synchronizing new download part: {e}"))
        });
        let part_ownership = crate::torrent::CleanupEntry::retain(&part, &file)?;
        drop(file);
        if let Err(error) = reservation.and_then(|_| sync_directory(&directory)) {
            let _ = fs::remove_file(&part);
            return Err(error);
        }
        let transport = if shared.is_some() { DownloadTransport::QuicLibrary } else { DownloadTransport::Http };
        state.jobs.insert(
            id.clone(),
            StoredJob {
                job: job.clone(),
                transport: Some(transport),
                expected_size,
                transfer_url: None,
                etag: None,
                last_modified: None,
                publication: None,
                history_dismissed: false,
                files_deleted: false,
                part_ownership: Some(part_ownership),
                published_ownership: None,
                segments: None,
                segmented: false,
                verification: None,
                native,
                space_acknowledgement,
            },
        );
        if let Err(error) = self.inner.save(&state) {
            state.jobs.remove(&id);
            let _ = fs::remove_file(&part);
            return Err(error);
        }
        if let Some(grant) = shared {
            self.inner.shared_grants.lock().map_err(|_| "Shared library grant lock is poisoned")?.insert(id.clone(), grant);
        }
        spawn_worker(&self.inner, &mut state, &id);
        Ok(DownloadSnapshot { job, transport, publication: None })
    }

    pub async fn pause(&self, id: &str) -> Result<(), String> {
        self.stop(id, Signal::Pause).await
    }

    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        self.stop(id, Signal::Cancel).await
    }

    pub(super) async fn stop(&self, id: &str, command: Signal) -> Result<(), String> {
        let running = {
            let mut state = self.inner.state()?;
            let previous = state.jobs.get(id).cloned().ok_or("Unknown download ID")?;
            if previous.job.status == "complete" {
                return Err("Completed downloads cannot be paused or cancelled; manage the archive in the library".into());
            }
            let running = state.running.get(id).cloned();
            let command = match running.as_ref().map(|running| *running.command.borrow()) {
                _ if previous.job.status == "cancelled" => Signal::Cancel,
                _ if command == Signal::Interrupt && previous.job.status == "paused"
                    && !previous.job.interrupted => Signal::Pause,
                Some(Signal::Cancel) => Signal::Cancel,
                Some(current) if command == Signal::Interrupt && current != Signal::Run => current,
                _ => command,
            };
            // A background transition during local verification is not a
            // pause: the pass needs no network and must finish from its current
            // offset. Explicit pause/cancel and privacy revocation still win.
            let finalizing_interrupt = command == Signal::Interrupt
                && previous.job.status == "verifying"
                && running.is_some();
            if !finalizing_interrupt {
                let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
                // The user's manual pause/cancel intent must survive a crash
                // even before the worker has flushed and acknowledged the stop.
                stored.job.status = if command == Signal::Cancel {
                    "cancelled"
                } else {
                    "paused"
                }
                .into();
                stored.job.interrupted = command == Signal::Interrupt;
                stored.job.speed = 0;
                stored.job.eta_seconds = None;
                stored.job.error = None;
                stored.job.updated_at = now();
                if let Err(error) = self.inner.save(&state) {
                    state.jobs.insert(id.to_string(), previous);
                    return Err(error);
                }
            }
            if let Some(running) = &running {
                running.command.send_replace(command);
            } else if command == Signal::Cancel {
                // Cancellation still succeeds when ownership cannot be proven;
                // the leftover file is reported on the cancelled row instead.
                let cleanup = remove_job_part(&self.inner, &mut state, id);
                let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
                stored.job.downloaded = 0;
                stored.segments = None;
                stored.segmented = false;
                stored.verification = None;
                if let Some(native) = &mut stored.native {
                    native.copied = 0;
                    native.total = None;
                    native.stage_identity = None;
                }
                if let Err(error) = cleanup {
                    stored.job.error = Some(error);
                }
                let journal = self.inner.stage(&state)?;
                drop(state);
                self.inner.publish(journal)?;
            }
            running
        };
        #[cfg(target_os = "ios")]
        if running.is_none() {
            let stored = self.inner.snapshot(id)?;
            if let Some(native) = &stored.native {
                let result = crate::ios_background::stop(
                    id, &native.generation, stored.job.status == "cancelled",
                ).await;
                self.inner.update(id, |stored| {
                    if let Some(native) = &mut stored.native {
                        native.retirement_error = result.as_ref().err().cloned();
                    }
                })?;
                result?;
            }
        }
        if let Some(mut running) = running {
            let wait = async {
                while !*running.finished.borrow() {
                    running
                        .finished
                        .changed()
                        .await
                        .map_err(|_| "Download worker exited without reporting its result")?;
                }
                Ok::<(), String>(())
            };
            if command == Signal::Interrupt && self.inner.snapshot(id)?.native.is_none() {
                // Background grace is finite. Timing out never aborts the
                // pass: the worker keeps its offset and finishes later.
                let _ = tokio::time::timeout(INTERRUPT_FINALIZE_WAIT, wait).await;
            } else {
                wait.await?;
            }
            let stored = self.inner.snapshot(id)?;
            if let Some(error) = stored.native.as_ref().and_then(|native| native.retirement_error.as_ref()) {
                return Err(error.clone());
            }
            if stored.job.status == "failed" {
                return Err(stored
                    .job
                    .error
                    .unwrap_or_else(|| "Download failed while stopping".into()));
            }
        }
        Ok(())
    }

    pub async fn resume(&self, id: &str, space_acknowledgement: Option<String>) -> Result<(), String> {
        self.resume_project(id, space_acknowledgement, |_| ()).await
    }

    pub(super) async fn resume_project<R>(
        &self, id: &str, space_acknowledgement: Option<String>,
        project: impl FnOnce(&StoredJob) -> R,
    ) -> Result<R, String> {
        let source = self.inner.snapshot(id)?.job.url;
        if let Some(grant) = shared_grant(&self.inner, id, &source)? { grant.check().await?; }
        let native_complete = self.has_local_native_resume(id).await?;
        self.queue_resume(id, false, space_acknowledgement, native_complete, project)
    }

    /// Native completion/adoption receipts require local writable authority,
    /// not Internet admission. The worker still checks policy before any start.
    pub(crate) async fn has_local_native_resume(&self, id: &str) -> Result<bool, String> {
        let stored = self.inner.snapshot(id)?;
        if let Some(native) = &stored.native {
            if native.total.is_some() {
                return Ok(true);
            }
            #[cfg(any(target_os = "ios", all(test, unix)))]
            return Ok(native_transport::snapshot(id, &native.generation).await?.state == "complete");
        }
        Ok(false)
    }

    pub(super) fn queue_resume<R>(
        &self, id: &str, interrupted_only: bool, space_acknowledgement: Option<String>,
        native_complete: bool, project: impl FnOnce(&StoredJob) -> R,
    ) -> Result<R, String> {
        let mut state = self.inner.state()?;
        let stored = state.jobs.get(id).ok_or("Unknown download ID")?;
        if !native_complete && !stored.native.as_ref().is_some_and(|native| native.total.is_some()) {
            require_download_network(&self.inner, id, &stored.job.url)?;
        }
        if stored.history_dismissed || stored.files_deleted
            || stored.part_ownership.as_ref().is_some_and(crate::torrent::CleanupEntry::cleanup_started)
            || stored.published_ownership.as_ref().is_some_and(crate::torrent::CleanupEntry::cleanup_started)
        {
            return Err("Download was removed from history or file cleanup has started; retry deletion instead".into());
        }
        if interrupted_only && !(stored.job.interrupted && stored.job.status == "paused") {
            return Ok(project(stored));
        }
        if let Some(running) = state.running.get(id) {
            return if *running.command.borrow() == Signal::Run {
                Ok(project(stored))
            } else {
                Err("Download is still stopping; await pause/cancel before resuming".into())
            };
        }
        if !matches!(stored.job.status.as_str(), "paused" | "failed") {
            return Err("Only paused or failed downloads can resume".into());
        }
        let (destination, part) = paths(&stored.job)?;
        if fs::symlink_metadata(&destination).is_ok() {
            if recover_publication(stored, &destination, &part)? {
                let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
                stored.job.status = "complete".into();
                stored.job.interrupted = false;
                stored.segments = None;
                stored.segmented = false;
                stored.verification = None;
                stored.job.error = None;
                stored.job.updated_at = now();
                let observation = project(stored);
                sync_directory(destination.parent().ok_or("Destination has no parent")?)?;
                self.inner.save(&state)?;
                return Ok(observation);
            }
            return Err("Destination already exists; refusing to overwrite it".into());
        }
        self.inner
            .store
            .with_http_acquisition_destinations(&[&destination], &stored.job.url, || Ok(()))?;
        let part_metadata = regular_file(&part)?;
        let retained = match part_metadata.as_ref() {
            Some(metadata) if stored.segmented => valid_segment_plan(stored)
                .filter(|plan| plan_fits(plan, metadata.len()))
                .map(SegmentPlan::durable)
                .unwrap_or(0),
            Some(metadata) => metadata.len(),
            None => 0,
        };
        let source = download_url(&self.inner, id, &stored.job.url)?;
        let unresolved_metalink = source.scheme() != "kiwix-library" && stored.transfer_url.is_none()
            && (source.path().ends_with(".meta4") || source.path().ends_with(".metalink"));
        let allocated = part_metadata.as_ref().map_or(0, |metadata| metadata.len());
        let remaining = if unresolved_metalink { None } else {
            stored.expected_size.or(stored.job.total)
                .map(|total| space_remaining(total, retained, allocated))
        };
        if let Some(remaining) = remaining { check_space(&part, remaining)?; }
        let warning_directory = destination.parent().ok_or("Download has no destination directory")?;
        let outstanding = outstanding_space_from_state(&state, warning_directory, id)?;
        // Native transport admission includes its retained stage and canonical
        // copy. Do not consume its larger, native-scoped confirmation here as
        // a different single-file prompt and create an endless retry loop.
        if stored.native.is_none() || native_complete
            || stored.native.as_ref().is_some_and(|native| native.total.is_some())
        {
            crate::disk_warning::check_with_outstanding(
                warning_directory, remaining, outstanding,
                &http_space_identity(&stored.job), space_acknowledgement.as_deref(),
            )?;
        }
        let mut new_ownership = None;
        if part_metadata.is_none() {
            let file = writable_options()
                .create_new(true)
                .open(&part)
                .map_err(|e| format!("Recreating missing download part: {e}"))?;
            crate::mobile_backup::exclude_payload(&file, &part)?;
            new_ownership = Some(crate::torrent::CleanupEntry::retain(&part, &file)?);
        }
        let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
        if let Some(ownership) = new_ownership { stored.part_ownership = Some(ownership); }
        if part_metadata.is_none() {
            stored.segments = None;
            stored.segmented = false;
            stored.verification = None;
            if let Some(native) = &mut stored.native {
                native.copied = 0;
                native.total = None;
                native.stage_identity = None;
            }
        }
        // Admission must leave an actionable row if its checkpoint fails.
        // Retain any newly recreated part's ownership, but restore the stopped
        // state: no worker exists until the queued checkpoint has succeeded.
        let retryable = stored.clone();
        stored.published_ownership = None;
        stored.space_acknowledgement = space_acknowledgement;
        stored.job.status = "queued".into();
        stored.job.interrupted = false;
        stored.publication = None;
        stored.job.error = None;
        stored.job.updated_at = now();
        let observation = project(stored);
        if let Err(error) = self.inner.save(&state) {
            state.jobs.insert(id.to_owned(), retryable);
            return Err(error);
        }
        spawn_worker(&self.inner, &mut state, id);
        Ok(observation)
    }

    /// Resume only jobs that were active at shutdown. Explicit pauses stay paused.
    /// Loading state never initiates network access; the settings owner opts in here.
    pub async fn resume_interrupted(&self) -> Result<(), String> {
        // A foreground return is also the bounded recovery point for a worker
        // that died after its last durable sync.
        if let Err(error) = self.recover_durable_local().await {
            crate::diagnostics::record(
                &self.inner.store,
                "downloads",
                Some(&format!("Download recovery sweep: {error}")),
            );
        }
        let ids: Vec<String> = self
            .inner
            .state()?
            .jobs
            .values()
            .filter(|stored| stored.job.status == "paused" && stored.job.interrupted && !stored.job.url.starts_with("kiwix-library:"))
            .map(|stored| stored.job.id.clone())
            .collect();
        let mut first_error = None;
        for id in ids {
            if let Err(error) = self.queue_resume(&id, true, None, false, |_| ()) {
                // A job that cannot auto-resume must never wedge the manager or
                // be retried on every launch: keep it visible with the reason
                // and clear the interruption so this decision is durable.
                let recorded = self.inner.update(&id, |stored| {
                    stored.job.error = Some(error.chars().take(2000).collect());
                    stored.job.interrupted = false;
                });
                if let Err(save_error) = recorded {
                    if first_error.is_none() {
                        first_error = Some(save_error);
                    }
                    continue;
                }
                if !error.starts_with(crate::disk_warning::WARNING_PREFIX) && first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Run the same recovery sweep the live ticker uses. Foreground entry and
    /// tests call this directly; it never touches the network.
    pub(crate) async fn recover_durable_local(&self) -> Result<(), String> {
        sweep_durable_finalize(Arc::clone(&self.inner)).await
    }

    /// Resume jobs interrupted by a network-policy retirement after the policy
    /// re-admits Internet. The reconciler captures the decision once, so a
    /// denied, retired, private or read-only application never starts a
    /// transfer, and an explicit user pause is never converted into work.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) async fn resume_interrupted_after_reconnect(
        &self,
        admission: crate::transfer_policy::ResumeAdmission,
    ) -> Result<Option<crate::transfer_policy::ResumeSkip>, String> {
        match admission {
            crate::transfer_policy::ResumeAdmission::Skip(reason) => Ok(Some(reason)),
            crate::transfer_policy::ResumeAdmission::Attempt => {
                self.resume_interrupted().await?;
                Ok(None)
            }
        }
    }

    /// Interrupt active workers durably without converting them into manual pauses.
    pub async fn shutdown(&self) -> Result<(), String> {
        self.shutdown_workers(false, false).await
    }

    /// Ordinary iOS suspension retires foreground/custom transports, but an
    /// admitted native lease remains OS-owned until explicit stop or policy loss.
    #[cfg(target_os = "ios")]
    pub(crate) async fn shutdown_for_background(&self) -> Result<(), String> {
        self.shutdown_workers(true, false).await
    }

    // Mobile policy loss retires Internet workers without stopping local grants.
    #[cfg(mobile)]
    pub(crate) async fn interrupt_internet(&self) -> Result<(), String> {
        self.shutdown_workers(false, true).await
    }

    pub(super) async fn shutdown_workers(&self, retain_native: bool, internet_only: bool) -> Result<(), String> {
        let ids: Vec<String> = {
            let state = self.inner.state()?;
            state.jobs.iter().filter(|(id, stored)| {
                !(retain_native && stored.native.is_some())
                    && !(internet_only && stored.job.url.starts_with("kiwix-library:"))
                    && (state.running.contains_key(*id)
                        || (stored.native.is_some() && stored.job.status != "complete"))
            }).map(|(id, _)| id.clone()).collect()
        };
        let mut first_error = None;
        for id in ids {
            #[cfg(target_os = "ios")]
            {
                let stored = self.inner.snapshot(&id)?;
                let worker_running = self.inner.state()?.running.contains_key(&id);
                if matches!(stored.job.status.as_str(), "failed" | "cancelled")
                    && !worker_running
                {
                    if let Some(native) = &stored.native {
                        let retirement = crate::ios_background::stop(
                            &id, &native.generation, stored.job.status == "cancelled",
                        ).await;
                        self.inner.update(&id, |stored| {
                            stored.native.as_mut().unwrap().retirement_error = retirement.as_ref().err().cloned();
                        })?;
                        if let Err(error) = retirement {
                            if first_error.is_none() { first_error = Some(error); }
                        }
                        continue;
                    }
                }
            }
            if let Err(error) = self.stop(&id, Signal::Interrupt).await {
                let terminal = self.inner.snapshot(&id).is_ok_and(|stored| {
                    matches!(stored.job.status.as_str(), "complete" | "cancelled")
                        && stored.native.as_ref().is_none_or(|native| native.retirement_error.is_none())
                });
                if !terminal && first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Reattach only existing OS work; this never starts a missing/paused task
    /// and therefore does not depend on the auto-resume preference or Internet.
    /// Caller holds the outer service mode/transfer/admission guards.
    #[cfg(target_os = "ios")]
    pub(crate) async fn reconcile_native(&self) -> Result<(), String> {
        if finalize_forbidden(&self.inner) {
            return Err("Native download reconciliation requires normal writable mode".into());
        }
        let saved: Vec<StoredJob> = self.inner.state()?.jobs.values()
            .filter(|stored| stored.native.is_some()
                && !stored.history_dismissed && !stored.files_deleted
                && !matches!(stored.job.status.as_str(), "complete" | "cancelled"))
            .cloned().collect();
        let live: Vec<(String, String)> = saved.iter().map(|stored| (
            stored.job.id.clone(), stored.native.as_ref().unwrap().generation.clone(),
        )).collect();
        crate::ios_background::reconcile(&live).await?;
        let allows_cellular = self.inner.store.settings()?.allow_metered_downloads;
        for stored in saved {
            let native = stored.native.as_ref().unwrap();
            let manual_pause = stored.job.status == "paused" && !stored.job.interrupted;
            if manual_pause {
                self.stop(&stored.job.id, Signal::Pause).await?;
                continue;
            }
            if stored.job.status == "failed" {
                crate::ios_background::stop(&stored.job.id, &native.generation, false).await?;
                continue;
            }
            let snapshot = crate::ios_background::snapshot(&stored.job.id, &native.generation).await?;
            if snapshot.state == "running" && snapshot.allows_cellular && !allows_cellular {
                self.stop(&stored.job.id, Signal::Interrupt).await?;
                continue;
            }
            if !matches!(snapshot.state.as_str(), "running" | "complete")
            {
                continue;
            }
            let mut state = self.inner.state()?;
            let Some(current) = state.jobs.get(&stored.job.id) else { continue };
            let worker_alive = state.running.get(&stored.job.id)
                .is_some_and(|running| running.finished.has_changed().is_ok());
            if worker_alive || current.history_dismissed || current.files_deleted
                || current.native.as_ref().map(|native| native.generation.as_str()) != Some(native.generation.as_str())
                || !matches!(current.job.status.as_str(), "queued" | "downloading" | "verifying" | "paused")
                || (current.job.status == "paused" && !current.job.interrupted)
            {
                continue;
            }
            let previous = current.clone();
            let current = state.jobs.get_mut(&stored.job.id).unwrap();
            current.job.status = "downloading".into();
            current.job.interrupted = false;
            current.job.error = None;
            current.job.updated_at = now();
            if let Err(error) = self.inner.save(&state) {
                state.jobs.insert(stored.job.id.clone(), previous);
                return Err(error);
            }
            spawn_download_worker(&self.inner, &mut state, &stored.job.id, WorkerMode::NativeAttach);
        }
        Ok(())
    }
}

/// Undo the in-memory `files_deleted` mark after the final journal write fails.
fn restore_deleted_row(state: &mut State, id: &str, previous: StoredJob) {
    if let Some(stored) = state.jobs.get_mut(id) {
        stored.files_deleted = previous.files_deleted;
    }
}
