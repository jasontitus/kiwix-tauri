// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
use crate::mobile::{self, BackgroundRequest, IncomingRequest};
#[cfg(target_os = "android")]
use crate::mobile::EntryRequest;
#[cfg(target_os = "ios")]
use std::time::Duration;
use tauri_plugin_kiwix_mobile::MobileExt;

const CHECKPOINT_EXPIRED: &str = "The native background checkpoint expired before background retirement completed";

fn background_checkpoint_ended() -> String {
    json!({
        "code": crate::user_error::SUSPENDED_CODE,
        "message": crate::user_error::SUSPENDED_MESSAGE,
        "retry": false,
        "reason": "background-checkpoint-ended",
    }).to_string()
}

fn background_checkpoint_open(state: &mobile::Lifecycle, request: &BackgroundRequest) -> bool {
    !state.desired_active && super::mobile_policy::checkpoint_open(
        request.generation, state.generation, request.acknowledged,
        request.deadline, tokio::time::Instant::now(),
    )
}

/// Validate a pinned-article link and rewrite it to the shell's canonical
/// `book-id\nencoded-location` form. This is the same parser used for external
/// `zim://` links, so a shortcut cannot bypass path or archive-identity rules.
#[cfg(target_os = "android")]
fn normalize_article_entry(raw: &str) -> Result<String, String> {
    let link = crate::zim_scheme::parse(raw)?;
    let path = link.path.as_deref().unwrap_or("");
    let value = format!(
        "{}\n{}{}",
        link.book_id,
        crate::server::encode_path(path),
        link.suffix
    );
    if value.chars().count() > 4096 {
        return Err("This pinned article link is too long to open".into());
    }
    Ok(value)
}

