// SPDX-License-Identifier: GPL-3.0-or-later
//! The shell command table: `Service::dispatch_inner` routes every frontend
//! command to its handler after window, generation and policy checks.
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use super::*;

impl Service {
    pub(super) async fn dispatch_inner(self: Arc<Self>, command: &str, args: Value) -> Result<Value, String> {
        let label = args
            .get("__windowLabel")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string();
        if self.store.read_only()? {
            if matches!(command, "save_session" | "record_visit" | "record_search") {
                return Ok(Value::Null);
            }
            if matches!(
                command,
                "save_bookmark" | "remove_bookmark" | "import_bookmarks" | "clear_history" | "remove_history_entry"
            ) {
                self.writable()?;
            }
        }
        if matches!(
            command,
            "start_download"
                | "pause_download"
                | "resume_download"
                | "cancel_download"
                | "remove_download_history"
                | "delete_download_files"
                | "forget_download_record"
                | "torrent_seeding"
                | "share_torrent"
                | "torrent_job"
                | "torrent_metadata"
                | "map_start"
                | "map_cancel"
                | "map_clear_cache"
                | "map_remove_history"
                | "map_forget_recovery"
                | "sharing_start"
                | "shared_library_browse"
                | "shared_library_download"
                | "localswarm_discovery"
                | "localswarm_inspect"
                | "localswarm_download"
                | "localswarm_seed"
                | "localswarm_share_all"
                | "edition_prepare" | "edition_download" | "edition_status"
                | "edition_commit" | "edition_cancel" | "edition_rollback"
                | "edition_discard_original"
        ) {
            self.normal_writable()?;
        }
        #[cfg(test)]
        if command == "__test_blocking_mutation" {
            let (entered, release) = self
                .mutation_test_gate
                .lock()
                .unwrap()
                .take()
                .ok_or("Missing mutation test gate")?;
            let service = self.clone();
            return blocking(move || {
                let _ = entered.send(());
                release.recv().map_err(|error| error.to_string())?;
                std::fs::write(service.store.data_dir().join("owned-mutation"), b"finished")
                    .map_err(|error| error.to_string())?;
                Ok(Value::Null)
            })
            .await;
        }
        match command {
            #[cfg(desktop)]
            "update_status" | "update_check" | "update_install" => {
                if command != "update_status" {
                    self.ensure_generation(&args)?;
                }
                crate::update::command(self.clone(), command, &args).await
            }
            #[cfg(target_os = "android")]
            "library_files_relocate" => self.mobile_library_files_relocate(&args).await,
            #[cfg(mobile)]
            command if command.starts_with("mobile_") => self.mobile_command(command, args, &label).await,
            #[cfg(desktop)]
            "desktop_dialog_directory" => {
                self.ensure_generation(&args)?;
                let _window = self.active_window_guard(&label)?;
                value(crate::desktop_workflow::default_path(
                    &self.store,
                    text(&args, "kind")?,
                    args.get("suggestedName").and_then(Value::as_str),
                )?)
            }
            #[cfg(desktop)]
            "remember_desktop_dialog_directory" => {
                self.ensure_generation(&args)?;
                let _window = self.active_window_guard(&label)?;
                crate::desktop_workflow::remember_directory(
                    &self.store,
                    text(&args, "kind")?,
                    text(&args, "path")?,
                )?;
                Ok(Value::Null)
            }
            "frontend_ready" => {
                let _window = self.active_window_guard(&label)?;
                self.mark_frontend_ready(&label)?;
                Ok(json!({"ready":true}))
            }
            "snapshot" => {
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                self.queue_notification_launch(&label)?;
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                let notifications = value(self.notification_capability())?;
                #[cfg(target_os = "android")]
                let notifications = {
                    self.queue_android_notification_launch(&label).await?;
                    self.android_notification_capability().await
                };
                #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
                let notifications = json!({"available":false,"reason":"Native completion notifications are unavailable on this platform"});
                if let Err(error) = self.reconcile_completed_map().await {
                    // A damaged optional map record must not deny the library
                    // snapshot; report it and keep the rest of startup intact.
                    self.note_startup_warning(format!(
                        "Saved map download state could not be read: {error}"
                    ));
                    crate::diagnostics::record(
                        &self.store,
                        "snapshot",
                        Some("Saved map download state could not be read"),
                    );
                }
                let generation = self
                    .privacy_generation
                    .load(std::sync::atomic::Ordering::SeqCst);
                let mut state = self.store.read()?;
                self.reconcile_pending_edition_locations(&state);
                let mut active_surface = state.windows.get(&label)
                    .map(|window| window.active_surface.clone())
                    .unwrap_or_else(|| if label == "main" && state.active_tab.is_some() {
                        "reader".into()
                    } else {
                        "library".into()
                    });
                if crate::store::validate_active_surface(&active_surface).is_err() {
                    // Recoverable: open the library quietly and record it in
                    // Troubleshooting instead of a blocking migration band.
                    crate::diagnostics::record(
                        &self.store,
                        "snapshot",
                        Some("A restored window named an unknown surface"),
                    );
                    active_surface = "library".into();
                }
                if let Some(window) = state.windows.get(&label) {
                    state.tabs = window.tabs.clone();
                    state.active_tab = window.active_tab.clone();
                } else if label != "main" {
                    state.tabs.clear();
                    state.active_tab = None;
                }
                #[cfg(mobile)]
                if active_surface == "reader" {
                    let active_book = state.active_tab.as_deref()
                        .and_then(|active| state.tabs.iter().find(|tab| tab.id == active))
                        .and_then(|tab| tab.book_id.as_deref())
                        .and_then(|id| state.books.iter().find(|book| book.metadata.id == id));
                    if let Some(book) = active_book.filter(|book| self.mobile_archive_unusable(book)) {
                        // A restored tab whose archive did not survive the
                        // restore must show the library with a named notice,
                        // never an empty reader frame.
                        let title = book.metadata.title.clone();
                        crate::diagnostics::record(
                            &self.store,
                            "snapshot",
                            Some("A restored archive was unavailable; opened the library"),
                        );
                        // A provider may simply be unmounted. Its actual read
                        // returns a source-scoped retry/permission error; do not
                        // retain a Locate-only warning after that source returns.
                        if book.source_reference.is_none() {
                            self.note_startup_warning(format!(
                                "“{title}” is not available in this app’s storage. Choose Locate in the library to reconnect it."
                            ));
                        }
                        active_surface = "library".into();
                    }
                }
                let sharing = self.sharing_status().await;
                let launch: Vec<_> = self
                    .launch_locations
                    .lock()
                    .map_err(|_| "Launch queue unavailable")?
                    .iter()
                    .filter(|location| location["targetWindow"].as_str() == Some(label.as_str()))
                    .map(|location| {
                        let mut public = location.clone();
                        if let Some(object) = public.as_object_mut() {
                            object.remove("__notification");
                            object.remove("__notificationGeneration");
                        }
                        public
                    })
                    .collect();
                #[cfg(any(desktop, test))]
                let launch_search = self
                    .launch_search
                    .lock()
                    .map_err(|_| "Launch search unavailable")?
                    .clone()
                    .filter(|search| search["targetWindow"].as_str() == Some(label.as_str()));
                #[cfg(not(any(desktop, test)))]
                let launch_search: Option<Value> = None;
                let books: Vec<Value> = state.books.iter().map(Book::shell_value).collect();
                #[cfg(desktop)]
                let session_restored = self
                    .restored_windows
                    .lock()
                    .map(|windows| windows.contains(&label))
                    .unwrap_or(false);
                #[cfg(not(desktop))]
                let session_restored = false;
                Ok(
                    json!({"books":books,"bookmarks":state.bookmarks,"history":state.history,"recentSearches":state.recent_searches,"tabs":state.tabs,"activeTab":state.active_tab,"activeSurface":active_surface,"sessionRestored":session_restored,"settings":state.settings,"sharing":sharing,"launchLocations":launch,"launchSearch":launch_search,"notifications":notifications,"privacyGeneration":generation,"privacyPolicyGeneration":self.privacy_policy_generation.load(Ordering::SeqCst),"referenceRevision":state.reference_revision,"referenceTransitions":state.reference_transitions,"startupWarnings":self.startup_warnings.lock().map_err(|_| "Startup warning state unavailable")?.clone()}),
                )
            }
            #[cfg(any(desktop, test))]
            "ack_launch_search" => {
                self.ensure_window_active(&label)?;
                let id = args["id"].as_str().ok_or("Missing launch search ID")?;
                let mut pending = self
                    .launch_search
                    .lock()
                    .map_err(|_| "Launch search unavailable")?;
                // A newer request replaced this one: keep it for the next snapshot.
                if pending.as_ref().is_some_and(|search| {
                    search["launchId"].as_str() == Some(id)
                        && search["targetWindow"].as_str() == Some(label.as_str())
                }) {
                    *pending = None;
                }
                Ok(json!({}))
            }
            #[cfg(desktop)]
            "agent_connector_status" => self.agent_connector_status(),
            #[cfg(desktop)]
            "pick_image" => {
                self.ensure_window_active(&label)?;
                self.pick_image(&label, &args).await
            }
            #[cfg(desktop)]
            "camera_capture_supported" => Ok(json!(cfg!(windows))),
            #[cfg(desktop)]
            "camera_capture_open" => {
                self.ensure_window_active(&label)?;
                self.camera_capture_open(&label, &args)
            }
            #[cfg(desktop)]
            "camera_capture_close" => self.camera_capture_close(&label),
            #[cfg(desktop)]
            "camera_capture_complete" => self.camera_capture_finish(&label, &args, false),
            #[cfg(desktop)]
            "camera_capture_cancel" => self.camera_capture_finish(&label, &args, true),
            "ack_launch_locations" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let ids: HashSet<_> = strings(&args, "ids")?.into_iter().collect();
                if ids.len() > 10000 {
                    return Err("Too many launch acknowledgements".into());
                }
                // Keep native acknowledgement AND runtime removal in the same
                // ordering as native inspection/runtime notification admission.
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                let _notification_launch = self.notification_launch_gate.lock()
                    .map_err(|_| "Notification launch admission unavailable")?;
                #[cfg(target_os = "ios")]
                let notification_foreground = self.notification_foreground_guard()?;
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                let notification_acknowledged = self.acknowledge_notification_launches(&label, &ids)?;
                #[cfg(target_os = "android")]
                self.acknowledge_android_notification_launches(&label, &ids).await?;
                self.launch_locations
                    .lock()
                    .map_err(|_| "Launch queue unavailable")?
                    .retain(|location| {
                        location["targetWindow"].as_str() != Some(label.as_str())
                            || !location
                                .get("launchId")
                                .and_then(Value::as_str)
                                .is_some_and(|id| ids.contains(id))
                    });
                #[cfg(target_os = "ios")]
                drop(notification_foreground);
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                drop(_notification_launch);
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                if notification_acknowledged { self.notify_library_changed(); }
                Ok(Value::Null)
            }
            "set_speech_preference" => {
                self.writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let language = text(&args, "language")?;
                let preference: store::SpeechPreferencePatch = serde_json::from_value(
                    args.get("preference").cloned().ok_or("Missing speech preference")?,
                ).map_err(|error| format!("Invalid speech preference: {error}"))?;
                let _window = self.active_window_guard(&label)?;
                let settings = self.store.update(|state| {
                    state.settings.update_speech_preference(language, preference)?;
                    Ok(state.settings.clone())
                })?;
                self.notify_library_changed();
                value(settings)
            }
            "set_book_zoom" => {
                self.writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let id = text(&args, "bookId")?;
                let zoom = args.get("zoom").ok_or("Missing book zoom")?;
                let zoom = if zoom.is_null() {
                    None
                } else {
                    Some(
                        zoom.as_f64()
                            .filter(|zoom| zoom.is_finite() && (0.25..=5.0).contains(zoom))
                            .ok_or("Book zoom must be from 25% to 500%")?,
                    )
                };
                let _window = self.active_window_guard(&label)?;
                let settings = self.store.update(|state| {
                    let id = edition_service::current_reading_reference(state, id);
                    if !state.books.iter().any(|book| book.metadata.id == id) {
                        return Err("Archive is not in the library".into());
                    }
                    if let Some(zoom) = zoom {
                        state.settings.book_zoom.insert(id.clone(), zoom);
                    } else {
                        state.settings.book_zoom.remove(&id);
                    }
                    Ok(state.settings.clone())
                })?;
                self.notify_library_changed();
                value(settings)
            }
            "set_book_pinned" => {
                self.writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let id = text(&args, "bookId")?;
                let pinned = args
                    .get("pinned")
                    .and_then(Value::as_bool)
                    .ok_or("Missing pin state")?;
                let _window = self.active_window_guard(&label)?;
                let (changed, pinned_book_ids) = self.store.set_book_pinned(id, pinned)?;
                if changed {
                    self.notify_library_changed();
                }
                Ok(json!({"pinnedBookIds": pinned_book_ids}))
            }
            "record_search" | "remove_search" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                if command == "remove_search" {
                    self.writable()?;
                }
                let query = text(&args, "query")?.trim();
                if query.is_empty() || query.len() > 65536 || query.contains('\0') || query.contains('\u{1f}') {
                    return Err("Search history query is invalid or too large".into());
                }
                let book_id = match args.get("bookId") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(id)) if id.is_empty() => None,
                    Some(Value::String(id)) => Some(id.as_str()),
                    Some(_) => return Err("Search history book identity is invalid".into()),
                };
                if book_id.is_some_and(|id| id.len() > 256 || id.contains('\0') || id.contains('\u{1f}')) {
                    return Err("Search history book identity is invalid".into());
                }
                if book_id.is_some_and(|id| id.len() + 1 + query.len() > 65536) {
                    return Err("Search history query is invalid or too large".into());
                }
                let key = book_id.map_or_else(|| query.to_string(), |id| format!("{id}\u{1f}{query}"));
                let _window = self.active_window_guard(&label)?;
                self.store.update(|state| {
                    state.recent_searches.retain(|old| old != &key);
                    if command == "record_search" && state.settings.history_enabled {
                        state.recent_searches.insert(0, key);
                        state.recent_searches.truncate(100);
                    }
                    Ok(())
                })?;
                self.notify_library_changed();
                Ok(Value::Null)
            }
            "article_preview" => {
                let id = text(&args, "bookId")?.to_string();
                let location = text(&args, "path")?.to_string();
                if location.len() > 16384 {
                    return Err("Article preview location is too long".into());
                }
                let service = self.clone();
                blocking(move || {
                    let archive = service.archive(&id)?;
                    let entry = archive.entry(&crate::server::decode_location_path(&location)?)?;
                    if entry.mime.len() > 256 {
                        return Err("Article preview MIME type is invalid".into());
                    }
                    let mime = entry.mime.split(';').next().unwrap_or("");
                    let html = crate::archive::is_html_mime(&entry.mime);
                    let (excerpt, thumbnail) = if html || mime.starts_with("text/") {
                        let bytes = archive.read(&entry.path, 0, entry.size.min(256 * 1024))?;
                        let text = String::from_utf8_lossy(&bytes);
                        if html {
                            use crate::exports::html;
                            // Archive markup is untrusted. A document too deeply
                            // nested or complex to parse cheaply still previews
                            // its title; a hover preview gets less CPU time.
                            let bounds = html::Bounds {
                                cpu: (std::time::Duration::from_millis(250), 1000),
                                ..html::ARTICLE_BOUNDS
                            };
                            match html::Html::parse_document_bounded(&text, &bounds, None) {
                                Ok(document) => {
                                    let excerpt = preview_excerpt(&document);
                                    (excerpt, preview_thumbnail(&archive, &entry.path, &document))
                                }
                                Err(_) => (String::new(), None),
                            }
                        } else {
                            (preview_text(std::iter::once(text.as_ref())), None)
                        }
                    } else {
                        (String::new(), None)
                    };
                    let path = format!("{}{}", encode_path(&entry.path), split_location(&location).1);
                    if path.len() > 16384 {
                        return Err("Resolved article preview location is too long".into());
                    }
                    Ok(json!({"title":entry.title.chars().take(2048).collect::<String>(),
                        "path":path,"excerpt":excerpt,"mime":entry.mime,"thumbnail":thumbnail}))
                }).await
            }
            "import_archives" => {
                if args.get("privacyGeneration").is_some() {
                    self.ensure_generation(&args)?;
                }
                let paths = strings(&args, "paths")?;
                let service = self.clone();
                blocking(move || service.import_paths(paths, cfg!(mobile))).await
            }
            "scan_folder" => {
                let root = PathBuf::from(text(&args, "path")?);
                let recursive = args
                    .get("recursive")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let service = self.clone();
                blocking(move || {
                    let mut paths = Vec::new();
                    scan(&root, recursive, &mut paths)?;
                    service.import_paths(paths, cfg!(mobile))
                })
                .await
            }
            "set_reader_restricted" => {
                self.set_reader_restricted(
                    text(&args, "bookId")?,
                    args.get("restricted")
                        .and_then(Value::as_bool)
                        .ok_or("Missing restricted state")?,
                    &label,
                )
                .await
            }
            "open_article" | "random_article" => {
                self.reader(
                    text(&args, "bookId")?.into(),
                    args.get("path").and_then(Value::as_str).map(str::to_owned),
                    command == "random_article",
                    text(&args, "parentOrigin")?.into(),
                    args.get("freshOrigin")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    args.get("restricted").and_then(Value::as_bool),
                    &label,
                )
                .await
            }
            "check_integrity" | "diagnostics_integrity" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let target = if command == "check_integrity" {
                    Some((text(&args, "id")?.to_string(), text(&args, "installationId")?.to_string()))
                } else { None };
                #[cfg(windows)]
                self.refuse_during_archive_move()?;
                value(self.integrity.start(self.store.clone(), target).await?)
            }
            "integrity_status" => {
                self.ensure_generation(&args)?;
                let service = self.clone();
                blocking(move || Ok(json!({
                    "job":service.integrity.status(&service.store)?,
                    "books":crate::integrity::book_statuses(&service.store)?
                }))).await
            }
            "cancel_integrity" => {
                self.ensure_generation(&args)?;
                self.integrity.cancel_and_wait(false).await?;
                Ok(Value::Null)
            }
            "book_illustration" => {
                let id = text(&args, "id")?.to_string();
                let service = self.clone();
                blocking(move || {
                    let image = service.archive(&id)?.illustration()?;
                    value(
                        image
                            .filter(|(mime, _)| {
                                matches!(
                                    mime.as_str(),
                                    "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                                )
                            })
                            .map(|(mime, bytes)| {
                                format!("data:{mime};base64,{}", STANDARD.encode(bytes))
                            }),
                    )
                })
                .await
            }
            "relink_book" => {
                self.writable()?;
                let id = text(&args, "id")?.to_string();
                if self.active_torrent_share(&id)?.is_some() {
                    return Err("Stop sharing this archive before relinking it".into());
                }
                // Refuse before the expensive admission when a share is known
                // to serve this archive; the authoritative check is repeated
                // under the slot held across the replacement below.
                if Self::selected_share_serves(&*self.sharing.lock().await, &id) {
                    return Err("Stop sharing this archive before relinking it".into());
                }
                let path = text(&args, "path")?.to_string();
                // Relinking is another manual admission: verify the chosen
                // bytes before they replace a working library entry.
                let book = blocking(move || load_book(&path, cfg!(mobile), true)).await?;
                if book.metadata.id != id {
                    return Err("Selected file is a different archive; import it separately".into());
                }
                let replacement = self.relink_installation(&id, book, &label, None).await?;
                value(replacement)
            }
            "remove_book" => {
                let expected_installation = text(&args, "installationId")?;
                let id = text(&args, "id")?.to_string();
                self.ensure_generation(&args)?;
                // A selected share holds strong archive handles and serves
                // directly from them; removal would otherwise keep the file
                // readable to LAN clients after it left the library. Refuse
                // early, then re-check under the slot held across the mutation.
                if Self::selected_share_serves(&*self.sharing.lock().await, &id) {
                    return Err("Stop sharing this archive before removing it from the library".into());
                }
                let cleanup_ownership = if let Some(source) = args.get("cleanupSource") {
                    self.normal_writable()?;
                    if args.get("deleteFile") != Some(&Value::Bool(true)) {
                        return Err("Acquisition cleanup requires explicit file deletion".into());
                    }
                    Some(self.cleanup_publications(text(source, "kind")?, text(source, "id")?).await?)
                } else {
                    None
                };
                #[cfg(mobile)]
                {
                    if self.store.book(&id)?.is_some_and(|book|
                        book.source_reference.as_ref().is_some_and(|source| !source.persistent)) {
                        self.ensure_generation(&args)?;
                        if args.get("deleteFile") == Some(&Value::Bool(true)) {
                            return Err("External provider content is read-only; remove its library reference instead".into());
                        }
                        let _source_guards = self.localswarm_source_mutation(Some(&id)).await?;
                        // Hold the slot across the store mutation so a start
                        // that raced the early check cannot pin the archive.
                        let sharing = self.sharing.lock().await;
                        if Self::selected_share_serves(&sharing, &id) {
                            return Err("Stop sharing this archive before removing it from the library".into());
                        }
                        let mut readers = self.readers.lock().await;
                        self.ensure_window_active(&label)?;
                        if self.store.book(&id)?.is_none_or(|book| book.installation_id != expected_installation) {
                            return Err("Archive installation changed while confirming removal; select it again".into());
                        }
                        let book = self.store.remove_provider_book(&id)?
                            .ok_or("Session archive changed while removing")?;
                        retire_readers(&mut readers, Some(&book.installation_id)).await?;
                        self.forget_installation(&book.installation_id);
                        self.invalidate(&id);
                        drop(readers);
                        self.notify_library_changed();
                        let reference = book.source_reference.unwrap();
                        if !self.store.retains_provider_reference(&reference.reference)? {
                            blocking(move || crate::provider_reference::forget_registered(&reference)).await?;
                        }
                        return Ok(Value::Null);
                    }
                }
                self.store.require_writable()?;
                let _source_guards = self.localswarm_source_mutation(Some(&id)).await?;
                if self.active_torrent_share(&id)?.is_some() {
                    return Err("Stop sharing this archive before removing it from the library".into());
                }
                // Hold the slot across the store mutation (after the source
                // guard, matching the relink path's order) so a start that
                // raced the early check cannot pin the archive being removed.
                let sharing = self.sharing.lock().await;
                if Self::selected_share_serves(&sharing, &id) {
                    return Err("Stop sharing this archive before removing it from the library".into());
                }
                let delete = args
                    .get("deleteFile")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut readers = self.readers.lock().await;
                self.ensure_window_active(&label)?;
                let book = self.store.book(&id)?.ok_or("Unknown archive")?;
                if book.installation_id != expected_installation {
                    return Err("Archive installation changed while confirming removal; select it again".into());
                }
                let provider_reference = book.source_reference.clone();
                if delete && provider_reference.is_some() {
                    return Err("External provider content is read-only; remove its library reference instead".into());
                }
                let installation_id = book.installation_id.clone();
                // Windows: an open archive is pinned by the native bridge
                // (no-delete leases on its parts and their folders), and NTFS
                // refuses to move or delete it while open. Release every
                // holder this service owns first: an integrity check is
                // refused, a spelling build is cancelled, and readers, cached
                // handles and whole-library share handles are closed. They
                // reopen on demand if the deletion fails. Unix unlinks open
                // files, so it keeps the original order.
                #[cfg(windows)]
                if delete {
                    if self.integrity.running_for(&id)? {
                        return Err("Stop the integrity check before deleting this archive".into());
                    }
                    let spelling = self.spelling.clone();
                    let spelling_installation = installation_id.clone();
                    // No job (or an uncancellable cache load) is not an error
                    // here; a handle that stays open is reported below.
                    let _ = blocking(move || spelling.retire_installation(&spelling_installation)).await;
                    retire_readers(&mut readers, Some(&installation_id)).await?;
                    self.invalidate(&id);
                    if let Some(server) = sharing.as_ref() {
                        server.evict_library_archive(&id);
                    }
                }
                if delete {
                    let trash = self.store.settings()?.trash_deleted_files;
                    let result = blocking(move || match cleanup_ownership {
                        Some(ownership) => crate::library_files::delete_book_with_ownership(&book, trash, &ownership),
                        None => crate::library_files::delete_book(&book, trash),
                    }).await;
                    #[cfg(windows)]
                    let result = result.map_err(crate::storage::explain_in_use);
                    result?;
                }
                self.store.update(|state| {
                    if state.books.iter().find(|book| book.metadata.id == id)
                        .is_none_or(|book| book.installation_id != expected_installation)
                    {
                        return Err("Archive installation changed while confirming removal; select it again".into());
                    }
                    state.books.retain(|b| b.metadata.id != id);
                    state.tabs.retain(|t| t.book_id.as_deref() != Some(&id));
                    if !state
                        .tabs
                        .iter()
                        .any(|t| Some(&t.id) == state.active_tab.as_ref())
                    {
                        state.active_tab = state.tabs.first().map(|t| t.id.clone());
                    }
                    for window in state.windows.values_mut() {
                        window.tabs.retain(|t| t.book_id.as_deref() != Some(&id));
                        if !window
                            .tabs
                            .iter()
                            .any(|t| Some(&t.id) == window.active_tab.as_ref())
                        {
                            window.active_tab = window.tabs.first().map(|t| t.id.clone());
                        }
                    }
                    state.reader_storage.remove(&installation_id);
                    state.settings.pinned_book_ids.retain(|value| value != &id);
                    let prefix = format!("{id}\u{1f}");
                    state.recent_searches.retain(|entry| !entry.starts_with(&prefix));
                    Ok(())
                })?;
                retire_readers(&mut readers, Some(&installation_id)).await?;
                self.invalidate(&id);
                self.forget_installation(&installation_id);
                self.notify_library_changed();
                #[cfg(mobile)]
                if let Some(reference) = provider_reference {
                    drop(readers);
                    if !self.store.retains_provider_reference(&reference.reference)? {
                        blocking(move || crate::provider_reference::forget_registered(&reference)).await?;
                    }
                }
                Ok(Value::Null)
            }
            "search" => {
                let cancel = self.search_token(&label, &args)?;
                let query = text(&args, "query")?.trim().to_string();
                let mode = crate::search::Mode::parse(args.get("mode").and_then(Value::as_str))?;
                let offset = u32::try_from(args.get("offset").and_then(Value::as_u64).unwrap_or(0))
                    .map_err(|_| "Search offset exceeds the archive API limit")?;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 200) as u32;
                if query.is_empty() {
                    return Ok(
                        json!({"hits":[],"total":0,"hasMore":false,"errors":[],"corrections":[],"spellingStatus":[]}),
                    );
                }
                let state = self.store.read()?;
                let show_snippets = state.settings.show_search_snippets;
                let requested = strings(&args, "bookIds")?;
                let service = self.clone();
                let request_cancel = cancel.clone();
                let result = blocking(move || {
                    let mut archives = Vec::new();
                    let mut dictionaries = Vec::new();
                    let spelling_enabled = state.settings.spelling_enabled
                        && !state.settings.private_mode
                        && !state.settings.read_only;
                    for book in state.books.into_iter().filter(|book| {
                        book.available && (requested.is_empty() || requested.contains(&book.metadata.id))
                    }) {
                        crate::search::check_cancel(&cancel)?;
                        let id = book.metadata.id.clone();
                        let spelling_book = spelling_enabled.then(|| (
                            book.installation_id.clone(), book.metadata.clone()
                        ));
                        let archive = service.archive_for_book(book);
                        crate::search::check_cancel(&cancel)?;
                        if let (Some((installation, metadata)), Ok(reader)) = (spelling_book, &archive) {
                            dictionaries.push((id.clone(), installation, metadata, reader.clone()));
                        }
                        archives.push((id, archive));
                    }
                    let mut result = crate::search::run(archives, &query, mode, offset, limit, show_snippets, &cancel)?;
                    let count = result["hits"].as_array().map_or(0, Vec::len);
                    let total = result["total"]
                        .as_u64()
                        .unwrap_or(0)
                        .max(u64::from(offset) + count as u64);
                    result["total"] = json!(total);
                    if count == 0 {
                        result["hasMore"] = json!(false);
                    }
                    let mut corrections = Vec::new();
                    let mut statuses = Vec::new();
                    if count == 0
                        && total == 0
                        && state.settings.spelling_enabled
                        && !state.settings.private_mode
                        && !state.settings.read_only
                    {
                        for (id, installation, metadata, archive) in &dictionaries {
                            crate::search::check_cancel(&cancel)?;
                            let spelling = (|| -> Result<_, String> {
                                let identity = crate::spelling::Identity::from_archive(archive, metadata, installation)?;
                                let status = service.spelling.status(&identity)?;
                                crate::search::check_cancel(&cancel)?;
                                let values = service.spelling.corrections(&identity, &query, total, true, &cancel);
                                identity.confirm(archive)?;
                                Ok((value(status)?, values))
                            })();
                            let values = spelling.and_then(|(status, values)| {
                                statuses.push(status);
                                values
                            });
                            match values {
                                Ok(values) => {
                                    for correction in values {
                                        let correction = value(correction)?;
                                        if !corrections.contains(&correction) {
                                            corrections.push(correction);
                                        }
                                    }
                                }
                                Err(error) => result["errors"]
                                    .as_array_mut()
                                    .ok_or("Invalid search error response")?
                                    .push(json!({"bookId":id,"error":error})),
                            }
                            crate::search::check_cancel(&cancel)?;
                        }
                    }
                    if !statuses.is_empty() {
                        // The archive may have been replaced during dictionary I/O.
                        let live = service.store.read()?;
                        if dictionaries.iter().any(|(id, installation, _, _)| !live.books.iter().any(|book|
                            book.available && book.metadata.id == *id && book.installation_id == *installation))
                        {
                            return Err("The archive installation changed or is unavailable; open it again".into());
                        }
                    }
                    result["corrections"] = json!(corrections);
                    result["spellingStatus"] = json!(statuses);
                    crate::search::check_cancel(&cancel)?;
                    Ok(result)
                })
                .await?;
                crate::search::check_cancel(&request_cancel)?;
                self.ensure_generation(&args)?;
                Ok(result)
            }
            "suggest" => {
                // Suggestions share the search owner's cancellation scope: a new
                // prefix supersedes the previous page request for this consumer.
                let cancel = self.search_token(&label, &args)?;
                let prefix = text(&args, "prefix")?.trim().to_string();
                let requested = strings(&args, "bookIds")?;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(15)
                    .clamp(1, u64::from(crate::search::SUGGEST_PAGE_MAX)) as u32;
                let token = args
                    .get("token")
                    .and_then(Value::as_str)
                    .filter(|token| !token.is_empty())
                    .map(str::to_owned);
                if requested.is_empty() {
                    return Err("Select an archive to suggest titles from".into());
                }
                if prefix.is_empty() {
                    return Ok(json!({"suggestions":[],"hasMore":false,"nextToken":null,"errors":[]}));
                }
                if prefix.len() > 512 {
                    return Err("Suggestion prefix is too long".into());
                }
                let scope: Vec<String> = self.store.books()?.into_iter()
                    .filter(|book| book.available && requested.contains(&book.metadata.id))
                    .map(|book| book.metadata.id)
                    .collect();
                let service = self.clone();
                let request_cancel = cancel.clone();
                let result = blocking(move || {
                    crate::search::suggest(
                        &scope, &prefix, token.as_deref(), limit, &cancel,
                        |book, cancel| {
                            crate::search::check_cancel(cancel)?;
                            service.archive(&scope[book])
                        },
                    )
                })
                .await?;
                crate::search::check_cancel(&request_cancel)?;
                self.ensure_generation(&args)?;
                Ok(result)
            }
            "save_session" => {
                let generation = args
                    .get("privacyGeneration")
                    .and_then(Value::as_u64)
                    .ok_or("This window's session has expired. Reopen the window to continue")?;
                if generation
                    != self
                        .privacy_generation
                        .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("Browsing session expired after a privacy transition".into());
                }
                let mut tabs: Vec<Tab> =
                    serde_json::from_value(args.get("tabs").cloned().unwrap_or(json!([])))
                        .map_err(|e| e.to_string())?;
                if tabs.len() > 200 {
                    return Err("Too many tabs".into());
                }
                let reference_revision = args.get("referenceRevision").and_then(Value::as_u64).unwrap_or(0);
                let active = args
                    .get("activeTab")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let surface = match args.get("activeSurface") {
                    None => "library",
                    Some(value) => value.as_str().ok_or("Invalid window surface")?,
                };
                crate::store::validate_active_surface(surface)?;
                let _window = self.active_window_guard(&label)?;
                // A window created by the explicit restore reports an empty
                // session before its snapshot arrives; keep the restored tabs
                // until the shell hydrates and saves them for real.
                #[cfg(desktop)]
                if tabs.is_empty() {
                    if let Ok(mut pending) = self.restored_windows.lock() {
                        if pending.contains(&label) {
                            let saved = self
                                .store
                                .read()
                                .ok()
                                .and_then(|state| state.windows.get(&label).cloned())
                                .is_some_and(|window| !window.tabs.is_empty());
                            if saved {
                                return Ok(Value::Null);
                            }
                            pending.remove(&label);
                        }
                    }
                } else if let Ok(mut pending) = self.restored_windows.lock() {
                    pending.remove(&label);
                }
                let saved_tabs = self.store.update(|state| {
                    state.translate_reference_tabs(&mut tabs, reference_revision)?;
                    let window = state.windows.entry(label.clone()).or_default();
                    window.tabs = tabs.clone();
                    window.active_tab = active.clone();
                    window.active_surface = surface.into();
                    if label == "main" {
                        state.tabs = tabs.clone();
                        state.active_tab = active.clone();
                    }
                    Ok(tabs)
                })?;
                #[cfg(desktop)]
                self.apply_window_title(&label, surface, &saved_tabs, active.as_deref());
                #[cfg(not(desktop))]
                let _ = saved_tabs;
                #[cfg(desktop)]
                self.refresh_session_snapshot();
                Ok(Value::Null)
            }
            "record_visit" => {
                let generation = args
                    .get("privacyGeneration")
                    .and_then(Value::as_u64)
                    .ok_or("This window's session has expired. Reopen the window to continue")?;
                if generation
                    != self
                        .privacy_generation
                        .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("Browsing session expired after a privacy transition".into());
                }
                if self.store.private_mode()? {
                    return Ok(Value::Null);
                }
                let create = args.get("create").and_then(Value::as_bool)
                    .ok_or("Missing visit creation intent")?;
                let id = text(&args, "bookId")?.to_string();
                let path = crate::server::normalize_location(text(&args, "path")?)?;
                let title = text(&args, "title")?.to_string();
                let scroll = args
                    .get("scroll")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .max(0.0);
                let _window = self.active_window_guard(&label)?;
                // A new history entry is user-visible history and writes
                // through. A position/title refresh for an existing entry is
                // repeatable browsing telemetry: the write is coalesced and the
                // full snapshot broadcast is skipped unless the rendered
                // identity actually changed.
                let visible = if create {
                    self.store.update(|state| {
                        let id = edition_service::current_reading_reference(state, &id);
                        if !state.settings.history_enabled {
                            return Ok(false);
                        }
                        state.history.retain(|entry| {
                            entry.book_id != id
                                || (entry.path != path
                                    && crate::server::normalize_location(&entry.path)
                                        .map(|existing| existing != path)
                                        .unwrap_or(true))
                        });
                        state.history.insert(
                            0,
                            HistoryEntry {
                                id: Uuid::new_v4().to_string(),
                                book_id: id,
                                path,
                                title,
                                visited_at: now(),
                                scroll,
                            },
                        );
                        state.history.truncate(10000);
                        Ok(true)
                    })?
                } else {
                    self.store.update_browsing(|state| {
                        let id = edition_service::current_reading_reference(state, &id);
                        if !state.settings.history_enabled {
                            return Ok(false);
                        }
                        let identity = store::history_resource_identity(&path);
                        let Some(entry) = state.history.iter_mut()
                            .filter(|entry| entry.book_id == id &&
                                store::history_resource_identity(&entry.path) == identity)
                            .max_by(|left, right| left.visited_at.cmp(&right.visited_at))
                        else {
                            return Ok(false);
                        };
                        let visible = entry.path != path || entry.title != title;
                        entry.path = path;
                        entry.title = title;
                        entry.scroll = scroll;
                        Ok(visible)
                    })?
                };
                if visible {
                    self.notify_library_changed();
                }
                Ok(Value::Null)
            }
            "close_window" => {
                self.retire_private_sync();
                self.editions.retire_window_authorizations(&label);
                if let Some(manager) = self
                    .sensors
                    .lock()
                    .map_err(|_| "Sensor manager unavailable")?
                    .remove(&label)
                {
                    manager.deactivate();
                }
                let mut readers = self.readers.lock().await;
                let prefix = format!("{label}:");
                let retired = readers.take_matching(|key, _| key.starts_with(&prefix));
                shutdown_readers(retired).await?;
                drop(readers);
                #[cfg(mobile)]
                if label == "main" { self.mobile_clear_external_books().await?; }
                if label != "main" && !self.store.read_only()? {
                    self.store.update(|state| {
                        state.windows.remove(&label);
                        Ok(())
                    })?;
                }
                Ok(Value::Null)
            }
            "remove_history_entry" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.writable()?;
                let id = text(&args, "id")?;
                self.store.update(|state| {
                    let Some(target) = state.history.iter().find(|entry| entry.id == id) else {
                        return Ok(());
                    };
                    let book_id = target.book_id.clone();
                    let path = target.path.clone();
                    let identity = store::history_resource_identity(&path);
                    state.history.retain(|entry| {
                        entry.book_id != book_id
                            || store::history_resource_identity(&entry.path) != identity
                    });
                    Ok(())
                })?;
                Ok(Value::Null)
            }
            "clear_history" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.writable()?;
                self.store.update(|s| {
                    s.history.clear();
                    s.recent_searches.clear();
                    Ok(())
                })?;
                Ok(Value::Null)
            }
            "export_history" => {
                let state = self.store.read()?;
                value(store::export_history(&state)?)
            }
            "save_bookmark" => {
                let input = args.get("bookmark").ok_or("Missing bookmark")?;
                let book_id = text(input, "bookId")?.to_string();
                let path = crate::server::normalize_location(text(input, "path")?)?;
                let title = input
                    .get("title")
                    .map(|_| text(input, "title").map(str::to_owned))
                    .transpose()?;
                let id = input.get("id").and_then(Value::as_str).map(str::to_owned);
                if input.get("highlightChange").is_some() && input.get("highlights").is_some() {
                    return Err("highlightChange and highlights cannot be supplied together".into());
                }
                let highlight_change = input
                    .get("highlightChange")
                    .map(|value| {
                        serde_json::from_value::<store::HighlightChange>(value.clone())
                            .map_err(|error| format!("Invalid highlightChange: {error}"))
                    })
                    .transpose()?;
                let highlights = input
                    .get("highlights")
                    .map(|value| {
                        serde_json::from_value::<Vec<store::Highlight>>(value.clone())
                            .map_err(|error| format!("Invalid highlights: {error}"))
                    })
                    .transpose()?;
                if let Some(highlights) = &highlights {
                    store::validate_highlights(highlights)?;
                }
                let bookmark = self.store.update(|s| {
                    let book_id = edition_service::current_reading_reference(s, &book_id);
                    let mut existing = s
                        .bookmarks
                        .iter()
                        .find(|b| {
                            id.as_ref() == Some(&b.id) || (b.book_id == book_id && b.path == path)
                        })
                        .cloned();
                    if existing.is_none()
                        && highlight_change
                            .as_ref()
                            .is_some_and(store::HighlightChange::requires_existing)
                    {
                        return Err("The bookmark no longer exists".into());
                    }
                    let annotation_edit = highlight_change.is_some();
                    let title = if annotation_edit {
                        existing
                            .as_ref()
                            .map(|bookmark| bookmark.title.clone())
                            .or(title)
                    } else {
                        title.or_else(|| existing.as_ref().map(|bookmark| bookmark.title.clone()))
                    }
                    .ok_or("New bookmarks require a title")?;
                    let mut highlights = highlights.unwrap_or_else(|| {
                        existing
                            .as_mut()
                            .map(|bookmark| std::mem::take(&mut bookmark.highlights))
                            .unwrap_or_default()
                    });
                    if let Some(change) = highlight_change {
                        change.apply(&mut highlights)?;
                    }
                    let bookmark = Bookmark {
                        id: existing
                            .as_ref()
                            .map(|b| b.id.clone())
                            .unwrap_or_else(|| Uuid::new_v4().to_string()),
                        book_id: existing.as_ref().map(|bookmark| bookmark.book_id.clone()).unwrap_or(book_id),
                        path: if annotation_edit {
                            existing
                                .as_ref()
                                .map(|bookmark| bookmark.path.clone())
                                .unwrap_or(path)
                        } else {
                            path
                        },
                        title,
                        note: match (annotation_edit, existing.as_ref()) {
                            (true, Some(bookmark)) => bookmark.note.clone(),
                            _ => input
                                .get("note")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                                .or_else(|| existing.as_ref().map(|b| b.note.clone()))
                                .unwrap_or_default(),
                        },
                        tags: match (annotation_edit, existing.as_ref()) {
                            (true, Some(bookmark)) => bookmark.tags.clone(),
                            _ if input.get("tags").is_some() => strings(input, "tags")?,
                            _ => existing
                                .as_ref()
                                .map(|b| b.tags.clone())
                                .unwrap_or_default(),
                        },
                        highlights,
                        created_at: existing
                            .as_ref()
                            .map(|b| b.created_at.clone())
                            .unwrap_or_else(now),
                        legacy_book: existing.and_then(|b| b.legacy_book),
                    };
                    s.bookmarks.retain(|b| b.id != bookmark.id);
                    s.bookmarks.push(bookmark.clone());
                    Ok(bookmark)
                })?;
                value(bookmark)
            }
            "remove_bookmark" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.writable()?;
                let id = text(&args, "id")?;
                self.store.update(|s| {
                    s.bookmarks.retain(|b| b.id != id);
                    Ok(())
                })?;
                Ok(Value::Null)
            }
            "export_bookmarks" => {
                let state = self.store.read()?;
                value(
                    if args.get("format").and_then(Value::as_str) == Some("xml") {
                        store::export_bookmarks_xml(&state)?
                    } else {
                        store::export_bookmarks(&state)?
                    },
                )
            }
            "import_bookmarks" => {
                if args.get("privacyGeneration").is_some() {
                    self.ensure_generation(&args)?;
                }
                let _window = self.active_window_guard(&label)?;
                let input = text(&args, "text")?;
                value(self.store.update(|s| store::import_bookmarks(input, s))?)
            }
            "migrate_library" => {
                if args.get("privacyGeneration").is_some() {
                    self.ensure_generation(&args)?;
                }
                self.ensure_window_active(&label)?;
                self.writable()?;
                let input = text(&args, "text")?.to_string();
                let base = args
                    .get("baseDirectory")
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                if base.as_ref().is_some_and(|base| !base.is_absolute()) {
                    return Err("The original library directory must be absolute".into());
                }
                let service = self.clone();
                blocking(move || {
                    let mut legacy = store::legacy_books(&input)?;
                    let mut paths = store::legacy_paths(&input)?;
                    if let Some(base) = base {
                        for path in &mut paths {
                            if Path::new(path).is_relative() {
                                *path = base.join(&*path).to_string_lossy().into_owned();
                            }
                        }
                        for book in &mut legacy {
                            if Path::new(&book.path).is_relative() {
                                book.path = base.join(&book.path).to_string_lossy().into_owned();
                            }
                        }
                    }
                    // Installed and imported books carry `load_book`'s
                    // `fs::canonicalize` spelling. On Windows that is a verbatim
                    // `\\?\C:\...` path, which never compares equal to the Win32
                    // spelling a legacy library records for the same file, so
                    // `register_books` (and the import below) would reject the
                    // very same archive as "another copy". Resolve existing
                    // files to that canonical spelling here, before the store
                    // transaction; a missing file keeps its literal fallback
                    // path. Unix keeps its literal-path behaviour unchanged.
                    #[cfg(windows)]
                    for book in &mut legacy {
                        if let Ok(canonical) = std::fs::canonicalize(&book.path) {
                            book.path = canonical.to_string_lossy().into_owned();
                        }
                    }
                    service.register_books(&legacy, None)?;
                    service.import_paths(paths, false)
                })
                .await
            }
            "save_settings" => {
                self.ensure_running()?;
                self.ensure_window_active(&label)?;
                let (previous, settings) = self.validate_settings_patch(&args)?;
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                let notification_policy_changed = settings.completion_notifications != previous.completion_notifications
                    || settings.private_mode != previous.private_mode || settings.read_only != previous.read_only;
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                if self.notification_gui.load(Ordering::SeqCst) && notification_policy_changed
                    && (!settings.completion_notifications || settings.private_mode || settings.read_only) {
                    self.notifications.configure(settings.completion_notifications, settings.private_mode || settings.read_only)?;
                }
                #[cfg(target_os = "android")]
                let android_notification_policy_changed = settings.completion_notifications != previous.completion_notifications
                    || settings.private_mode != previous.private_mode || settings.read_only != previous.read_only;
                #[cfg(target_os = "android")]
                if android_notification_policy_changed
                    && (!settings.completion_notifications || settings.private_mode || settings.read_only) {
                    self.configure_android_notifications(settings.completion_notifications, settings.private_mode || settings.read_only).await?;
                }
                #[cfg(target_os = "android")]
                if settings.backup_archives {
                    return Err("Android archive backup is excluded by static platform rules".into());
                }
                self.store.validate_portable_settings(&settings)?;
                self.store.validate_portable_directory(self.store.data_dir())?;
                let spelling_writable =
                    settings.spelling_enabled && !settings.private_mode && !settings.read_only;
                if !spelling_writable {
                    let spelling = self.spelling.clone();
                    blocking(move || spelling.set_writable(false)).await?;
                }
                if settings.private_mode != previous.private_mode
                    || settings.read_only != previous.read_only
                {
                    let replacement = Arc::new(ImportManager::new()?);
                    let old = self.import_manager()?;
                    old.shutdown_and_wait().await?;
                    *self
                        .imports
                        .lock()
                        .map_err(|_| "Import manager unavailable")? = replacement;
                }
                // Close the agent connector before anything else changes, so no
                // agent call runs across the switch into private or read-only
                // mode. Whatever happens next, the guard brings it back in line
                // with the saved settings when this command ends, even on an error.
                #[cfg(desktop)]
                let _agent = super::agent_connector::Reconcile(self.clone());
                #[cfg(desktop)]
                if !settings.agent_connector
                    || (settings.private_mode && !previous.private_mode)
                    || (settings.read_only && !previous.read_only)
                {
                    self.agent_connector_stop();
                }
                if (settings.read_only && !previous.read_only)
                    || (settings.private_mode && !previous.private_mode)
                {
                    if let Some(manager) = self.localswarm.get() {
                        manager.stop().await?;
                    }
                    if let Some(manager) = self.downloads.get() {
                        manager.shutdown().await?;
                    }
                    let maps = self
                        .maps
                        .lock()
                        .map_err(|_| "Map manager unavailable")?
                        .clone();
                    if let Some(manager) = maps {
                        manager.cancel().await?;
                    }
                    let torrents = self
                        .torrents
                        .lock()
                        .map_err(|_| "Torrent manager unavailable")?
                        .take();
                    if let Some(manager) = torrents {
                        manager.shutdown().await?;
                    }
                    // A hotspot-bound listener drains while its link still
                    // exists, then the reservation itself is released. The
                    // slot stays occupied until shutdown returns.
                    let mut sharing = self.sharing.lock().await;
                    let result = match sharing.as_mut() {
                        Some(server) => server.shutdown().await,
                        None => Ok(()),
                    };
                    *sharing = None;
                    result?;
                    #[cfg(target_os = "android")]
                    self.mobile_hotspot_stop().await?;
                }
                let privacy_changed = settings.private_mode != previous.private_mode
                    || settings.read_only != previous.read_only;
                if privacy_changed {
                    crate::maps::clear_recipe_cache();
                    self.privacy_generation
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    self.privacy_policy_generation.fetch_add(1, Ordering::SeqCst);
                    self.editions.retire_authorizations();
                    retire_readers(&mut *self.readers.lock().await, None).await?;
                    #[cfg(mobile)]
                    self.mobile_note_retirement()?;
                    #[cfg(mobile)]
                    self.mobile_clear_external_books().await?;
                    for manager in self.sensors.lock().unwrap().values() {
                        manager.clear_decisions();
                    }
                }
                #[cfg(target_os = "ios")]
                if privacy_changed || settings.catalog_url != previous.catalog_url {
                    self.official_catalog.lock().map_err(|_| "Official catalog state unavailable")?.clear();
                }
                #[cfg(target_os = "ios")]
                if settings.private_mode || settings.read_only {
                    crate::ios_background::set_policy(settings.private_mode, settings.read_only).await?;
                }
                if settings.read_only && !previous.read_only {
                    let crash_policy = self.synchronize_crash_policy(true).await;
                    #[cfg(target_os = "ios")]
                    if let Err(error) = &crash_policy {
                        if let Err(policy) = crate::ios_background::set_policy(previous.private_mode, previous.read_only).await {
                            return Err(format!("{error}; background download policy: {policy}"));
                        }
                    }
                    crash_policy?;
                }
                let patch = args.get("settings").and_then(Value::as_object).cloned()
                    .unwrap_or_default();
                let saved = self.store.update(|s| {
                    // Merge only the submitted keys. A stale full-form snapshot
                    // must not revert a concurrent language edit, background
                    // policy write or another window's setting.
                    s.settings = super::settings_with_patch(&s.settings, &patch)?;
                    Ok(())
                });
                // A durability error may follow a committed state replacement.
                // Effective Store policy, never the old form, owns crash writes.
                let capture = self.synchronize_crash_policy(self.store.read_only().unwrap_or(true)).await;
                #[cfg(target_os = "ios")]
                let capture = {
                    let actual = self.store.settings()?;
                    let background = crate::ios_background::set_policy(actual.private_mode, actual.read_only).await;
                    match (capture, background) {
                        (Ok(()), Ok(())) => Ok(()),
                        (Err(error), Ok(())) => Err(error),
                        (Ok(()), Err(error)) => Err(format!("Background download policy: {error}")),
                        (Err(error), Err(background)) => Err(format!("{error}; background download policy: {background}")),
                    }
                };
                if let Err(error) = saved {
                    let actual = self.store.settings()?;
                    self.spelling.set_writable(
                        actual.spelling_enabled && !actual.private_mode && !actual.read_only,
                    )?;
                    return Err(match capture {
                        Ok(()) => error,
                        Err(policy) => format!("{error}; crash capture policy: {policy}"),
                    });
                }
                capture?;
                // The under-page colour is native, so it cannot observe the
                // store itself: a theme or reader-theme change must reach every
                // window here or a dark reader keeps the previous paper in the
                // gap between documents.
                #[cfg(desktop)]
                if settings.theme != previous.theme || settings.reader_theme != previous.reader_theme {
                    let app = self.app.lock().map_err(|_| "Application handle unavailable")?.clone();
                    if let Some(app) = app {
                        crate::windows::apply_under_page_all(&app, &settings);
                    }
                }
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                if self.notification_gui.load(Ordering::SeqCst) && notification_policy_changed
                    && settings.completion_notifications && !settings.private_mode && !settings.read_only {
                    if let Err(error) = self.notifications.configure(true, false) { self.notification_warning(error); }
                }
                #[cfg(target_os = "android")]
                if android_notification_policy_changed && settings.completion_notifications && !settings.private_mode && !settings.read_only {
                    self.configure_android_notifications(true, false).await?;
                }
                #[cfg(mobile)]
                {
                    mobile_policy::set_allow_metered(settings.allow_metered_downloads);
                    crate::mobile_backup::set_archive_backup(settings.backup_archives)?;
                    if settings.allow_metered_downloads != previous.allow_metered_downloads {
                        self.mobile_network_reconcile_locked().await?;
                        let service = self.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = service.mobile_resume_after_network().await {
                                crate::mobile::error(&service, "network-resume", error);
                            }
                        });
                    }
                    if !settings.read_only && !settings.private_mode
                        && (settings.backup_archives != previous.backup_archives || previous.read_only || previous.private_mode)
                    {
                        self.mobile_apply_archive_backup_async().await?;
                    }
                }
                #[cfg(mobile)]
                let settings = if previous.read_only && !settings.read_only {
                    self.mobile_initialize()?;
                    self.store.settings()?
                } else {
                    settings
                };
                #[cfg(target_os="android")]
                if settings.auto_resume_downloads != previous.auto_resume_downloads {
                    self.mobile.policy_attempt_generation.store(0, Ordering::SeqCst);
                }
                self.spelling.set_writable(spelling_writable)?;
                if !settings.read_only && !settings.private_mode {
                    let torrent = self.torrents.lock().unwrap().clone();
                    if let Some(manager) = torrent {
                        manager
                            .limits(settings.download_limit, settings.upload_limit)
                            .await?;
                    }
                    if let Some(manager) = self.localswarm.get() {
                        manager.limits(settings.download_limit, settings.upload_limit);
                        manager.set_direct(settings.direct_connection).await?;
                    }
                    if let Some(manager) = self.downloads.get() {
                        manager.set_download_limit(settings.download_limit)?;
                        if settings.auto_resume_downloads && !cfg!(target_os = "android")
                            && crate::transfer_policy::require_internet().is_ok() {
                            // Settings were already saved. A job that cannot
                            // resume keeps its own visible error; it must not
                            // turn the whole save into a failure.
                            if let Err(error) = manager.resume_interrupted().await {
                                crate::diagnostics::record(
                                    &self.store, "downloads", Some(&error),
                                );
                            }
                        }
                    }
                }
                if settings.show_search_snippets != previous.show_search_snippets {
                    self.notify_library_changed();
                }
                value(self.store.settings()?)
            }
            "catalog_torrent" => {
                let generation = {
                    let _mode = self.privacy_gate.read().await;
                    self.ensure_running()?;
                    self.ensure_window_active(&label)?;
                    self.normal_writable()?;
                    self.ensure_generation(&args)?
                };
                let fetch = crate::catalog::resolve_official_torrent(text(&args, "url")?);
                tokio::pin!(fetch);
                let mut checks = tokio::time::interval(std::time::Duration::from_millis(250));
                let magnet = loop {
                    tokio::select! {
                        result = &mut fetch => break result?,
                        _ = checks.tick() => {
                            let _mode = self.privacy_gate.read().await;
                            self.ensure_running()?;
                            self.ensure_window_active(&label)?;
                            self.ensure_generation(&json!({"privacyGeneration":generation}))?;
                            self.normal_writable()?;
                            #[cfg(mobile)]
                            self.mobile_admit("catalog_torrent", &args)?;
                            crate::transfer_policy::require_internet()?;
                        }
                    }
                };
                value(magnet)
            }
            "catalog" | "catalog_facets" => {
                let generation = {
                    let _mode_guard = self.privacy_gate.read().await;
                    self.ensure_running()?;
                    self.ensure_window_active(&label)?;
                    self.ensure_generation(&args)?
                };
                let url = text(&args, "url")?;
                let locale = text(&args, "locale")?;
                if command == "catalog" {
                    let mut page = self.catalog_cache.fetch_page(
                        url, locale, &self.privacy_gate, &self.privacy_generation, generation,
                    ).await?;
                    #[cfg(target_os = "ios")]
                    {
                        let _mode = self.privacy_gate.read().await;
                        self.ensure_generation(&json!({"privacyGeneration":generation}))?;
                        if !self.store.private_mode()? && !self.store.read_only()? {
                            self.official_catalog.lock().map_err(|_| "Official catalog state unavailable")?
                                .record(url, page.source_url(), &page.page);
                        }
                    }
                    let official = crate::catalog::official_catalog_url(url)
                        && crate::catalog::official_catalog_url(page.source_url());
                    crate::catalog::offer_official_torrents(&mut page.page, official);
                    value(page)
                } else {
                    value(self.catalog_cache.fetch_facets(
                        url, locale, &self.privacy_gate, &self.privacy_generation, generation,
                    ).await?)
                }
            }
            "catalog_thumbnail" => {
                // Decorative cover artwork is still an outbound fetch, so it
                // needs the same active-window and privacy-generation admission
                // as the catalog itself. `fetch_thumbnail` keeps enforcing an
                // https public destination; feeds may legitimately host
                // artwork on another origin, so it is not pinned to the
                // configured catalog host.
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                value(crate::catalog::fetch_thumbnail(text(&args, "url")?, || {
                    self.ensure_window_active(&label)?;
                    self.ensure_generation(&args)?;
                    Ok(())
                }).await?)
            }
            "retry_import" => {
                self.normal_writable()?;
                value(self.retry_import(text(&args, "receipt")?)?)
            }
            "spelling_status" | "spelling_build" | "spelling_cancel" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let settings = self.store.settings()?;
                if !settings.spelling_enabled || settings.private_mode || settings.read_only {
                    return Err(
                        "Spelling correction is disabled in private/read-only mode or preferences"
                            .into(),
                    );
                }
                let id = text(&args, "bookId")?.to_owned();
                let service = self.clone();
                let command = command.to_owned();
                blocking(move || {
                    service.ensure_window_active(&label)?;
                    service.ensure_generation(&args)?;
                    let installation = text(&args, "installationId")?;
                    let book = service.store.book(&id)?.filter(|book|
                        book.available && book.installation_id == installation)
                        .ok_or("The archive installation changed or is unavailable; open it again")?;
                    let metadata = book.metadata.clone();
                    let archive = service.archive_for_book(book)?;
                    let identity = crate::spelling::Identity::from_archive(&archive, &metadata, installation)?;
                    let source_key = text(&args, "sourceKey")?;
                    if source_key != identity.key() {
                        return Err("The archive contents changed. Search again.".into());
                    }
                    if command == "spelling_build" {
                        let _window = service.active_window_guard(&label)?;
                        return service.store.with_current_installation(&id, installation, ||
                            value(service.spelling.build(archive, identity)?));
                    }
                    if command == "spelling_cancel" {
                        let _window = service.active_window_guard(&label)?;
                        let status = service.spelling.cancel(installation, source_key)?;
                        service.ensure_generation(&args)?;
                        return service.store.with_current_installation(&id, installation, || value(status));
                    }
                    let status = service.spelling.status(&identity)?;
                    identity.confirm(&archive)?;
                    service.ensure_generation(&args)?;
                    let _window = service.active_window_guard(&label)?;
                    service.store.with_current_installation(&id, installation, || value(status))
                })
                .await
            }
            "archive_files" | "reveal_book" => {
                let id = text(&args, "id")?;
                let book = self
                    .store
                    .book(id)?
                    .ok_or("Archive is not in the library")?;
                let reveal = command == "reveal_book";
                blocking(move || {
                    if reveal {
                        crate::library_files::reveal_book(&book)?;
                        Ok(Value::Null)
                    } else {
                        value(crate::library_files::archive_files(&book)?)
                    }
                })
                .await
            }
            "move_archives" | "recover_migrations" => {
                if self.store.read_only()? || self.store.private_mode()? {
                    if command == "recover_migrations" {
                        return Ok(json!({"moved":[],"errors":[],"skipped":0}));
                    }
                    self.normal_writable()?;
                }
                let service = self.clone();
                let recovery = command == "recover_migrations";
                let directories = if recovery {
                    None
                } else {
                    if args.get("confirmed") != Some(&Value::Bool(true)) {
                        return Err("Moving archives requires explicit confirmation".into());
                    }
                    Some((
                        PathBuf::from(text(&args, "fromDirectory")?),
                        PathBuf::from(text(&args, "destination")?),
                    ))
                };
                if let Some((_, destination)) = &directories {
                    self.store.validate_portable_directory(destination)?;
                }
                // An update install reads this counter and refuses while it is
                // non-zero, so a restart cannot land between the copy and the
                // library-path commit.
                let _migration = MigrationActivity::begin(&self.migration_active);
                let _source_guards = self.localswarm_source_mutation(None).await?;
                // Windows: open archives are pinned (see remove_book). For a
                // user-requested move, refuse while a share or integrity check
                // holds archives, cancel a spelling build, close every reader
                // and cached handle, and make `archive()` refuse new opens
                // until the files are moved; they reopen on demand afterwards.
                // Startup recovery skips this: it runs alongside launch-time
                // opens, and a pinned book only leaves a journaled warning.
                // The retirement below still runs.
                #[cfg(windows)]
                let moving = if recovery {
                    None
                } else {
                    // Raise the gate first: new archive opens, shares and
                    // integrity checks are refused from here on, so the checks
                    // below cannot be raced.
                    let moving = MigrationActivity::begin(&self.archive_moves);
                    if self.integrity.running()? {
                        return Err("Stop the integrity check before moving the library".into());
                    }
                    if self.sharing.lock().await.is_some() {
                        return Err("Stop sharing before moving the library".into());
                    }
                    let spelling = self.spelling.clone();
                    // No job is not an error; a still-open archive is reported
                    // per book by the move.
                    let _ = blocking(move || spelling.cancel_running()).await;
                    let mut readers = self.readers.lock().await;
                    retire_readers(&mut readers, None).await?;
                    self.archives.lock().map_err(|_| "Archive cache unavailable")?.clear();
                    Some(moving)
                };
                let result = blocking(move || match directories {
                    Some((from, to)) => {
                        crate::library_files::move_archives(service.store.clone(), from, to)
                    }
                    None => crate::library_files::recover(&service.store),
                })
                .await;
                #[cfg(windows)]
                drop(moving);
                let mut readers = self.readers.lock().await;
                retire_readers(&mut readers, None).await?;
                self.archives
                    .lock()
                    .map_err(|_| "Archive cache unavailable")?
                    .clear();
                #[cfg(mobile)]
                self.mobile_note_retirement()?;
                for manager in self
                    .sensors
                    .lock()
                    .map_err(|_| "Sensor manager unavailable")?
                    .values()
                {
                    manager.deactivate();
                }
                drop(readers);
                self.notify_library_changed();
                if recovery {
                    let mut warnings = self
                        .startup_warnings
                        .lock()
                        .map_err(|_| "Startup warning state unavailable")?;
                    match &result {
                        Err(error) => warnings.push(error.clone()),
                        Ok(result) => {
                            if let Some(errors) = result.get("errors").and_then(Value::as_array) {
                                warnings.extend(errors.iter().map(|error| {
                                    let message = error
                                        .get("error")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| error.to_string());
                                    // Name the archive and both folders, so the
                                    // warning says what to look at and where.
                                    let field = |name: &str| error.get(name).and_then(Value::as_str);
                                    match (field("title"), field("from"), field("to")) {
                                        (Some(title), Some(from), Some(to)) => format!(
                                            "Moving “{title}” from {from} to {to} did not finish: {message}"
                                        ),
                                        _ => message,
                                    }
                                }));
                            }
                        }
                    }
                }
                result
            }
            "share_book" => self.clone().share_book(&label, args).await,
            "open_attachment" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                if args.get("confirmed") != Some(&Value::Bool(true)) {
                    return Err("Opening an attachment requires explicit confirmation".into());
                }
                let id = text(&args, "bookId")?.to_owned();
                let path = crate::server::decode_location_path(text(&args, "path")?)?;
                let service = self.clone();
                let prepared = blocking(move || {
                    service.ensure_window_active(&label)?;
                    let archive = service.archive(&id)?;
                    let entry = archive.entry(&path)?;
                    crate::library_files::prepare_attachment(&archive, &entry.path, service.store.data_dir())
                }).await?;
                #[cfg(mobile)]
                self.mobile_export(&prepared.path, &prepared.mime, prepared.path.file_name().and_then(|name| name.to_str()).ok_or("Invalid attachment export name")?, true, false).await?;
                #[cfg(desktop)]
                crate::os_open::open(&prepared.path).map_err(|e| format!("Opening the attachment in its default application: {e}"))?;
                value(prepared)
            }
            "library_maintenance" => {
                // Not tied to library writes: private-mode exports leave scratch
                // too. Runs once per process, detached.
                self.start_pdf_scratch_sweep();
                if self.store.read_only()? || self.store.private_mode()? {
                    return Ok(Value::Null);
                }
                self.commit_ready_imports()?;
                self.reconcile_completed_map().await?;
                let _sources = self.localswarm_sources.lock().await;
                let service = self.clone();
                let flipped = blocking(move || {
                    // A security-scoped folder whose volume came back is usable
                    // again before its books are judged missing.
                    crate::sandbox::revive();
                    let state = service.store.read()?;
                    let availability: HashMap<_, _> = state
                        .books
                        .iter()
                        .map(|book| {
                            let available = if book.source_reference.is_some() {
                                crate::archive_source::retain(book).is_ok()
                            } else {
                                Path::new(&book.path).is_file()
                            };
                            (book.metadata.id.clone(), available)
                        })
                        .collect();
                    // A cached handle survives a rename over its path with its
                    // own size and mtime, so the path stays a file and nothing
                    // flips. Compare the handle with what the path names now.
                    let mut stale: Vec<(String, String, bool)> = service
                        .cached_archives()
                        .into_iter()
                        .filter_map(|(id, installation, path, archive)| {
                            let book = state.books.iter().find(|book| {
                                book.metadata.id == id && book.installation_id == installation && book.path == path
                            })?;
                            if availability.get(&id) != Some(&true) {
                                return None;
                            }
                            let current = crate::library_files::archive_content_identity(book).ok()?;
                            archive
                                .content_identity()
                                .map_or(true, |held| held != current)
                                .then_some((id, installation, true))
                        })
                        .collect();
                    let changed = state
                        .books
                        .iter()
                        .any(|book| availability.get(&book.metadata.id) != Some(&book.available));
                    if !changed {
                        return Ok(stale);
                    }
                    let flipped = service.store.update(|state| {
                        let mut flipped = Vec::new();
                        for book in &mut state.books {
                            if let Some(available) = availability.get(&book.metadata.id) {
                                if book.available != *available {
                                    flipped.push((book.metadata.id.clone(), book.installation_id.clone(), !*available));
                                }
                                book.available = *available;
                            }
                        }
                        Ok(flipped)
                    })?;
                    service.notify_library_changed();
                    stale.extend(flipped);
                    Ok(stale)
                }).await;
                // A cached handle is dropped whenever its book's file went away,
                // came back or was replaced: the next open reopens through the
                // identity check. A listener is retired when its file went away
                // (a removed volume's descriptors can only fail, and a deleted
                // file's space is released only when its last descriptor
                // closes) or was replaced, never just because it came back: a
                // listener opened while the file was present serves it fine.
                let mut released = Ok(());
                if let Some(flipped) = flipped.as_ref().ok().filter(|flipped| !flipped.is_empty()) {
                    let mut readers = self.readers.lock().await;
                    for (id, installation, retire) in flipped {
                        if *retire {
                            if let Err(error) = retire_readers(&mut readers, Some(installation)).await {
                                released = released.and(Err(error));
                            }
                        }
                        self.invalidate(id);
                    }
                }
                let proceed = flipped.and(released);
                let service = self.clone();
                let result = blocking(move || {
                    proceed?;
                    let state = service.store.read()?;
                    service.sweep_spelling_cache(&state);
                    if state.settings.monitor_directories.is_empty() {
                        return Ok(Value::Null);
                    }
                    let known: HashSet<_> = state
                        .books
                        .iter()
                        .map(|book| {
                            std::fs::canonicalize(&book.path)
                                .map(|path| path.to_string_lossy().into_owned())
                                .unwrap_or_else(|_| book.path.clone())
                        })
                        .collect();
                    let mut paths = Vec::new();
                    for directory in &state.settings.monitor_directories {
                        service.store.validate_portable_directory(Path::new(directory))?;
                        let entries = match std::fs::read_dir(directory) {
                            Ok(entries) => entries,
                            Err(error) if service.store.portable_mode() => return Err(format!("Cannot monitor portable data directory {directory}: {error}")),
                            Err(_) => continue,
                        };
                        for entry in entries {
                            let entry = entry
                                .map_err(|error| format!("Reading monitored directory: {error}"))?;
                            if !entry
                                .file_type()
                                .map_err(|error| error.to_string())?
                                .is_file()
                            {
                                continue;
                            }
                            let path = entry.path();
                            let extension = path
                                .extension()
                                .and_then(|extension| extension.to_str())
                                .unwrap_or("");
                            if !(extension.eq_ignore_ascii_case("zim")
                                || (!service.store.portable_mode() && extension.eq_ignore_ascii_case("zimaa")))
                            {
                                continue;
                            }
                            let canonical = path
                                .canonicalize()
                                .map_err(|error| error.to_string())?;
                            let path = canonical.to_string_lossy().into_owned();
                            if known.contains(&path) {
                                continue;
                            }
                            // Skip unchanged corrupt archives recorded by live watch or a
                            // previous maintenance scan; do not apply watch::is_settled here
                            // so fallback scans still pick up future-mtime clock-skewed files
                            // and immediate maintenance commands.
                            #[cfg(desktop)]
                            {
                                let failures = service
                                    .watch_failures
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner());
                                if failures.should_skip(&canonical) {
                                    continue;
                                }
                            }
                            paths.push(path);
                        }
                    }
                    if paths.is_empty() {
                        Ok(Value::Null)
                    } else {
                        let imported = service.import_monitored_paths(paths)?;
                        #[cfg(desktop)]
                        service.record_watch_result(&imported)?;
                        Ok(imported)
                    }
                })
                .await;
                if let Some(manager) = self.localswarm.get() {
                    manager.revoke_invalid_sources().await?;
                }
                result
            }
            "about_info" => Ok(json!({
                "appVersion": env!("CARGO_PKG_VERSION"),
                "buildNumber": env!("KIWIX_BUILD_COMMIT"),
                "tauriVersion": tauri::VERSION,
                "zimruVersion": env!("KIWIX_ZIMRU"),
                "glassVersion": env!("KIWIX_GLASS"),
                "zimruRevision": option_env!("KIWIX_ZIMRU_REVISION"),
                "glassRevision": option_env!("KIWIX_GLASS_REVISION"),
                "spellingIndexVersion": "Kiwix Words v2",
                "platform": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
            })),
            "storage_summary" => {
                let (directory, library_bytes, archive_count) = {
                    let state = self.store.read()?;
                    let directory = if state.settings.download_directory.is_empty() {
                        self.store.data_dir().to_path_buf()
                    } else {
                        PathBuf::from(&state.settings.download_directory)
                    };
                    let available: Vec<_> = state
                        .books
                        .iter()
                        .filter(|book| book.available)
                        .collect();
                    let library_bytes: u64 =
                        available.iter().map(|book| book.metadata.size).sum();
                    (directory, library_bytes, available.len())
                };
                // Free/total capacity is optional: iOS, read-only volumes and
                // some network filesystems may not expose it. Never report 0.
                let space = crate::storage::volume_space(&directory);
                Ok(json!({
                    "libraryBytes": library_bytes,
                    "archiveCount": archive_count,
                    "directory": directory.to_string_lossy(),
                    "freeBytes": space.map(|(free, _)| free),
                    "totalBytes": space.map(|(_, total)| total),
                }))
            }
            "downloads" => self.download_jobs().await,
            "kokoro_status" => self.kokoro_status().await,
            "kokoro_probe" => self.kokoro_probe().await,
            "kokoro_download" => self.kokoro_download().await,
            "kokoro_cancel" => self.kokoro_cancel().await,
            "kokoro_delete" => self.kokoro_delete().await,
            "transfer_history_notice" => {
                if self.store.read_only()? || self.store.private_mode()? {
                    return Ok(Value::Null);
                }
                let torrents = match self.torrent_manager() {
                    Ok(manager) => manager.history_notice(),
                    // A busy or unreadable profile already reports through the
                    // transfers panel's normal error surface; the notice is not
                    // allowed to add a second failure.
                    Err(_) => None,
                };
                // The HTTP downloader applies the same quiet quarantine policy
                // to an unreadable downloads.json. Its receipt belongs beside
                // the torrent receipt, never in the blocking error surface.
                let downloads = match self.download_manager().await {
                    Ok(manager) => manager.history_notice(),
                    Err(_) => None,
                };
                Ok(match (torrents, downloads) {
                    (None, None) => Value::Null,
                    (path, download_path) => json!({"path": path, "downloadPath": download_path}),
                })
            }
            "retained_download_files" => self.retained_download_jobs().await,
            "prepare_download_cleanup" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.normal_writable()?;
                let claims = args.get("installations").and_then(Value::as_array)
                    .ok_or("Missing download cleanup installations")?;
                if claims.len() > 10000 { return Err("Too many archive cleanup claims".into()); }
                let kind = text(&args, "kind")?;
                let id = text(&args, "id")?;
                if !matches!(kind, "download" | "map") { return Err("Unknown download cleanup source".into()); }
                // This read-only request authorizes no mutation. With no library
                // removals to confirm, defer the full byte proof to file deletion,
                // which independently checks installed reservations and ownership.
                if claims.is_empty() { return Ok(Value::Null); }
                let ownership = self.cleanup_publications(kind, id).await?;
                let paths: HashSet<_> = ownership.iter().map(|(path, _)| path.as_path()).collect();
                let owns_path = |path: &str| paths.contains(Path::new(path)) || ownership.iter().any(|(owned, _)| {
                    owned.to_str().is_some_and(|owned| store::acquisition_paths_equal(owned, path, path))
                });
                let state = self.store.read()?;
                let mut seen = HashSet::new();
                for claim in claims {
                    let id = text(claim, "id")?;
                    let installation = text(claim, "installationId")?;
                    let path = text(claim, "path")?;
                    if !seen.insert(id) || !owns_path(path) {
                        return Err(if ownership.is_empty() {
                            "This download has no file that can be unlinked here; remove its archive from the library first, then delete the download files".into()
                        } else {
                            "This archive is not owned by the selected download".into()
                        });
                    }
                    let book = state.books.iter().find(|book| book.metadata.id == id)
                        .ok_or("Archive installation changed while confirming cleanup")?;
                    if book.installation_id != installation || book.path != path {
                        return Err("Archive installation changed while confirming cleanup".into());
                    }
                }
                if state.books.iter().any(|book| owns_path(&book.path) && !seen.contains(book.metadata.id.as_str())) {
                    return Err("The installed archive list changed; close this dialog and try again".into());
                }
                Ok(Value::Null)
            }
            "localswarm_status" => {
                let status = self.localswarm_status().await?;
                #[cfg(target_os = "android")]
                let mut status = status;
                #[cfg(target_os = "android")]
                if let Some(object) = status.as_object_mut() {
                    object.insert("hotspot".into(), self.mobile_hotspot_status());
                }
                Ok(status)
            }
            "localswarm_discovery" => {
                let enabled = args.get("enabled").and_then(Value::as_bool).ok_or("Missing discovery enabled flag")?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let mut errors = Vec::new();
                if let Err(error) = self.localswarm_manager(None).await?.discovery(enabled).await { errors.push(error); }
                #[cfg(mobile)]
                // This arm already holds the privacy read lease from `operation`.
                if let Err(error) = self.clone().nearby_discover_leased(args.clone()).await { errors.push(error); }
                #[cfg(target_os = "android")]
                if let Err(error) = self.mobile_discovery_update().await { errors.push(error); }
                if errors.is_empty() { self.localswarm_status().await } else { Err(errors.join("; ")) }
            }
            "shared_library_browse" => {
                let (generation, client) = {
                    let _lease = self.privacy_gate.read().await;
                    self.ensure_running()?;
                    self.ensure_window_active(&label)?;
                    let generation = self.ensure_generation(&args)?;
                    self.normal_writable()?;
                    (generation, self.shared_library_client().await?)
                };
                let peer_id = text(&args, "peerId")?.to_string();
                let browse = async {
                    let native = self.nearby_status()?.get("peers").and_then(Value::as_array)
                        .is_some_and(|peers| peers.iter().any(|peer| peer.get("id").and_then(Value::as_str) == Some(peer_id.as_str())));
                    if native {
                        // Refresh/re-select of the sender this device is already
                        // connected to must reuse that live route. Reconnecting
                        // cancels its descriptor, which revokes every running
                        // download from it, leaves the sender's Wi-Fi and (on
                        // the reverse path) ends the sender's whole share. A
                        // browse that fails on the live route falls back to a
                        // fresh connection, so Retry still recovers.
                        if let Ok(descriptor) = self.nearby_resolve(&peer_id) {
                            if let Ok(inventory) = client.browse_selected(descriptor).await {
                                return Ok(inventory);
                            }
                        }
                        self.clone().nearby_command("nearby_connect", args.clone()).await?;
                        client.browse_selected(self.nearby_resolve(&peer_id)?).await
                    } else {
                        client.browse(&peer_id).await
                    }
                };
                tokio::pin!(browse);
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
                let result = loop {
                    tokio::select! {
                        result = &mut browse => break result,
                        _ = tick.tick() => {
                            self.ensure_running()?;
                            self.ensure_window_active(&label)?;
                            self.normal_writable()?;
                            if generation != self.launch_generation() {
                                return Err("Browsing session expired after a privacy transition".into());
                            }
                            #[cfg(mobile)]
                            self.mobile_admit(command, &args)?;
                        }
                    }
                };
                let _lease = self.privacy_gate.read().await;
                self.ensure_running()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.normal_writable()?;
                value(result?)
            }
            "shared_library_download" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let directory = PathBuf::from(self.store.settings()?.download_directory);
                self.store.validate_portable_directory(&directory)?;
                let client = self.shared_library_client().await?;
                let manager = self.download_manager().await?;
                let job = client.download(
                    &manager, directory, text(&args, "receiptId")?, text(&args, "bookId")?,
                    args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned),
                    args.get("replaceEarlier").and_then(Value::as_bool).unwrap_or(false),
                ).await?;
                value(job)
            }
            "localswarm_inspect" => {
                let (generation, admission) = self.localswarm_admission_token(None)?;
                let mut inspection = value(LocalSwarmManager::inspect(
                    self.localswarm_target(&args).await?,
                    text(&args, "swarmId")?,
                    args.get("pin").and_then(Value::as_str),
                    admission,
                    self.store.settings()?.download_limit,
                    self.store.has_books()?,
                ).await?)?;
                inspection["admissionGeneration"] = json!(generation);
                Ok(inspection)
            }
            "localswarm_download" => {
                let generation = args.get("admissionGeneration").and_then(Value::as_u64)
                    .ok_or("This window's session has expired. Reopen the window to continue")?;
                let directory = args.get("directory").and_then(Value::as_str)
                    .map(str::to_owned).unwrap_or(self.store.settings()?.download_directory);
                self.store.validate_portable_directory(Path::new(&directory))?;
                value(self.localswarm_manager(Some(generation)).await?.start(
                    self.localswarm_target(&args).await?,
                    text(&args, "swarmId")?.to_string(),
                    args.get("pin").and_then(Value::as_str).map(str::to_owned),
                    PathBuf::from(directory),
                    args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned),
                ).await?)
            }
            "localswarm_share_all" => {
                crate::nearby_log!("[nearby] share all begin hotspot={:?}", args.get("hotspot"));
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let _sources = self.localswarm_sources.lock().await;
                let manager = self.localswarm_manager(None).await?;
                let started = async {
                    let hotspot_use = self.hotspot_use.lock().await;
                    let name = crate::publisher::name(&self.store.settings()?).to_string();
                    #[cfg(target_os = "android")]
                    if args.get("hotspot").and_then(Value::as_bool) == Some(true) {
                        if !self.mobile_hotspot_hosting() { self.mobile_hotspot_start_bounded().await?; }
                    } else if !self.mobile_hotspot_hosting() {
                        let network = self.mobile_call("networkState", json!({})).await?;
                        if network.get("known").and_then(Value::as_bool) != Some(true) {
                            return Err("Waiting for network status. Try again when the connection is ready.".into());
                        }
                        // LAN Share all never creates an access point: the explicit
                        // Use temporary Wi-Fi action owns that route. Silently
                        // switching topology breaks LAN publication and discovery.
                        if network.get("localAvailable").and_then(Value::as_bool) != Some(true) {
                            return Err("NEARBY_LAN_REQUIRED".into());
                        }
                    }
                    let socket = self.nearby_library_socket(&label).await?;
                    #[cfg(target_os = "android")]
                    let hosted = socket.is_some();
                    let status = match socket {
                        Some((socket, addresses, cancel)) => manager.share_library_on_socket(name, socket, addresses, cancel).await?,
                        None => manager.share_library(name).await?,
                    };
                    drop(hotspot_use);
                    self.ensure_window_active(&label)?;
                    self.ensure_generation(&args)?;
                    let descriptor = manager.library_descriptor().await.ok_or("Nearby sharing ended before it was ready")?;
                    #[cfg(target_os = "android")]
                    let ended = descriptor.cancel.clone();
                    self.nearby_publish_library(&label, descriptor).await?;
                    // A share on the temporary Wi-Fi can end without Stop (an
                    // archive changed, the listener or route ended). Its network
                    // must not stay up, and a later plain Share all must not
                    // find and reuse it. The release keeps a hotspot that Web
                    // sharing, a receive or a newer share is still using.
                    #[cfg(target_os = "android")]
                    if hosted {
                        let service = self.clone();
                        tauri::async_runtime::spawn(async move {
                            ended.cancelled().await;
                            let _ = service.nearby_release_hotspot_if_unused().await;
                        });
                    }
                    value(status)
                }.await;
                if started.is_err() {
                    let _ = manager.stop_library().await;
                    let _ = self.nearby_release_hotspot_if_unused().await;
                }
                if let Err(error) = &started { crate::nearby_log!("[nearby] share all failed: {error}"); }
                started
            }
            "localswarm_stop_library" => {
                let _sources = self.localswarm_sources.lock().await;
                {
                    let _gate = self.nearby.gate.lock().await;
                    self.nearby_stop_publisher().await?;
                }
                if let Some(manager) = self.localswarm.get() { manager.stop_library().await?; }
                self.nearby_release_hotspot_if_unused().await?;
                self.localswarm_status().await
            }
            "localswarm_seed" => {
                let _sources = self.localswarm_sources.lock().await;
                value(self.localswarm_manager(None).await?.seed(
                    text(&args, "bookId")?,
                    args.get("pin").and_then(Value::as_str).map(str::to_owned),
                ).await?)
            }
            "localswarm_stop_seed" => {
                if let Some(manager) = self.localswarm.get() {
                    manager.stop_seed(text(&args, "id")?).await?;
                }
                Ok(Value::Null)
            }
            "localswarm_stop" => {
                let nearby_result = self.nearby_retire().await;
                let result = match self.localswarm.get() {
                    Some(manager) => manager.stop().await,
                    None => self.localswarm_status().await,
                };
                #[cfg(target_os = "android")]
                let hotspot = self.nearby_release_hotspot_if_unused().await;
                #[cfg(target_os = "android")]
                let discovery = self.mobile_call("discoveryState", json!({"enabled":false})).await;
                #[cfg(target_os = "android")]
                {
                    let mut errors = Vec::new();
                    if let Err(error) = nearby_result { errors.push(error); }
                    if let Err(error) = hotspot { errors.push(error); }
                    if let Err(error) = discovery { errors.push(error); }
                    match result {
                        Ok(value) if errors.is_empty() => Ok(value),
                        Ok(_) => Err(errors.join("; ")),
                        Err(error) => { errors.insert(0, error); Err(errors.join("; ")) }
                    }
                }
                #[cfg(not(target_os = "android"))]
                nearby_result.and(result)
            }
            "start_download" => {
                self.writable()?;
                let url = text(&args, "url")?.to_string();
                let directory = PathBuf::from(
                    args.get("directory")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or(self.store.settings()?.download_directory),
                );
                self.store.validate_portable_directory(&directory)?;
                crate::fs_ext::create_private_dir_all(&directory)
                    .map_err(|e| format!("Creating download directory: {e}"))?;
                if url.starts_with("magnet:")
                    || url::Url::parse(&url)
                        .ok()
                        .is_some_and(|u| u.path().to_ascii_lowercase().ends_with(".torrent"))
                {
                    let manager = self.torrent_manager()?;
                    #[cfg(target_os = "ios")]
                    manager.resume_admission().await?;
                    let settings = self.store.settings()?;
                    manager
                        .limits(settings.download_limit, settings.upload_limit)
                        .await?;
                    value(manager.start(url, directory, args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned)).await?)
                } else {
                    let manager = self.download_manager().await?;
                    let file_name = args.get("fileName").and_then(Value::as_str).map(str::to_owned);
                    let expected_size = args.get("expectedSize").and_then(Value::as_u64).filter(|size| *size > 0);
                    let acknowledgement = args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned);
                    #[cfg(target_os = "ios")]
                    let job = {
                        let official = self.official_catalog.lock()
                            .map_err(|_| "Official catalog state unavailable")?.lookup(&url);
                        if let Some(entry) = official {
                            // No name: as on every other platform the file is
                            // named after the archive URL, which carries the
                            // flavour and edition that the catalog name lacks.
                            manager.start_official_catalog(entry.url, directory, None,
                                Some(entry.size).filter(|size| *size > 0), acknowledgement).await?
                        } else {
                            manager.start(url, directory, file_name, expected_size, acknowledgement).await?
                        }
                    };
                    #[cfg(not(target_os = "ios"))]
                    let job = manager.start(url, directory, file_name, expected_size, acknowledgement).await?;
                    value(job)
                }
            }
            "pause_download" | "resume_download" | "cancel_download" => {
                self.writable()?;
                let id = text(&args, "id")?;
                if id.starts_with("localswarm:") {
                    let generation = if command == "resume_download" {
                        Some(args.get("admissionGeneration").and_then(Value::as_u64)
                            .ok_or("This window's session has expired. Reopen the window to continue")?)
                    } else { None };
                    let manager = self.localswarm_manager(generation).await?;
                    match command {
                        "pause_download" => manager.pause(id, false).await?,
                        "resume_download" => manager.resume(id, args.get("pin").and_then(Value::as_str).map(str::to_owned), args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned)).await?,
                        _ => manager.pause(id, true).await?,
                    }
                } else if id.starts_with("torrent:") {
                    let manager = self.torrent_manager()?;
                    #[cfg(target_os = "ios")]
                    if command == "resume_download" { manager.resume_admission().await?; }
                    match command {
                        "pause_download" => manager.pause(id).await?,
                        "resume_download" => manager.resume(id, args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned)).await?,
                        _ => manager.cancel(id).await?,
                    }
                } else {
                    let manager = self.download_manager().await?;
                    match command {
                        "pause_download" => manager.pause(id).await?,
                        "resume_download" => manager.resume(id, args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned)).await?,
                        _ => manager.cancel(id).await?,
                    }
                }
                Ok(Value::Null)
            }
            "delete_download_files" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.normal_writable()?;
                let id = text(&args, "id")?;
                let imports = self.import_manager()?;
                let tombstoned = imports.retire_acquisition(id);
                // A refused delete (not terminal, ownership changed, still
                // stopping) must not leave the import tombstone behind, unless
                // another in-flight deletion of the same job owns it.
                let refused = |error: String| { if tombstoned { imports.unretire_acquisition(id); } error };
                if id.starts_with("localswarm:") {
                    let manager = self.localswarm_manager(None).await.map_err(refused)?;
                    manager.delete_terminal(id).await.map_err(refused)?;
                    manager.remove_terminal(id)?;
                } else if id.starts_with("torrent:") {
                    let manager = self.torrent_manager().map_err(refused)?;
                    manager.delete_terminal(id).await.map_err(refused)?;
                    manager.remove_terminal(id).await?;
                } else {
                    let manager = self.download_manager().await.map_err(refused)?;
                    manager.delete_terminal(id).await.map_err(refused)?;
                    manager.remove_terminal(id)?;
                }
                self.store.update(|state| {
                    state.consumed_acquisitions.retain(|receipt| {
                        receipt != id && !receipt.strip_prefix(id).is_some_and(|suffix| suffix.starts_with(':'))
                    });
                    Ok(())
                })?;
                Ok(Value::Null)
            }
            "remove_download_history" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.writable()?;
                let id = text(&args, "id")?;
                if id.starts_with("localswarm:") {
                    self.localswarm_manager(None).await?.remove_terminal(id)?;
                    return Ok(Value::Null);
                }
                if id.starts_with("torrent:") {
                    self.torrent_manager()?.remove_terminal(id).await?;
                    return Ok(Value::Null);
                }
                self.download_manager().await?.remove_terminal(id)?;
                Ok(Value::Null)
            }
            "forget_download_record" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.normal_writable()?;
                let id = text(&args, "id")?;
                let imports = self.import_manager()?;
                let tombstoned = imports.retire_acquisition(id);
                let result = async {
                    if id.starts_with("localswarm:") {
                        self.localswarm_manager(None).await?.forget_terminal(id)
                    } else if id.starts_with("torrent:") {
                        self.torrent_manager()?.forget_terminal(id).await
                    } else {
                        self.download_manager().await?.forget_terminal(id)
                    }
                }.await;
                // Forgetting is all-or-nothing; a refusal keeps the record.
                if let Err(error) = result {
                    if tombstoned { imports.unretire_acquisition(id); }
                    return Err(error);
                }
                Ok(Value::Null)
            }
            "torrent_seeding" => {
                self.normal_writable()?;
                let manager = self.torrent_manager()?;
                #[cfg(target_os = "ios")]
                if args.get("enabled") == Some(&Value::Bool(true)) { manager.resume_admission().await?; }
                manager
                    .seeding(
                        text(&args, "id")?,
                        args.get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    )
                    .await?;
                Ok(Value::Null)
            }
            "share_torrent" => {
                self.normal_writable()?;
                let book = self
                    .store
                    .book(text(&args, "bookId")?)?
                    .ok_or("Archive is not in the library")?;
                let manager = self.torrent_manager()?;
                #[cfg(target_os = "ios")]
                manager.resume_admission().await?;
                value(manager.share(book).await?)
            }
            "torrent_job" => {
                let manager = self.torrent_manager()?;
                value(manager.job(text(&args, "id")?).await?)
            }
            "torrent_metadata" => {
                let manager = self.torrent_manager()?;
                Ok(json!(STANDARD.encode(
                    manager.torrent_metadata(text(&args, "id")?).await?
                )))
            }
            "map_catalog" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let configured = self.store.settings()?.map_catalog_url;
                let url = catalog_url(args.get("url").and_then(Value::as_str), &configured)?;
                value(crate::maps::fetch_catalog(&url).await?)
            }
            "map_recipe_metadata" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let configured = self.store.settings()?.map_catalog_url;
                let url = catalog_url(args.get("url").and_then(Value::as_str), &configured)?;
                value(crate::maps::fetch_recipe_metadata(
                    &url, text(&args, "regionId")?, text(&args, "language")?,
                ).await?)
            }
            "map_prepare" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let generation = self.privacy_policy_generation.load(Ordering::SeqCst);
                let settings = self.store.settings()?;
                let metadata = crate::maps::fetch_recipe_metadata(
                    &settings.map_catalog_url, text(&args, "regionId")?, text(&args, "language")?,
                ).await?;
                let _admission = self.privacy_gate.read().await;
                self.normal_writable()?;
                let current = self.store.settings()?;
                if current.map_catalog_url != settings.map_catalog_url
                    || current.map_proxy_url != settings.map_proxy_url
                    || current.download_directory != settings.download_directory {
                    return Err("Map settings changed; select the map again".into());
                }
                self.ensure_generation(&args)?;
                if generation != self.privacy_policy_generation.load(Ordering::SeqCst) {
                    return Err("Map choice expired after a privacy policy transition".into());
                }
                self.ensure_window_active(&label)?;
                value(self.map_manager()?.prepare(metadata, generation)?)
            }
            "map_start" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let generation = self.privacy_policy_generation.load(Ordering::SeqCst);
                let job = self.map_manager()?.start_prepared(
                    text(&args, "regionId")?, text(&args, "language")?,
                    text(&args, "decisionToken")?,
                    args.get("replaceExisting").and_then(Value::as_bool).unwrap_or(false),
                    generation,
                    args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned),
                )?;
                if job.status == "complete" {
                    let receipt = format!("map:{}", job.id);
                    let imports = self.import_manager()?;
                    if imports.status(&receipt).is_some_and(|status| status.state == crate::imports::ImportState::Failed) {
                        imports.retry(&receipt)?;
                    }
                }
                value(job)
            }
            "map_status" => {
                if self.store.read_only()? || self.store.private_mode()? {
                    let saved = read_saved(&self.store, "maps/map-job.json")?;
                    return Ok(if saved.get("historyDismissed") == Some(&Value::Bool(true)) {
                        Value::Null
                    } else {
                        saved
                    });
                }
                let manager = self.map_manager()?;
                manager.recover_terminal_publication()?;
                let status = manager.status()?;
                if let Some(job) = &status {
                    if job.status == "complete" {
                        if let Some(path) = &job.path {
                            self.enqueue_import(&format!("map:{}", job.id), vec![path.clone()])?;
                        }
                    }
                }
                self.commit_ready_imports()?;
                self.commit_ready_map_imports().await?;
                let paths = status
                    .as_ref()
                    .and_then(|job| job.path.clone())
                    .into_iter()
                    .collect();
                let mut status = value(status)?;
                if let Some(id) = status.get("id").and_then(Value::as_str).map(str::to_owned) {
                    self.decorate_import(&mut status, &format!("map:{id}"), paths)?;
                }
                Ok(status)
            }
            "map_pause" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let manager = self.map_manager()?;
                manager.pause_job(text(&args, "jobId")?).await?;
                manager.recover_terminal_publication()?;
                value(manager.status()?)
            }
            "map_resume" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let generation = self.privacy_policy_generation.load(Ordering::SeqCst);
                value(self.map_manager()?.resume(
                    text(&args, "jobId")?, generation,
                    args.get("diskSpaceAcknowledgement").and_then(Value::as_str).map(str::to_owned),
                )?)
            }
            "map_delete_download" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let manager = self.map_manager()?;
                let id = text(&args, "jobId")?.to_owned();
                blocking(move || manager.delete_download(&id)).await?;
                Ok(Value::Null)
            }
            "map_cancel" => {
                self.writable()?;
                self.map_manager()?.cancel().await?;
                Ok(Value::Null)
            }
            "map_remove_history" => {
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                self.writable()?;
                self.map_manager()?.remove_terminal(text(&args, "id")?)?;
                Ok(Value::Null)
            }
            "map_forget_recovery" => {
                self.normal_writable()?;
                self.ensure_window_active(&label)?;
                self.ensure_generation(&args)?;
                let path = PathBuf::from(text(&args, "path")?);
                self.map_manager()?.forget_recovery(&path)?;
                Ok(Value::Null)
            }
            "map_clear_cache" => {
                self.writable()?;
                self.map_manager()?.clear_cache()?;
                Ok(Value::Null)
            }
            "reader_storage_write" => {
                let generation = args
                    .get("privacyGeneration")
                    .and_then(Value::as_u64)
                    .ok_or("This window's session has expired. Reopen the window to continue")?;
                let current = self
                    .privacy_generation
                    .load(std::sync::atomic::Ordering::SeqCst);
                if generation != current {
                    return Ok(json!({"status":"retired","privacyGeneration":current}));
                }
                if self.store.read_only()? {
                    return Ok(
                        json!({"status":"rejected","message":"Read-only mode prevents archive storage writes"}),
                    );
                }
                if let Err(message) = self.ensure_window_active(&label) {
                    return Ok(json!({"status":"rejected","message":message}));
                }
                let book = text(&args, "bookId")?;
                let origin = text(&args, "origin")?;
                let batch = args.get("batch").ok_or("Missing archive storage batch")?;
                let readers = self.readers.lock().await;
                let closed = self
                    .closed_windows
                    .lock()
                    .map_err(|_| "Window state unavailable")?;
                if closed.contains(&label) {
                    return Ok(
                        json!({"status":"rejected","message":"This application window has closed"}),
                    );
                }
                let Some(reader) = readers.get(&format!("{label}:{book}")) else {
                    return Ok(
                        json!({"status":"rejected","message":"Archive storage listener is no longer active for this window"}),
                    );
                };
                if reader.origin != origin {
                    return Ok(
                        json!({"status":"rejected","message":"Archive storage origin does not match this window's listener"}),
                    );
                }
                match reader.write_storage_batch(batch) {
                    Ok(()) => Ok(json!({"status":"committed"})),
                    Err(crate::server::ReaderStorageError::Retired) => Ok(json!({
                        "status":"retired",
                        "privacyGeneration":self.privacy_generation.load(std::sync::atomic::Ordering::SeqCst)
                    })),
                    Err(crate::server::ReaderStorageError::Rejected(message)) => {
                        Ok(json!({"status":"rejected","message":message}))
                    }
                    Err(crate::server::ReaderStorageError::Retryable(message)) => Err(message),
                }
            }
            "sensor_activate" => {
                self.ensure_window_active(&label)?;
                let origin = text(&args, "origin")?;
                let book = text(&args, "archiveId")?;
                let readers = self.readers.lock().await;
                let closed = self
                    .closed_windows
                    .lock()
                    .map_err(|_| "Window state unavailable")?;
                if closed.contains(&label) {
                    return Err("This application window has closed".into());
                }
                let reader = readers
                    .get(&format!("{label}:{book}"))
                    .filter(|reader| reader.origin == origin)
                    .ok_or("Sensor origin is not an active archive listener for this window")?;
                let manager = self.sensor_manager(&label);
                manager.activate(
                    text(&args, "sessionId")?.into(),
                    book.into(),
                    reader.installation_id.clone(),
                    origin.into(),
                )?;
                Ok(self.sensor_status(&manager, text(&args, "sessionId")?))
            }
            "sensor_status" => {
                let manager = self.sensor_manager(&label);
                let session = args.get("sessionId").and_then(Value::as_str).unwrap_or("");
                let installation = args.get("installationId").and_then(Value::as_str);
                if let Some(installation) = installation {
                    let readers = self.readers.lock().await;
                    if !readers
                        .values()
                        .any(|reader| reader.installation_id == installation)
                    {
                        return Err("Sensor installation is not an active archive listener for this window".into());
                    }
                }
                Ok(self.sensor_status_for(&manager, session, installation))
            }
            "sensor_deactivate" => {
                self.sensor_manager(&label)
                    .deactivate_session(text(&args, "sessionId")?);
                Ok(Value::Null)
            }
            "sensor_consent" | "sensor_revoke" | "sensor_reset" => {
                let closed = self
                    .closed_windows
                    .lock()
                    .map_err(|_| "Window state unavailable")?;
                if closed.contains(&label) {
                    return Err("This application window has closed".into());
                }
                let manager = self.sensor_manager(&label);
                let sensor: SensorKind =
                    serde_json::from_value(args.get("sensor").cloned().ok_or("Missing sensor")?)
                        .map_err(|e| e.to_string())?;
                if command == "sensor_reset" {
                    manager.reset_consent(text(&args, "sessionId")?, sensor)?;
                } else {
                    manager.set_consent(
                        text(&args, "sessionId")?,
                        sensor,
                        command != "sensor_revoke"
                            && args
                                .get("allowed")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                    )?;
                }
                Ok(self.sensor_status(&manager, text(&args, "sessionId")?))
            }
            "sensor_location" => {
                if !self.native_location_supported() {
                    return Err(json!({
                        "code": 2,
                        "message": "Location is unavailable on this platform"
                    }).to_string());
                }
                let manager = self.sensor_manager(&label);
                manager
                    .location(
                        text(&args, "sessionId")?,
                        args.get("requestId")
                            .and_then(Value::as_u64)
                            .ok_or("Missing request ID")?,
                        args.get("highAccuracy")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        args.get("timeoutMs")
                            .and_then(Value::as_u64)
                            .unwrap_or(10000),
                    )
                    .await
                    .map_err(|e| serde_json::to_string(&e).unwrap_or(e.message))
                    .and_then(value)
            }
            "sensor_cancel" => {
                self.sensor_manager(&label).cancel_location(
                    text(&args, "sessionId")?,
                    args.get("requestId")
                        .and_then(Value::as_u64)
                        .ok_or("Missing request ID")?,
                );
                Ok(Value::Null)
            }
            "sharing_status" => Ok(self.sharing_status().await),
            "sharing_interfaces" => value(crate::sharing::interfaces()?),
            "hotspot_status" => Ok(self.mobile_hotspot_status()),
            "sharing_start" => {
                let _hotspot_use = self.hotspot_use.lock().await;
                self.writable()?;
                if self.store.private_mode()? {
                    return Err("Sharing is disabled in private mode".into());
                }
                let scope: crate::sharing::SharingScope = match args.get("scope") {
                    Some(value) => serde_json::from_value(value.clone())
                        .map_err(|error| format!("Invalid sharing scope: {error}"))?,
                    None => crate::sharing::SharingScope::Library,
                };
                let mut ids = strings(&args, "bookIds")?;
                if scope == crate::sharing::SharingScope::Library {
                    if !ids.is_empty() {
                        return Err(
                            "Library sharing serves every available archive; do not select individual archives"
                                .into(),
                        );
                    }
                } else {
                    if ids.is_empty() {
                        ids = self.store.settings()?.sharing_book_ids;
                    }
                    if ids.is_empty() {
                        return Err("Select at least one archive to share".into());
                    }
                }
                let binding: crate::sharing::SharingBinding = match args.get("binding") {
                    Some(value) => serde_json::from_value(value.clone())
                        .map_err(|error| format!("Invalid sharing binding: {error}"))?,
                    None => {
                        #[cfg(target_os = "android")]
                        if self.mobile_hotspot_hosting() {
                            crate::sharing::SharingBinding::Hotspot {}
                        } else {
                            crate::sharing::default_binding()?
                        }
                        #[cfg(not(target_os = "android"))]
                        crate::sharing::default_binding()?
                    }
                };
                let port = args.get("port").and_then(Value::as_u64).unwrap_or(0);
                if port > 65535 {
                    return Err("Invalid sharing port".into());
                }
                let mut sharing = self.sharing.lock().await;
                if sharing.is_some() {
                    return Err("Stop sharing before changing its selection or binding".into());
                }
                // Under the sharing lock: a library move raises its gate before
                // it checks this slot, so exactly one of the two proceeds.
                #[cfg(windows)]
                self.refuse_during_archive_move()?;
                let server = crate::sharing::SharingServer::start_session(
                    self.clone(),
                    scope,
                    ids,
                    port as u16,
                    binding,
                    args.get("pin").and_then(Value::as_str).map(str::to_owned),
                )
                .await?;
                let result = server.status();
                *sharing = Some(server);
                Ok(result)
            }
            "sharing_stop" => {
                self.stop_sharing().await?;
                Ok(self.sharing_status().await)
            }
            "export_article" => {
                if args.get("privacyGeneration").is_some() { self.ensure_generation(&args)?; }
                let service = self.clone();
                blocking(move || {
                    let id = text(&args, "bookId")?;
                    let (archive, installation) = service.export_archive(&args)?;
                    let entry = archive.entry(&crate::server::decode_location_path(text(&args, "path")?)?)?;
                    if entry.size > MAX_EXPORT {
                        return Err("Resource exceeds in-memory export limit; use the archive resource URL to save large media".into());
                    }
                    let bytes = archive.read(&entry.path, 0, entry.size)?;
                    let name = Path::new(&entry.path).file_name().and_then(|n| n.to_str()).unwrap_or("article.html");
                    let result = json!({"fileName": name, "mime": entry.mime, "bytes": STANDARD.encode(bytes)});
                    let _window = service.active_window_guard(&label)?;
                    if !service.store.book(id)?.is_some_and(|book| book.available && book.installation_id == installation) {
                        return Err("The archive installation changed or is unavailable; open it again".into());
                    }
                    Ok(result)
                }).await
            }
            "save_resource" | "save_mhtml" => {
                self.ensure_generation(&args)?;
                self.writable()?;
                let service = self.clone();
                let mhtml = command == "save_mhtml";
                blocking(move || {
                    let destination = Path::new(text(&args, "destination")?);
                    let (output, _, installation) = service.prepare_archive_export(&args, destination, mhtml)?;
                    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
                    service.with_export_installation(&label, text(&args, "bookId")?, &installation, || output.publish(overwrite))?;
                    Ok(Value::Null)
                }).await
            }
            "save_text" | "save_bytes" => {
                self.ensure_generation(&args)?;
                self.writable()?;
                let path = PathBuf::from(text(&args, "path")?);
                let bytes = if command == "save_text" {
                    text(&args, "text")?.as_bytes().to_vec()
                } else {
                    STANDARD
                        .decode(text(&args, "bytes")?)
                        .map_err(|e| e.to_string())?
                };
                blocking(move || {
                    let _window = self.active_window_guard(&label)?;
                    crate::exports::save_document(&path, &bytes)?;
                    Ok(Value::Null)
                })
                .await
            }
            "sandbox_folders_needing_access" => {
                let paths = strings(&args, "paths")?;
                blocking(move || value(crate::sandbox::folders_needing_access(paths.iter().map(PathBuf::from)))).await
            }
            "read_text" => {
                self.ensure_generation(&args)?;
                let path = PathBuf::from(text(&args, "path")?);
                blocking(move || crate::migration::read_import_document(&path).map(Value::String)).await
            }
            "open_external" => {
                // The mode read lease also excludes save_settings: a stale
                // dialog cannot override the current persisted link policy.
                let url = external_link_target(&self.store.settings()?, &args)?;
                self.ensure_window_active(&label)?;
                #[cfg(mobile)]
                self.mobile_call("openUrl", json!({"url":url.as_str()})).await?;
                #[cfg(desktop)]
                crate::os_open::open(url.as_str())?;
                Ok(Value::Null)
            }
            #[cfg(desktop)]
            "new_window" => {
                let app = self
                    .app
                    .lock()
                    .map_err(|_| "Application handle unavailable")?
                    .clone()
                    .ok_or("Native windows require the desktop application")?;
                let id = format!("window-{}", Uuid::new_v4());
                let label = id.clone();
                let handle = app.clone();
                let (sender, receiver) = tokio::sync::oneshot::channel();
                app.run_on_main_thread(move || {
                    let result = crate::quit::with_window_creation(&handle, || {
                        if handle.webview_windows().len() >= 16 {
                            Err("At most 16 native windows can be open".to_string())
                        } else {
                            crate::windows::create(&handle, &label, None)
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        }
                    });
                    let _ = sender.send(result);
                })
                .map_err(|e| e.to_string())?;
                receiver
                    .await
                    .map_err(|_| "Window creation was interrupted")??;
                value(id)
            }
            // Reopens the durable "previous session" on demand. Startup restore
            // remains governed by settings.restoreSession; this command never
            // runs in a private or read-only profile and never replaces live
            // reader tabs in an open window.
            #[cfg(desktop)]
            "restore_session" => {
                if self.store.private_mode()? {
                    return Err("Restore previous session is unavailable in private mode".into());
                }
                if self.store.read_only()? {
                    return Err(
                        "Restore previous session is unavailable in a read-only profile".into(),
                    );
                }
                let mut replacements = self.restore_replacements.clone().lock_owned().await;
                let Some(saved) = self.store.saved_session()? else {
                    return Err("No saved session is available to restore".into());
                };
                if !saved.has_content() {
                    return Err("The saved session has no open tabs to restore".into());
                }
                let app = self
                    .app
                    .lock()
                    .map_err(|_| "Application handle unavailable")?
                    .clone()
                    .ok_or("Native windows require the desktop application")?;
                let mut open: HashSet<String> = app.webview_windows().into_keys().collect();
                let native_open_count = open.len();
                let (plans, skipped, reference_revision) = {
                    // Hold liveness only through planning and the Store commit. The
                    // main-thread creation and AppKit restore below can reenter the app.
                    let retired = self.active_window_guard(&label)?;
                    open.retain(|window| !retired.contains(window));
                    let live = self.store.read()?;
                    let live_tabs: HashMap<String, usize> = live
                        .windows
                        .iter()
                        .map(|(window, session)| (window.clone(), session.tabs.len()))
                        .collect();
                    let main_tabs = live
                        .windows
                        .get("main")
                        .map(|window| window.tabs.len())
                        .unwrap_or(live.tabs.len());
                    let mut plan = plan_session_restore(
                        &saved, &live_tabs, main_tabs, &open, &label, &retired, &mut replacements,
                    );
                    plan.limit_creations(&open, native_open_count);
                    let RestorePlan { mut plans, skipped } = plan;
                    if plans.is_empty() {
                        return Err("The saved session is already open".into());
                    }
                    #[cfg(desktop)]
                    if let Ok(mut pending) = self.restored_windows.lock() {
                        for (window_label, _) in &plans {
                            // New windows hydrate from their snapshot; open windows
                            // receive the push below. Both must ignore an empty
                            // save that races ahead of adoption.
                            if window_label != &label {
                                pending.insert(window_label.clone());
                            }
                        }
                    }
                    let reference_revision = self.store.update(|state| {
                        for (source, replacement) in replacements.iter() {
                            if source != "main" && retired.contains(source)
                                && plans.iter().any(|(target, _)| target == replacement)
                            {
                                state.windows.remove(source);
                            }
                        }
                        for (window_label, session) in &mut plans {
                            state.translate_reference_tabs(&mut session.tabs, saved.reference_revision)?;
                            state.windows.insert(window_label.clone(), session.clone());
                            if window_label.as_str() == "main" {
                                state.tabs.clone_from(&session.tabs);
                                state.active_tab.clone_from(&session.active_tab);
                            }
                        }
                        Ok(state.reference_revision)
                    })?;
                    (plans, skipped, reference_revision)
                };
                let creations: Vec<(String, Option<Value>)> = plans
                    .iter()
                    .filter(|(window_label, _)| !open.contains(window_label))
                    .map(|(window_label, session)| {
                        (window_label.clone(), session.geometry.clone())
                    })
                    .collect();
                let mut failures: Vec<Value> = Vec::new();
                if !creations.is_empty() {
                    let handle = app.clone();
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    app.run_on_main_thread(move || {
                        let result = crate::quit::with_window_creation(&handle, || {
                            let mut failures = Vec::new();
                            for (window_label, geometry) in &creations {
                                if handle.webview_windows().len() >= 16 {
                                    failures.push(json!({
                                        "window": window_label,
                                        "error": "At most 16 native windows can be open",
                                    }));
                                    continue;
                                }
                                if let Err(error) = crate::windows::create(
                                    &handle,
                                    window_label,
                                    geometry.as_ref(),
                                ) {
                                    // Report and keep restoring the remaining
                                    // saved windows; one failure is not the session.
                                    crate::windows::report_window_creation_failure(
                                        &handle,
                                        window_label,
                                        &error.to_string(),
                                    );
                                    failures.push(json!({
                                        "window": window_label,
                                        "error": error.to_string(),
                                    }));
                                }
                            }
                            Ok(failures)
                        });
                        // The queued operation keeps the restore gate even if
                        // its IPC waiter is dropped before the main thread runs.
                        let _ = sender.send((result, replacements));
                    })
                    .map_err(|e| e.to_string())?;
                    let (result, lease) = receiver
                        .await
                        .map_err(|_| "Window creation was interrupted")?;
                    replacements = lease;
                    failures = result?;
                }
                let native_sessions: HashMap<_, _> = plans.iter().cloned().collect();
                crate::windows::restore_native_tabs(&app, &native_sessions)?;
                // An already-open window cannot receive tabs through the
                // command result or a startup snapshot: push its restored
                // session to it. The invoking window adopts the result below.
                {
                    use tauri::Emitter;
                    for (window_label, session) in plans
                        .iter()
                        .filter(|(window_label, _)| window_label != &label && open.contains(window_label))
                    {
                        let _ = app.emit_to(
                            window_label.as_str(),
                            "kiwix-session-restored",
                            json!({
                                "tabs": &session.tabs,
                                "activeTab": &session.active_tab,
                                "activeSurface": &session.active_surface,
                                "referenceRevision": reference_revision,
                            }),
                        );
                    }
                }
                let window = plans
                    .iter()
                    .find(|(window_label, _)| window_label == &label)
                    .map(|(_, session)| {
                        json!({
                            "tabs": session.tabs,
                            "activeTab": session.active_tab,
                            "activeSurface": session.active_surface,
                            "referenceRevision": reference_revision,
                        })
                    });
                let failed_labels: std::collections::HashSet<&str> = failures
                    .iter()
                    .filter_map(|failure| failure.get("window").and_then(Value::as_str))
                    .collect();
                drop(replacements);
                Ok(json!({
                    "window": window,
                    "restored": plans.iter()
                        .map(|(window_label, _)| window_label)
                        .filter(|window_label| !failed_labels.contains(window_label.as_str()))
                        .cloned()
                        .collect::<Vec<_>>(),
                    "skipped": skipped,
                    "failed": failures,
                }))
            }
            "diagnostics" => {
                let service = self.clone();
                blocking(move || {
                    let mut report = crate::diagnostics::report(&service.store)?;
                    report["integrityRun"] = crate::diagnostics::integrity_run(&service.integrity.status(&service.store)?);
                    Ok(report)
                }).await
            }
            "crash_report" => {
                let service = self.clone();
                blocking(move || {
                    Ok(json!({
                        "record": crate::crash::pending(service.store.data_dir())?,
                        // Every native target can hand a mailto: URL to its
                        // compose intent; the shell still requires isNative.
                        "canSend": cfg!(any(desktop, mobile)),
                    }))
                }).await
            }
            "crash_report_acknowledge" => {
                self.writable()?;
                let service = self.clone();
                blocking(move || {
                    Ok(json!({"reported": crate::crash::mark_reported(service.store.data_dir())?}))
                }).await
            }
            "crash_report_delete" => {
                self.writable()?;
                let service = self.clone();
                blocking(move || {
                    Ok(json!({"deleted": crate::crash::delete(service.store.data_dir())?}))
                }).await
            }
            #[cfg(debug_assertions)]
            "debug_crash" => {
                // Hidden acceptance path: only a debug build with the explicit
                // environment gate may deliberately panic the service.
                if std::env::var("KIWIX_DEBUG_CRASH").as_deref() != Ok("1") {
                    return Err("Debug crash trigger is disabled".into());
                }
                let _reading = match args.get("archiveId").and_then(Value::as_str) {
                    Some(id) => {
                        let book = self.store.book(id)?.ok_or("Debug crash archive is not in the library")?;
                        Some(crate::crash::reading(id, &book.installation_id, "debug_crash"))
                    }
                    None => None,
                };
                panic!(
                    "Synthetic Kiwix crash: path=/Users/example/Private/notes.txt                      url=https://example.org/article?token=leak#part token=leak-1234567890"
                );
            }
            "find_updates" => self.find_edition_updates(&args, &label).await,
            "edition_prepare" | "edition_download" | "edition_status" | "edition_commit"
                | "edition_cancel" | "edition_rollback" | "edition_discard_original" => self.edition_command(command, &args, &label).await,
            _ => Err(format!("Unknown application command: {command}")),
        }
    }
}

/// Resolve a map catalog command's URL against the configured catalog. The
/// commands may select a catalog, not fetch an arbitrary host: a caller-supplied
/// URL must share the configured catalog's origin.
fn catalog_url(supplied: Option<&str>, configured: &str) -> Result<String, String> {
    let Some(supplied) = supplied else {
        return Ok(configured.to_string());
    };
    let url = url::Url::parse(supplied).map_err(|error| format!("Invalid map catalog URL: {error}"))?;
    let configured = url::Url::parse(configured).map_err(|error| format!("Invalid map catalog URL: {error}"))?;
    if url.origin() != configured.origin() {
        return Err("The map catalog URL does not match the configured catalog".into());
    }
    Ok(supplied.to_string())
}
