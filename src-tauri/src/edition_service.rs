// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
use crate::edition_updates::EditionSelection;

fn configured_sources(settings: &store::Settings) -> Vec<String> {
    let mut sources = vec![settings.catalog_url.clone()];
    for source in &settings.opds_sources {
        if !sources.contains(source) { sources.push(source.clone()); }
    }
    sources
}

/// Delayed shell reading-data writes may name a retired UUID. Never use this
/// translator for reader storage, sensors, sharing, or source admission.
pub(super) fn current_reading_reference(state: &store::State, id: &str) -> String {
    store::current_reading_reference(state, id)
}

pub(super) fn edition_choices(current: &store::State, snapshots: &[crate::catalog::CatalogSnapshot]) -> Vec<Value> {
    let mut updates = Vec::new();
    let series_entries: Vec<_> = snapshots.iter().map(|snapshot| {
        let mut index: HashMap<_, Vec<_>> = HashMap::new();
        for candidate in &snapshot.entries {
            if let Some(mut key) = crate::catalog::series_key(&candidate.entry.name, &candidate.entry.language, &candidate.entry.tags) {
                let flavour = key.flavour.take();
                index.entry(key).or_default().push((flavour, candidate));
            }
        }
        index
    }).collect();
    for book in &current.books {
        let Some(series) = crate::catalog::series_key(&book.metadata.name, &book.metadata.language, &book.metadata.tags) else { continue };
        let mut grouping = series.clone();
        grouping.flavour = None;
        for (snapshot, index) in snapshots.iter().zip(&series_entries) {
            let Some(entries) = index.get(&grouping) else { continue };
            let candidates: Vec<_> = entries.iter().filter(|(flavour, candidate)|
                !series.flavour.as_ref().zip(flavour.as_ref()).is_some_and(|(left, right)| left != right)
                    && crate::catalog::edition_is_newer(&candidate.entry.date, &book.metadata.date)).collect();
            for (flavour, candidate) in &candidates {
                let mut selected_series = grouping.clone();
                selected_series.flavour.clone_from(flavour);
                let bound = current.edition_bindings.iter().any(|binding|
                    binding.installation_id == book.installation_id && binding.matches_series(&snapshot.source_id, &selected_series));
                // Bulk updates never guess lineage. Only a bound installation with exactly one
                // newer candidate is unambiguous enough to queue without per-archive review.
                let association_required = !bound || candidates.len() != 1;
                updates.push(json!({"bookId":book.metadata.id,"installationId":book.installation_id,
                    "selection":EditionSelection { source_id:snapshot.source_id.clone(),source_url:snapshot.source_url.clone(),
                        entry_id:candidate.entry_id.clone(),catalog_generation:snapshot.generation,entry:candidate.entry.clone() },
                    "associationRequired":association_required,
                    "bulkEligible":!association_required,
                    "reason":if !bound { Some("This installation has no verified association with this catalog series. Confirm that the selected series is its successor.") }
                        else if candidates.len() != 1 { Some("This catalog offers multiple matching editions. Select the intended successor explicitly.") } else { None }
                }));
            }
        }
    }
    updates
}

impl Service {
    pub(super) async fn find_edition_updates(self: &Arc<Self>, args: &Value, label: &str) -> Result<Value, String> {
        let (generation, state) = {
            let _mode = self.privacy_gate.read().await;
            self.ensure_running()?;
            self.ensure_window_active(label)?;
            (self.ensure_generation(args)?, self.store.read()?)
        };
        let locale = args.get("locale").and_then(Value::as_str).unwrap_or(&state.settings.locale);
        let mut snapshots = Vec::new();
        let mut errors = Vec::new();
        for source in configured_sources(&state.settings) {
            match self.catalog_index.refresh(&self.catalog_cache, &source, locale,
                &self.privacy_gate, &self.privacy_generation, generation).await {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(error) => errors.push(format!("{source}: {error}")),
            }
        }
        let _mode = self.privacy_gate.read().await;
        self.ensure_generation(args)?;
        self.ensure_window_active(label)?;
        let current = self.store.read()?;
        if configured_sources(&current.settings) != configured_sources(&state.settings) {
            return Err("Configured catalogs changed; check archive editions again".into());
        }
        let updates = edition_choices(&current, &snapshots);
        Ok(json!({"updates":updates,"sources":snapshots,"errors":errors}))
    }