impl Service {
    pub(crate) fn mobile_initialize(&self) -> Result<(), String> {
        let settings = self.store.settings()?;
        // Native handlers start disabled. Publish the effective Store policy
        // before either initialization or a writable-mode return can mutate it.
        crate::crash::set_read_only(true);
        let app = self.app.lock().map_err(|_| "Application handle unavailable")?.clone()
            .ok_or("Mobile native adapter is unavailable")?;
        app.kiwix_mobile().call_sync("crashCapturePolicy", json!({"enabled": !settings.read_only}))?;
        crate::crash::set_read_only(settings.read_only);
        super::mobile_policy::set_allow_metered(settings.allow_metered_downloads);
        crate::mobile_backup::set_archive_backup(settings.backup_archives)?;
        if self.store.read_only()? { return Ok(()); }
        // The provider resolver is installed before mobile_initialize. Recovery
        // may need it to finish an interrupted selected-provider move.
        match crate::library_root::recover(&self.store) {
            Ok(Some(warning)) => self.note_startup_warning(warning),
            Ok(None) => {}
            Err(error) => self.note_startup_warning(format!(
                "Kiwix could not finish a previous library storage move. {error}"
            )),
        }
        let archives = crate::library_files::mobile_archive_directory(self.store.data_dir())?;
        self.store.update(|state| {
            if state.settings.download_directory.is_empty() {
                state.settings.download_directory = archives.to_string_lossy().into_owned();
            }
            let configured = PathBuf::from(&state.settings.download_directory);
            if !configured.is_absolute()
                || (configured.starts_with(self.store.data_dir()) && !configured.is_dir())
            {
                state.settings.download_directory = archives.to_string_lossy().into_owned();
            }
            Ok(())
        })?;
        // A persisted external root is only trusted when it still carries this
        // profile's marker. A removed card keeps its setting so the library does
        // not silently appear to have moved back to internal storage; an
        // unrecognized existing folder falls back to internal app storage.
        let configured = PathBuf::from(self.store.settings()?.download_directory);
        if !configured.starts_with(self.store.data_dir()) {
            if configured.is_dir() && !crate::library_root::is_managed_path(&self.store, &configured) {
                self.store.update(|state| {
                    state.settings.download_directory = archives.to_string_lossy().into_owned();
                    Ok(())
                })?;
                self.note_startup_warning(
                    "The saved library location was not recognized, so Kiwix uses internal app storage. Existing files were kept."
                        .into(),
                );
            } else if !configured.is_dir() {
                self.note_startup_warning(
                    "The selected library storage is unavailable. Reconnect it or choose another location in Settings."
                        .into(),
                );
            }
        }
        Ok(())
    }
    /// Caller owns the privacy policy writer. Rust is disabled first, and the
    /// native reply is a write-drain barrier; only successful writable admission
    /// enables Rust again. A failed native transition remains fail-closed.
    pub(crate) async fn mobile_set_crash_capture_policy(&self, read_only: bool) -> Result<(), String> {
        crate::crash::set_read_only(true);
        self.mobile_call("crashCapturePolicy", json!({"enabled": !read_only})).await?;
        crate::crash::set_read_only(read_only);
        Ok(())
    }
    /// Caller retains its privacy lease while the descriptor-bound reconciliation
    /// runs off the native UI and async executor threads.
    pub(crate) async fn mobile_apply_archive_backup_async(self: &Arc<Self>) -> Result<(), String> {
        let settings = self.store.settings()?;
        if settings.private_mode || settings.read_only { return Ok(()); }
        let service = self.clone();
        blocking(move || service.mobile_apply_archive_backup()).await
    }
    pub(crate) async fn mobile_bootstrap_archive_backup(self: &Arc<Self>) -> Result<(), String> {
        let _mode = self.privacy_gate.read().await;
        self.mobile_apply_archive_backup_async().await
    }
    pub(crate) fn mobile_terminal(&self) -> bool { self.stopping.load(Ordering::SeqCst) }
    #[cfg(target_os = "ios")]
    pub(crate) fn mobile_begin_shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.mobile.wake.notify_waiters();
    }
    pub(crate) fn mobile_active(&self) -> bool {
        self.mobile.state.lock().is_ok_and(|state| state.active && state.desired_active && !state.inactive)
            && !self.mobile_terminal()
    }
    pub(super) fn mobile_root(&self, label: &str) -> Result<(), String> {
        if label != "main" { return Err("Mobile native actions require the trusted root window".into()); }
        self.ensure_window_active(label)
    }
    pub(super) fn mobile_admit(&self, command: &str, args: &Value) -> Result<(), String> {
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
        self.mobile_root(label)?;
        if matches!(command, "read_text" | "save_text" | "save_bytes" | "save_resource" | "save_mhtml") {
            return Err("Use the native mobile document import/export action".into());
        }
        match command {
            "scan_folder" => crate::library_files::mobile_owned_directory(self.store.data_dir(), Path::new(text(args,"path")?))?,
            "relink_book" => { crate::library_files::mobile_import_paths(self.store.data_dir(),std::iter::once(text(args,"path")?))?; }
            "start_download" | "localswarm_download" => {
                if let Some(directory) = args.get("directory").and_then(Value::as_str) {
                    // The configured library root may be this app's own folder
                    // on a removable volume. It was validated when selected;
                    // every other destination must still be inside the profile.
                    let configured = self.store.settings()?.download_directory;
                    if Path::new(directory) != Path::new(&configured) {
                        crate::library_files::mobile_owned_directory(self.store.data_dir(),Path::new(directory))?;
                    }
                }
            }
            "save_settings" => {
                if let Some(patch) = args.get("settings") {
                    if patch.get("trashDeletedFiles") == Some(&Value::Bool(true)) {
                        return Err("OS Trash is not supported on this mobile platform".into());
                    }
                    if let Some(directory) = patch.get("downloadDirectory").and_then(Value::as_str) {
                        crate::library_files::mobile_owned_directory(self.store.data_dir(),Path::new(directory))?;
                    }
                    if let Some(directories) = patch.get("monitorDirectories").and_then(Value::as_array) {
                        for directory in directories {
                            crate::library_files::mobile_owned_directory(self.store.data_dir(),Path::new(directory.as_str().ok_or("Invalid monitored mobile archive folder")?))?;
                        }
                    }
                }
            }
            _ => {}
        }
        let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
        if matches!(command, "mobile_lifecycle_state" | "mobile_validate_background" | "mobile_background_ack" | "mobile_ack_entry" | "mobile_voice_search_available" | "sensor_status" | "sensor_deactivate" | "sensor_cancel" | "cancel_search" | "search_release" | "close_window") {
            return Ok(());
        }
        let requested = args.get("mobileBackgroundRequestId")
            .map(|requested| requested.as_str().ok_or("Invalid mobile background request identity"))
            .transpose()?;
        if requested.is_some() || (!state.desired_active && super::mobile_policy::checkpoint_write_command(command)) {
            let current = state.background.as_ref()
                .map(|request| (request.id.as_str(), background_checkpoint_open(&state, request)));
            return if super::mobile_policy::checkpoint_write_admitted(command, requested, current, state.desired_active) {
                Ok(())
            } else {
                Err(background_checkpoint_ended())
            };
        }
        if state.active && state.desired_active && !state.inactive { return Ok(()); }
        if state.active && state.desired_active && state.inactive && super::mobile_policy::checkpoint_write_command(command) {
            return Ok(());
        }
        if state.active && state.desired_active && state.inactive && command == "sensor_consent" {
            // Completion for an already activated, source-bound sensor session
            // must survive the system authorization dialog that caused inactivity.
            return Ok(());
        }
        #[cfg(target_os = "android")]
        if matches!(command, "localswarm_status" | "localswarm_hotspot_stop")
            && self.hotspot.lock().is_ok_and(|hotspot| matches!(
                hotspot.phase(), crate::hotspot::HotspotPhase::Starting | crate::hotspot::HotspotPhase::Active))
        {
            // A user-initiated hotspot start is crossing the Android permission
            // dialog (or Samsung's own hotspot warning Activity). The native
            // start owns the continuation, so the UI may observe or stop that
            // hotspot; no other command is reopened by this exception.
            return Ok(());
        }
        #[cfg(target_os = "android")]
        if command == "localswarm_hotspot_start"
            && self.hotspot.lock().is_ok_and(|hotspot| hotspot.phase() == crate::hotspot::HotspotPhase::Starting)
        {
            return Ok(());
        }
        if matches!(command, "sensor_location" | "sensor_consent") {
            // Lifecycle admission may close before its event reaches the reader.
            // Reject the old request, but do not misreport retirement as denial
            // or a provider failure that an archive watch should retry.
            let error = crate::permissions::LocationError::cancelled();
            return Err(serde_json::to_string(&error).unwrap_or(error.message));
        }
        eprintln!(
            "Kiwix mobile admission rejected {command}: active={} desired_active={} inactive={} background={}",
            state.active, state.desired_active, state.inactive, state.background.is_some()
        );
        Err(crate::user_error::suspended())
    }
    pub(crate) async fn mobile_call(&self, command: &'static str, payload: Value) -> Result<Value, String> {
        let app = self.app.lock().map_err(|_| "Application handle unavailable")?.clone().ok_or("Mobile native adapter is unavailable")?;
        app.kiwix_mobile().call(command, payload).await
    }
    /// Android 17 (API 37) blocks every local-network socket until the
    /// ACCESS_LOCAL_NETWORK runtime permission is granted. Ask once before the
    /// first LAN command; older releases and an existing NEARBY_DEVICES grant
    /// resolve immediately without a sheet. The gate is held only for the
    /// request so concurrent commands cannot replace its ActivityResult callback.
    #[cfg(target_os = "android")]
    async fn mobile_local_network_request(&self, prompt: bool) -> Result<Value, String> {
        tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.mobile_call("localNetworkPermission", json!({"prompt": prompt})),
        ).await.map_err(|_| "LOCAL_NETWORK_PERMISSION_TIMEOUT".to_string())?
    }
    #[cfg(target_os = "android")]
    fn mobile_local_network_finish(&self, reply: &Value) -> Result<(), String> {
        if reply.get("granted").and_then(Value::as_bool) == Some(true) {
            // Android delivers the permission result before onResume. A
            // suspended error is retried by the API boundary on the foreground
            // gate, so the approved command resumes instead of failing on the
            // stale inactive flag the sheet just set.
            if !self.mobile_active() {
                return Err(crate::user_error::suspended());
            }
            Ok(())
        } else {
            Err("LOCAL_NETWORK_PERMISSION_DENIED".into())
        }
    }
    #[cfg(target_os = "android")]
    pub(super) async fn mobile_local_network_admit(&self) -> Result<(), String> {
        let _gate = self.mobile.local_network_gate.lock().await;
        let reply = self.mobile_local_network_request(true).await?;
        self.mobile_local_network_finish(&reply)
    }
    /// Automatic private-sync status polls reopen the LAN listener but must
    /// not launch a system sheet every two seconds after a denial. Prompt at
    /// most once per process; explicit user actions use the interactive path.
    #[cfg(target_os = "android")]
    pub(super) async fn mobile_local_network_admit_passive(&self) -> Result<(), String> {
        let _gate = self.mobile.local_network_gate.lock().await;
        let mut reply = self.mobile_local_network_request(false).await?;
        if reply.get("granted").and_then(Value::as_bool) != Some(true)
            && !self.mobile.local_network_prompted.swap(true, Ordering::SeqCst)
        {
            reply = self.mobile_local_network_request(true).await?;
        }
        self.mobile_local_network_finish(&reply)
    }
    pub(super) async fn mobile_documents(&self, command: &'static str, payload: Value) -> Result<Value, String> {
        let result = self.mobile_call(command, payload).await;
        mobile::emit(self, "kiwix-mobile-document-progress", json!({"operation":command,"bytes":0,"done":true}));
        result
    }
    pub(super) fn mobile_labels(&self, title: &str) -> Result<Value, String> {
        let language = self.store.settings()?.locale;
        let locale = crate::localization::Localizer::for_app(&language);
        Ok(json!({"title":locale.text(title,&[]),"cancel":locale.text("Cancel",&[]),"copying":locale.text("Copying documents…",&[])}))
    }
    /// A visible, resumed Activity is interactive again. Android does not
    /// always deliver a matching resume after a transient system dialog, so a
    /// stale "inactive" flag would keep every user command suspended.
    #[cfg(target_os = "android")]
    pub(crate) fn mobile_note_interactive(&self) {
        if let Ok(mut state) = self.mobile.state.lock() {
            if state.active && state.desired_active {
                state.inactive = false;
            }
        }
    }
    /// An obscuring Activity transition closes interaction, not the current
    /// document's finite durability ownership or foreground lifetime.
    #[cfg(target_os = "android")]
    pub(crate) fn mobile_note_inactive(&self) {
        if let Ok(mut state) = self.mobile.state.lock() {
            state.inactive = true;
        }
        self.retire_private_sync();
    }
    pub(super) fn mobile_note_retirement(&self) -> Result<(), String> {
        self.retire_private_sync();
        let mut state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
        state.retirement_revision = state.retirement_revision.checked_add(1).ok_or("Mobile retirement revision exhausted")?;
        Ok(())
    }
    /// The active reader's user-started media surface. Only iOS needs this: its
    /// audio background mode keeps the process alive, so the reader server and
    /// privacy generation must survive a suspension while media is audible or
    /// a PiP presentation is live.
    #[cfg(target_os = "ios")]
    pub(super) fn mobile_reader_media_retains(&self) -> bool {
        if self.mobile_terminal() { return false; }
        if !(self.mobile.reader_audible.load(Ordering::SeqCst)
            || self.mobile.reader_picture_in_picture.load(Ordering::SeqCst))
        { return false; }
        self.store.settings().is_ok_and(|settings| !settings.private_mode && !settings.read_only)
    }
    /// The trusted root shell reports the reader's audible/PiP state. This both
    /// drives the native playback session and decides whether a background
    /// transition may retain the reader document. `release` means the reader
    /// surface itself left the screen, so the session the first audible
    /// surface latched must be dropped with it.
    #[cfg(target_os = "ios")]
    pub(crate) async fn mobile_reader_media_state(self: Arc<Self>, args: Value) -> Result<Value, String> {
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
        self.mobile_root(label)?;
        let audible = args.get("audible").and_then(Value::as_bool).ok_or("Invalid reader media state")?;
        let picture_in_picture = args.get("pictureInPicture").and_then(Value::as_bool).ok_or("Invalid reader media state")?;
        let release = args.get("release").and_then(Value::as_bool).unwrap_or(false);
        self.mobile.reader_audible.store(audible, Ordering::SeqCst);
        self.mobile.reader_picture_in_picture.store(picture_in_picture, Ordering::SeqCst);
        let settings = self.store.settings()?;
        let mut active = (audible || picture_in_picture)
            && !settings.private_mode && !settings.read_only && !self.mobile_terminal();
        let release = release || settings.private_mode || settings.read_only || self.mobile_terminal();
        if let Err(error) = self.mobile_call("readerAudioState", json!({"active": active, "release": release})).await {
            // Never retain a document for a session the system refused; the
            // ordinary suspension path retires it instead.
            eprintln!("Kiwix reader audio session: {error}");
            self.mobile.reader_audible.store(false, Ordering::SeqCst);
            self.mobile.reader_picture_in_picture.store(false, Ordering::SeqCst);
            active = false;
        }
        if !active {
            let service = self.clone();
            tauri::async_runtime::spawn(async move { service.mobile_media_released().await; });
        }
        Ok(json!({"active": active}))
    }
    /// Native audio interruptions, output-route loss and failed activation are
    /// terminal for the retained reader even when the web content is throttled
    /// and cannot deliver its own pause report.
    #[cfg(target_os = "ios")]
    pub(crate) fn mobile_audio_event(self: &Arc<Self>, payload: &Value) {
        let Some(event) = payload.get("event").and_then(Value::as_str) else { return; };
        if !matches!(event, "interruption-began" | "route-unavailable" | "session-failed") { return; }
        if let Some(id) = payload.get("sessionId").and_then(Value::as_str) {
            let action = if event == "session-failed" { "stop" } else { "pause" };
            self.mobile_speech_action(json!({"action": action, "sessionId": id}));
        }
        self.mobile.reader_audible.store(false, Ordering::SeqCst);
        self.mobile.reader_picture_in_picture.store(false, Ordering::SeqCst);
        let service = self.clone();
        tauri::async_runtime::spawn(async move { service.mobile_media_released().await; });
    }
    /// iOS retains reader origins while audible media or PiP holds the document.
    /// When that surface ends in the background, retire them exactly once so the
    /// next foreground snapshot cannot reuse the authority.
    #[cfg(target_os = "ios")]
    pub(in crate::service) async fn mobile_media_released(self: Arc<Self>) {
        if self.mobile_active() || self.mobile_reader_media_retains() || self.mobile_speech_retained() { return; }
        let _serial = self.mobile.serial.lock().await;
        if self.mobile_active() || self.mobile_reader_media_retains() || self.mobile_speech_retained() { return; }
        if !self.mobile.media_retained.swap(false, Ordering::SeqCst) { return; }
        let _mode = self.privacy_gate.write().await;
        self.privacy_generation.fetch_add(1, Ordering::SeqCst);
        if let Err(error) = retire_readers(&mut *self.readers.lock().await, None).await {
            mobile::error(&self, "media-reader-retirement", error);
        }
        if let Err(error) = self.mobile_note_retirement() {
            mobile::error(&self, "media-retirement", error);
        }
        if let Ok(mut archives) = self.archives.lock() { archives.clear(); }
        let spelling = self.spelling.clone();
        if let Err(error) = blocking(move || spelling.set_writable(false)).await {
            mobile::error(&self, "media-spelling-retirement", error);
        }
    }
    pub(super) async fn mobile_export(&self, path: &Path, mime: &str, name: &str, open: bool, share: bool) -> Result<(), String> {
        self.mobile_export_authorized(path, mime, name, open, share, None).await
    }
    async fn mobile_export_authorized(&self, path: &Path, mime: &str, name: &str, open: bool, share: bool, installation: Option<(&str, &str, &str)>) -> Result<(), String> {
        use futures_util::FutureExt;
        let prepared = if open { None } else { Some(crate::library_files::MobileExport::pin(self.store.data_dir(), path)?) };
        // iOS uses UIActivityViewController for `open`; Android distinguishes
        // the system viewer (ACTION_VIEW) from the share sheet (ACTION_SEND).
        let title = if share { "Share" } else { "Save export" };
        let payload = json!({"path":path,"mimeType":mime,"suggestedName":name,"open":open,"share":share,"labels":self.mobile_labels(title)?});
        let mut request = Box::pin(self.mobile_documents("exportFile", payload));
        // The plugin dispatches the native command on its first poll. Fence
        // that handoff with the current installation, then release the store
        // while the user picks a destination for the admitted byte snapshot.
        let immediate = if let Some((window, book, installation)) = installation {
            match self.with_export_installation(window, book, installation, || Ok(request.as_mut().now_or_never())) {
                Ok(result) => result,
                Err(error) => {
                    if let Some(owned) = prepared { blocking(move || owned.cleanup()).await?; }
                    return Err(error);
                }
            }
        } else { None };
        let result = match immediate { Some(result) => result, None => request.await }
            .map_err(|error| if prepared.is_some() { format!("{error}. Prepared export retained at {}", path.display()) } else { error })?;
        result.get("cancelled").and_then(Value::as_bool).ok_or_else(|| if prepared.is_some() {
            format!("Native export did not return a completion state; prepared export retained at {}", path.display())
        } else {
            "Native export did not return a completion state".to_owned()
        })?;
        if let Some(prepared) = prepared {
            blocking(move || prepared.cleanup()).await.map_err(|error| format!("Native export completed, but owned temporary cleanup failed: {error}"))?;
        }
        Ok(())
    }
    fn mobile_staging(&self) -> Result<PathBuf, String> {
        let archives = crate::library_files::mobile_archive_directory(self.store.data_dir())?;
        crate::library_files::mobile_directory(&archives, "mobile-imports-v1")
    }
    #[cfg(target_os = "ios")]
    pub(crate) fn mobile_queue_incoming(&self, uris: Vec<String>) -> Result<(), String> {
        self.mobile_queue_request(Uuid::new_v4().to_string(), None, uris)
    }
    #[cfg(target_os="android")]
    pub(crate) fn mobile_queue_native_incoming(&self, id:&str, uris:Vec<String>)->Result<(),String> {
        if Uuid::parse_str(id).map_err(|_| "Native incoming batch ID is invalid")?.to_string()!=id {
            return Err("Native incoming batch ID must be canonical".into());
        }
        self.mobile_queue_request(id.to_string(),Some(id.to_string()),uris)
    }
    #[cfg(target_os="android")]
    pub(crate) async fn mobile_drain_native_entries(&self)->Result<(),String> {
        let policy = self.privacy_policy_generation.load(Ordering::SeqCst);
        let result = self.mobile_call("takeAppEntries",json!({})).await?;
        let entries = result.get("entries").and_then(Value::as_array).ok_or("Native app entry queue is invalid")?;
        // A pre-revocation native read cannot reinsert a retired action after
        // the privacy writer has cleared the Rust/native queues.
        let _mode = self.privacy_gate.read().await;
        for entry in entries {
            let id = text(entry,"requestId")?;
            let kind = text(entry,"kind")?;
            if kind == "download-action" && (policy != self.privacy_policy_generation.load(Ordering::SeqCst)
                || self.normal_writable().is_err()) { continue; }
            if kind == "article" {
                // Pinned shortcuts carry the same zim:// form as external links.
                // Validate and canonicalize before the shell can open a tab; a
                // malformed link becomes a library notice under the same ID so
                // the native queue is still acknowledged.
                match normalize_article_entry(text(entry,"value")?) {
                    Ok(value) => self.mobile_queue_native_entry(id,"article",value)?,
                    Err(message) => self.mobile_queue_native_entry(id,"library-notice",message)?,
                }
                continue;
            }
            self.mobile_queue_native_entry(id,kind,text(entry,"value")?.to_owned())?;
        }
        Ok(())
    }
    /// Queue one platform entry point (PROCESS_TEXT, kiwix:// link, shortcut).
    /// The payload is an opaque user-supplied query or an empty string; it
    /// carries no authority, so the shell still applies privacy and lifecycle gates.
    #[cfg(target_os = "android")]
    fn mobile_queue_native_entry(&self, id:&str, kind:&str, value:String)->Result<(),String> {
        if Uuid::parse_str(id).map_err(|_| "Native app entry ID is invalid")?.to_string()!=id {
            return Err("Native app entry ID must be canonical".into());
        }
        super::mobile_policy::validate_native_entry(kind, &value)?;
        {
            let mut state = self.mobile.state.lock().map_err(|_| "App entry queue unavailable")?;
            if let Some(existing)=state.entries.iter().find(|entry|entry.id==id) {
                if existing.kind!=kind || existing.value!=value { return Err("Native app entry changed its immutable payload".into()); }
                return Ok(());
            }
            if state.entries.len() >= 32 { return Err("Handle pending app entry requests before opening more".into()); }
            state.entries.push_back(EntryRequest { id:id.into(), kind:kind.into(), value });
        }
        mobile::emit(self, "kiwix-mobile-entry", json!({"requestId":id,"kind":kind}));
        Ok(())
    }
    async fn mobile_acknowledge_entry(self:&Arc<Self>,id:&str)->Result<(),String> {
        let known = self.mobile.state.lock().map_err(|_| "App entry queue unavailable")?
            .entries.iter().any(|entry|entry.id==id);
        // A repeated acknowledgement is harmless; only the native copy is removed.
        if !known { return Ok(()); }
        #[cfg(target_os="android")]
        self.mobile_call("acknowledgeAppEntry",json!({"requestId":id})).await?;
        self.mobile.state.lock().map_err(|_| "App entry queue unavailable")?.entries.retain(|entry|entry.id!=id);
        #[cfg(target_os="android")]
        {
            // The command still owns its privacy read lease. Drain separately
            // so a queued policy writer cannot deadlock a nested fair read.
            let service=self.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error)=mobile::drain_entries(service.clone()).await {mobile::error(&service,"entry",error);}
            });
        }
        Ok(())
    }
    fn mobile_queue_request(&self,id:String,native_id:Option<String>,uris:Vec<String>)->Result<(),String> {
        if uris.is_empty() { return Ok(()); }
        if uris.len() > 1024 || uris.iter().any(|uri| uri.len() > 65536) { return Err("Too many incoming documents or an oversized document identifier".into()); }
        for uri in &uris {
            let parsed = url::Url::parse(uri).map_err(|_| "Invalid incoming document identifier")?;
            if !((cfg!(target_os="ios") && parsed.scheme()=="file") || (cfg!(target_os="android") && parsed.scheme()=="content")) {
                return Err("Incoming document scheme is not supported by this native provider".into());
            }
        }
        let count = uris.len();
        {
            let mut state = self.mobile.state.lock().map_err(|_| "Incoming queue unavailable")?;
            if let Some(existing)=state.incoming.iter().find(|request|request.id==id) {
                if existing.uris!=uris || existing.native_id!=native_id { return Err("Native incoming batch changed its immutable source identifiers".into()); }
                return Ok(());
            }
            if state.incoming.len() >= 32 { return Err("Confirm or dismiss pending incoming archives before opening more documents".into()); }
            state.incoming.push_back(IncomingRequest { id:id.clone(), uris, native_id });
        }
        mobile::emit(self, "kiwix-mobile-incoming", json!({"requestId":id,"count":count}));
        Ok(())
    }
    async fn mobile_acknowledge_incoming(self:&Arc<Self>,id:&str)->Result<(),String> {
        let request={
            let state=self.mobile.state.lock().map_err(|_| "Incoming queue unavailable")?;
            let request=state.incoming.iter().find(|request|request.id==id).ok_or("Unknown incoming document request")?;
            #[cfg(target_os="android")]
            let authority=request.native_id.clone();
            #[cfg(target_os="ios")]
            let authority=request.uris.clone();
            authority
        };
        #[cfg(target_os="android")]
        if let Some(native_id)=request {
            self.mobile_call("acknowledgeIncomingDocuments",json!({"requestId":native_id})).await?;
        }
        #[cfg(target_os="ios")]
        self.mobile_call("releaseIncomingOriginals",json!({"uris":request})).await?;
        self.mobile.state.lock().map_err(|_| "Incoming queue unavailable")?.incoming.retain(|request|request.id!=id);
        #[cfg(target_os="android")]
        if let Err(error)=mobile::drain_incoming(self.clone()).await {mobile::error(self,"incoming",error);}
        Ok(())
    }
    async fn mobile_provider_book(reference: crate::provider_reference::ProviderReference) -> Result<Book, String> {
        reference.validate()?;
        blocking(move || {
            let retained = crate::provider_reference::retain_registered(&reference)?;
            let archive = retained.open()?;
            Ok(Book {
                metadata: archive.metadata()?,
                installation_id: crate::store::new_installation_id(),
                path: reference.path_marker(),
                added_at: crate::store::now(),
                available: true,
                managed: false,
                source_reference: Some(reference),
                integrity: None,
            })
        }).await
    }
    async fn mobile_open_external(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        let protected = self.store.private_mode()? || self.store.read_only()?;
        let replacement = match args.get("replaceId") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => {
                let book = self.store.book(id)?.ok_or("Archive to reconnect is no longer in the library")?;
                if protected && book.source_reference.as_ref().is_none_or(|source| source.persistent) {
                    return Err("Leave private or read-only mode before replacing a saved external archive reference".into());
                }
                Some(book)
            }
            _ => return Err("Invalid archive replacement identifier".into()),
        };
        let folder = args.get("folder").and_then(Value::as_bool).unwrap_or(false);
        let result = self.mobile_documents("pickExternalArchive", json!({
            "persistent":!protected, "folder":folder,
            "labels":self.mobile_labels("Open external archive")?
        })).await?;
        if result.get("cancelled").and_then(Value::as_bool)
            .ok_or("Native picker did not return cancellation state")? {
            return Ok(Value::Null);
        }
        let reference: crate::provider_reference::ProviderReference =
            serde_json::from_value(result.get("selection").cloned()
                .ok_or("Native external archive selection missing")?)
                .map_err(|error| error.to_string())?;
        reference.validate()?;
        let mut committed = false;
        // The accepted picker owns dispatch's privacy read lease. Its system
        // activity can suspend this app; foreground activation needs that same
        // lease's writer. Finish this accepted operation before retirement,
        // rather than requiring the foreground transition it currently blocks.
        let admission = async {
            self.ensure_generation(args)?;
            if reference.persistent == protected {
                return Err("Close this file and open it again after changing privacy settings".into());
            }
            let mut book = Self::mobile_provider_book(reference.clone()).await?;
            self.ensure_generation(args)?;
            if let Some(old) = replacement {
                if book.metadata.id != old.metadata.id {
                    return Err("Selected file is a different archive; open it separately".into());
                }
                let _source_guards = self.localswarm_source_mutation(Some(&old.metadata.id)).await?;
                let mut readers = self.readers.lock().await;
                self.ensure_generation(args)?;
                {
                let _window = self.active_window_guard("main")?;
                book.added_at = old.added_at.clone();
                if protected {
                    self.store.replace_provider_book(&old.installation_id, book.clone())?;
                } else {
                    self.store.update(|state| {
                        let current = state.books.iter_mut().find(|current|
                            current.metadata.id == old.metadata.id &&
                            current.installation_id == old.installation_id)
                            .ok_or("Archive changed while reconnecting")?;
                        *current = book.clone();
                        state.reader_storage.remove(&old.installation_id);
                        Ok(())
                    })?;
                }
                committed = true;
                self.retire_searches(None);
                }
                retire_readers(&mut readers, Some(&old.installation_id)).await?;
                self.forget_installation(&old.installation_id);
                self.invalidate(&old.metadata.id);
                drop(readers);
                self.notify_library_changed();
                if let Some(previous) = old.source_reference {
                    if !self.store.retains_provider_reference(&previous.reference)? {
                        blocking(move || crate::provider_reference::forget_registered(&previous)).await?;
                    }
                }
                return value(book);
            }
            let _window = self.active_window_guard("main")?;
            if protected {
                self.store.register_provider_book(book.clone())?;
                self.notify_library_changed();
            } else {
                self.register_books(std::slice::from_ref(&book), None)?;
            }
            committed = true;
            value(book)
        }.await;
        match admission {
            Err(error) if !committed && !self.store.retains_provider_reference(&reference.reference)? => {
                blocking(move || crate::provider_reference::forget_registered(&reference)).await
                    .map_err(|cleanup| format!("{error}; releasing external selection failed: {cleanup}"))?;
                Err(error)
            }
            result => result,
        }
    }
    /// Admit only native-registered reading authority. A repeated Add/Open With
    /// returns the current installation without replacing its source or state.
    async fn mobile_admit_original(self: &Arc<Self>, args: &Value,
        reference: crate::provider_reference::ProviderReference) -> Result<Book, String> {
        let mut committed = false;
        let admission: Result<Book, String> = async {
            self.ensure_generation(args)?;
            reference.validate()?;
            let protected = self.store.private_mode()? || self.store.read_only()?;
            if reference.persistent == protected {
                return Err("Close this file and open it again after changing privacy settings".into());
            }
            let book = Self::mobile_provider_book(reference.clone()).await?;
            self.ensure_generation(args)?;
            let _window = self.active_window_guard("main")?;
            if let Some(existing) = self.store.book(&book.metadata.id)? { return Ok(existing); }
            if protected {
                self.store.register_provider_book(book.clone())?;
                self.notify_library_changed();
            } else {
                self.register_books(std::slice::from_ref(&book), None)?;
            }
            committed = true;
            Ok(book)
        }.await;
        if !committed && !self.store.retains_provider_reference(&reference.reference)? {
            if let Err(cleanup) = blocking(move ||
                crate::provider_reference::forget_registered(&reference)).await {
                return Err(match admission {
                    Ok(_) => cleanup,
                    Err(error) => format!("{error}; releasing external selection failed: {cleanup}"),
                });
            }
        }
        admission
    }
    async fn mobile_add_external(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        let protected = self.store.private_mode()? || self.store.read_only()?;
        let folder = args.get("folder").and_then(Value::as_bool).unwrap_or(false);
        let result = self.mobile_documents("pickExternalArchives", json!({
            "persistent":!protected, "folder":folder,
            "labels":self.mobile_labels(if folder {"Add archive folder"} else {"Add files"})?
        })).await?;
        if result.get("cancelled").and_then(Value::as_bool)
            .ok_or("Native picker did not return cancellation state")? {
            return Ok(Value::Null);
        }
        let references: Vec<crate::provider_reference::ProviderReference> =
            serde_json::from_value(result.get("selections").cloned()
                .ok_or("Native original archive selections missing")?)
                .map_err(|error| error.to_string())?;
        let mut errors = result.get("errors").and_then(Value::as_array)
            .ok_or("Native original archive errors missing")?.clone();
        let mut books = Vec::new();
        for reference in references {
            let path = reference.display_name.clone();
            match self.mobile_admit_original(args, reference).await {
                Ok(book) => books.push(book),
                Err(error) => errors.push(json!({"path":path,"error":error})),
            }
        }
        if books.is_empty() && errors.is_empty() {
            errors.push(json!({"path":"","error":"No complete ZIM archives were found in the selected location"}));
        }
        Ok(json!({"books":books.iter().map(Book::shell_value).collect::<Vec<_>>(),"errors":errors}))
    }
    async fn mobile_open_incoming(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        let protected = self.store.private_mode()? || self.store.read_only()?;
        let _incoming = self.mobile.incoming_gate.lock().await;
        let id = text(args, "requestId")?;
        let (uris, native_id) = {
            let state = self.mobile.state.lock().map_err(|_| "Incoming queue unavailable")?;
            let request = state.incoming.iter().find(|request| request.id == id)
                .ok_or("Unknown or already handled incoming document request")?;
            (request.uris.clone(), request.native_id.clone())
        };
        let receipt = native_id.map(|id| format!("native-incoming:{id}"));
        if !protected && receipt.as_ref().map(|receipt| self.store.acquisition_consumed(receipt))
            .transpose()?.unwrap_or(false) {
            self.mobile_acknowledge_incoming(id).await?;
            return Ok(json!({"books":[],"errors":[]}));
        }
        let mut books = Vec::new();
        let mut errors = Vec::new();
        for uri in uris {
            // Only the immutable native intake queue supplies the URI. A web
            // caller cannot mint provider authority from an arbitrary path.
            let selected = self.mobile_documents("openIncomingArchive", json!({
                "uri":uri, "persistent":!protected, "labels":self.mobile_labels("Open external archive")?
            })).await;
            let reference = match selected.and_then(|result| {
                serde_json::from_value::<crate::provider_reference::ProviderReference>(
                    result.get("selection").cloned().ok_or("Native original selection missing")?)
                    .map_err(|error| error.to_string())
            }) {
                Ok(reference) => reference,
                Err(error) => {
                    errors.push(json!({"path":uri,"error":error}));
                    continue;
                }
            };
            match self.mobile_admit_original(args, reference).await {
                Ok(book) => books.push(book),
                Err(error) => errors.push(json!({"path":uri,"error":error})),
            }
        }
        if errors.is_empty() {
            if !protected {
                if let Some(receipt) = receipt {
                    self.store.update(|state| {
                        state.consumed_acquisitions.insert(receipt);
                        Ok(())
                    })?;
                }
            }
            self.mobile_acknowledge_incoming(id).await?;
        }
        Ok(json!({"books":books.iter().map(Book::shell_value).collect::<Vec<_>>(),"errors":errors}))
    }
    /// File names already represented in the library, used only for the
    /// advisory "in library" marker on scan results.
    fn library_file_names(&self) -> Result<Vec<String>, String> {
        Ok(self.store.read()?.books.iter().filter_map(|book| {
            book.source_reference.as_ref().map(|source| source.display_name.clone()).or_else(|| {
                Path::new(&book.path).file_name().and_then(|name| name.to_str()).map(str::to_owned)
            })
        }).collect())
    }
    /// App-owned library locations exposed by the native shell. Android lists
    /// internal storage and every app-specific external directory (including a
    /// removable card) without granting a raw shared-storage path.
    async fn mobile_storage_locations(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        let response = self.mobile_call("storageLocations", json!({})).await?;
        let native = crate::library_root::parse_native(&response)?;
        value(crate::library_root::locations(&self.store, &native)?)
    }
    /// Copy the managed library to a chosen app-owned root and switch atomically.
    /// The native list is re-read here so a caller cannot invent a filesystem
    /// path that the platform did not expose for this app.
    async fn mobile_library_root_set(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        if self.store.private_mode()? {
            return Err("Private mode cannot copy or move the library".into());
        }
        if self.store.read_only()? {
            return Err("Read-only mode cannot change the library location".into());
        }
        let target = PathBuf::from(text(args, "root")?);
        let remove_old = args
            .get("removeOld")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let response = self.mobile_call("storageLocations", json!({})).await?;
        let native = crate::library_root::parse_native(&response)?;
        self.ensure_generation(args)?;
        let activity = self.mobile_storage_activity().await?;
        let _migration = MigrationActivity::begin(&self.migration_active);
        let _sources = self.localswarm_source_mutation(None).await?;
        let mut readers = self.readers.lock().await;
        retire_readers(&mut readers, None).await?;
        let store = self.store.clone();
        let result = blocking(move || {
            crate::library_root::switch(&store, &target, remove_old, &native, &activity)
        })
        .await;
        self.archives
            .lock()
            .map_err(|_| "Archive cache unavailable")?
            .clear();
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
        value(result?)
    }
    async fn mobile_storage_activity(&self) -> Result<crate::library_root::Activity, String> {
        let transfers = self.download_manager().await?.list()?.into_iter()
            .map(|job| crate::library_root::TransferTarget {
                destination: job.destination,
                status: job.status,
            }).collect();
        let map_busy = self.map_manager()?.status()?.is_some_and(|job| {
            matches!(job.status.as_str(), "queued" | "downloading" | "composing" | "validating" | "paused")
        });
        Ok(crate::library_root::Activity { transfers, map_busy })
    }
    /// Selected relocation keeps the installation principal and browsing
    /// storage intact. Dispatch already owns updater/transfer/privacy admission.
    #[cfg(target_os = "android")]
    pub(super) async fn mobile_library_files_relocate(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
        self.mobile_root(label)?;
        self.mobile_admit("library_files_relocate", args)?;
        self.ensure_generation(args)?;
        self.normal_writable()?;
        if args.get("confirm") != Some(&Value::Bool(true)) {
            return Err("Copying or moving archives requires explicit confirmation".into());
        }
        let book_ids = strings(args, "bookIds")?;
        let target = PathBuf::from(text(args, "target")?);
        let remove_old = args.get("removeOld").and_then(Value::as_bool).unwrap_or(false);
        let response = self.mobile_call("storageLocations", json!({})).await?;
        let native = crate::library_root::parse_native(&response)?;
        self.ensure_generation(args)?;
        self.mobile_admit("library_files_relocate", args)?;
        let activity = self.mobile_storage_activity().await?;
        let _migration = MigrationActivity::begin(&self.migration_active);
        let _sources = self.localswarm_source_mutation(None).await?;
        let mut selected = Vec::with_capacity(book_ids.len());
        for id in &book_ids {
            if self.active_torrent_share(id)?.is_some() {
                return Err("Stop sharing the selected archive before copying or moving it".into());
            }
            selected.push(self.store.book(id)?.ok_or("Selected archive is no longer in the library")?);
        }
        let mut readers = self.readers.lock().await;
        for book in &selected {
            retire_readers(&mut readers, Some(&book.installation_id)).await?;
        }
        let store = self.store.clone();
        let result = blocking(move || {
            crate::library_root::relocate_selected(&store, &target, &book_ids, remove_old, &native, &activity)
        }).await;
        // A directory-sync failure can mean the Store already adopted the
        // destination. Retire caches and publish authoritative state on either
        // outcome, without forgetting this installation's reader storage.
        for book in &selected { self.invalidate(&book.metadata.id); }
        drop(readers);
        self.notify_library_changed();
        result
    }
    /// Android device-wide discovery. Native code enumerates MediaStore rows and
    /// every persisted SAF tree grant; Rust validates and deduplicates by
    /// physical identity. Nothing is persisted and no network is touched.
    async fn mobile_device_scan(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        if self.store.private_mode()? {
            return Err("Device scanning is unavailable in private mode. Leave private mode to look for ZIM files on this device.".into());
        }
        let response = self.mobile_call("scanDeviceArchives", json!({"maxEntries":512})).await?;
        let known: std::collections::HashSet<String> = self.library_file_names()?
            .into_iter().map(|name| name.to_ascii_lowercase()).collect();
        value(crate::device_scan::scan(response, &known)?)
    }
    /// Open a scanned archive through the existing retained-descriptor path.
    /// A readable SAF tree yields a persistable reference in normal mode; a
    /// MediaStore-only authority is session-scoped and is admitted only in
    /// read-only mode. The archive is never copied.
    async fn mobile_device_open_found(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        if self.store.private_mode()? {
            return Err("Device scanning is unavailable in private mode. Leave private mode to open a ZIM file from this device.".into());
        }
        let token = text(args, "token")?;
        crate::device_scan::validate_token(token)?;
        let protected = self.store.read_only()?;
        let result = self.mobile_call("openScannedArchive", json!({
            "token":token, "persistent":!protected,
        })).await?;
        if result.get("cancelled").and_then(Value::as_bool).unwrap_or(false) { return Ok(Value::Null); }
        let reference: crate::provider_reference::ProviderReference = serde_json::from_value(
            result.get("selection").cloned().ok_or("Native scanned archive selection missing")?)
            .map_err(|error| error.to_string())?;
        reference.validate()?;
        let rollback_reference = reference.clone();
        let mut committed = false;
        let admission = async {
            self.ensure_generation(args)?;
            let source = reference.clone();
            let book = blocking(move || {
                let retained = crate::provider_reference::retain_registered(&source)?;
                let archive = retained.open()?;
                Ok(Book {
                    metadata: archive.metadata()?,
                    installation_id: crate::store::new_installation_id(),
                    path: source.path_marker(),
                    added_at: crate::store::now(),
                    available: true,
                    managed: false,
                    source_reference: Some(source),
                    integrity: None,
                })
            }).await?;
            self.ensure_generation(args)?;
            // The same archive may already be installed from an earlier scan or
            // an explicit import. Never create a second installation for it.
            if let Some(existing) = self.store.read()?.books.iter()
                .find(|current| current.metadata.id == book.metadata.id).cloned() {
                committed = true;
                if !self.store.retains_provider_reference(&reference.reference)? {
                    blocking(move || crate::provider_reference::forget_registered(&reference)).await?;
                }
                return value(existing);
            }
            let _window = self.active_window_guard("main")?;
            if protected {
                self.store.register_provider_book(book.clone())?;
                self.notify_library_changed();
            } else {
                self.register_books(std::slice::from_ref(&book), None)?;
            }
            committed = true;
            value(book)
        }.await;
        match admission {
            Err(error) if !committed && !self.store.retains_provider_reference(&rollback_reference.reference)? => {
                blocking(move || crate::provider_reference::forget_registered(&rollback_reference)).await
                    .map_err(|cleanup| format!("{error}; releasing scanned archive selection failed: {cleanup}"))?;
                Err(error)
            }
            result => result,
        }
    }
    /// Persist one SAF tree grant so the next scan can see its files. This is a
    /// read grant only; it never copies or imports anything.
    async fn mobile_device_grant_folder(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        self.ensure_generation(args)?;
        if self.store.private_mode()? {
            return Err("Folder access cannot be saved in private mode. Leave private mode to scan this device.".into());
        }
        let result = self.mobile_documents("pickScanFolder", json!({
            "persistent":!self.store.read_only()?,
            "labels":self.mobile_labels("Add archive folder")?
        })).await?;
        if result.get("cancelled").and_then(Value::as_bool)
            .ok_or("Native folder picker did not return cancellation state")? {
            return Ok(Value::Null);
        }
        value(result)
    }
    pub(super) async fn mobile_clear_external_books(&self) -> Result<(), String> {
        let books = self.store.clear_provider_books()?;
        for book in &books {
            self.forget_installation(&book.installation_id);
            self.invalidate(&book.metadata.id);
        }
        let store = self.store.clone();
        blocking(move || {
            let mut errors = Vec::new();
            for book in books {
                if let Some(reference) = book.source_reference {
                    if store.retains_provider_reference(&reference.reference)? { continue; }
                    if let Err(error) = crate::provider_reference::forget_registered(&reference) {
                        errors.push(error);
                    }
                }
            }
            if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
        }).await
    }
    /// Consume a reconciled connectivity frontier, then decide from current
    /// settings and lifecycle only after acquiring the admission gates.
    pub(crate) async fn mobile_resume_after_network(&self) -> Result<(), String> {
        let Some(revision) = crate::service::mobile_policy::take_readmission() else { return Ok(()); };
        #[cfg(target_os = "android")]
        self.mobile_reconcile_transfer_events().await?;
        let _update = self.update_admission.admit().await;
        let _transfer = self.mobile.transfer_gate.lock().await;
        let _mode = self.privacy_gate.read().await;
        if !self.mobile_reconnect_admitted(revision)? { return Ok(()); }
        #[cfg(target_os = "android")]
        {
            self.mobile.policy_attempt_generation.store(0, Ordering::SeqCst);
            self.mobile_resume_policy_locked(Some(revision)).await
        }
        #[cfg(target_os = "ios")]
        {
            if let Some(manager) = self.downloads.get() {
                manager.reconcile_native().await?;
            }
            let generation = self.mobile_network_admit(true).await?;
            self.mobile_network_validate(generation, true)?;
            // Native admission awaited; a synchronous background callback may
            // have retired the app even though the policy writer is excluded.
            if !self.mobile_reconnect_admitted(revision)? { return Ok(()); }
            crate::service::mobile_policy::record_resume_attempt(revision);
            let mut errors = Vec::new();
            if let Some(manager) = self.downloads.get() {
                if let Err(error) = manager.resume_interrupted_after_reconnect(
                    crate::transfer_policy::ResumeAdmission::Attempt,
                ).await { errors.push(error); }
            }
            let torrents = self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
            if let Some(manager) = torrents {
                if let Err(error) = manager.resume_interrupted_after_reconnect(
                    crate::transfer_policy::ResumeAdmission::Attempt,
                ).await { errors.push(error); }
            }
            if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
        }
    }
    fn mobile_reconnect_admitted(&self, revision: u64) -> Result<bool, String> {
        let admission = crate::transfer_policy::reconnect_resume(
            &self.store.settings()?,
            crate::service::mobile_policy::internet_admission(),
            self.mobile_active(),
            revision,
            crate::service::mobile_policy::last_resume_attempt(),
        );
        if let crate::transfer_policy::ResumeAdmission::Skip(reason) = admission {
            eprintln!("Kiwix automatic resume skipped: {}", reason.as_str());
            return Ok(false);
        }
        Ok(true)
    }
    fn mobile_import_batch(&self,paths:Vec<String>,receipt:Option<&str>)->Result<Value,String> {
        self.normal_writable()?;
        let paths=crate::library_files::mobile_primary_archives(paths)?;
        if paths.is_empty() {return Err("Incoming batch contains no complete archive".into());}
        let mut books=Vec::with_capacity(paths.len());
        let mut errors=Vec::new();
        for path in paths {
            match load_book(&path,true,false) {
                Ok(book)=>books.push(book),
                Err(error)=>errors.push(json!({"path":path,"error":error})),
            }
        }
        if !errors.is_empty() {return Ok(json!({"books":[],"errors":errors}));}
        let identities=books.iter().map(|book|(book.path.clone(),book.metadata.id.clone())).collect::<HashMap<_,_>>();
        let paths=books.iter().map(|book|book.path.clone()).collect::<Vec<_>>();
        let (books,duplicates)=match self.admit_incoming_books(books,receipt) {
            Ok(result)=>result,
            Err(error)=>return Ok(json!({"books":[],"errors":paths.into_iter().map(|path|json!({"path":path,"error":&error})).collect::<Vec<_>>()})),
        };
        for path in duplicates {
            let Some(uuid)=identities.get(&path) else { continue };
            if let Err(error)=crate::library_files::remove_duplicate_intake_copy(&path,uuid) {
                eprintln!("Kiwix mobile intake: removing duplicate copy {path} failed: {error}");
            }
        }
        self.mobile_apply_archive_backup()?;
        Ok(json!({"books":books.iter().map(Book::shell_value).collect::<Vec<_>>(),"errors":[]}))
    }
    pub(super) async fn mobile_command(self: Arc<Self>, command: &str, args: Value, label: &str) -> Result<Value, String> {
        self.mobile_root(label)?;
        match command {
            #[cfg(target_os = "ios")]
            "mobile_dynamic_type_state" => {
                return self.mobile_call("dynamicTypeState", json!({})).await;
            }
            #[cfg(target_os = "android")]
            "mobile_apply_locale" => {
                self.ensure_generation(&args)?;
                let preserve_platform = match args.get("preservePlatform") {
                    None => false,
                    Some(Value::Bool(value)) => *value,
                    Some(_) => return Err("Invalid platform locale preservation choice".into()),
                };
                return self.mobile_call("applyLocale", json!({
                    "locale": self.store.settings()?.locale,
                    "preservePlatform": preserve_platform,
                })).await;
            }
            "mobile_open_external_archive" => return self.mobile_open_external(&args).await,
            "mobile_add_external_archives" => return self.mobile_add_external(&args).await,
            "mobile_open_incoming" => return self.mobile_open_incoming(&args).await,
            "mobile_storage_locations" => return self.mobile_storage_locations(&args).await,
            "mobile_library_root_set" => return self.mobile_library_root_set(&args).await,
            "mobile_device_scan" => return self.mobile_device_scan(&args).await,
            "mobile_device_open_found" => return self.mobile_device_open_found(&args).await,
            "mobile_device_grant_folder" => return self.mobile_device_grant_folder(&args).await,
            "mobile_lifecycle_state" => {
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                return Ok(json!({"active":state.active && state.desired_active,"interactive":state.active && state.desired_active && !state.inactive && !self.stopping.load(Ordering::SeqCst),"backgroundRequestId":state.background.as_ref().filter(|request| background_checkpoint_open(&state, request)).map(|request| &request.id),"retirementRevision":state.retirement_revision,"incoming":state.incoming.iter().map(mobile::incoming_summary).collect::<Vec<_>>(),"entries":state.entries.iter().map(|entry|json!({"requestId":entry.id,"kind":entry.kind,"value":entry.value})).collect::<Vec<_>>()}));
            }
            "mobile_validate_background" | "mobile_background_ack" => {
                let id = text(&args,"requestId")?;
                let mut state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if !state.background.as_ref().is_some_and(|request| request.id == id && background_checkpoint_open(&state, request)) {
                    return Err(background_checkpoint_ended());
                }
                let request = state.background.as_mut().ok_or_else(background_checkpoint_ended)?;
                if command == "mobile_background_ack" {
                    request.acknowledged = true;
                    request.error = args.get("error").and_then(Value::as_str).map(|error| error.chars().take(4096).collect());
                    self.mobile.wake.notify_waiters();
                }
                return Ok(Value::Null);
            }
            "mobile_dismiss_incoming" => {
                // Dismissing a queued native intake request performs no
                // persistent library mutation, so it remains available in
                // private and read-only mode. Otherwise the queue could reach
                // its limit with no action the user is allowed to take.
                let _incoming = self.mobile.incoming_gate.lock().await;
                let id = text(&args,"requestId")?;
                self.mobile_acknowledge_incoming(id).await?;
                return Ok(Value::Null);
            }
            "mobile_ack_entry" => {
                // Acknowledging a consumed entry is not a persistent mutation.
                let _incoming = self.mobile.incoming_gate.lock().await;
                let id = text(&args,"requestId")?;
                self.mobile_acknowledge_entry(id).await?;
                return Ok(Value::Null);
            }
            "mobile_update_search_widget" => {
                // Widget state lives outside the library profile. Restricted
                // modes still update it so a stale title cannot remain visible.
                let mut titles = strings(&args,"titles")?;
                let restricted = args.get("privateMode").and_then(Value::as_bool).unwrap_or(false)
                    || self.store.private_mode()? || self.store.read_only()?;
                titles.truncate(8);
                let titles: Vec<String> = titles.into_iter()
                    .map(|title| title.chars().take(120).collect())
                    .collect();
                self.mobile_call("updateSearchWidget", json!({"titles":titles,"restricted":restricted})).await?;
                return Ok(Value::Null);
            }
            "mobile_pin_shortcut" => {
                // A pinned shortcut publishes an article title on the home
                // screen, so it follows the same private/read-only boundary.
                if self.store.private_mode()? || self.store.read_only()? {
                    return Err("Pinning articles is unavailable in private or read-only mode".into());
                }
                let book_id = text(&args,"bookId")?;
                let path = text(&args,"path")?;
                let title = text(&args,"title")?;
                if path.chars().count() > 3072 || path.chars().any(char::is_control) {
                    return Err("This article location cannot be pinned".into());
                }
                return self.mobile_call("pinArticleShortcut",
                    json!({"bookId":book_id,"path":path,"title":title})).await;
            }
            "mobile_voice_search_available" => {
                return self.mobile_call("speechRecognitionAvailable", json!({})).await;
            }
            "mobile_voice_search" => {
                let language = args.get("language").and_then(Value::as_str).unwrap_or("");
                return self.mobile_call("recognizeSpeech", json!({"language":language})).await;
            }
            _ => {}
        }
        // Explicit import/export retains the desktop command's writable-mode
        // policy; reading an external text document has no persistent side effect.
        if command != "mobile_read_text" { self.writable()?; }
        if args.get("privacyGeneration").is_some() { self.ensure_generation(&args)?; }
        match command {
            "mobile_pick_archive_copies" | "mobile_pick_directory_copy" => {
                self.normal_writable()?;
                let root = self.mobile_staging()?;
                let directory = command == "mobile_pick_directory_copy";
                let result = self.mobile_documents(if directory {"pickLibraryDirectoryCopy"} else {"pickArchiveCopies"}, json!({"destinationDirectory":root,"labels":self.mobile_labels(if directory {"Import a folder copy"} else {"Import a managed copy"})?})).await?;
                if result.get("cancelled").and_then(Value::as_bool).ok_or("Native picker did not return cancellation state")? { return Ok(if directory {Value::Null} else {json!([])}); }
                let paths = strings(&result,"paths")?;
                crate::library_files::mobile_import_paths(&root, paths.iter().map(String::as_str))?;
                if directory {
                    let returned = text(&result,"directory")?;
                    if Path::new(returned) != root { return Err("Native directory picker returned an unexpected owned directory".into()); }
                    value(returned)
                } else { value(crate::library_files::mobile_primary_archives(paths)?) }
            }
            "mobile_read_text" => {
                let result = self.mobile_documents("readTextDocument",json!({"maxBytes":MAX_EXPORT,"labels":self.mobile_labels("Import document")?})).await?;
                if result.get("cancelled").and_then(Value::as_bool).ok_or("Native reader did not return cancellation state")? { return Ok(Value::Null); }
                let text = text(&result,"text")?;
                if text.len() as u64 > MAX_EXPORT { return Err("Import document exceeds 16 MiB".into()); }
                value(text)
            }
            "mobile_save_text" | "mobile_save_bytes" => {
                let name = text(&args,"suggestedName")?.to_string();
                let (bytes,mime) = if command == "mobile_save_text" {
                    let text = text(&args,"text")?;
                    if text.len() as u64 > MAX_EXPORT { return Err("Text export exceeds 16 MiB".into()); }
                    (text.as_bytes().to_vec(), if name.to_ascii_lowercase().ends_with(".json") {"application/json"} else if name.to_ascii_lowercase().ends_with(".xml") {"application/xml"} else {"text/plain"}.to_string())
                } else {
                    let encoded = text(&args,"bytes")?;
                    if encoded.len() as u64 > MAX_EXPORT.div_ceil(3)*4 { return Err("Byte export exceeds 16 MiB; use streaming resource export".into()); }
                    let bytes = STANDARD.decode(encoded).map_err(|e| e.to_string())?;
                    if bytes.len() as u64 > MAX_EXPORT { return Err("Byte export exceeds 16 MiB".into()); }
                    let mime = text(&args,"mime")?;
                    if mime.is_empty() || mime.len()>256 || mime.chars().any(char::is_control) { return Err("Invalid export MIME type".into()); }
                    (bytes,mime.to_string())
                };
                let profile = self.store.data_dir().to_path_buf();
                let export_name = name.clone();
                let path = blocking(move || crate::library_files::mobile_write_export(&profile,&export_name,&bytes)).await?;
                self.mobile_export(&path,&mime,&name,false,false).await?;
                Ok(Value::Null)
            }
            "mobile_save_resource" | "mobile_save_mhtml" => {
                self.ensure_generation(&args)?;
                let name = text(&args,"suggestedName")?.to_string();
                let id = text(&args,"bookId")?.to_string();
                let service = self.clone();
                let export_name = name.clone();
                let mhtml = command == "mobile_save_mhtml";
                let (path, mime, installation) = blocking(move || {
                    let destination = crate::library_files::mobile_export_path(service.store.data_dir(), &export_name)?;
                    let (output, mime, installation) = service.prepare_archive_export(&args, &destination, mhtml)?;
                    let window = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
                    service.with_export_installation(window, text(&args, "bookId")?, &installation, || output.publish(false))?;
                    Ok((destination, mime, installation))
                }).await?;
                self.mobile_export_authorized(&path, &mime, &name, false, false, Some((label, &id, &installation))).await?;
                Ok(Value::Null)
            }
            "mobile_copy_incoming" => {
                self.normal_writable()?;
                let _incoming = self.mobile.incoming_gate.lock().await;
                let id = text(&args,"requestId")?;
                let (uris,native_id) = {
                    let state = self.mobile.state.lock().map_err(|_| "Incoming queue unavailable")?;
                    let request=state.incoming.iter().find(|request| request.id==id).ok_or("Unknown or already imported incoming document request")?;
                    (request.uris.clone(),request.native_id.clone())
                };
                let receipt=native_id.map(|id|format!("native-incoming:{id}"));
                let consumed=match receipt.as_ref() {Some(receipt)=>self.store.acquisition_consumed(receipt)?,None=>false};
                if consumed {
                    self.mobile_acknowledge_incoming(id).await?;
                    return Ok(json!({"books":[],"errors":[]}));
                }
                let root = self.mobile_staging()?;
                let response = self.mobile_documents("copyIncomingArchives",json!({"uris":uris,"destinationDirectory":root,"labels":self.mobile_labels("Import a managed copy")?})).await?;
                if response.get("cancelled").and_then(Value::as_bool).ok_or("Native import did not return cancellation state")? { return Ok(json!({"books":[],"errors":[]})); }
                let paths = strings(&response,"paths")?;
                crate::library_files::mobile_import_paths(&root,paths.iter().map(String::as_str))?;
                let service = self.clone();
                let result = blocking(move || service.mobile_import_batch(paths,receipt.as_deref())).await?;
                if result.get("errors").and_then(Value::as_array).is_none_or(|errors|!errors.is_empty()) {
                    return Ok(result);
                }
                self.mobile_acknowledge_incoming(id).await?;
                Ok(result)
            }
            _ => Err(format!("Unknown mobile command: {command}")),
        }
    }
    /// Revoke live sensor access, retaining only the exact dormant reader.
    /// Installation/privacy/source retirement clears that owner separately.
    pub(crate) fn mobile_suspend_sensor_owners(&self) {
        if let Ok(managers) = self.sensors.lock() {
            let generation = self.launch_generation();
            for manager in managers.values() { manager.suspend(generation); }
        }
    }
    pub(crate) fn mobile_expire_checkpoint(&self, payload: &Value, expired: bool) {
        let Some(token) = payload.get("token").and_then(Value::as_u64) else { return; };
        let matches = if let Ok(mut state) = self.mobile.state.lock() {
            if state.checkpoint == Some(token) && !state.desired_active {
                #[cfg(target_os = "ios")]
                { state.checkpoint_retiring = true; }
                if expired {
                    if let Some(request) = state.background.as_mut() {
                        request.acknowledged = true;
                        request.error.get_or_insert_with(|| CHECKPOINT_EXPIRED.into());
                    }
                }
                true
            } else { false }
        } else { false };
        if matches {
            if let Some(manager) = self.localswarm.get() { manager.interrupt(); }
            let maps = self.maps.lock().ok().and_then(|maps| maps.clone());
            if let Some(manager) = maps {
                if let Err(error) = manager.request_pause() { mobile::error(self, "map-checkpoint", error); }
            }
            self.mobile.wake.notify_waiters();
        }
    }
    #[cfg(target_os = "ios")]
    async fn mobile_local_transfer_active(&self) -> Result<bool, String> {
        if self.mobile_terminal() || self.store.private_mode()? || self.store.read_only()? { return Ok(false); }
        let swarm = match self.localswarm.get() {
            Some(manager) => manager.has_active_transfers().await?,
            None => false,
        };
        let receiving = match self.downloads.get() {
            Some(manager) => manager.has_active_shared_library_work()?,
            None => false,
        };
        Ok(swarm || receiving || self.mobile_map_transfer_active()? || self.sharing.lock().await.is_some())
    }
    #[cfg(target_os = "ios")]
    fn mobile_map_transfer_active(&self) -> Result<bool, String> {
        let maps = self.maps.lock().map_err(|_| "Map manager unavailable")?.clone();
        match maps { Some(manager) => manager.has_active_work(), None => Ok(false) }
    }
    #[cfg(target_os = "ios")]
    pub(crate) async fn mobile_local_transfer_update(&self) -> Result<bool, String> {
        let _update = self.update_admission.admit().await;
        let _transfer = self.mobile.transfer_gate.lock().await;
        self.mobile_local_transfer_update_locked().await
    }
    #[cfg(target_os = "ios")]
    pub(super) async fn mobile_local_transfer_update_locked(&self) -> Result<bool, String> {
        let foreground = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?
            .desired_active;
        let policy = if foreground && self.mobile_active()
            && self.mobile.sharing_policy_retry.load(Ordering::SeqCst) {
            match self.store.settings() {
                Ok(settings) => {
                    if let Some(manager) = self.localswarm.get() {
                        manager.set_sharing_admission(!settings.private_mode && !settings.read_only).await;
                    }
                    self.mobile.sharing_policy_retry.store(false, Ordering::SeqCst);
                    Ok(())
                }
                Err(error) => Err(error),
            }
        } else { Ok(()) };
        let activity = if foreground { self.mobile_local_transfer_active().await } else { Ok(false) };
        let active = activity.as_ref().copied().unwrap_or(false);
        self.mobile_publish_local_transfer(active).await?;
        policy.and(activity)
    }
    #[cfg(target_os = "ios")]
    async fn mobile_publish_local_transfer(&self, active: bool) -> Result<(), String> {
        let mut sent = self.mobile.local_transfer_snapshot.lock().await;
        if *sent == Some(active) { return Ok(()); }
        // A rejected/cancelled bridge call may already have reached native.
        *sent = None;
        self.mobile_call("localTransferState", json!({"active": active})).await?;
        *sent = Some(active);
        Ok(())
    }
    #[cfg(target_os = "ios")]
    pub(super) async fn mobile_release_transfer_lifetime(&self) -> Result<(), String> {
        let idle = self.mobile_publish_local_transfer(false).await;
        let token = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?.checkpoint.take();
        let checkpoint = match token {
            Some(token) => self.mobile_call("endCheckpoint", json!({"token": token})).await.map(|_| ()),
            None => Ok(()),
        };
        idle.and(checkpoint)
    }
    #[cfg(target_os = "ios")]
    async fn mobile_end_checkpoint(&self, token: Option<u64>) -> Result<(), String> {
        let Some(token) = token else { return Ok(()); };
        {
            let mut state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            if state.checkpoint == Some(token) { state.checkpoint = None; }
        }
        self.mobile_call("endCheckpoint", json!({"token": token})).await.map(|_| ())
    }
    #[cfg(target_os = "ios")]
    async fn mobile_start_transfer_retirement(self: &Arc<Self>, swarm: bool, maps: bool) {
        if !swarm && !maps { return; }
        let service = self.clone();
        let (complete, done) = tokio::sync::watch::channel(None);
        let task = tokio::spawn(async move {
            let result = async {
                if swarm { service.retire_localswarm(); }
                let _retirement = LocalSwarmRetirement { service: service.as_ref(), retired: swarm };
                let _transfer = service.mobile.transfer_gate.lock().await;
                let swarm_result = if swarm {
                    match service.localswarm.get() { Some(manager) => manager.stop().await.map(|_| ()), None => Ok(()) }
                } else { Ok(()) };
                let map_result = if maps {
                    let manager = service.maps.lock().map(|maps| maps.clone()).map_err(|_| "Map manager unavailable".to_string());
                    match manager {
                        Ok(Some(manager)) => manager.pause().await,
                        Ok(None) => Ok(()),
                        Err(error) => Err(error),
                    }
                } else { Ok(()) };
                swarm_result.and(map_result)
            }.await;
            // Publish only after all authority/operation guards have dropped.
            complete.send_replace(Some(result));
            service.mobile.wake.notify_waiters();
            mobile::retry_activation(service.clone());
        });
        self.mobile.transfer_retirements.lock().await.push(mobile::TransferRetirement { task, done });
    }
    #[cfg(target_os = "ios")]
    pub(super) async fn mobile_join_transfer_retirements(&self) -> Result<(), String> {
        loop {
            let completions: Vec<_> = self.mobile.transfer_retirements.lock().await.iter()
                .map(|entry| entry.done.clone()).collect();
            if completions.is_empty() { return Ok(()); }
            // Only cloned receipts cross this await. Owners remain retained and
            // new cleanup can be queued without blocking lifecycle's serial lock.
            for mut done in completions { let _ = done.wait_for(Option::is_some).await; }
            let mut pending = self.mobile.transfer_retirements.lock().await;
            if pending.iter().all(|entry| entry.task.is_finished()) {
                let mut errors = Vec::new();
                for entry in pending.drain(..) {
                    match entry.done.borrow().as_ref() {
                        Some(Ok(())) => {}
                        Some(Err(error)) => errors.push(error.clone()),
                        None => errors.push("Transfer retirement worker stopped before completion".into()),
                    }
                }
                return if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) };
            }
            drop(pending);
            tokio::task::yield_now().await;
        }
    }
    #[cfg(target_os = "ios")]
    pub(super) async fn mobile_lock_lifecycle_after_retirement(&self)
        -> Result<(tokio::sync::MutexGuard<'_, ()>, Vec<String>), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut errors = Vec::new();
        loop {
            match tokio::time::timeout_at(deadline, self.mobile_join_transfer_retirements()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(error),
                Err(_) => {
                    self.mobile.activation_retry.store(true, Ordering::SeqCst);
                    return Err("Transfer retirement is still pending; foreground admission remains closed and will retry when its owner completes".into());
                }
            }
            let serial = match tokio::time::timeout_at(deadline, self.mobile.serial.lock()).await {
                Ok(serial) => serial,
                Err(_) => {
                    self.mobile.activation_retry.store(true, Ordering::SeqCst);
                    return Err("Lifecycle retirement is still pending; foreground activation will retry".into());
                }
            };
            let drained = self.mobile.transfer_retirements.try_lock()
                .is_ok_and(|pending| pending.is_empty());
            if drained { return Ok((serial, errors)); }
            drop(serial);
        }
    }
    #[cfg(target_os = "ios")]
    async fn mobile_finish_local_grace(self: Arc<Self>, generation: u64, token: Option<u64>,
        grace_deadline: tokio::time::Instant, os_deadline: tokio::time::Instant,
        defer_swarm: bool, defer_map: bool) -> Result<(), String> {
        let mut activity_error = None;
        loop {
            let wake = self.mobile.wake.notified();
            let (current, retiring) = {
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                (state.generation == generation && !state.desired_active && state.checkpoint == token,
                    state.checkpoint_retiring)
            };
            if !current { return self.mobile_end_checkpoint(token).await; }
            if retiring || self.mobile_terminal() || tokio::time::Instant::now() >= grace_deadline { break; }
            let activity = async {
                if self.store.private_mode()? || self.store.read_only()? { return Ok(false); }
                let swarm = if defer_swarm {
                    match self.localswarm.get() { Some(manager) => manager.has_active_transfers().await?, None => false }
                } else { false };
                Ok::<_, String>(swarm || (defer_map && self.mobile_map_transfer_active()?))
            }.await;
            match activity {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => { activity_error = Some(error); break; }
            }
            let next = grace_deadline.min(tokio::time::Instant::now() + Duration::from_millis(250));
            let _ = tokio::time::timeout_at(next, wake).await;
        }
        {
            let _serial = self.mobile.serial.lock().await;
            let current = {
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                state.generation == generation && !state.desired_active && state.checkpoint == token
            };
            if current {
                if defer_swarm { if let Some(manager) = self.localswarm.get() { manager.interrupt(); } }
                if defer_map {
                    let manager = self.maps.lock().ok().and_then(|maps| maps.clone());
                    if let Some(manager) = manager { if let Err(error) = manager.request_pause() { activity_error = Some(error); } }
                }
                self.mobile_start_transfer_retirement(defer_swarm, defer_map).await;
            }
        }
        // OS execution time is not worker ownership. Always relinquish the
        // exact task at its deadline; unfinished joins remain service-owned and
        // must complete before foreground admission can reopen.
        let joined = tokio::time::timeout_at(os_deadline, self.mobile_join_transfer_retirements()).await;
        let released = self.mobile_end_checkpoint(token).await;
        match joined {
            Ok(result) => result.and(released).and(activity_error.map_or(Ok(()), Err)),
            Err(_) => released.and(Err("Background execution time ended; transfer retirement remains pending until execution resumes".into())),
        }
    }
    pub(crate) async fn mobile_transition(self: Arc<Self>, generation: u64, active: bool) -> Result<(), String> {
        let _update = if active { Some(self.update_admission.admit().await) } else { None };
        #[cfg(target_os = "ios")]
        let (_serial, retirement_errors) = if active {
            self.mobile_lock_lifecycle_after_retirement().await?
        } else { (self.mobile.serial.lock().await, Vec::new()) };
        #[cfg(not(target_os = "ios"))]
        let _serial = self.mobile.serial.lock().await;
        let checkpoint_transition = {
            let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            if generation != state.generation || active != state.desired_active { return Ok(()); }
            state.checkpoint_transition
        };
        #[cfg(not(target_os="ios"))]
        let _ = checkpoint_transition;
        if active {
            let _transfer = self.mobile.transfer_gate.lock().await;
            let _mode = self.privacy_gate.write().await;
            self.ensure_running()?;
            {
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if generation != state.generation || !state.desired_active { return Ok(()); }
            }
            let mut errors = Vec::new();
            #[cfg(target_os = "ios")]
            {
                errors.extend(retirement_errors);
                if let Err(error) = self.mobile_release_transfer_lifetime().await { errors.push(error); }
            }
            #[cfg(target_os = "ios")]
            if let Some(manager) = self.localswarm.get() {
                // A failed policy read must not preserve a formerly open
                // listener. Only a successful current-policy read reopens it.
                manager.set_sharing_admission(false).await;
            }
            let settings = self.store.settings()?;
            #[cfg(target_os = "ios")]
            if !settings.private_mode && !settings.read_only {
                if let Some(manager) = self.downloads.get() {
                    // Adopt native leases/receipts before any interrupted row
                    // can start another worker. Reconciliation is local and
                    // must also run offline or with automatic resume disabled.
                    manager.reconcile_native().await?;
                }
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if generation != state.generation || !state.desired_active { return Ok(()); }
            }
            #[cfg(target_os="android")]
            {
                let torrents = self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
                if crate::transfer_policy::require_internet().is_ok() {
                    if let Some(manager) = torrents { if let Err(error) = manager.resume_admission().await { errors.push(error); } }
                }
            }
            #[cfg(target_os="ios")]
            if !settings.private_mode && !settings.read_only && crate::transfer_policy::require_internet().is_ok() {
                if let Some(manager) = self.downloads.get() {
                    if settings.auto_resume_downloads { if let Err(error) = manager.resume_interrupted().await { errors.push(error); } }
                }
                let torrents = self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
                if let Some(manager) = torrents { if let Err(error) = manager.resume_interrupted().await { errors.push(error); } }
            }
            if let Err(error) = self.spelling.set_writable(settings.spelling_enabled && !settings.private_mode && !settings.read_only) { errors.push(error); }
            #[cfg(target_os = "ios")]
            if let Some(manager) = self.localswarm.get() {
                manager.set_sharing_admission(!settings.private_mode && !settings.read_only).await;
            }
            if let Err(error) = self.mobile_restore_retained_reader(generation).await { errors.push(error); }
            let mut state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            if generation != state.generation || !state.desired_active { return Ok(()); }
            state.active=true;
            state.background=None;
            #[cfg(any(target_os = "android", target_os = "ios"))]
            self.mobile.media_retained.store(false, Ordering::SeqCst);
            drop(state);
            mobile::emit(&self,"kiwix-mobile-active",json!({}));
            return if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) };
        }
        #[cfg(target_os = "ios")]
        if let Some(manager) = self.localswarm.get() { manager.set_sharing_admission(false).await; }
        // Cancel accepted search work before a native await or privacy writer
        // can wait on the search's read lease.
        self.retire_searches(None);
        // Adopt the finite task reserved synchronously by the native callback.
        #[cfg(target_os="ios")]
        let (checkpoint, checkpoint_error) = match self.mobile_call("beginCheckpoint",json!({"transition":checkpoint_transition})).await {
            Ok(result) => match result.get("token").and_then(Value::as_u64).filter(|token| *token > 0) {
                Some(token) => (Some(token), None),
                None => (None, Some("The background task could not be restored. Try again".to_string())),
            },
            Err(error) => (None, Some(format!("Native background checkpoint unavailable: {error}"))),
        };
        #[cfg(not(target_os="ios"))]
        let (checkpoint, checkpoint_error): (Option<u64>, Option<String>) = (None, None);
        let current = {
            let mut state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            if generation != state.generation || state.desired_active {
                None
            } else {
                state.active=false;
                state.checkpoint=checkpoint;
                #[cfg(target_os = "ios")]
                { state.checkpoint_retiring = false; }
                let request = state.background.as_mut()
                    .filter(|request| request.generation == generation)
                    .ok_or("The mobile background checkpoint was not issued by its lifecycle transition")?;
                if let Some(error) = checkpoint_error.as_ref() {
                    request.acknowledged = true;
                    request.error.get_or_insert_with(|| error.clone());
                }
                #[cfg(target_os = "android")]
                if !self.mobile.android_ui_attached.load(Ordering::SeqCst) {
                    request.acknowledged = true;
                }
                Some((request.id.clone(), request.deadline))
            }
        };
        let Some((request_id, deadline)) = current else {
            #[cfg(target_os = "ios")]
            // Native may already have discarded the reservation on resume.
            // Failure to begin an obsolete checkpoint isn't a user error.
            let mut errors = Vec::<String>::new();
            #[cfg(target_os="ios")]
            if let Some(token)=checkpoint {
                if let Err(error)=self.mobile_call("endCheckpoint",json!({"token":token})).await { errors.push(error); }
            }
            #[cfg(target_os = "ios")]
            return if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) };
            #[cfg(not(target_os = "ios"))]
            return Ok(());
        };
        #[cfg(target_os = "ios")]
        let mut grace_deadline = None;
        #[cfg(target_os = "ios")]
        let mut os_deadline = tokio::time::Instant::now();
        #[cfg(target_os="ios")]
        if let Some(token)=checkpoint {
            // The expiry event can beat the Rust assignment above. Query the
            // native live-task table AFTER installation: earlier expiry is
            // caught here, later expiry observes the installed token in the
            // event handler. No historical token set or ordering assumption.
            let status = self.mobile_call("checkpointState",json!({"token":token})).await;
            let failure = match status {
                Ok(result) => {
                    let now = tokio::time::Instant::now();
                    os_deadline = result.get("osSeconds").and_then(Value::as_f64)
                        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0 && *seconds < 86400.0)
                        .map(|seconds| now + Duration::from_secs_f64(seconds)).unwrap_or(now);
                    match result.get("active").and_then(Value::as_bool) {
                    Some(true) if result.get("retiring").and_then(Value::as_bool) == Some(true) => {
                        self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?.checkpoint_retiring = true;
                        None
                    }
                    Some(true) => {
                        grace_deadline = result.get("graceSeconds").and_then(Value::as_f64)
                            .filter(|seconds| seconds.is_finite() && *seconds > 0.0 && *seconds < 86400.0)
                            .map(|seconds| tokio::time::Instant::now() + Duration::from_secs_f64(seconds));
                        None
                    }
                    Some(false) => Some(CHECKPOINT_EXPIRED.to_string()),
                    None => Some("Native background checkpoint returned no active state".to_string()),
                    }
                },
                Err(error) => Some(format!("Native background checkpoint state unavailable: {error}")),
            };
            if let Some(error)=failure {
                let mut state=self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if state.checkpoint==Some(token) {
                    if let Some(request)=state.background.as_mut().filter(|request|
                        request.generation == generation && request.id == request_id) {
                        request.acknowledged=true;
                        request.error.get_or_insert(error);
                    }
                }
            }
        }
        loop {
            let wake = self.mobile.wake.notified();
            let done = {
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                generation != state.generation || state.desired_active ||
                    state.background.as_ref().is_none_or(|request|
                        request.id != request_id || request.acknowledged)
            };
            if done || tokio::time::Instant::now()>=deadline { break; }
            let _ = tokio::time::timeout_at(deadline,wake).await;
        }
        let (superseded, request) = {
            let mut state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            let superseded = generation != state.generation || state.desired_active;
            let request = if state.background.as_ref().is_some_and(|request|
                request.generation == generation && request.id == request_id) {
                state.background.take()
            } else { None };
            if superseded && state.checkpoint == checkpoint { state.checkpoint = None; }
            (superseded, request)
        };
        let mut errors = Vec::new();
        if let Some(request) = request {
            if let Some(error) = request.error.filter(|_| !superseded) { errors.push(error); }
            else if !request.acknowledged && !superseded {
                errors.push("Reader storage did not acknowledge the finite background checkpoint deadline".into());
            }
        } else if !superseded {
            errors.push("The mobile background checkpoint ended without a durability acknowledgement".into());
        }
        if superseded {
            #[cfg(target_os = "ios")]
            if let Some(token) = checkpoint {
                if let Err(error) = self.mobile_call("endCheckpoint", json!({"token": token})).await {
                    errors.push(error);
                }
            }
            return if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) };
        }
        #[cfg(not(target_os = "ios"))]
        self.mobile_suspend_sensor_owners();
        self.retire_private_sync();
        if let Err(error) = self.nearby_retire().await { errors.push(error); }
        #[cfg(target_os = "ios")]
        let (defer_swarm, defer_map) = {
            // Close new payload sessions before inspecting active ones. An idle
            // inspection cannot turn into a new background transfer.
            let _transfer = self.mobile.transfer_gate.lock().await;
            if let Some(manager) = self.localswarm.get() { manager.set_sharing_admission(false).await; }
            let permitted = !self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?
                .checkpoint_retiring && grace_deadline.is_some();
            let swarm = match self.localswarm.get() {
                Some(manager) => manager.has_active_transfers().await,
                None => Ok(false),
            };
            let active_swarm = match swarm {
                Ok(active) => active,
                Err(error) => { errors.push(error); false }
            };
            let active_map = match self.mobile_map_transfer_active() {
                Ok(active) => active,
                Err(error) => { errors.push(error); false }
            };
            (permitted && active_swarm, permitted && active_map)
        };
        {
            let _transfer = self.mobile.transfer_gate.lock().await;
            #[cfg(target_os = "ios")]
            if let Some(client) = self.shared_library.get() { client.invalidate(); }
            #[cfg(target_os = "android")]
            let keep_sharing = self.mobile.foreground_transfers.load(Ordering::SeqCst);
            #[cfg(target_os = "ios")]
            let keep_sharing = false;
            // Retire in place so the slot stays occupied until listeners stop.
            let mut sharing = self.sharing.lock().await;
            if !keep_sharing {
                if let Some(client) = self.shared_library.get() { client.invalidate(); }
                if let Some(server)=sharing.as_mut() { if let Err(error)=server.shutdown().await {errors.push(error);} }
                *sharing = None;
            }
            drop(sharing);
            #[cfg(target_os = "android")]
            if !keep_sharing {
                if let Some(manager)=self.localswarm.get() {
                    if let Err(error)=manager.stop_sharing().await { errors.push(error); }
                }
            }
            #[cfg(target_os="android")]
            if !keep_sharing {
                if let Err(error)=self.mobile_call("discoveryState",json!({"enabled":false})).await { errors.push(error); }
            }
            #[cfg(target_os="android")]
            if !keep_sharing {
                if let Err(error)=self.mobile_hotspot_stop().await { errors.push(error); }
            }
            crate::maps::clear_recipe_cache();
            #[cfg(target_os="android")]
            let maps=self.maps.lock().map_err(|_| "Map manager unavailable")?.clone();
            #[cfg(target_os = "android")]
            if let Some(manager)=maps { if let Err(error)=manager.pause().await {errors.push(error);} }
            #[cfg(target_os="ios")]
            {
                if let Some(manager)=self.downloads.get() {if let Err(error)=manager.shutdown_for_background().await {errors.push(error);}}
                let torrents=self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
                if let Some(manager)=torrents {if let Err(error)=manager.suspend().await {errors.push(error);}}
            }
        #[cfg(target_os="android")]
        if !self.mobile.foreground_transfers.load(Ordering::SeqCst) {
            if let Some(manager)=self.downloads.get() {if let Err(error)=manager.shutdown().await {errors.push(error);}}
            let torrents=self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
            if let Some(manager)=torrents {if let Err(error)=manager.suspend().await {errors.push(error);}}
        }
        }
        #[cfg(target_os = "ios")]
        self.mobile_start_transfer_retirement(!defer_swarm, !defer_map).await;
        #[cfg(target_os = "android")]
        let retain_media = self.mobile_speech_retained();
        #[cfg(target_os = "ios")]
        let retain_media = self.mobile_reader_media_retains() || self.mobile_speech_retained();
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let retain_media = false;
        if retain_media {
            // A native read-aloud session (Android) or audible archive
            // media/PiP (iOS) still owns this reader document. Android's media
            // service and iOS's audio background mode keep the process alive;
            // readers, privacy generation and the archive cache stay intact
            // until the surface ends.
            self.mobile.media_retained.store(true, Ordering::SeqCst);
        } else {
            // All already accepted storage/setting writes finish before readers
            // and their source authority are retired. No terminal shutdown here.
            let _mode = self.privacy_gate.write().await;
            self.privacy_generation.fetch_add(1,Ordering::SeqCst);
            if let Err(error)=retire_readers(&mut *self.readers.lock().await,None).await {errors.push(error);}
            self.mobile_note_retirement()?;
            self.archives.lock().map_err(|_| "Archive cache unavailable")?.clear();
            let spelling=self.spelling.clone();
            if let Err(error)=blocking(move || spelling.set_writable(false)).await {errors.push(error);}
        }
        #[cfg(target_os = "ios")]
        {
            let service = self.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = service.clone().mobile_finish_local_grace(generation, checkpoint,
                    grace_deadline.unwrap_or_else(tokio::time::Instant::now), os_deadline, defer_swarm, defer_map).await {
                    mobile::error(&service, "local-transfer-retirement", error);
                }
            });
        }
        #[cfg(not(target_os = "ios"))]
        { self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?.checkpoint=None; }
        #[cfg(target_os="android")]
        if let Err(error)=self.mobile_transfer_update_locked().await { errors.push(error); }
        if let Err(error) = self.store.flush_browsing() { errors.push(error); }
        if errors.is_empty() {Ok(())} else {Err(errors.join("; "))}
    }
    pub(super) fn mobile_transfer_command(&self, command:&str,args:&Value)->bool {
        matches!(command,"start_download"|"edition_download"|"resume_download"|"share_torrent"|"localswarm_download"|"localswarm_seed"|"localswarm_share_all"|"map_start"|"map_resume"|"sharing_start"|"shared_library_download") || (command=="torrent_seeding" && args.get("enabled")==Some(&Value::Bool(true)))
    }
    #[cfg(target_os="android")]
    fn mobile_transfer_labels(&self,active:bool,progress:u64,total:Option<u64>)->Result<Value,String> {
        let locale=crate::localization::Localizer::for_app(&self.store.settings()?.locale);
        Ok(json!({"active":active,"title":locale.text("Transfers",&[]),"body":locale.text("Transferring archives",&[]),"pauseLabel":locale.text("Pause",&[]),"channelName":locale.text("Archive transfers",&[]),"progress":progress,"total":total}))
    }
    #[cfg(target_os="android")]
    async fn mobile_download_notifications(&self)->Result<Vec<crate::acquisition::DownloadSnapshot>,String> {
        let settings=self.store.settings()?;
        let locale=crate::localization::Localizer::for_app(&settings.locale);
        let jobs=match self.downloads.get() {
            Some(manager)=>manager.list_snapshots()?,
            None=>Vec::new()
        };
        let mut payload_jobs: Vec<Value> = jobs.iter()
            .map(serde_json::to_value).collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        if let Ok(manager) = self.torrent_manager() {
            for job in manager.list().await {
                payload_jobs.push(json!({
                    "id": job.id, "url": job.url, "fileName": job.file_name,
                    "destination": job.destination, "status": job.status,
                    "downloaded": job.downloaded, "total": job.total,
                    "error": job.error, "speed": job.speed, "etaSeconds": job.eta_seconds,
                    "transport": "torrent"
                }));
            }
        }
        self.mobile_call("downloadNotifications",json!({
            "enabled":!settings.private_mode && !settings.read_only,
            "jobs":payload_jobs,
            "labels":{
                "channel":locale.text("Archive transfers",&[]),
                "pause":locale.text("Pause",&[]),"resume":locale.text("Resume",&[]),
                "cancel":locale.text("Cancel",&[]),"retry":locale.text("Retry",&[]),
                "queued":locale.text("Queued",&[]),"connecting":locale.text("Connecting",&[]),
                "resolving":locale.text("Resolving",&[]),"initializing":locale.text("Preparing",&[]),
                "metadata":locale.text("Fetching metadata",&[]),"checking":locale.text("Checking",&[]),
                "downloading":locale.text("Downloading",&[]),"verifying":locale.text("Verifying",&[]),
                "publishing":locale.text("Preparing",&[]),"paused":locale.text("Paused",&[]),
                "failed":locale.text("Failed",&[]),"error":locale.text("Failed",&[])
            }
        })).await?;
        Ok(jobs)
    }
    #[cfg(target_os="android")]
    pub(super) async fn mobile_discovery_update(&self)->Result<(),String> {
        let status=self.localswarm_status().await?;
        let serving = self.sharing.lock().await.is_some();
        let enabled=(self.mobile_active() || self.mobile.foreground_transfers.load(Ordering::SeqCst)) &&
            (serving || status.get("discovery").and_then(Value::as_bool).unwrap_or(false) || status.get("librarySharing").and_then(|share|share.get("running")).and_then(Value::as_bool).unwrap_or(false) || status.get("seeds").and_then(Value::as_array).is_some_and(|seeds|!seeds.is_empty()));
        self.mobile_call("discoveryState",json!({"enabled":enabled})).await?;
        Ok(())
    }
    #[cfg(target_os="android")]
    fn mobile_transfer_events_pending(&self)->bool {
        // Read the acknowledged frontier first: a concurrent signal must never
        // be mistaken for the older generation an empty peek just reconciled.
        let reconciled=self.mobile.transfer_reconciled_generation.load(Ordering::SeqCst);
        self.mobile.transfer_signal_generation.load(Ordering::SeqCst)!=reconciled
    }
    #[cfg(target_os="android")]
    pub(crate) async fn mobile_reconcile_transfer_events(&self)->Result<(),String> {
        let _events=self.mobile.transfer_events_gate.lock().await;
        loop {
            let signal_generation=self.mobile.transfer_signal_generation.load(Ordering::SeqCst);
            let reply=self.mobile_call("takeTransferEvents",json!({})).await?;
            let events=reply.get("events").and_then(Value::as_array).ok_or("Native pending transfer event queue is invalid")?;
            if events.is_empty() {
                if self.mobile.transfer_signal_generation.load(Ordering::SeqCst)!=signal_generation {continue;}
                self.mobile.transfer_reconciled_generation.store(signal_generation,Ordering::SeqCst);
                if self.mobile.transfer_signal_generation.load(Ordering::SeqCst)!=signal_generation {continue;}
                return Ok(());
            }
            self.mobile.transfer_signal_generation.fetch_add(1,Ordering::SeqCst);
            for event in events {
                let kind=text(event,"event")?;
                if !matches!(kind,"transfers-pause"|"transfers-timeout"|"transfers-error") {return Err("Unknown native transfer control event".into());}
                let id=text(event,"requestId")?;
                if Uuid::parse_str(id).map_err(|_|"Invalid native transfer event ID")?.to_string()!=id {return Err("Native transfer event ID is not canonical".into());}
            }
            let settings=self.store.settings()?;
            if settings.private_mode||settings.read_only {return Ok(());}
            self.mobile_pause_transfers().await?;
            // Mode can change between durable pause and native acknowledgement.
            // Retain native ownership until a normal-writable lease is available.
            let _mode=self.privacy_gate.read().await;
            self.normal_writable()?;
            for event in events {
                let kind=text(event,"event")?;
                self.mobile_call("acknowledgeTransferEvent",json!({"event":kind,"requestId":text(event,"requestId")?})).await?;
                if kind=="transfers-error" {mobile::error(self,kind,event.to_string());}
            }
        }
    }
    #[cfg(target_os="android")]
    pub(crate) async fn mobile_resume_policy(&self)->Result<(),String> {
        let _update=self.update_admission.admit().await;
        let _transfer=self.mobile.transfer_gate.lock().await;
        let _mode=self.privacy_gate.read().await;
        self.mobile_resume_policy_locked(None).await
    }
    #[cfg(target_os="android")]
    async fn mobile_resume_policy_locked(&self, reconnect_revision: Option<u64>)->Result<(),String> {
        let settings=self.store.settings()?;
        if self.mobile_transfer_events_pending() {return Ok(());}
        if !self.mobile_active() || settings.private_mode || settings.read_only || !settings.auto_resume_downloads {return Ok(());}
        let generation=self.mobile.state.lock().map_err(|_|"Mobile lifecycle unavailable")?.generation;
        if self.mobile.policy_attempt_generation.load(Ordering::SeqCst)==generation {return Ok(());}
        if crate::transfer_policy::require_internet().is_err() { return Ok(()); }
        let network_generation = self.mobile_network_admit(true).await?;
        let downloads=self.download_manager().await?;
        let torrents=self.torrent_manager()?;
        let http_pending=downloads.list()?.iter().any(|job|job.status=="paused"&&job.interrupted);
        if !http_pending && !torrents.has_interrupted()? {return Ok(());}
        self.mobile.policy_attempt_generation.store(generation,Ordering::SeqCst);
        let already_foreground=self.mobile.foreground_transfers.load(Ordering::SeqCst);
        if let Err(error)=self.mobile_call("transferState",self.mobile_transfer_labels(true,0,None)?).await {
            // Native visibility is the final authority; a cold OS handoff may
            // initialize Rust before its Activity is actually foregrounded.
            if error.contains("VISIBLE_ACTIVITY_REQUIRED") {
                self.mobile.policy_attempt_generation.store(0,Ordering::SeqCst);
                return Ok(());
            }
            return Err(error);
        }
        let pending_pause=self.mobile_transfer_events_pending();
        if !self.mobile_active() || pending_pause {
            if pending_pause || !already_foreground {
                self.mobile_call("transferState",self.mobile_transfer_labels(false,0,None)?).await?;
            }
            return Ok(());
        }
        self.mobile_network_validate(network_generation, true)?;
        if let Some(revision) = reconnect_revision {
            if !self.mobile_reconnect_admitted(revision)? {
                if !already_foreground {
                    self.mobile_call("transferState",self.mobile_transfer_labels(false,0,None)?).await?;
                }
                return Ok(());
            }
            crate::service::mobile_policy::record_resume_attempt(revision);
        }
        self.mobile.foreground_transfers.store(true,Ordering::SeqCst);
        let mut errors=Vec::new();
        if let Err(error)=downloads.resume_interrupted().await {errors.push(error);}
        if let Err(error)=torrents.resume_interrupted().await {errors.push(error);}
        if let Err(error)=self.mobile_transfer_update_locked().await {errors.push(error);}
        if errors.is_empty(){Ok(())}else{Err(errors.join("; "))}
    }
    #[cfg(target_os="android")]
    pub(super) async fn mobile_transfer_begin(&self,label:&str,command:&str)->Result<(),String> {
        self.mobile_root(label)?;
        if self.mobile_transfer_events_pending() {
            return Err("A native Pause or timeout must be durably reconciled before starting transfers".into());
        }
        if !self.mobile_active() {return Err("Start or resume transfers while the application is visible".into());}
        self.mobile_call("transferState",self.mobile_transfer_labels(true,0,None)?).await?;
        if !self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?.desired_active || self.mobile_transfer_events_pending() {
            self.mobile_call("transferState",self.mobile_transfer_labels(false,0,None)?).await?;
            return Err("Transfer authorization was superseded by a lifecycle or native Pause action; resume after reconciliation".into());
        }
        self.mobile.foreground_transfers.store(true,Ordering::SeqCst);
        let torrents=self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
        if let Some(manager)=torrents {
            // Local Nearby transfers do not use the Internet. Only reopen
            // torrent admission when the current policy actually admits it;
            // a later network change resumes suspended torrents.
            if crate::transfer_policy::require_internet().is_ok() { manager.resume_admission().await?; }
        }
        if matches!(command, "localswarm_seed" | "localswarm_share_all" | "sharing_start") {self.mobile_call("discoveryState",json!({"enabled":true})).await?;}
        Ok(())
    }
    #[cfg(target_os="android")]
    pub(crate) async fn mobile_transfer_update(&self)->Result<(),String> {
        let _transfer = self.mobile.transfer_gate.lock().await;
        self.mobile_transfer_update_locked().await
    }
    #[cfg(target_os="android")]
    pub(super) async fn mobile_transfer_update_locked(&self)->Result<(),String> {
        let downloads=self.mobile_download_notifications().await?;
        if !self.mobile.foreground_transfers.load(Ordering::SeqCst) {return Ok(());}
        let mut active=false; let mut progress=0u64; let mut total=Some(0u64);
        for job in downloads {if matches!(job.status.as_str(),"queued"|"connecting"|"downloading"|"verifying"|"publishing") {active=true;progress=progress.saturating_add(job.downloaded);total=total.zip(job.total).map(|(a,b)|a.saturating_add(b));}}
        let torrents=self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
        if let Some(manager)=torrents {
            for job in manager.list().await {if matches!(job.status.as_str(),"queued"|"resolving"|"initializing"|"metadata"|"checking"|"downloading"|"publishing"|"seeding"|"live") {active=true;progress=progress.saturating_add(job.downloaded);total=total.zip(job.total).map(|(a,b)|a.saturating_add(b));}}
        }
        if let Some(manager)=self.localswarm.get() {
            for job in manager.list()? {if matches!(job.status.as_str(),"queued"|"connecting"|"downloading"|"verifying"|"publishing") {active=true;progress=progress.saturating_add(job.downloaded);total=total.zip(job.total).map(|(a,b)|a.saturating_add(b));}}
            let status=manager.status().await?;
            let library_active = status.get("librarySharing").and_then(|share|share.get("running")).and_then(Value::as_bool).unwrap_or(false);
            let discovery_active = (self.mobile_active() || self.mobile.foreground_transfers.load(Ordering::SeqCst)) && (status.get("discovery").and_then(Value::as_bool).unwrap_or(false) || library_active || status.get("seeds").and_then(Value::as_array).is_some_and(|seeds|!seeds.is_empty()) || self.sharing.lock().await.is_some());
            self.mobile_call("discoveryState",json!({"enabled":discovery_active})).await?;
            active |= library_active || status.get("seeds").and_then(Value::as_array).is_some_and(|seeds|!seeds.is_empty());
        }
        active |= self.sharing.lock().await.is_some();
        let maps=self.maps.lock().map_err(|_| "Map manager unavailable")?.clone();
        if let Some(manager)=maps {if let Some(job)=manager.status()? {if super::mobile_policy::map_transfer_running(&job.status) {active=true;progress=progress.saturating_add(job.written);total=total.map(|sum|sum.saturating_add(job.total));}}}
        self.mobile_call("transferState",self.mobile_transfer_labels(active,progress,total)?).await?;
        if !active {self.mobile.foreground_transfers.store(false,Ordering::SeqCst);}
        Ok(())
    }
    /// Phase and, once active, the platform's published credentials.
    #[cfg(target_os = "android")]
    pub(crate) fn mobile_hotspot_status(&self) -> Value {
        let Ok(state) = self.hotspot.lock() else { return Value::Null; };
        match state.phase() {
            crate::hotspot::HotspotPhase::Idle => Value::Null,
            crate::hotspot::HotspotPhase::Starting => json!({"phase":"starting"}),
            crate::hotspot::HotspotPhase::Stopping => json!({"phase":"stopping"}),
            crate::hotspot::HotspotPhase::Active => state.info().map(|info| {
                let mut value = info.to_json();
                value["phase"] = json!("active");
                // The peer types this address after joining; it is the live
                // access-point interface, not the client Wi-Fi link.
                if let Ok(address) = crate::hotspot::interface_address() {
                    value["address"] = json!(address.to_string());
                }
                value
            }).unwrap_or_else(|| json!({"phase":"starting"})),
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) async fn mobile_hotspot_start_bounded(&self) -> Result<(), String> {
        match tokio::time::timeout(std::time::Duration::from_secs(120), self.mobile_hotspot_start()).await {
            Ok(result) => result.map(|_| ()),
            Err(_) => {
                let _ = self.mobile_hotspot_stop().await;
                Err("The temporary Wi-Fi permission dialog was not completed. Dismiss it and try again.".into())
            }
        }
    }
    #[cfg(target_os = "android")]
    pub(crate) async fn mobile_hotspot_start(&self) -> Result<Value, String> {
        let generation = self.hotspot.lock().map_err(|_| "Hotspot state unavailable")?.begin_start()?;
        let response = self.mobile_call("hotspotStart", json!({})).await;
        let (info, start_error) = {
            let mut state = self.hotspot.lock().map_err(|_| "Hotspot state unavailable")?;
            let parsed = match response {
                Ok(value) => serde_json::from_value::<crate::hotspot::HotspotInfo>(value)
                    .map_err(|error| format!("Android returned invalid hotspot credentials: {error}"))
                    .and_then(crate::hotspot::HotspotInfo::validate),
                Err(error) => Err(format!("Creating the Wi-Fi hotspot failed: {error}")),
            };
            match parsed {
                Ok(info) => (
                    if state.accept(generation, info.clone()) { Some(info) } else { None },
                    None,
                ),
                Err(error) => (None, Some(error)),
            }
        };
        if let Some(error) = start_error {
            // The platform may already have created the reservation before it
            // returned unusable credentials. Release it through the owned stop
            // path instead of abandoning a live hotspot the UI cannot show.
            let owned = self
                .hotspot
                .lock()
                .map_err(|_| "Hotspot state unavailable")?
                .failed_start(generation);
            if !owned {
                // A concurrent Stop already owns the native release.
                return Err(error);
            }
            return match self.mobile_hotspot_stop().await {
                Ok(()) => Err(error),
                Err(stop) => Err(format!("{error}. {stop}")),
            };
        }
        let Some(info) = info else {
            // A Stop arrived while the platform was still starting. Close the
            // reservation that just became live instead of adopting it.
            self.mobile_call("hotspotStop", json!({})).await?;
            return Err("The Wi-Fi hotspot start was stopped before it completed".into());
        };
        let foreground = {
            let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            state.active && state.desired_active
        };
        if !foreground {
            // The native start resolved after background retirement. Do not
            // leave an unowned reservation hosting with no further retirement.
            self.mobile_hotspot_stop().await?;
            return Err("The Wi-Fi hotspot start finished after the app left the foreground".into());
        }
        Ok(info.to_json())
    }

    #[cfg(target_os = "android")]
    pub(crate) async fn mobile_hotspot_stop(&self) -> Result<(), String> {
        self.mobile_hotspot_release().await
    }

    #[cfg(target_os = "android")]
    async fn mobile_hotspot_release(&self) -> Result<(), String> {
        let (generation, previous, native) = {
            let mut state = self.hotspot.lock().map_err(|_| "Hotspot state unavailable")?;
            let previous = state.info().cloned();
            let (generation, native) = state.begin_stop();
            (generation, previous, native)
        };
        if !native { return Ok(()); }
        let mut errors = Vec::new();
        // Drain a server bound to the access-point address while the link is
        // still up. The listener cannot outlive the reservation it advertises,
        // and an explicit hotspot stop is an explicit stop of that share.
        if let Err(error) = self.mobile_hotspot_retire_sharing().await {
            errors.push(error);
        }
        let result = self.mobile_call("hotspotStop", json!({})).await;
        let mut state = self.hotspot.lock().map_err(|_| "Hotspot state unavailable")?;
        match result {
            Ok(_) => {
                state.stopped(generation);
                if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
            }
            Err(error) => {
                if let Some(info) = previous { state.retry(generation, info); }
                else { state.stopped(generation); }
                errors.insert(0, format!("Stopping the Wi-Fi hotspot failed: {error}"));
                Err(errors.join("; "))
            }
        }
    }

    /// Native ownership ended without an application Stop.
    #[cfg(target_os = "android")]
    pub(crate) fn mobile_hotspot_lost(&self) {
        if let Ok(mut state) = self.hotspot.lock() { state.lost(); }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn mobile_hotspot_hosting(&self) -> bool {
        self.hotspot.lock().is_ok_and(|state| matches!(
            state.phase(), crate::hotspot::HotspotPhase::Starting | crate::hotspot::HotspotPhase::Active))
    }

    /// The address a joined peer reaches this server on. Only an active
    /// reservation may answer: the interface is created and removed with it,
    /// so a stale or guessed address would advertise a dead link.
    #[cfg(target_os = "android")]
    pub(crate) fn hotspot_binding_address(&self) -> Result<std::net::IpAddr, String> {
        let state = self.hotspot.lock().map_err(|_| "Hotspot state unavailable")?;
        if state.phase() != crate::hotspot::HotspotPhase::Active {
            return Err("Start the Wi-Fi hotspot before sharing over it".into());
        }
        crate::hotspot::interface_address()
    }

    /// Retire a listener that lives on the access-point address. Called before
    /// the reservation is released so requests drain while the link is still
    /// up; a server bound elsewhere is untouched.
    #[cfg(target_os = "android")]
    pub(crate) async fn mobile_hotspot_retire_sharing(&self) -> Result<(), String> {
        let mut sharing = self.sharing.lock().await;
        if !sharing.as_ref().is_some_and(|server| server.uses_hotspot()) {
            return Ok(());
        }
        let result = match sharing.as_mut() {
            Some(server) => server.shutdown().await,
            None => Ok(()),
        };
        *sharing = None;
        result
    }

    #[cfg(all(target_os = "android", test))]
    pub(super) async fn mobile_shutdown_adapter(&self)->Result<(),String> {
        let hotspot=self.mobile_hotspot_stop().await;
        let discovery=self.mobile_call("discoveryState",json!({"enabled":false})).await;
        let transfers=self.mobile_call("transferState",self.mobile_transfer_labels(false,0,None)?).await;
        self.mobile.foreground_transfers.store(false,Ordering::SeqCst);
        hotspot?;
        discovery?;
        transfers?;
        Ok(())
    }
    #[cfg(target_os = "android")]
    pub(crate) async fn mobile_pause_transfers(&self)->Result<(),String> {
        let _serial=self.mobile.serial.lock().await;
        let _transfer = self.mobile.transfer_gate.lock().await;
        self.retire_localswarm();
        let _retirement=LocalSwarmRetirement {service:self,retired:true};
        let _mode=self.privacy_gate.write().await;
        self.normal_writable()?;
        #[cfg(target_os="android")]
        {
            // Constructors hold recovered work without networking. Notification
            // Pause must reach persisted jobs even before the first UI poll.
            self.download_manager().await?;
            self.torrent_manager()?;
        }
        let mut errors=Vec::new();
        if let Err(error) = self.stop_sharing().await { errors.push(error); }
        if let Some(manager)=self.downloads.get() {
            for job in manager.list()? {if !matches!(job.status.as_str(),"complete"|"cancelled"|"paused"|"failed") || (job.status=="paused" && job.interrupted) {if let Err(error)=manager.pause(&job.id).await {errors.push(error);}}}
        }
        let torrents=self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
        if let Some(manager)=torrents {
            for job in manager.list().await {if !matches!(job.status.as_str(),"cancelled"|"error"|"failed") && (job.status!="completed"||job.seeding) {if let Err(error)=manager.pause(&job.id).await {errors.push(error);}}}
            if !self.mobile_active() {if let Err(error)=manager.suspend().await {errors.push(error);}}
        }
        if let Some(manager)=self.localswarm.get() {if let Err(error)=manager.stop().await {errors.push(error);}}
        let maps=self.maps.lock().map_err(|_| "Map manager unavailable")?.clone();
        if let Some(manager)=maps {if let Err(error)=manager.cancel().await {errors.push(error);}}
        #[cfg(target_os="android")]
        {
            if let Err(error)=self.mobile_call("discoveryState",json!({"enabled":false})).await {errors.push(error);}
            if let Err(error)=self.mobile_hotspot_stop().await {errors.push(error);}
            if let Err(error)=self.mobile_call("transferState",self.mobile_transfer_labels(false,0,None)?).await {errors.push(error);}
            self.mobile.foreground_transfers.store(false,Ordering::SeqCst);
        }
        if errors.is_empty(){Ok(())}else{Err(errors.join("; "))}
    }
}
