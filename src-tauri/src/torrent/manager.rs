// SPDX-License-Identifier: GPL-3.0-or-later
//! The public `TorrentManager` API: admission, sharing, control, import handoff and persistence.
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use super::*;
use futures_util::FutureExt;

/// Serializes staged journal publication with the in-memory commit. Journaled
/// writers snapshot the state under its mutex and write the journal after
/// releasing it (the acquisition staged-journal pattern); without this gate two
/// writers could publish out of commit order. It is never held with the state
/// mutex, and a poisoned gate still admits writes: another writer's panic must
/// not make the history permanently unwritable.
static JOURNAL: Mutex<()> = Mutex::new(());

/// Whether this host provides the IPv6 sockets behind the `[::]` dual-stack
/// TCP and UDP binds the torrent engine uses by default. A kernel without IPv6
/// refuses to create them at all. Only that refusal counts: any other failure
/// is left for the engine to report when it binds.
pub(super) fn dual_stack_available() -> bool {
    let unsupported = |kind, protocol| {
        socket2::Socket::new(socket2::Domain::IPV6, kind, Some(protocol))
            .is_err_and(|error| address_family_unsupported(&error))
    };
    !unsupported(socket2::Type::STREAM, socket2::Protocol::TCP)
        && !unsupported(socket2::Type::DGRAM, socket2::Protocol::UDP)
}

/// The same rule as the vendored socket crate's IPv4 fallback.
fn address_family_unsupported(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(error.raw_os_error(), Some(libc::EAFNOSUPPORT | libc::EPROTONOSUPPORT))
    }
    #[cfg(windows)]
    {
        // WSAEAFNOSUPPORT
        error.raw_os_error() == Some(10047)
    }
}

impl TorrentManager {
    /// Snapshot one durably published member, including a completed member of
    /// a still-running multi-file torrent. Metadata and filenames are not proof.
    pub(crate) fn import_publication(&self, id: &str, destination: &str) -> Result<crate::acquisition::ImportPublication, String> {
        self.ensure_not_shutdown()?;
        let state = self.inner.state.lock().map_err(display)?;
        let record = state.jobs.iter().find(|record| record.job.id == id).ok_or("Torrent download no longer exists")?;
        import_publication(record, destination)
    }

    /// The verifier already checked the complete digest outside manager locks.
    /// Fence cleanup/history mutation through the final Store registration.
    pub(crate) fn with_import_publication<T>(
        &self, id: &str, destination: &str, source: &crate::archive_source::RetainedSource,
        verified: &crate::acquisition::ImportPublication, commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.ensure_not_shutdown()?;
        let state = self.inner.state.lock().map_err(display)?;
        let record = state.jobs.iter().find(|record| record.job.id == id).ok_or("Torrent download no longer exists")?;
        let current = import_publication(record, destination)?;
        if &current != verified { return Err("Torrent publication changed during verification; retry the import".into()); }
        current.validate_source(source, false)?;
        commit()
    }

    /// Quarantine receipt for the transfers panel, or `None` on a clean launch.
    pub fn history_notice(&self) -> Option<String> {
        self.inner.history_notice.clone()
    }

    #[cfg(test)]
    pub(crate) fn load_warnings(&self) -> &[String] {
        &self.inner.load_warnings
    }

    /// Uses only borrowed durable history. The engine is deliberately untouched.
    pub(crate) fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        let state = self.inner.state.lock().map_err(display)?;
        for record in &state.jobs {
            if record.claims_monitored_path(path)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `share` refuses a second live share of one archive, and share metadata is
    /// deterministic, so a restarted stopped share of the same file has the
    /// same info hash. The engine would refuse it as already managed and the
    /// old row would turn into an error; refuse the restart up front instead.
    fn refuse_duplicate_share(&self, record: &Record) -> Result<(), String> {
        let Some(share) = record.share.as_ref() else { return Ok(()) };
        let state = self.inner.state.lock().map_err(display)?;
        if state.jobs.iter().any(|other| {
            other.job.id != record.job.id
                && !other.history_dismissed
                && !other.files_deleted
                && other.job.seeding
                && !matches!(other.job.status.as_str(), "cancelled" | "error")
                && other.share.as_ref().is_some_and(|live| live.path == share.path)
        }) {
            return Err("This archive is already being shared".into());
        }
        Ok(())
    }

    /// A live share holds the library archive as read-only input. Library
    /// deletion and relinking must stop it first instead of invalidating the
    /// seed underneath the engine.
    pub(crate) fn active_share_for_book(&self, book_id: &str) -> Result<Option<String>, String> {
        let state = self.inner.state.lock().map_err(display)?;
        Ok(state
            .jobs
            .iter()
            .find(|record| {
                record.job.seeding
                    && !record.files_deleted
                    && !record.history_dismissed
                    && !matches!(record.job.status.as_str(), "cancelled" | "error")
                    && record.share.as_ref().is_some_and(|share| share.book_id == book_id)
            })
            .map(|record| record.job.id.clone()))
    }

    pub(crate) fn saved_active_share(store: &Store, book_id: &str) -> Result<Option<String>, String> {
        let state: Option<State> = crate::acquisition::saved_monitored_record(
            store, &["torrents", "jobs.json"], MAX_METADATA as u64,
            crate::profile_paths::RecordKind::Torrents,
        )?;
        let Some(state) = state else { return Ok(None) };
        if state.version != STATE_VERSION {
            return Err("Unsupported torrent monitor history version".into());
        }
        Ok(state
            .jobs
            .into_iter()
            .find(|record| {
                record.job.seeding
                    && !record.files_deleted
                    && !record.history_dismissed
                    && !matches!(record.job.status.as_str(), "cancelled" | "error")
                    && record.share.as_ref().is_some_and(|share| share.book_id == book_id)
            })
            .map(|record| record.job.id))
    }

    /// Staged payload files of unfinished acquisitions, for another transport
    /// to reuse after verifying each range against its own hashes. Shares,
    /// published, cancelled and deleted jobs are excluded. Reads saved history
    /// only; it never opens the engine or touches payload.
    pub(crate) fn saved_partial_files(store: &Store) -> Result<Vec<crate::partial_donor::Donor>, String> {
        let state: Option<State> = crate::acquisition::saved_monitored_record(
            store, &["torrents", "jobs.json"], MAX_METADATA as u64,
            crate::profile_paths::RecordKind::Torrents,
        ).map_err(|e| e.to_string())?;
        let Some(state) = state else { return Ok(Vec::new()) };
        if state.version != STATE_VERSION {
            return Err("Unsupported torrent history version".into());
        }
        let mut donors = Vec::new();
        for record in state.jobs {
            if record.share.is_some() || record.verified || record.files_deleted
                || !record.publication_intents.is_empty()
                || matches!(record.job.status.as_str(), "cancelled" | "completed" | "seeding" | "publishing")
            {
                continue;
            }
            for file in &record.space_files {
                if safe_relative(&file.path).is_err() || !record.files.contains(&file.path) {
                    continue;
                }
                donors.push(crate::partial_donor::Donor {
                    path: record.staging.join(&file.path),
                    size: file.size,
                    ranges: None,
                });
            }
        }
        Ok(donors)
    }

    pub(crate) fn saved_monitored_paths(store: &Store) -> Result<crate::acquisition::MonitoredPathClaims, String> {
        let state: Option<State> = crate::acquisition::saved_monitored_record(
            store, &["torrents", "jobs.json"], MAX_METADATA as u64,
            crate::profile_paths::RecordKind::Torrents,
        )?;
        let mut claims = Vec::new();
        if let Some(state) = state {
            if state.version != STATE_VERSION {
                return Err("Unsupported torrent monitor history version".into());
            }
            for record in state.jobs {
                if let Some(share) = &record.share {
                    claims.push(share.path.clone());
                }
                for file in &record.files {
                    safe_relative(file).map_err(display)?;
                    if is_zim(file) {
                        claims.push(record.directory.join(file.file_name().ok_or("Missing torrent filename")?));
                    }
                }
                for intent in record.publication_intents {
                    let path = record.directory.join(&intent.file_name);
                    crate::acquisition::monitored_child_matches(&path, &record.directory, &intent.file_name)?;
                    claims.push(path);
                }
                claims.extend(record.job.completed_paths.into_iter().map(PathBuf::from));
            }
        }
        crate::acquisition::MonitoredPathClaims::new(claims)
    }

    /// Loads the profile without starting discovery unless automatic restoration
    /// is enabled and at least one unfinished, non-manually-paused job exists.
    pub fn new(
        data_dir: PathBuf,
        auto_resume: bool,
        store: Arc<Store>,
    ) -> Result<Arc<Self>, String> {
        let folder = data_dir.join("torrents");
        crate::fs_ext::create_private_dir_all(&folder).map_err(display)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(folder.join("manager.lock"))
            .map_err(display)?;
        lock.try_flock_exclusive()
            .map_err(|e| format!("Torrent profile is already in use: {e}"))?;
        let path = folder.join("jobs.json");
        let mut history_notice = None;
        #[cfg(test)]
        let mut load_warnings = Vec::new();
        let mut state = if let Some(file) = crate::acquisition::open_history(&path)? {
            match load_state(&file, &folder, &store) {
                Ok((state, warnings)) => {
                    if !warnings.is_empty() {
                        crate::diagnostics::record(
                            &store,
                            "downloads",
                            Some("Some saved transfers could not be read and were skipped"),
                        );
                    }
                    #[cfg(test)]
                    { load_warnings = warnings; }
                    state
                }
                Err(reason) => {
                    // Preserve the unreadable history byte-for-byte and start
                    // empty. A bad record must never block the profile: transfer
                    // history is recoverable, the rest of the app is not optional.
                    let preserved = crate::acquisition::preserve_history(&path, file)?;
                    crate::diagnostics::record(&store, "downloads", Some(&reason));
                    eprintln!(
                        "Saved transfer history was unreadable ({reason}); preserved at {}",
                        preserved.display()
                    );
                    history_notice = Some(preserved.to_string_lossy().into_owned());
                    State {
                        version: STATE_VERSION,
                        download_limit: 0,
                        upload_limit: 0,
                        jobs: vec![],
                    }
                }
            }
        } else {
            State {
                version: STATE_VERSION,
                download_limit: 0,
                upload_limit: 0,
                jobs: vec![],
            }
        };
        let mut restore = Vec::new();
        for record in &mut state.jobs {
            record.job.speed = 0.0;
            record.job.upload_speed = 0.0;
            record.job.eta_seconds = None;
            if restore_record(record, auto_resume) {
                restore.push(record.job.id.clone());
            }
        }
        let manager = Arc::new(Self {
            inner: Arc::new(Inner {
                folder,
                store,
                state: Mutex::new(state),
                runtimes: Mutex::new(HashMap::new()),
                retirement_failures: Mutex::new(HashMap::new()),
                gate: AsyncMutex::new(()),
                tasks: Mutex::new(JoinSet::new()),
                shutdown_gate: AsyncMutex::new(()),
                storage_gates: Mutex::new(HashMap::new()),
                engine: Mutex::new(Arc::new(OnceCell::new())),
                stop: CancellationToken::new(),
                suspended: AtomicBool::new(false),
                reaps: Mutex::new(HashMap::new()),
                #[cfg(test)]
                engine_hook: Mutex::new(None),
                history_notice,
                #[cfg(test)]
                load_warnings,
                _lock: lock,
            }),
        });
        // Reclaim even cancelled, paused and completed records. A crash bypasses
        // the worker's cleanup, and none of these jobs need start an engine.
        let records = manager.inner.state.lock().map_err(display)?.jobs.clone();
        for record in records {
            if record.history_dismissed || record.files_deleted { continue; }
            if let Err(error) = manager.reclaim_temporaries(&record) {
                manager.update_record(&record.job.id, |r| r.job.error = Some(error))?;
            }
        }
        if !restore.is_empty() {
            tokio::runtime::Handle::try_current()
                .map_err(|_| "Torrent restoration requires a Tokio runtime")?;
            for id in restore {
                manager.spawn(id)?;
            }
        }
        Ok(manager)
    }

    pub async fn start(
        &self,
        url: String,
        destination_dir: PathBuf,
        acknowledgement: Option<String>,
    ) -> Result<TorrentJob, String> {
        self.start_inner(url, destination_dir, acknowledgement, Vec::new()).await
    }

    #[cfg(test)]
    pub async fn start_with_peers(
        &self,
        url: String,
        destination_dir: PathBuf,
        acknowledgement: Option<String>,
        initial_peers: Vec<SocketAddr>,
    ) -> Result<TorrentJob, String> {
        self.start_inner(url, destination_dir, acknowledgement, initial_peers).await
    }

    pub(super) async fn start_inner(
        &self,
        url: String,
        destination_dir: PathBuf,
        acknowledgement: Option<String>,
        initial_peers: Vec<SocketAddr>,
    ) -> Result<TorrentJob, String> {
        let _gate = self.inner.gate.lock().await;
        crate::transfer_policy::require_internet()?;
        validate_source(&url)?;
        self.ensure_running()?;
        let directory = fs::canonicalize(destination_dir).map_err(display)?;
        if !directory.is_dir() {
            return Err("Choose an existing download directory".into());
        }
        let suffix = Uuid::new_v4().to_string();
        let staging = directory.join(format!(".kiwix-torrent-{suffix}"));
        #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&staging).map_err(display)?;
        let staging_identity = PublicationIdentity::of(&open_staging(&staging)?.dir_metadata().map_err(display)?);
        let metadata = self.inner.folder.join(format!("{suffix}.torrent"));
        let job = TorrentJob {
            id: format!("torrent:{suffix}"),
            url,
            file_name: "".into(),
            destination: directory.to_string_lossy().into_owned(),
            status: "queued".into(),
            downloaded: 0,
            total: None,
            error: None,
            speed: 0.0,
            eta_seconds: None,
            import_error: None,
            transport: "torrent".into(),
            seeding: false,
            upload_speed: 0.0,
            uploaded: 0,
            completed_paths: vec![],
            shared: false,
            magnet: None,
            torrent_path: None,
            created_at: Some(crate::store::now()),
        };
        self.update(|s| {
            s.jobs.push(Record {
                job: job.clone(),
                staging,
                directory,
                metadata,
                files: vec![],
                paused: false,
                interrupted: false,
                verified: false,
                publication_intents: vec![],
                publication_temporaries: vec![],
                space_files: vec![],
                space_acknowledgements: acknowledgement.into_iter().collect(),
                history_dismissed: false,
                files_deleted: false,
                staging_directories: vec![(PathBuf::new(), staging_identity)],
                staging_cleanup: vec![],
                metadata_cleanup: None,
                metadata_temporary: None,
                published_cleanup: vec![],
                share: None,
                initial_peers,
            });
            Ok(())
        })?;
        self.spawn(job.id.clone())?;
        Ok(job)
    }

    /// Share an existing library archive without copying or moving it. The
    /// caller supplies the book identity it saw; the archive's device, inode,
    /// size and modification time are pinned here and rechecked before metadata
    /// creation and again before the engine opens the file.
    pub async fn share(&self, book: crate::store::Book) -> Result<TorrentJob, String> {
        let _gate = self.inner.gate.lock().await;
        crate::transfer_policy::require_internet()?;
        self.ensure_running()?;
        if !book.available {
            return Err("The selected archive is unavailable; reconnect it before sharing".into());
        }
        let path = PathBuf::from(&book.path);
        if !path.is_absolute() || !is_zim(&path) {
            return Err("Only a single complete .zim archive can be shared".into());
        }
        if path.file_name().and_then(|name| name.to_str()).is_none() {
            return Err("This file name cannot be shared. Rename the file and try again".into());
        }
        let pinned_book = book.clone();
        let (name, identity) = tokio::task::spawn_blocking(move || {
            let pinned = crate::library_files::retain_archive_files(&pinned_book)?;
            if pinned.files.len() != 1 {
                return Err("Split archives cannot be shared as a single torrent".into());
            }
            let (name, _) = pinned.files.first().ok_or("Archive has no primary file")?;
            if !name.to_ascii_lowercase().ends_with(".zim") {
                return Err("Only the complete .zim archive can be shared".into());
            }
            Ok::<_, String>((name.clone(), ShareIdentity::of(&pinned.stamps[0])?))
        })
        .await
        .map_err(display)??;
        self.require_current_library_file(&book, &path, &identity)?;
        let directory = path.parent().ok_or("Archive has no parent directory")?.to_path_buf();
        let suffix = Uuid::new_v4().to_string();
        let staging = directory.join(format!(".kiwix-torrent-{suffix}"));
        let metadata = self.inner.folder.join(format!("{suffix}.torrent"));
        {
            let state = self.inner.state.lock().map_err(display)?;
            if state.jobs.iter().any(|record| {
                !record.history_dismissed
                    && !record.files_deleted
                    && record.job.seeding
                    && !matches!(record.job.status.as_str(), "cancelled" | "error")
                    && record.share.as_ref().is_some_and(|share| share.path == path)
            }) {
                return Err("This archive is already being shared".into());
            }
        }
        let job = TorrentJob {
            id: format!("torrent:{suffix}"),
            url: String::new(),
            file_name: name,
            destination: path.to_string_lossy().into_owned(),
            status: "creating".into(),
            downloaded: identity.size,
            total: Some(identity.size),
            error: None,
            speed: 0.0,
            eta_seconds: None,
            import_error: None,
            transport: "torrent".into(),
            seeding: true,
            upload_speed: 0.0,
            uploaded: 0,
            completed_paths: vec![],
            shared: true,
            magnet: None,
            torrent_path: Some(metadata.to_string_lossy().into_owned()),
            created_at: Some(crate::store::now()),
        };
        self.update(|state| {
            state.jobs.push(Record {
                job: job.clone(),
                staging,
                directory,
                metadata,
                files: vec![],
                paused: false,
                interrupted: false,
                verified: true,
                publication_intents: vec![],
                publication_temporaries: vec![],
                space_files: vec![],
                space_acknowledgements: vec![],
                history_dismissed: false,
                files_deleted: false,
                staging_directories: vec![],
                staging_cleanup: vec![],
                metadata_cleanup: None,
                metadata_temporary: None,
                published_cleanup: vec![],
                share: Some(ShareSource {
                    book_id: book.metadata.id.clone(),
                    installation_id: book.installation_id.clone(),
                    path,
                    identity,
                    info_hash: String::new(),
                    piece_length: SHARE_PIECE_LENGTH,
                }),
                initial_peers: vec![],
            });
            Ok(())
        })?;
        self.spawn(job.id.clone())?;
        // Small archives usually finish metadata before the command returns;
        // large ones return the live job immediately and the UI follows
        // `torrent_job` while the worker reports its phase.
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let current = self.record(&job.id)?.job;
            if current.magnet.is_some() || current.status == "error" {
                return Ok(current);
            }
        }
        Ok(self.record(&job.id)?.job)
    }