    pub(super) async fn edition_command(self: &Arc<Self>, command: &str, args: &Value, label: &str) -> Result<Value, String> {
        self.normal_writable()?;
        self.ensure_window_active(label)?;
        self.ensure_generation(args)?;
        let epoch = self.privacy_policy_generation.load(Ordering::SeqCst);
        match command {
            "edition_prepare" => {
                let state = self.store.read()?;
                let old = state.books.iter().find(|book| book.metadata.id == text(args,"bookId").unwrap_or("")
                    && book.installation_id == text(args,"installationId").unwrap_or(""))
                    .ok_or("The installed archive changed; select its edition again")?.clone();
                let (source_id, entry_id, revision, entry) = if let Some(intent_id) = args.get("intentId").and_then(Value::as_str) {
                    let intent = state.edition_intents.iter().find(|intent| intent.id == intent_id
                        && intent.target.installation_id == old.installation_id && intent.target.metadata.id == old.metadata.id
                        && matches!(intent.status.as_str(), "reserved" | "downloading" | "ready"))
                        .ok_or("The retained edition decision changed; check for editions again")?;
                    let (revision, entry) = self.catalog_index.lookup_current(&intent.selection.source_id, &intent.selection.entry_id)?;
                    if entry != intent.selection.entry {
                        return Err("The catalog artifact changed. The retained verified output was kept; select an edition again".into());
                    }
                    (intent.selection.source_id.clone(), intent.selection.entry_id.clone(), revision, entry)
                } else {
                    let source_id = text(args, "sourceId")?.to_owned();
                    let entry_id = text(args, "entryId")?.to_owned();
                    let revision = args.get("catalogGeneration").and_then(Value::as_u64).ok_or("Select the catalog edition again")?;
                    let entry = self.catalog_index.lookup(&source_id, &entry_id, revision)?;
                    (source_id, entry_id, revision, entry)
                };
                let source_url = configured_sources(&state.settings).into_iter().find(|source|
                    crate::catalog::source_id(source).as_deref() == Ok(source_id.as_str()))
                    .ok_or("This catalog is no longer configured")?;
                let selection = EditionSelection { source_id, source_url, entry_id, catalog_generation: revision, entry };
                let associated = args.get("associationConfirmed").and_then(Value::as_bool).unwrap_or(false);
                let service = self.clone();
                let window = label.to_owned();
                let prepared = blocking(move || service.editions.prepare(&service.store,epoch,&window,&old,selection,associated)).await?;
                if let Err(error) = self.ensure_window_active(label) {
                    self.editions.retire_window_authorizations(label);
                    return Err(error);
                }
                self.ensure_generation(args)?;
                value(prepared)
            }
            "edition_download" => {
                let token = text(args,"decisionToken")?;
                let intent = self.editions.reserve(&self.store,token,epoch,label)?;
                let manager = self.download_manager().await?;
                self.reconcile_edition_jobs(&manager).await?;
                self.ensure_window_active(label)?;
                self.ensure_generation(args)?;
                let intent = self.store.read()?.edition_intents.into_iter().find(|current| current.id == intent.id)
                    .ok_or("The edition decision is no longer available")?;
                if let Some(id) = &intent.download_id {
                    if let Some(job) = manager.list()?.into_iter().find(|job| &job.id == id) { return value(job); }
                    return Err("The edition transfer is no longer available; retained files have not been changed".into());
                }
                // Recheck the native selection at start; IPC never supplies the acquisition URL.
                let entry = self.catalog_index.lookup(&intent.selection.source_id,&intent.selection.entry_id,intent.selection.catalog_generation)?;
                if entry != intent.selection.entry { return Err("The catalog edition changed; select it again".into()); }
                let destination = PathBuf::from(&intent.destination);
                let directory = destination.parent().ok_or("Edition destination has no parent")?.to_owned();
                self.store.validate_portable_directory(&directory)?;
                let file_name = destination.file_name().and_then(|name| name.to_str()).ok_or("Invalid edition output name")?.to_owned();
                let acknowledgement = args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned);
                #[cfg(target_os="ios")]
                let job = if self.catalog_index.lookup_official(&intent.selection.source_id,&intent.selection.entry_id,intent.selection.catalog_generation)? {
                    manager.start_official_catalog(entry.url,directory,Some(file_name),Some(entry.size).filter(|size| *size>0),acknowledgement).await?
                } else { manager.start(entry.url,directory,Some(file_name),Some(entry.size).filter(|size| *size>0),acknowledgement).await? };
                #[cfg(not(target_os="ios"))]
                let job = manager.start(entry.url,directory,Some(file_name),Some(entry.size).filter(|size| *size>0),acknowledgement).await?;
                self.editions.bind_download(&self.store,&intent.id,&job)?;
                value(job)
            }
            "edition_status" => {
                let manager = self.download_manager().await?;
                self.reconcile_edition_jobs(&manager).await?;
                let imports = self.import_manager()?;
                let mut intents = Vec::new();
                for status in self.editions.status(&self.store,epoch,label)? {
                    let mut row = value(&status)?;
                    let receipt = status.intent.receipt.as_ref().or(status.intent.download_id.as_ref());
                    if let Some(error) = receipt.and_then(|receipt| imports.status(receipt)).and_then(|status| status.error) {
                        row["error"] = json!(error);
                    }
                    if let Some(error) = self.edition_errors.lock().map_err(|_| "Edition errors unavailable")?.get(&status.intent.id) {
                        row["error"] = json!(error);
                    }
                    intents.push(row);
                }
                Ok(json!({"intents":intents,"transactions":self.store.read()?.edition_transactions}))
            }
            "edition_cancel" => {
                let id = text(args,"intentId")?;
                let intent = self.store.read()?.edition_intents.into_iter().find(|intent| intent.id==id).ok_or("Unknown edition intent")?;
                if intent.status == "committed" { return Err("Use explicit rollback for a committed edition".into()); }
                // Cancellation fences replacement first. Even cancellation I/O failure cannot authorize import.
                self.editions.cancel(&self.store,id)?;
                if let Some(download) = intent.download_id {
                    let manager = self.download_manager().await?;
                    if manager.list()?.iter().any(|job| job.id == download && !matches!(job.status.as_str(), "complete" | "cancelled")) {
                        if let Err(error) = manager.cancel(&download).await {
                            if !manager.list()?.iter().any(|job| job.id == download && matches!(job.status.as_str(), "complete" | "cancelled")) {
                                return Err(error);
                            }
                        }
                    }
                }
                Ok(Value::Null)
            }
            "edition_commit" => {
                let intent_id = text(args,"intentId")?;
                let old = self.store.read()?.edition_intents.into_iter().find(|intent| intent.id==intent_id)
                    .ok_or("Unknown edition intent")?.target;
                if old.installation_id != text(args,"installationId")? { return Err("Edition installation changed; review it again".into()); }
                let _sources = self.localswarm_source_mutation(Some(&old.metadata.id)).await?;
                let sharing = self.sharing.lock().await;
                if Self::selected_share_serves(&sharing,&old.metadata.id) || self.active_torrent_share(&old.metadata.id)?.is_some() {
                    return Err("Stop sharing this archive before replacing its edition".into());
                }
                let mut readers = self.readers.lock().await;
                self.ensure_window_active(label)?;
                self.ensure_generation(args)?;
                let imports = self.import_manager()?;
                let manager = self.download_manager().await?;
                self.reconcile_edition_jobs(&manager).await?;
                let intent = self.store.read()?.edition_intents.into_iter().find(|intent| intent.id==intent_id).ok_or("Unknown edition intent")?;
                let ready = imports.ready().into_iter().find(|ready| Some(ready.receipt()) == intent.receipt.as_deref())
                    .ok_or("Wait for archive verification to finish before replacing")?;
                let selection = self.editions.selection_for_token(text(args,"decisionToken")?)?;
                if self.catalog_index.lookup(&selection.source_id,&selection.entry_id,selection.catalog_generation)? != selection.entry {
                    return Err("The catalog edition changed; review the verified output again".into());
                }
                let result = self.editions.commit(&self.store,intent_id,text(args,"decisionToken")?,epoch,label,&imports,&ready,&manager);
                // Store errors may follow a committed replacement. Fence old authority and publish the transition in either case.
                let committed = self.store.read()?.edition_transactions.into_iter().find(|transaction| transaction.intent_id==intent_id);
                if let Some(transaction) = committed {
                    let retirement = retire_readers(&mut readers,Some(&old.installation_id)).await;
                    self.forget_installation(&old.installation_id);
                    self.invalidate(&old.metadata.id);
                    self.invalidate(&transaction.replacement.metadata.id);
                    imports.ack(&ready);
                    self.reconcile_pending_edition_locations(&self.store.read()?);
                    self.notify_library_changed();
                    retirement?;
                }
                value(result?)
            }
            "edition_rollback" => {
                let service = self.clone();
                let transaction = text(args,"transactionId")?.to_owned();
                let installation = text(args,"expectedInstallationId")?.to_owned();
                let window = label.to_owned();
                let prepared = blocking(move || service.editions.prepare_rollback(&service.store,&transaction,&installation,epoch,&window)).await?;
                let old = &prepared.current;
                let _sources = self.localswarm_source_mutation(Some(&old.metadata.id)).await?;
                let sharing = self.sharing.lock().await;
                if Self::selected_share_serves(&sharing,&old.metadata.id) || self.active_torrent_share(&old.metadata.id)?.is_some() {
                    return Err("Stop sharing this archive before rolling back its edition".into());
                }
                let mut readers = self.readers.lock().await;
                self.ensure_window_active(label)?;
                self.ensure_generation(args)?;
                let result = self.editions.rollback(&self.store,&prepared,epoch,label);
                let committed = self.store.read()?.edition_transactions.into_iter().find(|transaction|
                    transaction.id==prepared.transaction.id && transaction.rolled_back_installation_id.is_some());
                if let Some(transaction) = committed {
                    let retirement = retire_readers(&mut readers,Some(&old.installation_id)).await;
                    self.forget_installation(&old.installation_id);
                    self.invalidate(&old.metadata.id);
                    self.invalidate(&transaction.original.metadata.id);
                    self.reconcile_pending_edition_locations(&self.store.read()?);
                    self.notify_library_changed();
                    retirement?;
                }
                value(result?)
            }
            _ => Err("Unknown edition operation".into()),
        }
    }

    pub(super) async fn reconcile_edition_jobs(self: &Arc<Self>, downloads: &DownloadManager) -> Result<(), String> {
        let service = self.clone();
        let downloads = downloads.clone();
        blocking(move || service.reconcile_edition_jobs_blocking(&downloads)).await
    }

    fn reconcile_edition_jobs_blocking(&self, downloads: &DownloadManager) -> Result<(), String> {
        let imports = self.import_manager()?;
        let jobs = downloads.list()?;
        let state = self.store.read()?;
        for job in &jobs {
            let Some(intent) = state.edition_intents.iter().find(|intent| intent.destination == job.destination
                && matches!(intent.status.as_str(),"reserved"|"downloading"|"ready")) else { continue };
            let id = intent.id.clone();
            if intent.download_id.is_none() { self.editions.bind_download(&self.store,&id,job)?; }
            if job.status == "complete" { self.enqueue_import(&job.id,vec![job.destination.clone()])?; }
        }
        let state = self.store.read()?;
        for ready in imports.ready() {
            let Some(intent) = state.edition_intents.iter().find(|intent|
                matches!(intent.status.as_str(),"reserved"|"downloading"|"ready")
                    && intent.download_id.as_deref()==Some(ready.receipt())) else { continue };
            if let Err(error) = self.editions.ready(&self.store,&intent.id,&ready,&imports,downloads) {
                // Keep the verified output/ticket. This row's retryable error must not hide unrelated transfers.
                self.edition_errors.lock().map_err(|_| "Edition errors unavailable")?.insert(intent.id.clone(),error);
            } else {
                self.edition_errors.lock().map_err(|_| "Edition errors unavailable")?.remove(&intent.id);
            }
        }
        Ok(())
    }

    pub(super) fn reconcile_pending_edition_locations(&self, state: &store::State) {
        if let Ok(mut locations) = self.launch_locations.lock() {
            for location in locations.iter_mut() {
                let revision = location.get("referenceRevision").and_then(Value::as_u64).unwrap_or(0);
                let mut id = location.get("bookId").and_then(Value::as_str).map(str::to_owned);
                for transition in &state.reference_transitions {
                    if transition.revision > revision && id.as_deref()==Some(&transition.from_book_id) {
                        id = Some(transition.to_book_id.clone());
                    }
                }
                if let Some(object) = location.as_object_mut() {
                    object.insert("bookId".into(),json!(id));
                    object.insert("referenceRevision".into(),json!(state.reference_revision));
                }
            }
        }
    }
}
