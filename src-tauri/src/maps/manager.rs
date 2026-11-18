// SPDX-License-Identifier: GPL-3.0-or-later
//! The public `MapManager` API: catalog, preparation, composition jobs and control.
#[allow(unused_imports)]
use super::*;

impl MapManager {
    /// Publication intent precedes visibility; complete_publication replaces it
    /// with job.path atomically under this same lock, including failed saves.
    pub(crate) fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        match &state.job {
            Some(job) => job.claims_monitored_path(path),
            None => Ok(false),
        }
    }

    pub(crate) fn saved_monitored_paths(store: &crate::store::Store) -> Result<crate::acquisition::MonitoredPathClaims, String> {
        let job: Option<MapJob> = crate::acquisition::saved_monitored_record(
            store, &["maps", "map-job.json"], MAX_STATE as u64,
            crate::profile_paths::RecordKind::Maps,
        )?;
        let mut claims = Vec::new();
        if let Some(job) = job {
            if let Some(path) = job.path {
                claims.push(PathBuf::from(path));
            }
            claims.extend(job.monitored_paths);
            for record in job.owned_partial.into_iter().chain(job.recovery_records) {
                if let Some(publication) = record.publication {
                    // Validate the borrowed directory/name representation too;
                    // snapshot and live admission must reject the same record.
                    let path = record.destination.join(&publication.name);
                    crate::acquisition::monitored_child_matches(
                        &path, &record.destination, Path::new(&publication.name),
                    )?;
                    claims.push(path);
                }
            }
        }
        crate::acquisition::MonitoredPathClaims::new(claims)
    }

    pub fn new(data_dir: PathBuf, store: Arc<crate::store::Store>) -> Result<Self, String> {
        fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        let cache = data_dir.join("map-parts");
        fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let state_path = data_dir.join("map-job.json");
        let mut job: Option<MapJob> = match crate::acquisition::open_history(&state_path)? {
            Some(file) => {
                use std::io::Read;
                let mut bytes = Vec::new();
                file.take(MAX_STATE as u64 + 1).read_to_end(&mut bytes)
                    .map_err(|e| format!("Cannot read saved map job: {e}"))?;
                if bytes.len() > MAX_STATE { return Err("Map job history exceeds its state limit; saved claims were not changed".into()); }
                #[cfg(desktop)]
                if store.portable_mode() {
                    let mut value: serde_json::Value = serde_json::from_slice(&bytes)
                        .map_err(|e| format!("Cannot read saved map job: {e}"))?;
                    store.portable_rebase_saved_json(crate::profile_paths::RecordKind::Maps, &mut value)?;
                    Some(serde_json::from_value(value).map_err(|e| format!("Cannot read saved map job: {e}"))?)
                } else {
                    Some(serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read saved map job: {e}"))?)
                }
                #[cfg(not(desktop))]
                Some(serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read saved map job: {e}"))?)
            }
            None => None,
        };
        if let Some(job) = job.as_mut() {
            let mut changed = false;
            let recovery_warning = if let Some(checkpoint) = job.resume.clone().filter(|_| job.owned_partial.as_ref()
                .is_some_and(|record| record.capture.is_none())) {
                changed = true;
                let recovered = (|| {
                    if checkpoint.invalid || checkpoint.written > job.total.saturating_sub(16)
                        || !valid_hex(&checkpoint.prefix_sha256, 64) {
                        return Err("This map download is damaged. Delete the download to start again".into());
                    }
                    let mut record = job.owned_partial.clone().ok_or("This map download changed unexpectedly. Delete the download and start again")?;
                    if let Some(published) = recover_resume_publication(&mut record, checkpoint.written)? {
                        job.path = Some(published.record.destination.join(&published.record.name).to_string_lossy().into_owned());
                        job.published = Some(published);
                        job.owned_partial = None;
                        job.resume = None;
                        job.resumable = false;
                        job.status = "complete".into();
                        job.written = job.total;
                        job.error = None;
                    } else {
                        open_map_checkpoint(&record, checkpoint.written)?;
                        job.owned_partial = Some(record);
                        job.resumable = true;
                        job.written = checkpoint.written;
                    }
                    Ok::<_, String>(())
                })();
                match recovered {
                    Ok(()) => None,
                    Err(error) => { job.resumable = false; Some(error) }
                }
            } else if let Some(mut record) = job.owned_partial.clone() {
                changed = true;
                match reclaim_partial(&mut record, |record| {
                    job.owned_partial = Some(record.clone());
                    persist(&state_path, job)
                }) {
                    Ok(PartialRecovery::Reclaimed) => {
                        job.owned_partial = None;
                        job.resume = None;
                        job.resumable = false;
                        None
                    }
                    Ok(PartialRecovery::Published(path)) => {
                        if let Some(publication) = record.publication.take() {
                            record.name = publication.name;
                            job.published = Some(PublishedMap { record: record.clone(), size: publication.size, sha256: publication.sha256 });
                        }
                        job.resume = None;
                        job.resumable = false;
                        job.owned_partial = None;
                        job.status = "complete".into();
                        job.path = Some(path.to_string_lossy().into_owned());
                        job.written = job.total;
                        job.error = None;
                        job.append_recovery_warning();
                        None
                    }
                    Ok(PartialRecovery::Preserved) => {
                        job.owned_partial = Some(record.clone());
                        Some(format!(
                            "A partial map download could not be verified and was kept at {}",
                            record.destination.join(&record.name).display()
                        ))
                    }
                    Err(_) => Some("Kiwix could not clean up a partial map download. Delete the download and start again".to_owned()),
                }
            } else {
                None
            };
            if matches!(
                job.status.as_str(),
                "queued" | "downloading" | "composing" | "validating"
            ) {
                job.status = if job.resumable { "paused" } else { "failed" }.into();
                job.error = if job.resumable { None } else { Some(
                    "Composition was interrupted without a reusable output checkpoint; Get map can reuse retained verified cache blocks".into(),
                ) };
                job.retain_current_path();
                changed = true;
            }
            if let Some(warning) = recovery_warning {
                job.error = Some(match job.error.take() {
                    Some(error) => format!("{error}; {warning}"),
                    None => warning,
                });
            }
            if changed {
                persist(&state_path, job)?;
            }
        }
        Ok(Self {
            inner: Arc::new(Inner {
                cache,
                store,
                state_path,
                client: OnceCell::new(),
                state: Mutex::new(ManagerState { job, active: None, admission: 0, retirement_error: None, preparation: None, cache_reserved: false }),
                session: Uuid::new_v4().to_string(),
                active_work: std::sync::atomic::AtomicBool::new(false),
                pause_control: Mutex::new(None),
                #[cfg(test)]
                before_publish: Mutex::new(None),
                #[cfg(test)]
                before_publish_commit: Mutex::new(None),
                #[cfg(test)]
                before_worker: Mutex::new(None),
            }),
        })
    }

    pub(crate) fn prepare(&self, metadata: MapRecipeMetadata, generation: u64) -> Result<MapPreparation, String> {
        let state = self.inner.store.read()?;
        let mut matching = state.books.iter().filter(|book|
            book.metadata.name == metadata.name || book.metadata.id == metadata.uuid);
        let target = matching.next().cloned();
        if matching.next().is_some() {
            return Err("Multiple library installations match this map; resolve them before replacing it".into());
        }
        let decision = MapDecision {
            token: Uuid::new_v4().to_string(), session: self.inner.session.clone(), generation,
            catalog_url: state.settings.map_catalog_url,
            proxy_url: state.settings.map_proxy_url,
            destination: state.settings.download_directory,
            region_id: metadata.region_id.clone(), language: metadata.language.clone(),
            name: metadata.name.clone(), md5: metadata.md5.clone(),
            uuid: metadata.uuid.clone(), size: metadata.assembled_bytes, target: target.clone(),
        };
        let preparation = MapPreparation {
            token: decision.token.clone(), region_id: metadata.region_id, language: metadata.language,
            name: metadata.name, md5: metadata.md5, version: metadata.version,
            same_version: target.as_ref().is_some_and(|book| book.metadata.id == metadata.uuid),
            installed: target,
        };
        self.inner.state.lock().map_err(|_| "Map state lock poisoned")?.preparation = Some(decision);
        Ok(preparation)
    }

    pub(crate) fn validate_decision(&self, decision: &MapDecision, generation: u64) -> Result<(), String> {
        if decision.session != self.inner.session || decision.generation != generation {
            return Err("Map replacement authorization expired; select the map again".into());
        }
        decision.validate(&self.inner.store.read()?)
    }

    pub(crate) fn start_prepared(
        &self, region_id: &str, language: &str, token: &str, replace: bool,
        generation: u64, acknowledgement: Option<String>,
    ) -> Result<MapJob, String> {
        let decision = {
            let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            if state.active.is_some() { return Err("A map is already being composed".into()); }
            let decision = state.preparation.as_ref().filter(|decision| decision.token == token)
                .or_else(|| state.job.as_ref().filter(|job| job.status == "failed")
                    .and_then(|job| job.decision.as_ref()).filter(|decision| decision.token == token))
                .cloned().ok_or("Map choice expired; select the map again")?;
            if decision.region_id != region_id || decision.language != language
                || replace != decision.target.is_some()
            {
                return Err("Explicit replacement is required only for the selected installed map".into());
            }
            state.preparation = None;
            decision
        };
        self.validate_decision(&decision, generation)?;
        if let Some(job) = self.reauthorize_publication(&decision)? { return Ok(job); }
        let state = self.inner.store.read()?;
        let candidates = state.books.iter().filter(|book| book.available)
            .map(|book| PathBuf::from(&book.path)).collect();
        self.start(
            decision.catalog_url.clone(), decision.proxy_url.clone(),
            region_id.into(), language.into(), PathBuf::from(&decision.destination),
            candidates, acknowledgement, Some(decision),
        )
    }

    pub(super) fn reauthorize_publication(&self, decision: &MapDecision) -> Result<Option<MapJob>, String> {
        let candidate = {
            let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            state.job.as_ref().filter(|job| job.status == "complete" && job.published.is_some()
                && job.decision.as_ref().is_some_and(|previous|
                    previous.name == decision.name && previous.uuid == decision.uuid
                        && previous.md5 == decision.md5 && previous.size == decision.size)).cloned()
        };
        let Some(candidate) = candidate else { return Ok(None) };
        let receipt = format!("map:{}", candidate.id);
        if self.inner.store.acquisition_consumed(&receipt)? { return Ok(None); }
        let paths = self.validate_terminal_owned_paths(&candidate.id)?;
        if paths.is_empty() { return Ok(None); }
        if self.inner.store.with_acquisition_destinations_available(&paths, || Ok(()))?.is_err() {
            return Ok(None);
        }
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        if state.active.is_some() { return Err("A map is already being composed".into()); }
        let job = state.job.as_ref().filter(|job| job.id == candidate.id && job.status == "complete")
            .ok_or("Verified map output changed while authorizing its import")?;
        let mut renewed = job.clone();
        // This is a new explicit choice, never a retry silently adopting a new
        // target. Reverify the retained complete archive through ImportManager.
        decision.validate(&self.inner.store.read()?)?;
        renewed.decision = Some(decision.clone());
        renewed.history_dismissed = false;
        persist(&self.inner.state_path, &renewed)?;
        state.job = Some(renewed.clone());
        Ok(Some(renewed))
    }

    /// Linearizes cancellation, job replacement and library registration. The
    /// service already holds source mutation and reader admission gates.
    pub(crate) fn with_import_decision<T>(
        &self, id: &str, generation: u64,
        commit: impl FnOnce(Option<&MapDecision>) -> Result<T, String>,
    ) -> Result<T, String> {
        let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let job = state.job.as_ref().filter(|job| job.id == id && job.status == "complete" && !job.history_dismissed)
            .ok_or("Map publication is no longer the current verified job")?;
        if let Some(decision) = &job.decision {
            if decision.session.is_empty()
                || (decision.target.is_some()
                    && (decision.session != self.inner.session || decision.generation != generation))
                || (decision.session == self.inner.session && decision.generation != generation) {
                return Err("Map replacement authorization expired; select the map again".into());
            }
        }
        commit(job.decision.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn complete_preparation_for_test(&self, token: &str, path: &Path) -> Result<MapJob, String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let decision = state.preparation.take().filter(|decision| decision.token == token)
            .ok_or("Missing test preparation")?;
        let mut job: MapJob = serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4().to_string(), "status": "complete",
            "regionId": decision.region_id, "language": decision.language,
            "written": decision.size, "total": decision.size, "incoming": 0, "incomingTotal": 0,
            "error": null, "path": path.to_str(),
        })).map_err(|error| error.to_string())?;
        job.decision = Some(decision);
        let directory = MapDirectory::open(path.parent().ok_or("Test publication has no directory")?)
            .map_err(|error| error.to_string())?;
        let name = path.file_name().and_then(|name| name.to_str()).ok_or("Invalid test publication name")?;
        let metadata = directory.directory.symlink_metadata(name).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1 || metadata.len() != job.total {
            return Err("Test publication does not match its prepared archive".into());
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory.directory.open_with(name, &options).map_err(|error| error.to_string())?;
        let sha256 = Some(map_file_sha256(&file.into_std(), job.total)?);
        job.published = Some(PublishedMap {
            record: OwnedPartial {
                destination: directory.path, directory_identity: directory.identity,
                name: name.into(), identity: Some(FileIdentity::of(&metadata)),
                capture: None, publication: None,
            },
            size: job.total, sha256,
        });
        persist(&self.inner.state_path, &job)?;
        state.job = Some(job.clone());
        state.active = None;
        Ok(job)
    }

    pub(super) async fn catalog_controlled(&self, url: &str, control: &Control) -> Result<MapCatalog, String> {
        crate::transfer_policy::require_internet()?;
        control.check()?;
        checked_url(url)?;
        let client = self.inner.client(control).await?;
        fetch_catalog_controlled(client, url, control).await
    }

    /// Requires a Tokio runtime. The manager, not the command future, owns the job.
    // Job-admission boundary: these are the independently validated selection,
    // destination, reusable sources and request-scoped space/replacement authority.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        &self,
        catalog_url: String,
        proxy_url: String,
        region_id: String,
        language: String,
        destination: PathBuf,
        local_candidates: Vec<PathBuf>,
        space_acknowledgement: Option<String>,
        decision: Option<MapDecision>,
    ) -> Result<MapJob, String> {
        crate::transfer_policy::require_internet()?;
        checked_url(&catalog_url)?;
        checked_url(&proxy_url)?;
        if !destination.is_absolute() {
            return Err("Map destination must be an absolute directory".into());
        }
        // The acknowledgement is request-scoped, not tied to a fresh job UUID.
        // A retry must select the same catalog, language, destination and sources.
        let space_identity = hex::encode(Sha256::digest(
            serde_json::to_vec(&(
                &catalog_url,
                &proxy_url,
                &region_id,
                &language,
                &destination,
                &local_candidates,
            ))
            .map_err(|e| e.to_string())?,
        ));
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "Map manager requires a Tokio runtime")?;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "Map state lock poisoned")?;
        if state.active.is_some() {
            return Err("A map is already being composed".into());
        }
        if state.cache_reserved {
            return Err("The map cache is being cleared; try again".into());
        }
        if let Some(previous) = &state.job {
            if previous.resume.is_some()
                || ((previous.published.is_some()
                    || (!previous.history_dismissed && previous.path.is_some()))
                    && !self.inner.store.acquisition_consumed(&format!("map:{}", previous.id))?) {
                return Err("Resume, finish importing or delete the previous map download before starting another map".into());
            }
        }
        // Carry the previous visible output into the same durable replacement
        // checkpoint. A completed composition may still await its verified import
        // receipt, and discovery must never launder it into a manual import.
        let mut monitored_paths = state.job.as_ref()
            .map(|previous| previous.monitored_paths.clone()).unwrap_or_default();
        if let Some(path) = state.job.as_ref().and_then(|previous| previous.path.as_ref()) {
            let path = PathBuf::from(path);
            if !monitored_paths.contains(&path) {
                monitored_paths.push(path);
            }
        }
        let mut retained_publications = state.job.as_ref()
            .map(|previous| previous.retained_publications.clone()).unwrap_or_default();
        if let Some(previous) = &state.job {
            if let Some(published) = &previous.published {
                retained_publications.push(RetainedMap {
                    id: previous.id.clone(), region_id: previous.region_id.clone(),
                    language: previous.language.clone(), published: published.clone(),
                });
            }
        }
        let mut recovery_records = state
            .job
            .as_ref()
            .map(|job| job.recovery_records.clone())
            .unwrap_or_default();
        // The unresolved record moved out of the previous job below. If the
        // new job cannot be persisted it must go back: otherwise the live
        // monitored-path claim and its warning vanish, and the next persist of
        // the previous job (cancel, recovery, a retried start) makes the loss
        // durable.
        let mut moved_partial = None;
        if let Some(job) = state.job.as_mut() {
            if let Some(mut record) = job.owned_partial.take() {
                if job.resume.is_some() {
                    // A retained prefix is not disposable cache. List-only
                    // dismissal can preserve it, never silently reclaim it.
                    moved_partial = Some(record.clone());
                    recovery_records.push(record);
                } else {
                    let recovery = reclaim_partial(&mut record, |record| {
                        job.owned_partial = Some(record.clone());
                        persist(&self.inner.state_path, job)
                    });
                    if !matches!(recovery, Ok(PartialRecovery::Reclaimed)) {
                        // Move, never discard, unresolved ownership into the next
                        // durable status. It must not veto another region's job.
                        moved_partial = Some(record.clone());
                        recovery_records.push(record);
                    }
                    // Whatever was preserved now lives in `recovery_records`;
                    // the new job must not inherit an owned-partial reservation.
                    job.owned_partial = None;
                }
            }
        }
        let mut job = MapJob {
            id: Uuid::new_v4().to_string(),
            status: "queued".into(),
            created_at: Some(crate::store::now()),
            region_id,
            language,
            written: 0,
            total: 0,
            incoming: 0,
            incoming_total: 0,
            error: None,
            path: None,
            blocks_total: None, blocks_ready: None, blocks_downloaded: None,
            decision,
            resumable: false, resume: None, published: None,
            can_remove_from_list: false,
            history_dismissed: false,
            owned_partial: None,
            recovery_records,
            monitored_paths,
            retained_publications,
        };
        job.append_recovery_warning();
        if let Err(error) = persist(&self.inner.state_path, &job) {
            if let (Some(previous), Some(record)) = (state.job.as_mut(), moved_partial) {
                previous.owned_partial = Some(record);
            }
            return Err(error);
        }
        let mut acknowledgements: MapSpaceAcknowledgements = space_acknowledgement
            .as_deref()
            .filter(|supplied| {
                state.job.as_ref().is_some_and(|previous| {
                    previous.status == "failed"
                        && previous.error.as_deref()
                            .and_then(|error| error.strip_prefix("LOW_DISK_SPACE:"))
                            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                            .is_some_and(|warning| warning["acknowledgement"].as_str() == Some(*supplied))
                })
            })
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default();
        // Only the currently retained warning can continue its challenge. Once
        // admitted, completed, cancelled or replaced, its token cannot be reused.
        let attempt = acknowledgements.attempt.get_or_insert_with(|| job.id.clone());
        let mut control = Control::new();
        control.space_identity = Some(Arc::new(format!("{space_identity}:{attempt}")));
        control.space_acknowledgements = Arc::new(acknowledgements);
        let control = Arc::new(control);
        state.job = Some(job.clone());
        state.admission = state.admission.wrapping_add(1);
        state.active = Some(control.clone());
        *self.inner.pause_control.lock().map_err(|_| "Map pause signal unavailable")? = Some(control.clone());
        state.retirement_error = None;
        self.spawn_run(&runtime, job.id.clone(), control, catalog_url, proxy_url, destination, local_candidates);
        Ok(job)
    }

    // Ownership boundary into the detached worker: retain the explicitly admitted
    // runtime, job identity/control and immutable network/filesystem inputs.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn_run(
        &self, runtime: &tokio::runtime::Handle, id: String, control: Arc<Control>,
        catalog_url: String, proxy_url: String, destination: PathBuf, candidates: Vec<PathBuf>,
    ) {
        let manager = self.clone();
        runtime.spawn(async move {
            let result = std::panic::AssertUnwindSafe(async {
                #[cfg(test)]
                {
                    let hook = manager.inner.before_worker.lock().unwrap().take();
                    if let Some(hook) = hook { hook(); }
                }
                let run = manager.run(&id, &control, &catalog_url, &proxy_url, &destination, candidates);
                tokio::pin!(run);
                tokio::select! {
                    biased;
                    _ = crate::transfer_policy::internet_denied() => { control.request_pause(); run.await }
                    result = &mut run => result,
                }
            }).catch_unwind().await.unwrap_or_else(|_| {
                let _ = control.retirement.compare_exchange(1, 0,
                    std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
                Err("Map worker stopped unexpectedly; its last durable checkpoint was retained".into())
            });
            manager.finish(&id, &control, result);
        });
    }

    #[cfg(any(target_os = "ios", test))]
    pub(crate) fn has_active_work(&self) -> Result<bool, String> {
        Ok(self.inner.active_work.load(std::sync::atomic::Ordering::Acquire))
    }

    #[cfg(any(mobile, test))]
    pub(crate) fn request_pause(&self) -> Result<(), String> {
        if let Some(control) = self.inner.pause_control.lock().map_err(|_| "Map pause signal unavailable")?.clone() {
            control.request_pause();
        }
        Ok(())
    }

    pub(crate) async fn pause(&self) -> Result<(), String> {
        let control = self.inner.pause_control.lock().map_err(|_| "Map pause signal unavailable")?.clone();
        if let Some(control) = control {
            let mut done = control.done.subscribe();
            control.request_pause();
            let result = done.wait_for(Option::is_some).await.map_err(|_| "Map pause worker stopped")?.clone();
            result.ok_or("Map pause omitted its durable result")?
        } else {
            let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            if state.retirement_error.is_some() {
                if let Some(job) = &state.job { persist(&self.inner.state_path, job)?; }
                state.retirement_error = None;
            }
            Ok(())
        }
    }

    pub(crate) async fn pause_job(&self, id: &str) -> Result<(), String> {
        let control = {
            let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            if state.job.as_ref().is_none_or(|job| job.id != id) { return Err("Unknown map download".into()); }
            state.active.clone()
        };
        if let Some(control) = control {
            let mut done = control.done.subscribe();
            control.request_pause();
            let result = done.wait_for(Option::is_some).await.map_err(|_| "Map pause worker stopped")?.clone();
            result.ok_or("Map pause omitted its durable result")?
        } else { Ok(()) }
    }

    pub(crate) fn resume(&self, id: &str, generation: u64, acknowledgement: Option<String>) -> Result<MapJob, String> {
        crate::transfer_policy::require_internet()?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| "Map resume requires a Tokio runtime")?;
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        if state.active.is_some() { return Err("A map is already being composed".into()); }
        let mut job = state.job.as_ref().filter(|job| job.id == id && !job.history_dismissed)
            .cloned().ok_or("Unknown map download")?;
        if !matches!(job.status.as_str(), "paused" | "failed" | "cancelled") || !job.resumable {
            return Err("This map has no retained output checkpoint to resume; select Get map instead".into());
        }
        let checkpoint = job.resume.as_ref().ok_or("Map checkpoint is missing")?;
        let record = job.owned_partial.as_ref().ok_or("Map checkpoint ownership is missing")?;
        open_map_checkpoint(record, checkpoint.written)?;
        let decision = job.decision.as_mut().ok_or("This older map has no resumable recipe authorization")?;
        if decision.session.is_empty()
            || (decision.session == self.inner.session && decision.generation != generation) {
            return Err("Map choice expired after cancellation or a privacy transition; select the map again".into());
        }
        decision.validate(&self.inner.store.read()?)?;
        // Explicit Resume can renew only this original snapshot after restart;
        // it cannot adopt another installation, recipe or destination.
        decision.session.clone_from(&self.inner.session);
        decision.generation = generation;
        let catalog = decision.catalog_url.clone();
        let proxy = decision.proxy_url.clone();
        let destination = PathBuf::from(&decision.destination);
        let candidates = self.inner.store.read()?.books.iter().filter(|book| book.available)
            .map(|book| PathBuf::from(&book.path)).collect();
        let mut control = Control::new();
        control.space_identity = Some(Arc::new(format!("map-resume:{}:{}", job.id, checkpoint.recipe)));
        control.space_acknowledgements = Arc::new(acknowledgement.as_deref().filter(|supplied|
            job.error.as_deref().and_then(|error| error.strip_prefix("LOW_DISK_SPACE:"))
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .is_some_and(|warning| warning["acknowledgement"].as_str() == Some(*supplied)))
            .and_then(|value| serde_json::from_str(value).ok()).unwrap_or_default());
        let control = Arc::new(control);
        job.status = "queued".into();
        job.error = None;
        job.incoming = 0;
        job.incoming_total = 0;
        persist(&self.inner.state_path, &job)?;
        state.job = Some(job.clone());
        state.admission = state.admission.wrapping_add(1);
        state.active = Some(control.clone());
        *self.inner.pause_control.lock().map_err(|_| "Map pause signal unavailable")? = Some(control.clone());
        state.retirement_error = None;
        self.spawn_run(&runtime, id.into(), control, catalog, proxy, destination, candidates);
        Ok(job)
    }

    pub fn status(&self) -> Result<Option<MapJob>, String> {
        let (mut job, active) = {
            let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            (state.job.clone().filter(|job| !job.history_dismissed), state.active.is_some())
        };
        if let Some(job) = &mut job {
            job.can_remove_from_list = !active
                && matches!(job.status.as_str(), "complete" | "failed" | "cancelled" | "paused")
                && job.owned_partial.is_none() && job.resume.is_none() && job.recovery_records.is_empty()
                && (job.path.is_none() || self.inner.store.acquisition_consumed(&format!("map:{}", job.id))?);
        }
        Ok(job)
    }

    pub async fn cancel(&self) -> Result<(), String> {
        let (active, admission) = {
            let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            // Retirement joins the publication linearization point before
            // deciding whether an unpublished choice should be revoked.
            state.preparation = None;
            if state.active.is_none() && state.retirement_error.is_some() {
                // A previous worker already stopped and retained its exact terminal job.
                // Retry only that failed durable checkpoint, never networking or composition.
                let job = state.job.as_ref().ok_or("Missing retained terminal map job")?;
                persist(&self.inner.state_path, job)
                    .map_err(|error| format!("Cannot save map job state: {error}"))?;
                state.retirement_error = None;
            }
            (state.active.clone(), state.admission)
        };
        let retirement = if let Some(control) = active {
            let mut done = control.done.subscribe();
            control.retirement.store(2, std::sync::atomic::Ordering::SeqCst);
            control.cancel.send_replace(true);
            let result = done.wait_for(Option::is_some)
                .await
                .map_err(|_| "Map worker ended unexpectedly")?
                .clone();
            result.ok_or_else(|| "Map worker omitted its retirement result".to_string())?
        } else {
            Ok(())
        };
        retirement?;
        self.recover_terminal_publication_for(Some(admission))?;
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        // The lock was released while the worker retired. A resume/start
        // admitted meanwhile (desktop takes only shared admission locks) is a
        // newer intent: never revoke its authorization or reclaim the partial
        // file its live worker is writing.
        if state.active.is_some() || state.admission != admission {
            return Ok(());
        }
        if let Some(job) = state.job.as_mut() {
            if job.status != "complete" {
                if let Some(decision) = &mut job.decision { decision.session.clear(); }
                self.reclaim_job_partial(job)?;
                if job.status == "paused" { job.status = "cancelled".into(); }
                persist(&self.inner.state_path, job)?;
            }
        }
        Ok(())
    }

    pub(super) fn reclaim_job_partial(&self, job: &mut MapJob) -> Result<(), String> {
        let Some(mut record) = job.owned_partial.clone() else { return Ok(()) };
        let mut paths = vec![record.destination.join(&record.name)];
        if let Some(publication) = &record.publication { paths.push(record.destination.join(&publication.name)); }
        let recovery = self.inner.store.with_acquisition_destinations(&paths, || {
            reclaim_partial(&mut record, |updated| {
                job.owned_partial = Some(updated.clone());
                persist(&self.inner.state_path, job)
            })
        })?;
        match recovery {
            PartialRecovery::Reclaimed => { job.owned_partial = None; }
            PartialRecovery::Published(path) => {
                let publication = record.publication.take().ok_or("Recovered map lost its publication stamp")?;
                record.name = publication.name;
                job.published = Some(PublishedMap { record, size: publication.size, sha256: publication.sha256 });
                job.owned_partial = None;
                job.path = Some(path.to_string_lossy().into_owned());
                if job.status != "cancelled" { job.status = "complete".into(); }
            }
            PartialRecovery::Preserved => return Err("Map file ownership changed; no bytes were deleted".into()),
        }
        job.resume = None;
        job.resumable = false;
        persist(&self.inner.state_path, job)
    }

    /// Retained predecessors (and a dismissed current output) are cleanup-only.
    /// Listing never reopens a file, imports it, or infers ownership from a path.
    pub(crate) fn retained_jobs(&self) -> Result<Vec<RetainedMapJob>, String> {
        let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let Some(job) = &state.job else { return Ok(Vec::new()) };
        let row = |id: &str, region: &str, language: &str, published: &PublishedMap| {
            let destination = published.record.destination.join(&published.record.name)
                .to_string_lossy().into_owned();
            RetainedMapJob {
                id: id.into(), region_id: region.into(), language: language.into(),
                url: "", file_name: published.record.name.clone(), downloaded: published.size,
                error: None, speed: 0, eta_seconds: None,
                status: "complete", completed_paths: vec![destination.clone()], destination,
                total: published.size, written: published.size, transport: "map",
                // Cleanup-only rows: hiding one would strand its file with no
                // remaining surface to delete it.
                can_remove_from_list: false,
            }
        };
        let mut rows: Vec<_> = job.retained_publications.iter()
            .map(|saved| row(&saved.id, &saved.region_id, &saved.language, &saved.published)).collect();
        if job.history_dismissed {
            if let Some(published) = &job.published {
                rows.push(row(&job.id, &job.region_id, &job.language, published));
            }
        }
        Ok(rows)
    }

    /// Read-only ownership evidence for a download cleanup confirmation. This
    /// never grants deletion authority to an ambient path or a later inode.
    pub(crate) fn terminal_owned_publications(&self, id: &str) -> Result<Vec<(PathBuf, crate::torrent::CleanupEntry)>, String> {
        let (published, expected_status) = {
            let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            if state.active.is_some() { return Err("Pause the map before preparing file cleanup".into()); }
            let job = state.job.as_ref().ok_or("Unknown map download")?;
            if job.id != id && job.publication(id).is_none() { return Err("Unknown map download".into()); }
            if job.id == id && job.published.is_none() && job.path.is_some() {
                return Err("This older map cannot be removed from Kiwix. Remove the map from your library instead".into());
            }
            (job.publication(id).cloned(), job.status.clone())
        };
        let Some(published) = published else {
            return Ok(Vec::new());
        };
        let record = &published.record;
        if record.capture.is_some() { return Ok(Vec::new()); }
        let name = Path::new(&record.name);
        if name.file_name() != Some(name.as_os_str())
            || !matches!(name.components().next(), Some(std::path::Component::Normal(_))) {
            return Err("Map publication ownership has an invalid filename".into());
        }
        let directory = MapDirectory::open(&record.destination).map_err(|error| error.to_string())?;
        if directory.identity != record.directory_identity {
            return Err("The original map directory changed; no files were authorized for deletion".into());
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = match directory.directory.open_with(&record.name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != published.size
            || record.identity.as_ref() != Some(&FileIdentity::of(&metadata)) {
            return Err("The original downloaded map file changed; no files were authorized for deletion".into());
        }
        let verified = crate::publication::VerifiedFile::new(file.into_std(), published.size)
            .map_err(|error| error.to_string())?;
        let expected = published.sha256.ok_or("This older map cannot be checked for damage. Remove the map from your library instead")?;
        if map_file_sha256(verified.file(), published.size)? != expected {
            return Err("The original downloaded map contents changed; no deletion was authorized".into());
        }
        if !verified.matches_in(&directory.directory, name).map_err(|error| error.to_string())? {
            return Err("The original downloaded map file changed while preparing cleanup".into());
        }
        let path = directory.path.join(&record.name);
        let proof = crate::torrent::CleanupEntry::retain(&path, verified.file())?
            .with_publication(published.size, published.sha256);
        directory.unchanged().map_err(|error| error.to_string())?;
        let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        if state.active.is_some() || state.job.as_ref().is_none_or(|job|
            job.status != expected_status || job.publication(id) != Some(&published)) {
            return Err("Map download changed while preparing cleanup; no deletion was authorized".into());
        }
        Ok(vec![(path, proof)])
    }

    pub(crate) fn validate_terminal_owned_paths(&self, id: &str) -> Result<Vec<PathBuf>, String> {
        Ok(self.terminal_owned_publications(id)?.into_iter().map(|(path, _)| path).collect())
    }

    pub(crate) fn delete_download(&self, id: &str) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        if state.active.is_some() { return Err("Pause the map before deleting its downloaded files".into()); }
        let job = state.job.as_mut().ok_or("Unknown map download")?;
        if job.id != id {
            let index = job.retained_publications.iter().position(|saved| saved.id == id)
                .ok_or("Unknown map download")?;
            let mut published = job.retained_publications[index].published.clone();
            let (size, sha256) = (published.size, published.sha256);
            reclaim_published_map(&mut published, self.inner.store.as_ref(), |updated| {
                job.retained_publications[index].published = PublishedMap { record: updated.clone(), size, sha256 };
                persist(&self.inner.state_path, job)
            })?;
            let mut checkpoint = job.clone();
            checkpoint.retained_publications.remove(index);
            persist(&self.inner.state_path, &checkpoint)?;
            *job = checkpoint;
            return Ok(());
        }
        // Retire publication/import authority before unlinking. This lock also
        // serializes any already-ready receipt trying to install the same file.
        job.status = "cancelled".into();
        job.resumable = false;
        if let Some(decision) = &mut job.decision { decision.session.clear(); }
        persist(&self.inner.state_path, job)?;
        self.reclaim_job_partial(job)?;
        if let Some(mut published) = job.published.clone() {
            let (size, sha256) = (published.size, published.sha256);
            reclaim_published_map(&mut published, self.inner.store.as_ref(), |updated| {
                job.published = Some(PublishedMap { record: updated.clone(), size, sha256 });
                persist(&self.inner.state_path, job)
            })?;
            job.published = None;
        } else if let Some(path) = &job.path {
            self.inner.store.with_acquisition_destination(Path::new(path), || {
                match fs::symlink_metadata(path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                    Ok(_) => Err("This older map has no owned publication stamp; remove the installed archive with its protected file action first".into()),
                }
            })?;
        }
        // Record proven absence before reporting unrelated historical recovery
        // warnings; restart must not resurrect an already-reclaimed publication.
        job.path = None;
        persist(&self.inner.state_path, job)?;
        if !job.recovery_records.is_empty() {
            return Err("Map recovery warnings remain unresolved; this download stays visible and no unproven files were deleted".into());
        }
        // Keep exclusion/recovery evidence; never clear the shared source cache.
        job.history_dismissed = true;
        persist(&self.inner.state_path, job)?;
        state.retirement_error = None;
        Ok(())
    }

    /// Dismiss the exact terminal job, not reusable parts or archive files.
    /// Keep the native journal: unresolved publication ownership and monitored
    /// path exclusions must survive both dismissal and the next map download.
    pub fn remove_terminal(&self, id: &str) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let job = state.job.as_ref().ok_or("Unknown map job")?;
        if job.id != id {
            return Err("Delete the retained map files instead of only removing them from the list".into());
        }
        if state.active.is_some() || !matches!(job.status.as_str(), "complete" | "failed" | "cancelled" | "paused") {
            return Err("Only finished map downloads can be removed from history".into());
        }
        if job.owned_partial.is_some() || job.resume.is_some() || !job.recovery_records.is_empty()
            || (job.path.is_some() && !self.inner.store.acquisition_consumed(&format!("map:{}", job.id))?) {
            return Err("Resume or delete retained map data first.".into());
        }
        let mut checkpoint = job.clone();
        checkpoint.history_dismissed = true;
        persist(&self.inner.state_path, &checkpoint)?;
        state.job = Some(checkpoint);
        state.retirement_error = None;
        Ok(())
    }

    /// Forget one preserved recovery entry without touching its bytes.
    /// Mirrors `forget_download_record`: the warning disappears, the files and
    /// every ownership exclusion stay exactly as they are.
    pub fn forget_recovery(&self, path: &Path) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let mut job = state.job.clone().ok_or("Unknown map job")?;
        let index = job
            .recovery_records
            .iter()
            .position(|record| {
                let name = record
                    .publication
                    .as_ref()
                    .map_or(record.name.as_str(), |saved| saved.name.as_str());
                record.destination.join(name) == path
            })
            .ok_or("This map recovery entry is no longer in the current download. Refresh and try again")?;
        job.recovery_records.remove(index);
        job.clear_recovery_warning();
        job.append_recovery_warning();
        persist(&self.inner.state_path, &job)?;
        state.job = Some(job);
        Ok(())
    }

    pub fn clear_cache(&self) -> Result<(), String> {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| "Map state lock poisoned")?;
            if state.active.is_some() {
                return Err("Cancel the active map before clearing downloaded parts".into());
            }
            if state.cache_reserved {
                return Err("The map cache is already being cleared".into());
            }
            // Reserve the private parts directory under the state lock, then
            // release it: removal can take long enough to block every other
            // state-taking map command, and only job admission needs the
            // reservation (no worker runs while `active` is empty).
            state.cache_reserved = true;
        }
        let result = (|| {
            // Only this private cache is removed, never completed files or candidates.
            if self.inner.cache.exists() {
                fs::remove_dir_all(&self.inner.cache).map_err(|e| e.to_string())?;
            }
            fs::create_dir_all(&self.inner.cache).map_err(|e| e.to_string())
        })();
        // Commit by releasing the reservation even if the lock was poisoned,
        // so a failed clear cannot wedge every later map start.
        let mut state = self.inner.state.lock().unwrap_or_else(|error| error.into_inner());
        state.cache_reserved = false;
        drop(state);
        result
    }

    pub(super) fn update(&self, id: &str, change: impl FnOnce(&mut MapJob)) {
        if let Ok(mut state) = self.inner.state.lock() {
            if state.active.is_some() {
                if let Some(job) = state.job.as_mut().filter(|job| job.id == id) {
                    change(job);
                }
            }
        }
    }

    pub(super) fn checkpoint_partial(&self, id: &str, partial: Option<OwnedPartial>) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "Map state lock poisoned")?;
        let job = state
            .job
            .as_mut()
            .filter(|job| job.id == id)
            .ok_or("Missing map job")?;
        let mut checkpoint = job.clone();
        checkpoint.owned_partial = partial;
        if checkpoint.owned_partial.is_none() {
            checkpoint.resume = None;
            checkpoint.resumable = false;
        }
        persist(&self.inner.state_path, &checkpoint)?;
        *job = checkpoint;
        Ok(())
    }

    pub(super) fn checkpoint_resume(&self, id: &str, resume: MapResume) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let job = state.job.as_mut().filter(|job| job.id == id).ok_or("Missing map job")?;
        if job.owned_partial.as_ref().is_none_or(|record| record.identity.is_none()) {
            return Err("Map prefix checkpoint has no owned output".into());
        }
        let mut checkpoint = job.clone();
        checkpoint.written = resume.written;
        checkpoint.resume = Some(resume);
        checkpoint.resumable = true;
        persist(&self.inner.state_path, &checkpoint)?;
        *job = checkpoint;
        Ok(())
    }

    pub(super) fn invalidate_resume(&self, id: &str) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let job = state.job.as_mut().filter(|job| job.id == id).ok_or("Missing map job")?;
        if let Some(resume) = &mut job.resume { resume.invalid = true; }
        job.resumable = false;
        persist(&self.inner.state_path, job)
    }

    pub(crate) fn recover_terminal_publication(&self) -> Result<(), String> {
        self.recover_terminal_publication_for(None)
    }

    fn recover_terminal_publication_for(&self, expected: Option<u64>) -> Result<(), String> {
        let snapshot = {
            let state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            if state.active.is_some() || expected.is_some_and(|id| state.admission != id) { return Ok(()); }
            state.job.as_ref().and_then(|job| {
                let resume = job.resume.as_ref().filter(|resume| !resume.invalid)?;
                let record = job.owned_partial.as_ref().filter(|record| record.publication.is_some() && record.capture.is_none())?;
                Some((state.admission, job.id.clone(), record.clone(), resume.written))
            })
        };
        let Some((admission, id, original, written)) = snapshot else { return Ok(()) };
        let mut recovered = original.clone();
        let result = recover_resume_publication(&mut recovered, written);
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        if state.active.is_some() || state.admission != admission { return Ok(()); }
        let Some(job) = state.job.as_mut().filter(|job| job.id == id
            && job.owned_partial.as_ref() == Some(&original)
            && job.resume.as_ref().is_some_and(|resume| resume.written == written)) else { return Ok(()) };
        match result {
            Ok(Some(published)) => {
                job.path = Some(published.record.destination.join(&published.record.name).to_string_lossy().into_owned());
                job.published = Some(published);
                job.owned_partial = None;
                job.resume = None;
                job.resumable = false;
                job.written = job.total;
                job.status = "complete".into();
                job.error = None;
            }
            Ok(None) => { job.owned_partial = Some(recovered); job.resumable = true; }
            Err(error) => { job.resumable = false; job.error = Some(error); }
        }
        persist(&self.inner.state_path, job)?;
        state.retirement_error = None;
        Ok(())
    }

    pub(super) async fn checkpoint_output(
        &self, id: &str, output: &tokio::fs::File, recipe: &str, written: u64,
        prefix: &Sha256, blocks: &BlockProgress<'_>,
    ) -> Result<(), String> {
        output.sync_all().await.map_err(|error| format!("Cannot sync map prefix: {error}"))?;
        self.checkpoint_resume(id, MapResume {
            recipe: recipe.into(), written,
            prefix_sha256: hex::encode(prefix.clone().finalize()),
            blocks: blocks.states.clone(),
            invalid: false,
        })
    }

    pub(super) fn complete_publication(&self, id: &str, path: &Path) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
        let job = state.job.as_mut().filter(|job| job.id == id).ok_or("Missing map job")?;
        let mut checkpoint = job.clone();
        checkpoint.retain_current_path();
        checkpoint.status = "complete".into();
        checkpoint.path = Some(path.to_string_lossy().into_owned());
        checkpoint.written = checkpoint.total;
        checkpoint.incoming = 0;
        checkpoint.incoming_total = 0;
        checkpoint.error = None;
        if let Some(mut record) = checkpoint.owned_partial.clone() {
            if let Some(publication) = record.publication.take() {
                record.name = publication.name;
                checkpoint.published = Some(PublishedMap { record, size: publication.size, sha256: publication.sha256 });
            }
        }
        checkpoint.resume = None;
        checkpoint.resumable = false;
        checkpoint.owned_partial = None;
        checkpoint.append_recovery_warning();
        // Keep both the in-memory and persisted recovery evidence until this
        // single completion checkpoint has succeeded.
        persist(&self.inner.state_path, &checkpoint)?;
        *job = checkpoint;
        Ok(())
    }

    pub(super) fn finish(&self, id: &str, control: &Control, result: Result<PathBuf, String>) {
        let retirement = (|| {
            let mut state = self.inner.state.lock().map_err(|_| "Map state lock poisoned")?;
            let job = state.job.as_mut().filter(|job| job.id == id).ok_or("Missing map job")?;
            // Successful publication already committed completion durably.
            let retirement = if let Err(error) = result {
                let paused = error == CANCELLED && control.is_pausing() && job.resumable;
                job.status = if paused { "paused" }
                    else if error == CANCELLED { "cancelled" } else { "failed" }.into();
                job.error = if paused { None } else { Some(error) };
                job.retain_current_path();
                job.append_recovery_warning();
                job.incoming = 0;
                job.incoming_total = 0;
                if let Some(resume) = &job.resume {
                    job.written = resume.written;
                    job.blocks_ready = Some(resume.blocks.iter().filter(|state| **state & 1 != 0).count() as u64);
                    job.blocks_downloaded = Some(resume.blocks.iter().filter(|state| **state & 2 != 0).count() as u64);
                }
                persist(&self.inner.state_path, job).map_err(|error| {
                    let error = format!(
                        "{}Cannot save map job state: {error}",
                        job.error.as_ref().map(|s| format!("{s}; ")).unwrap_or_default()
                    );
                    job.error = Some(error.clone());
                    error
                })
            } else {
                Ok(())
            };
            state.retirement_error = retirement.as_ref().err().cloned();
            self.inner.active_work.store(false, std::sync::atomic::Ordering::Release);
            if let Ok(mut signal) = self.inner.pause_control.lock() {
                if signal.as_ref().is_some_and(|pending| std::ptr::eq(pending.as_ref(), control)) {
                    *signal = None;
                }
            }
            state.active = None;
            retirement
        })();
        control.done.send_replace(Some(retirement));
    }

    pub(super) async fn run(
        &self,
        id: &str,
        control: &Control,
        catalog_url: &str,
        proxy_url: &str,
        destination: &Path,
        candidates: Vec<PathBuf>,
    ) -> Result<PathBuf, String> {
        control.check()?;
        let _activity = MapWorkActivity::new(&self.inner.active_work);
        self.update(id, |j| j.status = "downloading".into());
        let catalog = self.catalog_controlled(catalog_url, control).await?;
        let job = self.status()?.ok_or("Missing map job")?;
        let region = catalog
            .regions
            .iter()
            .find(|r| r.id == job.region_id)
            .ok_or("Selected map region is not in the catalog")?;
        let version = region
            .menu
            .iter()
            .find(|v| v.lang == job.language)
            .ok_or("Selected map language is not offered")?;
        let index = checked_url(catalog_url)?;
        let proxy = checked_url(proxy_url)?;
        let recipe_source = recipe_url(&index, &version.recipe)?;
        let client = self.inner.client(control).await?;
        let recipe: Recipe = fetch_json(
            client,
            recipe_source,
            control,
        )
        .await?;
        if !recipe.matches_catalog(region, version) {
            return Err("Recipe does not match the selected catalog version".into());
        }
        if let Some(decision) = &job.decision {
            decision.validate(&self.inner.store.read()?)?;
            if decision.name != recipe.name || decision.md5 != recipe.md5
                || decision.uuid != recipe.uuid || decision.size != recipe.size {
                return Err("Map version changed; select the map again before downloading".into());
            }
        }
        self.compose(
            id,
            control,
            recipe,
            region.base.url.as_deref().unwrap_or(&region.base.download),
            &proxy,
            destination,
            candidates,
        )
        .await
    }

    // Both network jobs and offline regression fixtures enter this complete production
    // pipeline: recipe validation, verified source reuse, composition and publication.
    // Complete composition boundary shared with offline/evaluation fixtures.
    // Keep recipe/source identity, output capability and cancellation explicit.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn compose(
        &self,
        id: &str,
        control: &Control,
        recipe: Recipe,
        base_url: &str,
        proxy: &Url,
        destination: &Path,
        candidates: Vec<PathBuf>,
    ) -> Result<PathBuf, String> {
        self.update(id, |j| {
            j.status = "validating".into();
            j.total = recipe.size;
        });
        let validation_control = control.clone();
        let fingerprint_base = base_url.to_owned();
        let fingerprint_proxy = proxy.clone();
        let (recipe, masks, fingerprint) = tokio::task::spawn_blocking(move || {
            let masks = recipe.validate(&validation_control)?;
            let fingerprint = recipe.fingerprint(&fingerprint_base, &fingerprint_proxy)?;
            Ok::<_, String>((recipe, masks, fingerprint))
        })
        .await
        .map_err(|e| format!("Recipe validation worker failed: {e}"))??;
        let saved = self.status()?.filter(|job| job.id == id).ok_or("Missing map job")?;
        let resume = saved.resume.clone();
        let resume_written = resume.as_ref().map_or(0, |resume| resume.written);
        if resume.as_ref().is_some_and(|resume|
            resume.invalid || resume.recipe != fingerprint || resume.written > recipe.size - 16) {
            return Err("Map recipe or source version changed; the retained prefix was not modified".into());
        }
        let mut blocks = BlockProgress::new(&recipe, control)?;
        if let Some(resume) = &resume {
            blocks.restore(&resume.blocks)?;
            if resume.written != 0 && blocks.states[0] & 1 == 0 {
                return Err("Map checkpoint lacks its verified base identity".into());
            }
        }
        self.update(id, |job| {
            job.blocks_total = Some(blocks.required.len() as u64);
            job.blocks_ready = Some(blocks.ready);
            job.blocks_downloaded = Some(blocks.downloaded);
        });
        #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
        control.evaluation_mark("recipe_validation");
        let base_url = source_url(base_url, proxy)?;
        let mut urls = HashMap::new();
        urls.insert("base".into(), base_url);
        for (key, source) in &recipe.packs {
            urls.insert(
                format!("pack:{key}"),
                source_url(source.url.as_deref().ok_or("Missing pack URL")?, proxy)?,
            );
        }
        tokio::fs::create_dir_all(destination)
            .await
            .map_err(|e| e.to_string())?;
        let mut local = Vec::new();
        for path in candidates {
            control.check()?;
            // Candidates are read-only, and are merely hints: each reused chunk must hash correctly.
            // Only their admitted identity is retained; reads open the path again
            // so dozens of candidates never pin dozens of descriptors for hours.
            if let Some(candidate) = LocalCandidate::open(path, recipe.base.size).await {
                local.push(candidate);
            }
        }
        // Resolve the selected directory once, then walk and operate through an
        // opened no-follow capability. Journal its identity, not just its path.
        let directory = Arc::new(
            MapDirectory::open(&fs::canonicalize(destination).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Cannot open map destination safely: {e}"))?);
        let inner = self.inner.clone();
        let cache_control = Control::clone(control);
        let cache = tokio::task::spawn_blocking(move || MapChunkCache::open(&inner.cache, || cache_control.check()))
            .await.map_err(|error| format!("Map cache worker failed: {error}"))??;
        cache.unchanged()?;
        directory.unchanged().map_err(|error| error.to_string())?;
        preflight(
            &recipe,
            !local.is_empty(),
            &self.inner.cache,
            &directory.path,
            control,
            recipe.size - resume_written,
            cache.bytes()?,
        )?;
        cache.unchanged()?;
        #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
        control.evaluation_mark("source_open_preflight");
        control.check()?;
        directory.unchanged().map_err(|e| e.to_string())?;
        let record = if resume.is_some() {
            let record = saved.owned_partial.ok_or("Map checkpoint ownership is missing")?;
            if record.destination != directory.path || record.directory_identity != directory.identity {
                return Err("Map checkpoint destination changed; no bytes were modified".into());
            }
            record
        } else {
            let record = OwnedPartial {
                destination: directory.path.clone(), directory_identity: directory.identity.clone(),
                name: format!(".streetzim-{}.part", Uuid::new_v4()),
                identity: None, capture: None, publication: None,
            };
            self.checkpoint_partial(id, Some(record.clone()))?;
            record
        };
        // A durable prefix survives failures. Only explicit cancellation/Delete
        // may reclaim it; write and verification handles close before cleanup.
        let mut cleanup = OwnedPartialGuard {
            manager: self, id, record, armed: resume.is_none(),
        };
        let (file, mut md5, mut prefix_sha256) = if let Some(resume) = &resume {
            let file = open_map_checkpoint(&cleanup.record, resume.written)?;
            let resume = resume.clone();
            let control = control.clone();
            let record = cleanup.record.clone();
            let store = self.inner.store.clone();
            match tokio::task::spawn_blocking(move || verify_map_prefix(file, &resume, &record, &store, &control))
                .await.map_err(|error| format!("Map prefix verification worker failed: {error}"))? {
                Ok(verified) => verified,
                Err(failure) => {
                    if failure.invalid { self.invalidate_resume(id)?; }
                    return Err(failure.message);
                }
            }
        } else {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).write(true).create_new(true).follow(FollowSymlinks::No);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            #[cfg(windows)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.share_mode(1 | 4);
            }
            let file = directory.directory.open_with(&cleanup.record.name, &options)
                .map_err(|error| format!("Cannot create map output: {error}"))?;
            crate::mobile_backup::exclude_payload(&file, &directory.path.join(&cleanup.record.name))?;
            cleanup.record.identity = Some(FileIdentity::of(&file.metadata().map_err(|error| error.to_string())?));
            self.checkpoint_partial(id, Some(cleanup.record.clone()))?;
            (file.into_std(), md5::Context::new(), Sha256::new())
        };
        let mut output = tokio::fs::File::from_std(file);
        if resume.is_none() {
            self.checkpoint_output(id, &output, &fingerprint, 0, &prefix_sha256, &blocks).await?;
        }
        cleanup.armed = false;
        let mut last_checkpoint = resume_written;
        let mut chunks = ChunkReader {
            manager: self, id, control, recipe: &recipe, masks: &masks, urls: &urls,
            local, local_verified: HashMap::new(), local_failed: HashSet::new(), last_local: None,
            destination: directory.clone(), last: None, written: resume_written,
            fallback_space_admitted: false, cache, blocks,
        };
        let composition: Result<(), CompositionFailure> = std::panic::AssertUnwindSafe(async {
            if resume_written == 0 {
                let head = chunks.get("base", 0).await?;
                let base_uuid = Uuid::parse_str(&recipe.base.uuid).map_err(|error| error.to_string())?;
                if head.len() < 24 || &head[8..24] != base_uuid.as_bytes() {
                    return Err(CompositionFailure::invalid("This map download does not match the selected map. Delete it and download the map again"));
                }
            }
            self.update(id, |job| job.status = "composing".into());
            let mut position = 0u64;
            for segment in &recipe.segments {
                control.check()?;
                let length = if segment.src == "md5" { 16 } else { segment.len.ok_or("Missing segment length")? };
                if position + length <= resume_written {
                    position += length;
                    continue;
                }
                let skip = resume_written.saturating_sub(position);
                match segment.src.as_str() {
                    "inline" => {
                        let bytes = STANDARD.decode(segment.data.as_deref().ok_or("Missing inline data")?)
                            .map_err(|error| error.to_string())?;
                        let bytes = &bytes[skip as usize..];
                        output.write_all(bytes).await.map_err(|error| format!("Map write failed: {error}"))?;
                        md5.consume(bytes);
                        prefix_sha256.update(bytes);
                        chunks.written += bytes.len() as u64;
                    }
                    "md5" => {
                        let digest = md5.clone().finalize().0;
                        if hex::encode(digest) != recipe.md5 { return Err(CompositionFailure::invalid("Composed ZIM MD5 mismatch")); }
                        md5.consume(digest);
                        if recipe.file_md5.as_ref().is_some_and(|expected|
                            hex::encode(md5.clone().finalize().0) != *expected) {
                            return Err(CompositionFailure::invalid("Composed whole-file MD5 mismatch"));
                        }
                        output.write_all(&digest).await.map_err(|error| format!("Map write failed: {error}"))?;
                        prefix_sha256.update(digest);
                        chunks.written += 16;
                    }
                    _ => {
                        let key = segment.source_key()?;
                        let source = recipe.source(&key)?;
                        let mut offset = segment.off.ok_or("Missing source offset")? + skip;
                        let end = segment.off.ok_or("Missing source offset")? + length;
                        while offset < end {
                            control.check()?;
                            let bytes = chunks.get(&key, offset / source.chunk).await?;
                            let from = (offset % source.chunk) as usize;
                            let count = ((bytes.len() - from) as u64).min(end - offset) as usize;
                            let slice = &bytes[from..from + count];
                            output.write_all(slice).await.map_err(|error| format!("Map write failed: {error}"))?;
                            md5.consume(slice);
                            prefix_sha256.update(slice);
                            offset += count as u64;
                            chunks.written += count as u64;
                            self.update(id, |job| job.written = chunks.written);
                            #[cfg(test)]
                            if control.pause_after.is_some_and(|limit| chunks.written >= limit) {
                                control.request_pause();
                            }
                            if chunks.written - last_checkpoint >= MAX_CHUNK {
                                self.checkpoint_output(id, &output, &fingerprint, chunks.written,
                                    &prefix_sha256, &chunks.blocks).await?;
                                last_checkpoint = chunks.written;
                            }
                        }
                    }
                }
                position += length;
                self.update(id, |job| job.written = chunks.written);
                #[cfg(test)]
                if control.pause_after.is_some_and(|limit| chunks.written >= limit) { control.request_pause(); }
                if chunks.written <= recipe.size - 16
                    && (chunks.written - last_checkpoint >= MAX_CHUNK || position == recipe.size - 16) {
                    self.checkpoint_output(id, &output, &fingerprint, chunks.written,
                        &prefix_sha256, &chunks.blocks).await?;
                    last_checkpoint = chunks.written;
                }
                #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
                if control.evaluation_cancel_after.is_some_and(|limit| chunks.written >= limit) {
                    control.cancel.send_replace(true);
                }
            }
            if chunks.written != recipe.size { return Err("Composed file size mismatch".into()); }
            output.sync_all().await.map_err(|error| format!("Cannot sync composed map: {error}"))?;
            Ok(())
        }).catch_unwind().await.unwrap_or_else(|_| {
            let _ = control.retirement.compare_exchange(1, 0,
                std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
            Err(CompositionFailure {
                message: "Map writer stopped unexpectedly; its last durable checkpoint was retained".into(),
                retain: true, checkpoint_current: false,
            })
        });
        // Join Tokio's outstanding write worker before checkpointing or cleanup.
        let output = output.into_std().await;
        #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
        control.evaluation_mark("assembly_sync");
        if let Err(error) = composition {
            let retain = error.retain;
            let checkpoint_current = error.checkpoint_current;
            let error = error.message;
            if !retain || (error == CANCELLED && !control.is_pausing()) {
                cleanup.armed = true;
            } else if checkpoint_current && chunks.written <= recipe.size - 16 {
                let output = tokio::task::spawn_blocking(move || {
                    output.sync_all().map_err(|error| error.to_string())?;
                    Ok::<_, String>(output)
                }).await.map_err(|error| error.to_string())??;
                self.checkpoint_resume(id, MapResume {
                    recipe: fingerprint, written: chunks.written,
                    prefix_sha256: hex::encode(prefix_sha256.finalize()),
                    blocks: chunks.blocks.states.clone(),
                    invalid: false,
                })?;
                drop(output);
            }
            return Err(error);
        }
        let output_sha256: [u8; 32] = prefix_sha256.finalize().into();
        let verified = Arc::new(
            crate::publication::VerifiedFile::new(output, recipe.size)
                .map_err(|e| format!("Cannot retain verified map identity: {e}"))?,
        );
        #[cfg(test)]
        {
            let hook = self
                .inner
                .before_publish
                .lock()
                .map_err(|_| "Map test hook poisoned")?
                .take();
            if let Some(hook) = hook {
                hook(&directory.path.join(&cleanup.record.name));
            }
        }
        control.check()?;
        // The filesystem enforces atomic no-replace publication, including exFAT.
        for suffix in 0u64.. {
            control.check()?;
            let filename = if suffix == 0 {
                format!("{}.zim", recipe.output_stem())
            } else {
                format!("{}.{}.zim", recipe.output_stem(), suffix)
            };
            directory.unchanged().map_err(|e| e.to_string())?;
            let path = directory.path.join(&filename);
            let from = PathBuf::from(&cleanup.record.name);
            let to = PathBuf::from(&filename);
            let capability = directory.directory.try_clone().map_err(|e| e.to_string())?;
            let identity = verified.clone();
            let store = self.inner.store.clone();
            let guarded_path = path.clone();
            cleanup.record.publication = Some(PublicationRecovery {
                name: filename,
                size: recipe.size,
                sha256: Some(output_sha256),
            });
            self.checkpoint_partial(id, Some(cleanup.record.clone()))?;
            #[cfg(test)]
            if let Some(hook) = self
                .inner
                .before_publish_commit
                .lock()
                .map_err(|_| "Map test hook poisoned")?
                .take()
            {
                hook(&path);
            }
            match tokio::task::spawn_blocking(move || {
                store.with_acquisition_destinations_available(&[&guarded_path], || {
                    let published = crate::publication::publish_verified_noreplace_in(
                        &capability,
                        &from,
                        &to,
                        &identity,
                    );
                    if published.is_ok() {
                        crate::mobile_backup::archive_policy(identity.file(), &guarded_path)?;
                    }
                    Ok(published)
                })
            })
            .await
            .map_err(|e| e.to_string())??
            {
                Ok(Ok(())) => {
                    // Capability publication does not prove the ambient path:
                    // its parent may have been moved/replaced after our check.
                    let ambient = MapDirectory::open(&directory.path).map_err(|e| {
                        format!(
                            "Published map directory changed; identity retained for recovery: {e}"
                        )
                    })?;
                    if ambient.identity != directory.identity
                        || !verified
                            .matches_in(
                                &ambient.directory,
                                Path::new(path.file_name().ok_or("Published map has no filename")?),
                            )
                            .map_err(|e| e.to_string())?
                    {
                        return Err(format!(
                            "Published map path changed identity; verified inode {:?} in directory {:?} is retained for recovery; no output was removed",
                            cleanup.record.identity, directory.identity
                        ));
                    }
                    ambient.unchanged().map_err(|e| format!(
                        "Published map directory moved; verified identity retained for recovery: {e}"
                    ))?;
                    // Once published, never run partial cleanup if the completion
                    // save fails: its journal is the only recovery authority.
                    cleanup.armed = false;
                    self.complete_publication(id, &path)?;
                    // No cancellation checks after a validated publication.
                    return Ok(path);
                }
                Err(_) => {
                    cleanup.record.publication = None;
                    self.checkpoint_partial(id, Some(cleanup.record.clone()))?;
                    continue;
                }
                Ok(Err(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
                    cleanup.record.publication = None;
                    self.checkpoint_partial(id, Some(cleanup.record.clone()))?;
                    continue;
                }
                Ok(Err(e)) => {
                    return Err(format!(
                        "Cannot publish verified map without overwriting: {e}"
                    ))
                }
            }
        }
        Err("No available map filename".into())
    }
}