    pub async fn job(&self, id: &str) -> Result<TorrentJob, String> {
        Ok(self.record(id)?.job)
    }

    /// Bounded `.torrent` bytes for an existing job. Callers save them; this
    /// never moves or rewrites the shared library archive.
    pub async fn torrent_metadata(&self, id: &str) -> Result<Vec<u8>, String> {
        let record = self.record(id)?;
        if record.history_dismissed || record.files_deleted {
            return Err("Torrent metadata is no longer available".into());
        }
        read_torrent_metadata(&record)
    }

    pub async fn list(&self) -> Vec<TorrentJob> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .iter()
            .filter(|record| !record.history_dismissed)
            .map(|r| r.job.clone())
            .collect()
    }

    /// Snapshot hidden ownership ledgers without engine or resume admission.
    pub async fn retained_jobs(&self) -> Vec<TorrentJob> {
        self.inner.state.lock().unwrap_or_else(|error| error.into_inner()).jobs.iter()
            .filter(|record| record.history_dismissed)
            .map(|record| record.job.clone()).collect()
    }

    /// Manual stop intent remains admissible while policy holds network admission.
    pub async fn pause(&self, id: &str) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        let record = self.record(id)?;
        if record.job.status == "cancelled" {
            return Err("Torrent was cancelled".into());
        }
        self.update_record(id, |r| {
            r.paused = true;
            r.interrupted = false;
            if r.share.is_some() {
                r.job.seeding = false;
            }
            r.job.status = if r.verified { "completed" } else { "paused" }.into();
            r.job.speed = 0.0;
            r.job.upload_speed = 0.0;
            r.job.eta_seconds = None;
        })?;
        let retired = self.inner.runtimes.lock().map_err(display)?.get_mut(id).and_then(|runtime| {
            runtime.token.cancel();
            runtime.retired.take()
        });
        // Worker cleanup needs the gate to remove its engine handle. Serialize
        // resume/lifecycle operations until that worker, including copies, exits.
        drop(gate);
        let retirement = match retired {
            Some(retired) => retired.await.unwrap_or_else(|_| {
                Err("Torrent worker failed before acknowledging its pause".into())
            }),
            None => Ok(()),
        };
        {
            let gate = self.storage_gate(id)?;
            let _storage = gate.write().map_err(display)?;
        }
        retirement
    }

    pub async fn resume(
        &self,
        id: &str,
        acknowledgement: Option<String>,
    ) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let _gate = self.inner.gate.lock().await;
        self.ensure_running()?;
        crate::transfer_policy::require_internet()?;
        let record = self.record(id)?;
        if record.history_dismissed || record.files_deleted || record.cleanup_started() { return Err("Torrent was removed from history or file cleanup has started; retry deletion instead".into()); }
        if record.job.status == "cancelled" {
            return Err("Start a new job to acquire a cancelled torrent".into());
        }
        if record.verified && !record.job.seeding {
            return Ok(());
        }
        self.refuse_duplicate_share(&record)?;
        self.check_space(id, acknowledgement.as_deref())?;
        // No engine call here: unpausing starts tracker tasks, and this holds
        // both gates. A live worker sees the cleared pause and unpauses its
        // handle within one 250 ms tick, outside the gates; otherwise `spawn`
        // starts a worker that adds the torrent unpaused.
        self.update_record(id, resume_record)?;
        self.spawn(id.into())
    }

    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let _gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        self.with_record(id, |_| ())?;
        self.update_record(id, |r| {
            r.paused = true;
            r.interrupted = false;
            r.job.seeding = false;
            r.job.status = "cancelled".into();
            r.job.speed = 0.0;
            r.job.upload_speed = 0.0;
            r.job.eta_seconds = None;
        })?;
        // The worker removes the engine handle, never the user's files. Retained
        // staging is intentional: cancellation cannot erase a completed archive.
        let active = if let Some(runtime) = self.inner.runtimes.lock().map_err(display)?.get(id) {
            runtime.token.cancel();
            true
        } else {
            false
        };
        if !active {
            self.reclaim_temporaries(&self.record(id)?)?;
        }
        Ok(())
    }

    /// Hide terminal rows, retaining files and their durable cleanup authority.
    pub async fn remove_terminal(&self, id: &str) -> Result<(), String> {
        self.finish_terminal(id, false).await
    }

    /// Drop a retained row without touching any file. A retained row may have
    /// no provable ownership, so forgetting never deletes the kept path.
    pub async fn forget_terminal(&self, id: &str) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        let record = self.record(id)?;
        if self.inner.retirement_failures.lock().map_err(display)?.contains_key(id) {
            return Err("The transfer did not stop cleanly. Restart the app, then retry".into());
        }
        if !removable_terminal(&record) || self.inner.runtimes.lock().map_err(display)?.contains_key(id) {
            return Err("Torrent changed while its history was being removed".into());
        }
        drop(gate);
        drop(_shutdown);
        self.update(|state| {
            state.jobs.retain(|record| record.job.id != id);
            Ok(())
        })
    }

    /// Explicit deletion also authorizes stopping this exact completed seed.
    pub async fn delete_terminal(&self, id: &str) -> Result<(), String> {
        self.finish_terminal(id, true).await
    }

    /// Read-only provenance for the dialog and guarded library deletion. A live
    /// verified seed can be inspected without changing seed intent or workers.
    pub async fn terminal_owned_publications(&self, id: &str) -> Result<Vec<(PathBuf, CleanupEntry)>, String> {
        let (record, generation) = {
            let _shutdown = self.inner.shutdown_gate.lock().await;
            let _gate = self.inner.gate.lock().await;
            self.ensure_not_shutdown()?;
            if self.inner.retirement_failures.lock().map_err(display)?.contains_key(id) {
                return Err("The transfer did not stop cleanly, so Kiwix did not delete any files. Restart the app, then retry".into());
            }
            let gate = self.storage_gate(id)?;
            let _storage = gate.write().map_err(display)?;
            let record = self.record(id)?;
            if !record.can_authorize_cleanup() { return Err("Only finished torrents can authorize cleanup".into()); }
            let generation = self.inner.runtimes.lock().map_err(display)?.get(id).map(|runtime| runtime.generation);
            if generation.is_some()
                && !(record.verified && matches!(record.job.status.as_str(), "completed" | "complete" | "seeding"))
            {
                return Err("The transfer is still stopping. Wait a moment, then try again".into());
            }
            (record, generation)
        };
        let (record, proofs) = tokio::task::spawn_blocking(move || {
            let proofs = validate_terminal_publications(&record)?;
            Ok::<_, String>((record, proofs))
        }).await.map_err(|error| format!("Validating torrent cleanup ownership: {error}"))??;
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let _gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        let current = self.record(id)?;
        let current_generation = self.inner.runtimes.lock().map_err(display)?.get(id).map(|runtime| runtime.generation);
        if !record.same_cleanup_identity(&current) || generation != current_generation
            || self.inner.retirement_failures.lock().map_err(display)?.contains_key(id)
        {
            return Err("Torrent changed during cleanup authorization; retry with its current state".into());
        }
        Ok(proofs)
    }

    pub(super) async fn finish_terminal(&self, id: &str, delete_files: bool) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        if self.inner.retirement_failures.lock().map_err(display)?.get(id).is_some() {
            return Err("The transfer did not stop cleanly. No files were deleted. Restart Kiwix, then try again".into());
        }
        let initial = self.record(id)?;
        let stopping_seed = initial.verified && initial.job.seeding
            && matches!(initial.job.status.as_str(), "completed" | "complete" | "seeding");
        // An errored seed/share keeps its seed intent (Retry restarts it) but
        // nothing serves it, so `removable_terminal` accepts it unchanged. The
        // intent is not cleared here: a later refusal (still stopping, files
        // changed) must leave Retry working.
        if !stopping_seed && !removable_terminal(&initial) {
            return Err("Only finished, non-seeding torrents can be removed from history".into());
        }
        let retired = {
            let mut runtimes = self.inner.runtimes.lock().map_err(display)?;
            if let Some(runtime) = runtimes.get_mut(id) {
                // Cancelling this token only retires an already terminal worker.
                // An active transfer never reaches this point.
                let retired = runtime.retired.take()
                    .ok_or("The transfer is already stopping. Wait a moment")?;
                runtime.token.cancel();
                Some(retired)
            } else {
                None
            }
        };
        drop(gate);
        if let Some(retired) = retired {
            retired.await.map_err(|_| "The transfer stopped unexpectedly. Restart the app, then retry")??;
        }
        // Reacquire the gate so the durable intent change and the final
        // admission check stay serialized with pause/cancel.
        let _gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        if self.inner.retirement_failures.lock().map_err(display)?.get(id).is_some() {
            return Err("The transfer did not stop cleanly. No files were deleted. Restart Kiwix, then try again".into());
        }
        // Persist the stopped seed intent only after retirement joined: a
        // refusal above or a failed retirement must leave the durable record
        // claiming the seed it may still serve.
        if stopping_seed {
            self.update_record(id, |record| {
                // A cancellation that landed while the worker retired already
                // cleared the seed intent and owns the terminal status.
                if record.job.status == "cancelled" { return; }
                record.job.seeding = false;
                record.paused = true;
                record.job.status = "completed".into();
            })?;
        }
        if !removable_terminal(&self.record(id)?) {
            return Err("Only finished, non-seeding torrents can be removed from history".into());
        }
        drop(_gate);
        // A blocking worker re-acquires the gates and rechecks admission after
        // retirement. No borrowed lock or filesystem work crosses this await.
        drop(_shutdown);
        let manager = self.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || manager.finish_terminal_files(&id, delete_files))
            .await.map_err(|error| format!("Finalizing terminal torrent cleanup: {error}"))?
    }

    pub(super) fn finish_terminal_files(&self, id: &str, delete_files: bool) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.blocking_lock();
        let _gate = self.inner.gate.blocking_lock();
        self.ensure_not_shutdown()?;
        // rqbit network retirement is cancellation-based. This same fence is
        // held by every storage mutation of this job and stays held through
        // disk cleanup.
        let storage_gate = self.storage_gate(id)?;
        let _storage = storage_gate.write().map_err(display)?;
        let record = self.record(id)?;
        if self.inner.retirement_failures.lock().map_err(display)?.contains_key(id) {
            return Err("The transfer did not stop cleanly. Restart the app, then retry".into());
        }
        if !removable_terminal(&record) || self.inner.runtimes.lock().map_err(display)?.contains_key(id) {
            return Err("Torrent changed while its history was being removed".into());
        }
        if delete_files {
            self.delete_terminal_files(&record)
        } else {
            self.update(|state| {
                if record.files_deleted { state.jobs.retain(|record| record.job.id != id); }
                else {
                    state.jobs.iter_mut().find(|record| record.job.id == id).ok_or("Unknown torrent job")?.history_dismissed = true;
                }
                Ok(())
            })
        }
    }

    pub(super) fn delete_terminal_files(&self, record: &Record) -> Result<(), String> {
        let id = &record.job.id;
        // Prove the entire remaining set before the first irreversible unlink.
        // Legacy or replaced staging/metadata must not fail after archive removal.
        let publications = validate_terminal_publications(record)?;
        let mut reserved = vec![record.metadata.clone()];
        for path in &record.files {
            safe_relative(path).map_err(display)?;
            reserved.push(record.staging.join(path));
            if is_zim(path) {
                reserved.push(record.directory.join(path.file_name().ok_or("Missing archive filename")?));
            }
        }
        reserved.extend(record.job.completed_paths.iter().map(PathBuf::from));
        self.inner.store.with_acquisition_destinations(&reserved, || {
            // Test persistence before touching any payload, including legacy
            // records whose files are already absent.
            self.persist()?;
            for path in record.files.iter().filter(|path| is_zim(path)) {
                let name = path.file_name().ok_or("Missing torrent archive filename")?;
                let final_path = record.directory.join(name);
                let intent = record.publication_intents.iter().find(|intent| intent.file_name == Path::new(name));
                let existing = self.with_record(id, |r| r.published_cleanup.clone())?
                    .into_iter()
                    .find(|entry| entry.name == name.to_string_lossy());
                let mut entry = if let Some(entry) = existing {
                    let original = intent.ok_or("Published archive has no ownership intent")?.cleanup_entry()?;
                    if entry.directory != original.directory || entry.file != original.file {
                        return Err("Publication cleanup journal does not match the original download".into());
                    }
                    entry.with_publication(original.expected_size.ok_or("Missing publication size")?, original.sha256)
                } else if let Some((_, entry)) = publications.iter().find(|(path, _)| path == &final_path) {
                    entry.clone()
                } else {
                    continue;
                };
                delete_owned_file(&final_path, &mut entry, |entry| {
                    self.update_record(id, |record| {
                        record.published_cleanup.retain(|previous| previous.name != entry.name);
                        record.published_cleanup.push(entry.clone());
                    })
                })?;
            }
            self.reclaim_temporaries(&self.record(id)?)?;
            match fs::symlink_metadata(&record.staging) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    let root = open_staging(&record.staging)?;
                    let expected = record.staging_directories.iter().find(|(path, _)| path.as_os_str().is_empty())
                        .ok_or("Cannot prove ownership of legacy torrent staging; files and history were preserved")?;
                    if PublicationIdentity::of(&root.dir_metadata().map_err(display)?) != expected.1 {
                        return Err("Torrent staging directory was replaced; files and history were preserved".into());
                    }
                    // Only the identity check needs it. Windows cannot remove the
                    // staging root below while this capability is open.
                    drop(root);
                    for (relative, mut entry) in record.staging_cleanup.clone() {
                        safe_relative(&relative).map_err(display)?;
                        delete_owned_file(&record.staging.join(&relative), &mut entry, |entry| {
                            self.update_record(id, |record| {
                                if let Some((_, previous)) = record.staging_cleanup.iter_mut().find(|(path, _)| path == &relative) {
                                    *previous = entry.clone();
                                }
                            })
                        })?;
                    }
                    // Empty-only removal leaves every unrecorded file intact.
                    // Check each directory's creation stamp before removing it.
                    let mut directories = record.staging_directories.clone();
                    directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
                    for (relative, expected) in directories {
                        if !relative.as_os_str().is_empty() { safe_relative(&relative).map_err(display)?; }
                        let path = record.staging.join(&relative);
                        match crate::storage::open_directory_nofollow(&path) {
                            Ok(directory) => {
                                if PublicationIdentity::of(&directory.dir_metadata().map_err(display)?) != expected {
                                    return Err("Torrent staging subdirectory was replaced; directory preserved".into());
                                }
                                // Windows cannot remove a directory whose capability
                                // is open; the removal re-verifies `expected` there.
                                drop(directory);
                                let parent = crate::storage::open_directory_nofollow(path.parent().ok_or("Staging has no parent")?).map_err(display)?;
                                validate_publication_directory(&parent)?;
                                remove_owned_empty_directory(&parent, path.file_name().ok_or("Staging has no filename")?, &expected).map_err(|error| format!("Removing owned empty torrent staging (unknown files are preserved): {error}"))?;
                                sync_publication_directory(&parent)?;
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(display(error)),
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(display(error)),
                Ok(_) => return Err("Torrent staging is no longer an owned directory; entry and history were preserved".into()),
            }
            if let Some(mut entry) = self.with_record(id, |r| r.metadata_cleanup.clone())? {
                delete_owned_file(&record.metadata, &mut entry, |entry| {
                    self.update_record(id, |record| record.metadata_cleanup = Some(entry.clone()))
                })?;
            } else {
                match fs::symlink_metadata(&record.metadata) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(display(error)),
                    Ok(_) => return Err("Cannot prove ownership of legacy torrent metadata; file and history were preserved".into()),
                }
            }
            self.update_record(id, |record| record.files_deleted = true)
        })
    }

    /// Global peer-payload limits, bytes/second, persisted across restarts.
    /// Zero is unlimited. Engine's per-message burst requires at least 16 KiB/s.
    /// Configuration remains writable while lifecycle/network admission is suspended.
    pub async fn limits(&self, download: u64, upload: u64) -> Result<(), String> {
        let _gate = self.inner.gate.lock().await;
        self.ensure_not_shutdown()?;
        let config = limits_config(download, upload)?;
        self.update(|s| {
            s.download_limit = download;
            s.upload_limit = upload;
            Ok(())
        })?;
        if let Some(engine) = self.engine_cell()?.get() {
            engine.ratelimits.set_download_bps(config.download_bps);
            engine.ratelimits.set_upload_bps(config.upload_bps);
        }
        Ok(())
    }

    pub async fn seeding(&self, id: &str, enabled: bool) -> Result<(), String> {
        {
            let _gate = self.inner.gate.lock().await;
            self.ensure_running()?;
            let record = self.record(id)?;
            if record.history_dismissed || record.files_deleted || record.cleanup_started() {
                return Err("Torrent was removed from history or file cleanup has started".into());
            }
            if enabled {
                self.refuse_duplicate_share(&record)?;
            }
            self.update_record(id, |r| r.job.seeding = enabled)?;
        }
        if self.with_record(id, |r| r.verified)? {
            if enabled {
                self.resume(id, None).await
            } else {
                self.pause(id).await
            }
        } else {
            Ok(())
        }
    }

    /// Reversible lifecycle checkpoint. Cancel and join adapter work (including
    /// publication), remove peer handles without deleting payload, and retain the
    /// distinction between policy interruption and an explicit user pause.
    #[cfg(any(mobile, test))]
    pub async fn suspend(&self) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let mut tasks = {
            let _gate = self.inner.gate.lock().await;
            if self.inner.stop.is_cancelled() {
                return Err("Torrent manager has shut down".into());
            }
            self.inner.suspended.store(true, Ordering::SeqCst);
            for runtime in self.inner.runtimes.lock().map_err(display)?.values() {
                runtime.token.cancel();
            }
            std::mem::take(&mut *self.inner.tasks.lock().map_err(display)?)
        };
        let mut failure = None;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                failure = Some(display(error));
            }
        }
        let retired = {
            let mut engine = self.inner.engine.lock().map_err(display)?;
            std::mem::replace(&mut *engine, Arc::new(OnceCell::new()))
        };
        if let Some(engine) = retired.get() {
            // Stop the entire session, including DHT and incoming listeners.
            // Foreground resumption lazily creates a distinct fresh session.
            engine.stop().await;
        }
        self.inner.retirement_failures.lock().map_err(display)?.clear();
        // Each storage factory carries its worker token, so cancelled engine
        // tasks cannot write after this fence, even though rqbit does not join
        // all network tasks when a torrent is removed. Every job's fence is
        // taken: an op that starts after this snapshot sees a cancelled token
        // and fails before any I/O.
        {
            let gates: Vec<_> = self
                .inner
                .storage_gates
                .lock()
                .map_err(display)?
                .values()
                .cloned()
                .collect();
            let _storage = gates
                .iter()
                .map(|gate| gate.write())
                .collect::<Result<Vec<_>, _>>()
                .map_err(display)?;
        }
        self.update(|state| {
            for record in &mut state.jobs {
                restore_record(record, false);
                record.job.speed = 0.0;
                record.job.upload_speed = 0.0;
                record.job.eta_seconds = None;
            }
            Ok(())
        })?;
        if let Some(error) = failure {
            return Err(format!("Torrent worker failed during suspension: {error}"));
        }
        Ok(())
    }

    /// Permit explicit visible user actions again without starting an engine or
    /// consulting automatic-resume policy (Android foreground-service admission).
    #[cfg(any(mobile, test))]
    pub async fn resume_admission(&self) -> Result<(), String> {
        crate::transfer_policy::require_internet()?;
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let _gate = self.inner.gate.lock().await;
        if self.inner.stop.is_cancelled() {
            return Err("Torrent manager has shut down".into());
        }
        self.inner.suspended.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Inspect held automatic-resume candidates without starting any networking
    /// or changing state. Android can establish its foreground service first.
    #[cfg(any(target_os = "android", test))]
    pub fn has_interrupted(&self) -> Result<bool, String> {
        Ok(self.inner.state.lock().map_err(display)?.jobs.iter().any(|record| {
            record.interrupted && can_restore_record(record)
        }))
    }

    /// Reopen lifecycle admission and, only under the current automatic-resume
    /// policy, restart interrupted jobs. Manual pauses and terminal jobs remain
    /// untouched; an explicit later resume continues to work with policy off.
    #[cfg(any(mobile, test))]
    pub async fn resume_interrupted(&self) -> Result<(), String> {
        crate::transfer_policy::require_internet()?;
        let _shutdown = self.inner.shutdown_gate.lock().await;
        let _gate = self.inner.gate.lock().await;
        if self.inner.stop.is_cancelled() {
            return Err("Torrent manager has shut down".into());
        }
        self.inner.suspended.store(false, Ordering::SeqCst);
        if !self.inner.store.settings()?.auto_resume_downloads {
            return Ok(());
        }
        let restore = self.update(|state| {
            let mut restore = Vec::new();
            for record in &mut state.jobs {
                if record.interrupted && can_restore_record(record) {
                    restore_record(record, true);
                    restore.push(record.job.id.clone());
                }
            }
            Ok(restore)
        })?;
        for id in restore {
            self.spawn(id)?;
        }
        Ok(())
    }

    /// Resume torrents interrupted by a network-policy retirement after the
    /// policy re-admits Internet. The reconciler captures the decision once, so
    /// a denied, retired, private or read-only application never opens an
    /// engine, and manual pauses/cancels remain untouched.
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

    pub async fn shutdown(&self) -> Result<(), String> {
        let _shutdown = self.inner.shutdown_gate.lock().await;
        self.inner.stop.cancel();
        let mut tasks = {
            let _gate = self.inner.gate.lock().await;
            // Preserve intentional running vs manual-pause state for restoration.
            std::mem::take(&mut *self.inner.tasks.lock().map_err(display)?)
        };
        let mut failure = None;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                failure = Some(display(error));
            }
        }
        if let Some(engine) = self.engine_cell()?.get() {
            engine.stop().await;
        }
        // rqbit stop cancels rather than joining its tasks. Fence all storage
        // mutation, including an already-running synchronous write/init: every
        // worker token is cancelled above, so an op that starts after this
        // snapshot fails its cancellation check before any I/O.
        {
            let gates: Vec<_> = self
                .inner
                .storage_gates
                .lock()
                .map_err(display)?
                .values()
                .cloned()
                .collect();
            let _storage = gates
                .iter()
                .map(|gate| gate.write())
                .collect::<Result<Vec<_>, _>>()
                .map_err(display)?;
        }
        // App shutdown and privacy/read-only cutovers must not leave a durable
        // share row advertising a seed that no worker serves. Shares require an
        // explicit "Share again"; ordinary acquisition restoration is unchanged.
        self.update(|state| {
            for record in &mut state.jobs {
                if record.share.is_some() {
                    restore_record(record, false);
                    record.job.speed = 0.0;
                    record.job.upload_speed = 0.0;
                    record.job.eta_seconds = None;
                }
            }
            Ok(())
        })?;
        self.persist()?;
        if let Some(error) = failure {
            return Err(format!("Torrent worker failed during shutdown: {error}"));
        }
        Ok(())
    }

    pub(super) fn ensure_not_shutdown(&self) -> Result<(), String> {
        if self.inner.stop.is_cancelled() {
            Err("Torrent manager has shut down".into())
        } else {
            Ok(())
        }
    }

    pub(super) fn ensure_running(&self) -> Result<(), String> {
        self.ensure_not_shutdown()?;
        if self.inner.suspended.load(Ordering::SeqCst) {
            Err("Torrent manager is suspended".into())
        } else {
            Ok(())
        }
    }

    /// Borrow one record under the state lock. Callers that only read a flag
    /// or one field use this instead of `record`, which clones every file list
    /// and cleanup ledger entry.
    pub(super) fn with_record<T>(&self, id: &str, read: impl FnOnce(&Record) -> T) -> Result<T, String> {
        let state = self.inner.state.lock().map_err(display)?;
        let record = state
            .jobs
            .iter()
            .find(|r| r.job.id == id)
            .ok_or("Unknown torrent job")?;
        Ok(read(record))
    }

    pub(super) fn record(&self, id: &str) -> Result<Record, String> {
        self.with_record(id, Clone::clone)
    }

    /// The journaled ownership entry for exactly `path`, without cloning the
    /// rest of the record. Every payload open uses this; the ledger can hold
    /// thousands of entries and the record also holds every torrent file.
    pub(super) fn cleanup_entry(&self, id: &str, path: &Path) -> Result<Option<CleanupEntry>, String> {
        self.with_record(id, |record| {
            record
                .staging_cleanup
                .iter()
                .find(|(relative, _)| relative == path)
                .map(|(_, entry)| entry.clone())
        })
    }

    #[cfg(all(test, unix))] // Only the Unix-only torrent-thread tests use it.
    pub(super) fn handle(&self, id: &str) -> Option<Arc<ManagedTorrent>> {
        self.inner.runtimes.lock().ok()?.get(id)?.handle.clone()
    }

    pub(super) fn update<T>(&self, change: impl FnOnce(&mut State) -> Result<T, String>) -> Result<T, String> {
        let _journal = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
        // Snapshot under the state mutex, then publish the journal without it:
        // the write (including its fsync) must never block status readers. The
        // journal gate keeps publication ordered with every other journaled
        // writer, and memory advances only after the write succeeds.
        let mut next = self.inner.state.lock().map_err(display)?.clone();
        let result = change(&mut next)?;
        save_state(&self.inner.folder, &next)?;
        *self.inner.state.lock().map_err(display)? = next;
        Ok(result)
    }

    /// In-place mutation of one record. The change cannot fail, so the only
    /// failure is the journal write: that leaves memory holding exactly what
    /// the journal holds. The state is snapshotted under its mutex and the
    /// journal is written after releasing it, so a full rewrite never blocks
    /// status readers.
    ///
    /// Cost note: the admission path calls this once per created file, so the
    /// journal is rewritten per file by design (a created file must be
    /// journaled before anything can write to it). The journal always contains
    /// the whole history, so the snapshot is the whole state; only the write
    /// moved out of the state lock.
    pub(super) fn update_record(&self, id: &str, change: impl FnOnce(&mut Record)) -> Result<(), String> {
        let _journal = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
        let mut next = self.inner.state.lock().map_err(display)?.clone();
        let index = next
            .jobs
            .iter()
            .position(|r| r.job.id == id)
            .ok_or("Unknown torrent job")?;
        change(&mut next.jobs[index]);
        save_state(&self.inner.folder, &next)?;
        // Commit only this record: another job's in-memory progress must not be
        // rolled back by a whole-state replacement.
        let changed = next.jobs.swap_remove(index);
        let mut state = self.inner.state.lock().map_err(display)?;
        let index = state
            .jobs
            .iter()
            .position(|r| r.job.id == id)
            .ok_or("Unknown torrent job")?;
        state.jobs[index] = changed;
        Ok(())
    }

    /// The storage fence for one job. Every init/read/write of that job's
    /// files and its terminal cleanup take it, so a cleanup never unlinks a
    /// file while a storage call on it is in flight. Jobs never share a
    /// staging tree, so the fence is per job rather than app-wide.
    pub(super) fn storage_gate(&self, id: &str) -> Result<Arc<std::sync::RwLock<()>>, String> {
        Ok(self
            .inner
            .storage_gates
            .lock()
            .map_err(display)?
            .entry(id.to_owned())
            .or_default()
            .clone())
    }

    pub(super) fn persist(&self) -> Result<(), String> {
        let _journal = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
        // Same staged shape as `update`: snapshot under the state mutex, write
        // the journal after releasing it.
        let state = self.inner.state.lock().map_err(display)?.clone();
        save_state(&self.inner.folder, &state)
    }

    pub(super) fn engine_cell(&self) -> Result<Arc<OnceCell<Arc<Session>>>, String> {
        Ok(self.inner.engine.lock().map_err(display)?.clone())
    }

    pub(super) async fn engine(&self) -> Result<Arc<Session>, String> {
        crate::transfer_policy::require_internet()?;
        self.ensure_running()?;
        // librqbit's HTTPS tracker client panics on Android until the platform
        // verifier has the application Context; never start an engine without it.
        // This is the only initialization point: it runs after setup (and the
        // crash hook), only when BitTorrent is actually used.
        #[cfg(target_os = "android")]
        crate::android_tls::ensure_platform_verifier()?;
        let cell = self.engine_cell()?;
        let engine = cell.get_or_try_init(|| async {
            let config = {
                let state = self.inner.state.lock().map_err(display)?;
                limits_config(state.download_limit, state.upload_limit)?
            };
            let startup = self.inner.stop.child_token();
            let startup_guard = startup.clone().drop_guard();
            // The engine's default peer listener and DHT socket are `[::]`
            // dual-stack. A host without IPv6 cannot create them, so use the
            // engine's IPv4 addresses there instead of failing every torrent.
            let ipv4_only = !dual_stack_available();
            let opts = SessionOptions {
                // Persistent torrent metadata belongs to this adapter; avoid
                // engine auto-restoration opening paths before our validation.
                persistence: None, fastresume: false,
                dht: Some(DhtSessionConfig { persistence: None, ..Default::default() }),
                listen: Some(ListenerOptions { ipv4_only, ..Default::default() }),
                ipv4_only,
                cancellation_token: Some(startup),
                ratelimits: config, concurrent_init_limit: Some(2),
                client_name_and_version: Some("Kiwix Workbench".into()),
                ..Default::default()
            };
            // The session's DHT, listener and tracker tasks start on the runtime
            // that creates it. If this caller is abandoned, the startup guard
            // cancels whatever session the routed call still produces.
            let folder = self.inner.folder.clone();
            crate::install_crypto_provider();
            let session = tokio::select! {
                _ = self.inner.stop.cancelled() => Err("Torrent manager has shut down".to_string()),
                result = on_torrent_runtime(Session::new_with_opts(folder, opts)) => result?.map_err(display),
            }?;
            startup_guard.disarm();
            Ok::<_, String>(session)
        }).await?;
        let config = {
            let state = self.inner.state.lock().map_err(display)?;
            limits_config(state.download_limit, state.upload_limit)?
        };
        if engine.ratelimits.get_config() != config {
            engine.ratelimits.set_download_bps(config.download_bps);
            engine.ratelimits.set_upload_bps(config.upload_bps);
        }
        Ok(engine.clone())
    }

    pub(super) fn spawn(&self, id: String) -> Result<(), String> {
        crate::transfer_policy::require_internet()?;
        self.ensure_running()?;
        if self.inner.retirement_failures.lock().map_err(display)?.contains_key(&id) {
            return Err("The transfer did not stop cleanly. Restart Kiwix, then resume".into());
        }
        let mut tasks = self.inner.tasks.lock().map_err(display)?;
        // A worker panic is captured inside its own task (below), so a clean
        // completion is the only expected result; never silently discard one.
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result {
                eprintln!("Torrent worker task failed: {error}");
            }
        }
        let mut runtimes = self.inner.runtimes.lock().map_err(display)?;
        if let Some(runtime) = runtimes.get(&id) {
            if !runtime.token.is_cancelled() {
                return Ok(());
            }
            return Err("Torrent worker is still retiring".into());
        }
        let token = self.inner.stop.child_token();
        let (retirement, retired) = tokio::sync::oneshot::channel();
        let generation = Uuid::new_v4();
        runtimes.insert(
            id.clone(),
            Runtime {
                generation,
                token: token.clone(),
                retired: Some(retired),
                handle: None,
            },
        );
        let this = self.clone();
        tasks.spawn(async move {
            // A panicking worker must still reach the retirement path below;
            // otherwise its runtime/engine entry stays live and the job can
            // never be retried. Capture the panic as this worker's error, the
            // same boundary the map manager uses.
            let cleanup = this.clone();
            let cleanup_id = id.clone();
            let cleanup_token = token.clone();
            let result = std::panic::AssertUnwindSafe(async move {
                let run = this.run(&id, &token);
                tokio::pin!(run);
                let result = tokio::select! {
                    biased;
                    _ = crate::transfer_policy::internet_denied() => {
                        this.interrupt_network_workers().await;
                        let result = run.await;
                        let _ = this.update(|state| {
                            for record in &mut state.jobs { restore_record(record, false); }
                            Ok(())
                        });
                        result
                    }
                    result = &mut run => result,
                };
                // An inner admission check can see a denial after the select has
                // already polled its policy future. Retire the same owned work; the
                // explicit policy error must never become a terminal download error.
                if result.as_ref().is_err_and(|error| error == "MOBILE_NETWORK_BLOCKED") {
                    this.interrupt_network_workers().await;
                    let _ = this.update(|state| {
                        for record in &mut state.jobs {
                            restore_record(record, false);
                        }
                        Ok(())
                    });
                }
                result
            }).catch_unwind().await.unwrap_or_else(|_| {
                Err("Torrent worker stopped unexpectedly; its last durable checkpoint was retained".into())
            });
            let this = cleanup;
            let id = cleanup_id;
            let token = cleanup_token;
            let runtime = {
                let _gate = this.inner.gate.lock().await;
                let mut runtimes = match this.inner.runtimes.lock() {
                    Ok(r) => r,
                    Err(_) => return,
                };
                if runtimes
                    .get(&id)
                    .is_some_and(|r| r.generation == generation)
                {
                    // Preserve an in-memory fence even if engine retirement
                    // panics or drops its acknowledgement after losing Runtime.
                    if let Ok(mut failures) = this.inner.retirement_failures.lock() {
                        failures.insert(id.clone(), "The transfer did not stop cleanly".into());
                    } else {
                        return;
                    }
                    runtimes.remove(&id)
                } else {
                    None
                }
            };
            // Never await the engine while holding the gate: pause, suspend and
            // terminal cleanup all need it. The failure fence above keeps the
            // job unretryable until this retirement reconciles.
            let retirement_result = if let Some(handle) = runtime.and_then(|runtime| runtime.handle) {
                match this.engine_cell() {
                    Ok(cell) => match cell.get() {
                        Some(engine) => engine.delete(TorrentIdOrHash::Id(handle.id()), false).await.map_err(display),
                        None => Err("The transfer stopped unexpectedly".into()),
                    },
                    Err(error) => Err(error),
                }
            } else {
                Ok(())
            };
            // Reacquire the gate to reconcile the durable record and the
            // failure fence with commands that ran while the engine retired.
            let _gate = this.inner.gate.lock().await;
            if let Err(error) = result {
                let cancelled = token.is_cancelled() || this.inner.stop.is_cancelled();
                let _ = this.update_record(&id, |record| {
                    apply_worker_error(record, error, cancelled);
                });
            }
            if let Ok(mut failures) = this.inner.retirement_failures.lock() {
                match &retirement_result {
                    Ok(()) => { failures.remove(&id); }
                    Err(error) => { failures.insert(id.clone(), error.clone()); }
                }
            }
            let _ = retirement.send(retirement_result);
        });
        Ok(())
    }

    pub(super) async fn interrupt_network_workers(&self) {
        self.inner.suspended.store(true, Ordering::SeqCst);
        if let Ok(runtimes) = self.inner.runtimes.lock() {
            for runtime in runtimes.values() {
                runtime.token.cancel();
            }
        }
        let retired = match self.inner.engine.lock() {
            Ok(mut engine) => std::mem::replace(&mut *engine, Arc::new(OnceCell::new())),
            Err(_) => return,
        };
        if let Some(engine) = retired.get() {
            engine.stop().await;
        }
    }

    /// Adds a torrent on the torrent runtime, holding no gate, so pause, cancel
    /// and suspend never wait for a torrent thread. Returns `None` once the
    /// worker's token is cancelled.
    ///
    /// librqbit registers the torrent synchronously after the add's last await,
    /// so a routed add can still register after this worker has given up on it.
    /// An abandoned add is therefore aborted and reaped: a detached task awaits
    /// it and deletes any torrent it registered.
    pub(super) async fn add_on_torrent_runtime(
        &self,
        id: &str,
        token: &CancellationToken,
        engine: &Arc<Session>,
        add: AddTorrent<'static>,
        options: AddTorrentOptions,
        already_managed: &str,
    ) -> Result<Option<Arc<ManagedTorrent>>, String> {
        // An earlier worker's abandoned add may still hold this torrent in the
        // session. Adding it again first would fail as "already managed".
        // A reap only waits for a torrent thread, unless the abandoned add is
        // stuck opening storage; report that rather than wait silently.
        let pending = self.pending_reaps(id)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        for reap in pending {
            tokio::select! {
                _ = token.cancelled() => return Ok(None),
                _ = reap.cancelled() => {}
                _ = tokio::time::sleep_until(deadline) => {
                    return Err("The previous transfer of this torrent is still stopping. Try again in a moment".into());
                }
            }
        }
        let adding = Arc::clone(engine);
        #[cfg(test)]
        let hook = self.inner.engine_hook.lock().map_err(display)?.clone();
        let mut call = TorrentCall::spawn(async move {
            let response = adding.add_torrent(add, Some(options)).await;
            #[cfg(test)]
            if let Some(hook) = hook {
                hook(EngineCall::Added(matches!(response, Ok(AddTorrentResponse::Added(..)))));
            }
            response
        })?;
        // Biased: a finished add is recorded even if cancellation also fired,
        // so retirement deletes it and no reaper is needed.
        let finished = tokio::select! {
            biased;
            result = &mut call => Some(result),
            _ = token.cancelled() => None,
        };
        let Some(result) = finished else {
            self.reap_abandoned_add(id, Arc::clone(engine), call.into_task());
            return Ok(None);
        };
        let handle = match result?.map_err(display)? {
            AddTorrentResponse::Added(_, handle) => handle,
            _ => return Err(already_managed.into()),
        };
        // Only this worker's retirement deletes `Runtime::handle`, and it runs
        // after this returns, so recording needs no gate: a pause that cancels
        // the token now still retires and deletes this handle. If the entry
        // cannot be updated, undo the registration rather than orphan it.
        let recorded = match self.inner.runtimes.lock() {
            Ok(mut runtimes) => runtimes
                .get_mut(id)
                .map(|runtime| runtime.handle = Some(handle.clone()))
                .is_some(),
            Err(_) => false,
        };
        if !recorded {
            let _ = engine.delete(TorrentIdOrHash::Id(handle.id()), false).await;
            return Err("The transfer stopped unexpectedly".into());
        }
        Ok(Some(handle))
    }

    /// Reaps for this job that are still running.
    pub(super) fn pending_reaps(&self, id: &str) -> Result<Vec<CancellationToken>, String> {
        let mut reaps = self.inner.reaps.lock().map_err(display)?;
        let Some(pending) = reaps.get_mut(id) else { return Ok(Vec::new()) };
        pending.retain(|reap| !reap.is_cancelled());
        let pending = pending.clone();
        if pending.is_empty() {
            reaps.remove(id);
        }
        Ok(pending)
    }

    /// Detached, so no pause, suspend or shutdown waits on it: the abort only
    /// takes effect at the task's next poll, which needs a torrent thread.
    pub(super) fn reap_abandoned_add(
        &self,
        id: &str,
        engine: Arc<Session>,
        task: tokio::task::JoinHandle<anyhow::Result<AddTorrentResponse>>,
    ) {
        task.abort();
        let done = CancellationToken::new();
        if let Ok(mut reaps) = self.inner.reaps.lock() {
            let pending = reaps.entry(id.to_owned()).or_default();
            pending.retain(|reap| !reap.is_cancelled());
            pending.push(done.clone());
        }
        // Cancelled when the reap ends, including if its task is dropped.
        let done = done.drop_guard();
        let inner = Arc::downgrade(&self.inner);
        let id = id.to_owned();
        tokio::spawn(async move {
            // `AlreadyManaged` names another job's torrent; it is never deleted.
            if let Ok(Ok(AddTorrentResponse::Added(torrent, _))) = task.await {
                if let Err(error) = engine.delete(TorrentIdOrHash::Id(torrent), false).await {
                    eprintln!("Could not remove an abandoned torrent from the engine: {error:#}");
                }
            }
            drop(done);
            if let Some(inner) = inner.upgrade() {
                let manager = TorrentManager { inner };
                let _ = manager.pending_reaps(&id);
            }
        });
    }

    /// Unpauses on the torrent runtime, holding no gate. Returns `false` once
    /// the worker's token is cancelled; the worker then retires and deletes the
    /// handle. An abandoned unpause that has not started never runs. One that
    /// is already running either finishes before the deletion, which pauses
    /// the torrent again, or fails on the deleted handle.
    pub(super) async fn unpause_on_torrent_runtime(
        &self,
        token: &CancellationToken,
        engine: &Arc<Session>,
        handle: &Arc<ManagedTorrent>,
    ) -> Result<bool, String> {
        let (resuming, torrent) = (Arc::clone(engine), Arc::clone(handle));
        #[cfg(test)]
        let hook = self.inner.engine_hook.lock().map_err(display)?.clone();
        let call = on_torrent_runtime(async move {
            #[cfg(test)]
            if let Some(hook) = hook {
                hook(EngineCall::Unpause);
            }
            resuming.unpause(&torrent).await
        });
        tokio::select! {
            biased;
            result = call => {
                result?.map_err(display)?;
                Ok(true)
            }
            _ = token.cancelled() => Ok(false),
        }
    }

    pub(super) fn check_space(&self, id: &str, acknowledgement: Option<&str>) -> Result<(), String> {
        let record = self.record(id)?;
        if record.job.total.is_none() {
            return Ok(());
        }
        let mut approved = Vec::new();
        for (directory, remaining, scope) in space_requirements(&record)? {
            let identity = serde_json::to_string(&(
                &record.job.id,
                &record.job.url,
                &record.staging,
                &record.directory,
                &record.space_files,
                scope,
            ))
            .map_err(display)?;
            let warning = match crate::disk_warning::check(
                &directory, Some(remaining), &identity, None,
            ) {
                Ok(()) => continue,
                Err(error) if error.starts_with("LOW_DISK_SPACE:") => error,
                Err(error) => return Err(error),
            };
            let mut accepted = None;
            for candidate in acknowledgement
                .into_iter()
                .chain(record.space_acknowledgements.iter().map(String::as_str))
            {
                match crate::disk_warning::check(
                    &directory, Some(remaining), &identity, Some(candidate),
                ) {
                    Ok(()) => {
                        accepted = Some(candidate);
                        break;
                    }
                    Err(error) if error.starts_with("LOW_DISK_SPACE:") => {}
                    Err(error) => return Err(error),
                }
            }
            if let Some(accepted) = accepted {
                approved.push(accepted.to_owned());
                // Keep the first volume's explicit approval if the next volume
                // needs a separate confirmation on this same resume attempt.
                if !record.space_acknowledgements.iter().any(|prior| prior == accepted) {
                    self.update_record(id, |r| {
                        r.space_acknowledgements.push(accepted.to_owned());
                    })?;
                }
            } else {
                return Err(warning);
            }
        }
        if !self.with_record(id, |r| r.space_acknowledgements == approved)? {
            self.update_record(id, |r| r.space_acknowledgements = approved)?;
        }
        Ok(())
    }

    pub(super) fn recover_metadata(&self, record: &Record) -> Result<(), String> {
        let Some(temporary) = &record.metadata_temporary else { return Ok(()); };
        let ownership = recover_metadata_publication(
            &record.metadata, temporary, record.metadata_cleanup.as_ref(),
        )?;
        self.update_record(&record.job.id, |record| {
            record.metadata_temporary = None;
            record.metadata_cleanup = ownership;
        })
    }

    pub(super) fn publish_metadata(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        let record = self.record(id)?;
        publish_metadata(&record.metadata, bytes, |temporary, ownership| {
            self.update_record(id, |record| {
                record.metadata_temporary = temporary;
                record.metadata_cleanup = ownership;
            })
        })
    }

    pub(super) fn reclaim_temporaries(&self, record: &Record) -> Result<(), String> {
        self.recover_metadata(record)?;
        if record.publication_temporaries.is_empty() {
            return Ok(());
        }
        let destination = open_publication_directory(&record.directory)?;
        for temporary in &record.publication_temporaries {
            reclaim_publication_temporary(&record.directory, &destination, temporary)?;
            self.update_record(&record.job.id, |r| {
                r.publication_temporaries
                    .retain(|prior| prior.name != temporary.name);
            })?;
        }
        Ok(())
    }

    pub(super) async fn run(&self, id: &str, token: &CancellationToken) -> Result<(), String> {
        if token.is_cancelled() {
            return Ok(());
        }
        self.reclaim_temporaries(&self.record(id)?)?;
        let record = self.record(id)?;
        if let Some(share) = record.share.clone() {
            return self.run_share(id, token, record, share).await;
        }
        self.update_record(id, |r| {
            if !token.is_cancelled() && !r.paused {
                r.job.status = "resolving".into();
            }
        })?;
        let record = self.record(id)?;
        if !record.verified && !record.publication_intents.is_empty() {
            self.update_record(id, |r| {
                if !token.is_cancelled() && !r.paused {
                    r.job.status = "publishing".into();
                }
            })?;
            let recovery_record = record.clone();
            let stop = token.clone();
            let manager = self.clone();
            let job_id = id.to_owned();
            let recovered = tokio::task::spawn_blocking(move || {
                let destination =
                    Dir::open_ambient_dir(&recovery_record.directory, ambient_authority())
                        .map_err(display)?;
                recover_archives(&recovery_record, &destination, &stop, |path| {
                    manager.update_record(&job_id, |r| {
                        if !stop.is_cancelled() && !r.paused && !r.job.completed_paths.contains(&path) {
                            r.job.completed_paths.push(path);
                        }
                    })
                })
            })
            .await
            .map_err(display)??;
            if token.is_cancelled() {
                return Ok(());
            }
            if !recovered.is_empty()
                && recovered.len() == record.files.iter().filter(|path| is_zim(path)).count()
            {
                self.update_record(id, |r| {
                    if token.is_cancelled() || r.paused {
                        return;
                    }
                    r.verified = true;
                    r.job.completed_paths = recovered;
                    r.job.destination = r.job.completed_paths[0].clone();
                    r.job.downloaded = r.job.total.unwrap_or(r.job.downloaded);
                    r.job.status = "completed".into();
                    // Recovery can finish without staging, peers, or an
                    // engine. A future explicit Seed action starts the worker.
                    r.job.seeding = false;
                    r.job.error = None;
                    r.job.speed = 0.0;
                    r.job.upload_speed = 0.0;
                    r.job.eta_seconds = None;
                })?;
                // Completion recovery must not depend on missing staging data
                // or peers, even when future seeding is permitted.
                return Ok(());
            }
        }
        // Known jobs are admitted before discovery or an engine can touch payload.
        self.check_space(id, None)?;
        let root = open_staging(&record.staging)?;
        let engine = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = self.engine() => result?,
        };
        let add = if record.metadata.exists() {
            let bytes = read_torrent_metadata(&record)?;
            AddTorrent::from_bytes(bytes)
        } else if record.job.url.starts_with("magnet:") {
            AddTorrent::from_url(record.job.url.clone())
        } else {
            AddTorrent::from_bytes(tokio::select! {
                _ = token.cancelled() => return Ok(()),
                result = fetch_metadata(&record.job.url) => result?,
            })
        };
        // list_only resolves magnets but does not create/open torrent files.
        let initial_peers = if record.initial_peers.is_empty() {
            None
        } else {
            Some(record.initial_peers.clone())
        };
        let options = AddTorrentOptions {
            list_only: true, output_folder: Some(record.staging.to_string_lossy().into_owned()),
            sub_folder: None, initial_peers: initial_peers.clone(), ..Default::default()
        };
        let listing = Arc::clone(&engine);
        // Magnet resolution polls tracker streams, including HTTPS handshakes,
        // inside this future, so it runs on the torrent runtime. A list-only
        // add registers nothing in the session, so abandoning it is safe.
        let listed = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = on_torrent_runtime(async move { listing.add_torrent(add, Some(options)).await }) => result?.map_err(display)?,
        };
        let listed = match listed {
            AddTorrentResponse::ListOnly(listed) => listed,
            _ => return Err("This torrent is already managed by another acquisition job".into()),
        };
        if listed.torrent_bytes.len() > MAX_METADATA {
            return Err("Torrent metadata exceeds 16 MiB".into());
        }
        let mut files = Vec::new();
        let mut names = BTreeSet::new();
        let mut expected = HashMap::new();
        let mut total = 0u64;
        let mut space_files = Vec::new();
        for details in listed.info.iter_file_details() {
            if details.attrs().symlink || details.symlink_path.is_some() {
                return Err("Torrent symlink entries are not permitted".into());
            }
            let path = details.filename.to_pathbuf();
            safe_relative(&path).map_err(display)?;
            if !names.insert(path.clone()) {
                return Err("Torrent contains duplicate paths".into());
            }
            // Torrent totals can exceed FAT's limit when each real output fits.
            // Check staging and the separate publication volume per file before
            // the engine can create, resize or write any torrent payload.
            if !details.attrs().padding {
                space_files.push(SpaceFile { path: path.clone(), size: details.len });
                let staging_file = record.staging.join(&path);
                crate::storage::check_file_size(
                    staging_file
                        .parent()
                        .ok_or("Torrent file has no staging directory")?,
                    details.len,
                )
                .map_err(|e| format!("Cannot stage {}: {e}", path.display()))?;
                if is_zim(&path) {
                    crate::storage::check_file_size(&record.directory, details.len)
                        .map_err(|e| format!("Cannot publish {}: {e}", path.display()))?;
                }
            }
            total = total
                .checked_add(details.len)
                .ok_or("Torrent size overflow")?;
            expected.insert(path.clone(), details.len);
            files.push(path);
        }
        if files.is_empty() || files.len() > 100_000 {
            return Err("Torrent has an invalid number of files".into());
        }
        let zim_files: Vec<_> = files.iter().filter(|p| is_zim(p)).collect();
        if zim_files.is_empty() {
            return Err("Torrent does not contain a complete .zim archive".into());
        }
        if !record.files.is_empty() && record.files != files {
            return Err("Torrent metadata changed since this job was created".into());
        }
        let mut published_names = BTreeSet::new();
        for file in &zim_files {
            if !published_names.insert(file.file_name().unwrap().to_owned()) {
                return Err("Torrent archives have colliding destination filenames".into());
            }
        }
        if token.is_cancelled() {
            return Ok(());
        }
        if !record.metadata.exists() {
            self.publish_metadata(id, &listed.torrent_bytes)?;
        }
        let name = zim_files
            .first()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        self.update_record(id, |r| {
            r.files = files;
            r.job.total = Some(total);
            r.space_files = space_files;
            r.job.file_name = name;
            if !token.is_cancelled() && r.job.status != "cancelled" {
                r.job.status = if r.paused { "paused" } else { "checking" }.into();
            }
        })?;
        // Persist total and metadata before warning so explicit resume can use
        // exactly the same job identity and estimate without network discovery.
        self.check_space(id, None)?;
        let sources = Arc::new(Mutex::new(HashMap::new()));
        let mut factory = ConfinedFactory {
            root: Arc::new(root),
            path: record.staging.clone(),
            expected: Arc::new(expected),
            sources: sources.clone(),
            existing: !record.files.is_empty(),
            stop: token.clone(),
            gate: self.storage_gate(id)?,
            dirs: Arc::new(Mutex::new(HashSet::new())),
            owner: Some((self.clone(), id.to_owned())),
        };
        if record.files.is_empty() {
            // First run only: no engine has opened or written these files yet.
            let prefill = factory.clone();
            let store = self.inner.store.clone();
            let staged = self.with_record(id, |r| r.space_files.clone())?;
            let created = tokio::task::spawn_blocking(move || prefill_from_partials(&prefill, &store, &staged))
                .await
                .map_err(display)??;
            if token.is_cancelled() {
                return Ok(());
            }
            // Created files are owned and journaled; the engine must open them
            // and hash every piece in its initial check before counting any.
            factory.existing |= created;
        }
        let web_seed = match super::web_seed::parse(&listed.torrent_bytes) {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("Ignoring malformed torrent web-seed metadata: {error}");
                None
            }
        };
        let already_verified = self.with_record(id, |record| record.verified)?;
        if let Some(seed) = web_seed.filter(|_| !already_verified) {
            if !seed.urls.is_empty() {
                self.update_record(id, |r| {
                    if !token.is_cancelled() && !r.paused && r.job.status != "cancelled" {
                        r.job.status = "downloading".into();
                    }
                })?;
                // The engine must open any files this adapter created. A
                // missing file is still created by ConfinedFactory.
                factory.existing = true;
                // A row is display progress, not recovery evidence. Resume
                // reopens the owned files and verifies against torrent pieces.
                let result = super::web_seed::download(token, &factory, &seed, &|downloaded, total, speed| {
                    if let Ok(mut state) = self.inner.state.lock() {
                        if let Some(record) = state.jobs.iter_mut().find(|record| record.job.id == id
                            && !token.is_cancelled() && !record.paused && record.job.status != "cancelled")
                        {
                            record.job.downloaded = downloaded.min(total);
                            record.job.total = Some(total);
                            record.job.speed = speed;
                            record.job.eta_seconds = (speed > 0.0 && downloaded < total)
                                .then(|| ((total - downloaded) as f64 / speed).ceil() as u64);
                        }
                    }
                }).await;
                if let Ok(mut state) = self.inner.state.lock() {
                    if let Some(record) = state.jobs.iter_mut().find(|record| record.job.id == id) {
                        record.job.speed = 0.0;
                        record.job.eta_seconds = None;
                    }
                }
                match result {
                    Ok(()) => {}
                    Err(_) if token.is_cancelled() => return Ok(()),
                    Err(error) => {
                        // librqbit's byte-add path leaves seen_peers empty even
                        // when trackers can supply peers, so a web-seed failure
                        // cannot be treated as proof that no peer source exists.
                        eprintln!("Torrent web seed failed; continuing with peers: {error}");
                    }
                }
            }
        }
        let options = {
            let _gate = self.inner.gate.lock().await;
            if token.is_cancelled() {
                return Ok(());
            }
            AddTorrentOptions {
                paused: self.with_record(id, |r| r.paused)?, overwrite: true,
                output_folder: Some(record.staging.to_string_lossy().into_owned()),
                storage_factory: Some(factory.boxed()),
                initial_peers: Some(initial_peers.unwrap_or(listed.seen_peers)), ..Default::default()
            }
        };
        let Some(handle) = self
            .add_on_torrent_runtime(
                id, token, &engine, AddTorrent::from_bytes(listed.torrent_bytes), options,
                "This torrent is already managed by another acquisition job",
            )
            .await?
        else {
            return Ok(());
        };
        let mut checkpoint = Instant::now();
        loop {
            tokio::select! {
                _ = token.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            if token.is_cancelled() {
                return Ok(());
            }
            let stats = handle.stats();
            if let Some(error) = stats.error {
                return Err(error);
            }
            let finished = stats.finished;
            let mut should_publish = false;
            {
                let mut state = self.inner.state.lock().map_err(display)?;
                let r = state
                    .jobs
                    .iter_mut()
                    .find(|r| r.job.id == id)
                    .ok_or("Unknown torrent job")?;
                r.job.downloaded = stats.progress_bytes;
                r.job.uploaded = stats.uploaded_bytes;
                r.job.total = Some(stats.total_bytes);
                r.job.speed = stats
                    .live
                    .as_ref()
                    .map(|s| s.download_speed.mbps * 1024.0 * 1024.0)
                    .unwrap_or(0.0);
                r.job.upload_speed = stats
                    .live
                    .as_ref()
                    .map(|s| s.upload_speed.mbps * 1024.0 * 1024.0)
                    .unwrap_or(0.0);
                r.job.eta_seconds = if r.job.speed > 0.0 && !finished {
                    Some(
                        ((stats.total_bytes.saturating_sub(stats.progress_bytes)) as f64
                            / r.job.speed)
                            .ceil() as u64,
                    )
                } else {
                    None
                };
                if r.job.status != "cancelled" {
                    r.job.status = if r.paused {
                        if r.verified { "completed" } else { "paused" }
                    } else if finished && !r.verified {
                        "publishing"
                    } else if finished {
                        if r.job.seeding { "seeding" } else { "completed" }
                    } else {
                        match stats.state {
                            TorrentStatsState::Initializing { .. } => "checking",
                            _ => "downloading",
                        }
                    }
                    .into();
                    should_publish = finished && !r.verified && !r.paused && !token.is_cancelled();
                    if r.paused {
                        r.job.speed = 0.0;
                        r.job.upload_speed = 0.0;
                        r.job.eta_seconds = None;
                    }
                }
            }
            // Decide under the gate, but never await the engine while holding
            // it: every command needs this gate and pause can wait on the
            // torrent runtime.
            let (pause, unpause) = {
                let _gate = self.inner.gate.lock().await;
                // Two flags only: this runs four times a second per job.
                let (paused, seeding) =
                    self.with_record(id, |record| (record.paused, record.job.seeding))?;
                let current = handle.stats().state;
                if finished && !seeding {
                    (matches!(current, TorrentStatsState::Live), false)
                } else {
                    // In particular, resume during an in-flight hash check must
                    // not leave the eventual paused state stranded.
                    (false, !paused && matches!(current, TorrentStatsState::Paused))
                }
            };
            if pause {
                engine.pause(&handle).await.map_err(display)?;
                // Reconcile: a seeding/resume intent that landed while the
                // engine was pausing must be honoured in this tick.
                let resume = {
                    let _gate = self.inner.gate.lock().await;
                    let (paused, seeding) =
                        self.with_record(id, |record| (record.paused, record.job.seeding))?;
                    !(finished && !seeding) && !paused
                        && matches!(handle.stats().state, TorrentStatsState::Paused)
                };
                if resume && !self.unpause_on_torrent_runtime(token, &engine, &handle).await? {
                    return Ok(());
                }
            } else if unpause && !self.unpause_on_torrent_runtime(token, &engine, &handle).await? {
                return Ok(());
            }
            if should_publish {
                let record = {
                    let _gate = self.inner.gate.lock().await;
                    let record = self.record(id)?;
                    if token.is_cancelled() || record.paused || record.job.status == "cancelled" {
                        continue;
                    }
                    self.check_space(id, None)?;
                    record
                };
                let stop = token.clone();
                let manager = self.clone();
                let job_id = id.to_owned();
                let sources = sources.lock().map_err(display)?.clone();
                let paths = tokio::task::spawn_blocking(move || {
                    publish_archives(
                        &record,
                        &manager.inner.store,
                        &sources,
                        &stop,
                        |temporary, remove| {
                            manager.update_record(&job_id, |r| {
                                r.publication_temporaries
                                    .retain(|prior| prior.name != temporary.name);
                                if !remove {
                                    r.publication_temporaries.push(temporary);
                                }
                            })
                        },
                        |intent| {
                            manager.update_record(&job_id, |r| {
                                r.publication_intents
                                    .retain(|prior| prior.file_name != intent.file_name);
                                r.publication_intents.push(intent);
                            })
                        },
                        |path| {
                            manager.update_record(&job_id, |r| {
                                if !stop.is_cancelled() && !r.paused && !r.job.completed_paths.contains(&path) {
                                    r.job.completed_paths.push(path);
                                }
                            })
                        },
                    )
                })
                .await
                .map_err(display)??;
                if token.is_cancelled() {
                    return Ok(());
                }
                self.update_record(id, |r| {
                    if token.is_cancelled() || r.paused {
                        return;
                    }
                    r.verified = true;
                    r.job.completed_paths = paths;
                    if let Some(path) = r.job.completed_paths.first() {
                        r.job.destination = path.clone();
                    }
                })?;
            }
            if checkpoint.elapsed() >= Duration::from_secs(5) {
                self.persist()?;
                checkpoint = Instant::now();
            }
        }
    }

    /// Create deterministic metadata for a read-only library archive and seed
    /// it. No staging directory, copy, rename, or truncate is performed. The
    /// archive identity is rechecked before metadata creation and before the
    /// engine opens the file, then periodically while seeding.
    pub(super) async fn run_share(
        &self,
        id: &str,
        token: &CancellationToken,
        record: Record,
        share: ShareSource,
    ) -> Result<(), String> {
        self.verify_share(&record, &share)?;
        if token.is_cancelled() {
            return Ok(());
        }
        let piece_length = if share.piece_length == 0 {
            SHARE_PIECE_LENGTH
        } else {
            share.piece_length
        };
        if !record.metadata.exists() {
            self.update_record(id, |r| {
                if !r.paused && !token.is_cancelled() {
                    r.job.status = "creating".into();
                }
            })?;
            let file_name = share
                .path
                .file_name()
                .ok_or("This file cannot be shared because it has no name. Rename it and try again")?
                .to_string_lossy()
                .into_owned();
            let spawner = librqbit::spawn_utils::BlockingSpawner::new(1);
            let created = tokio::select! {
                _ = token.cancelled() => return Ok(()),
                result = librqbit::create_torrent(
                    &share.path,
                    CreateTorrentOptions {
                        name: Some(file_name.as_str()),
                        trackers: Vec::new(),
                        piece_length: Some(piece_length),
                    },
                    &spawner,
                ) => result.map_err(display)?,
            };
            // The engine hashed a path; prove the path still names the exact
            // selected inode before any metadata or peer can be exposed.
            self.verify_share(&record, &share)?;
            if created.info_hash() != created.as_info().info_hash {
                return Err("Created torrent info hash is inconsistent".into());
            }
            let info_hash = created.info_hash().as_string();
            let bytes = created.as_bytes().map_err(display)?;
            let magnet_uri = created.as_magnet().to_string();
            self.publish_metadata(id, &bytes)?;
            let torrent_path = record.metadata.to_string_lossy().into_owned();
            self.update_record(id, |r| {
                if let Some(share) = r.share.as_mut() {
                    share.info_hash = info_hash.clone();
                    share.piece_length = piece_length;
                }
                r.job.url = magnet_uri.clone();
                r.job.magnet = Some(magnet_uri.clone());
                r.job.torrent_path = Some(torrent_path);
                if !token.is_cancelled() && !r.paused {
                    r.job.status = "checking".into();
                }
            })?;
        } else {
            let bytes = read_torrent_metadata(&record)?;
            let parsed = librqbit::torrent_from_bytes(&bytes).map_err(display)?;
            let info_hash = parsed.info_hash.as_string();
            if !share.info_hash.is_empty() && share.info_hash != info_hash {
                return Err("Torrent metadata changed since this share was created".into());
            }
            if let Some(length) = parsed.info.data.length {
                if length != share.identity.size {
                    return Err("Torrent metadata does not describe the selected archive".into());
                }
            }
            let magnet_uri = librqbit::Magnet::from_id20(parsed.info_hash, Vec::new(), None)
                .to_string();
            let torrent_path = record.metadata.to_string_lossy().into_owned();
            self.update_record(id, |r| {
                if let Some(share) = r.share.as_mut() {
                    share.info_hash = info_hash.clone();
                    share.piece_length = piece_length;
                }
                r.job.url = magnet_uri.clone();
                r.job.magnet = Some(magnet_uri.clone());
                r.job.torrent_path = Some(torrent_path);
                if !token.is_cancelled() && !r.paused {
                    r.job.status = "checking".into();
                }
            })?;
            if record.metadata_cleanup.is_none() {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&record.metadata)
                    .map_err(display)?;
                let ownership = CleanupEntry::retain(&record.metadata, &file)?;
                self.update_record(id, |r| r.metadata_cleanup = Some(ownership))?;
            }
        }
        let parent = share
            .path
            .parent()
            .ok_or("This shared file is no longer where Kiwix last saw it. Add it to the library again")?
            .to_path_buf();
        let engine = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = self.engine() => result?,
        };
        let factory = LibraryFactory {
            path: share.path.clone(),
            expected: share.identity.clone(),
            stop: token.clone(),
            gate: self.storage_gate(id)?,
        };
        let (bytes, options) = {
            let _gate = self.inner.gate.lock().await;
            if token.is_cancelled() {
                return Ok(());
            }
            let current = self.record(id)?;
            (read_torrent_metadata(&current)?, AddTorrentOptions {
                paused: current.paused,
                overwrite: true,
                output_folder: Some(parent.to_string_lossy().into_owned()),
                storage_factory: Some(factory.boxed()),
                disable_trackers: true,
                ..Default::default()
            })
        };
        let Some(handle) = self
            .add_on_torrent_runtime(
                id, token, &engine, AddTorrent::from_bytes(bytes), options,
                "This torrent is already managed by another job",
            )
            .await?
        else {
            return Ok(());
        };
        let mut checkpoint = Instant::now();
        let mut identity_check = Instant::now();
        loop {
            tokio::select! {
                _ = token.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            if token.is_cancelled() {
                return Ok(());
            }
            if identity_check.elapsed() >= Duration::from_secs(5) {
                identity_check = Instant::now();
                if self.verify_share(&record, &share).is_err() {
                    return Err("The shared file changed, so sharing stopped. Start sharing again".into());
                }
            }
            let stats = handle.stats();
            if let Some(error) = stats.error {
                return Err(error);
            }
            let finished = stats.finished;
            {
                let mut state = self.inner.state.lock().map_err(display)?;
                let r = state
                    .jobs
                    .iter_mut()
                    .find(|r| r.job.id == id)
                    .ok_or("Unknown torrent job")?;
                r.job.uploaded = stats.uploaded_bytes;
                r.job.upload_speed = stats
                    .live
                    .as_ref()
                    .map(|s| s.upload_speed.mbps * 1024.0 * 1024.0)
                    .unwrap_or(0.0);
                r.job.total = Some(share.identity.size);
                r.job.downloaded = if finished {
                    share.identity.size
                } else {
                    stats.progress_bytes.min(share.identity.size)
                };
                if r.job.status != "cancelled" {
                    r.job.status = if r.paused {
                        "completed"
                    } else if finished {
                        "seeding"
                    } else {
                        "checking"
                    }
                    .into();
                    if r.paused {
                        r.job.upload_speed = 0.0;
                    }
                }
            }
            // Decide under the gate, but never await the engine while holding
            // it: every command needs this gate and pause can wait on the
            // torrent runtime.
            let (pause, unpause) = {
                let _gate = self.inner.gate.lock().await;
                // Two flags only: this runs four times a second per job.
                let (paused, seeding) =
                    self.with_record(id, |record| (record.paused, record.job.seeding))?;
                let current = handle.stats().state;
                if (!seeding || paused) && matches!(current, TorrentStatsState::Live) {
                    (true, false)
                } else {
                    (false, !paused && matches!(current, TorrentStatsState::Paused))
                }
            };
            if pause {
                engine.pause(&handle).await.map_err(display)?;
                // Reconcile: a share/resume intent that landed while the engine
                // was pausing must be honoured in this tick.
                let resume = {
                    let _gate = self.inner.gate.lock().await;
                    let (paused, seeding) =
                        self.with_record(id, |record| (record.paused, record.job.seeding))?;
                    let current = handle.stats().state;
                    !((!seeding || paused) && matches!(current, TorrentStatsState::Live))
                        && !paused && matches!(current, TorrentStatsState::Paused)
                };
                if resume && !self.unpause_on_torrent_runtime(token, &engine, &handle).await? {
                    return Ok(());
                }
            } else if unpause && !self.unpause_on_torrent_runtime(token, &engine, &handle).await? {
                return Ok(());
            }
            if checkpoint.elapsed() >= Duration::from_secs(5) {
                self.persist()?;
                checkpoint = Instant::now();
            }
        }
    }

    pub(super) fn require_current_library_file(
        &self,
        book: &crate::store::Book,
        path: &Path,
        identity: &ShareIdentity,
    ) -> Result<(), String> {
        let current = self
            .inner
            .store
            .book(&book.metadata.id)?
            .ok_or("The archive was removed from the library")?;
        if current.installation_id != book.installation_id
            || current.path != path.to_string_lossy()
            || !current.available
        {
            return Err("The library archive changed or was relinked after selection".into());
        }
        let actual = share_identity_at(path)?;
        if &actual != identity {
            return Err("The library archive changed after selection".into());
        }
        Ok(())
    }

    pub(super) fn verify_share(&self, record: &Record, share: &ShareSource) -> Result<(), String> {
        if record.history_dismissed || record.files_deleted || record.cleanup_started() {
            return Err("The share was removed from history".into());
        }
        let current = self
            .inner
            .store
            .book(&share.book_id)?
            .ok_or("This file is no longer in your library, so sharing stopped")?;
        if current.installation_id != share.installation_id
            || current.path != share.path.to_string_lossy()
            || !current.available
        {
            return Err("The library archive changed or was relinked".into());
        }
        if share_identity_at(&share.path)? != share.identity {
            return Err("The library archive identity or size changed".into());
        }
        Ok(())
    }
}

fn errored(record: &Record) -> bool {
    matches!(record.job.status.as_str(), "failed" | "error")
}

/// Terminal for history removal: finished and not actually seeding. An errored
/// row has no serving worker even when its seed intent is still recorded
/// (`apply_worker_error` keeps it so Retry restarts the seed), so the flag
/// alone must not make it unremovable.
fn removable_terminal(record: &Record) -> bool {
    (!record.job.seeding || errored(record))
        && matches!(record.job.status.as_str(), "completed" | "complete" | "failed" | "error" | "cancelled")
}
