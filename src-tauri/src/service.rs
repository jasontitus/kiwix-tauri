// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    acquisition::{DownloadManager, MonitoredPathClaims},
    archive::Archive,
    imports::{load_book, EnqueueError, ImportManager},
    localswarm::{ConnectionTarget, LocalSwarmManager},
    maps::MapManager,
    permissions::{SensorKind, SensorManager},
    server::{encode_path, split_location, ReaderServer},
    spelling::SpellingManager,
    store::{self, Book, Bookmark, HistoryEntry, Store, Tab},
    torrent::TorrentManager,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};
#[cfg(desktop)]
use tauri::Manager;
use uuid::Uuid;
#[path = "pdf_export.rs"]
mod pdf_export;
#[path = "share.rs"]
mod share;
#[path = "nearby_service.rs"]
mod nearby_service;
#[path = "edition_service.rs"]
mod edition_service;
#[path = "private_sync_service.rs"]
mod private_sync_service;

#[cfg(mobile)]
#[path = "mobile_service.rs"]
mod mobile_service;
#[cfg(any(mobile, test))]
#[path = "mobile_policy.rs"]
pub(crate) mod mobile_policy;
// Read-aloud ownership on every platform. Mobile also drives the native
// system voices; desktop drives only Kokoro (system voices stay in the browser).
#[path = "mobile_speech.rs"]
mod mobile_speech;
#[cfg(any(target_os = "ios", target_os = "macos"))]
#[path = "notifications_service.rs"]
mod notifications_service;
#[cfg(any(target_os = "ios", test))]
#[path = "live_activity.rs"]
mod live_activity;
#[cfg(any(target_os = "android", test))]
#[path = "android_notifications_service.rs"]
mod android_notifications_service;
#[path = "kokoro.rs"]
mod kokoro;
#[cfg(desktop)]
#[path = "agent_connector.rs"]
mod agent_connector;
#[cfg(desktop)]
#[path = "photo_capture.rs"]
pub(crate) mod photo_capture;

#[derive(Clone)]
struct CachedArchive {
    id: String,
    installation_id: String,
    path: String,
    archive: Arc<OnceLock<Result<Archive, String>>>,
}

struct MonitoredImportClaims {
    downloads: Option<MonitoredPathClaims>,
    maps: Option<MonitoredPathClaims>,
    torrents: Option<MonitoredPathClaims>,
    localswarm: Option<MonitoredPathClaims>,
}

struct SearchOwner {
    request_id: u64,
    cancel: Option<tokio_util::sync::CancellationToken>,
}
struct SearchSession {
    id: String,
    owners: HashMap<String, SearchOwner>,
}
impl SearchSession {
    fn cancel(&self) {
        for owner in self.owners.values() {
            if let Some(cancel) = &owner.cancel { cancel.cancel(); }
        }
    }
}

struct LocalSwarmAdmission {
    cancel: tokio_util::sync::CancellationToken,
    retirements: usize,
    generation: u64,
}

struct LocalSwarmRetirement<'a> {
    service: &'a Service,
    retired: bool,
}
impl Drop for LocalSwarmRetirement<'_> {
    fn drop(&mut self) {
        let _ = self.service.finish_localswarm_retirement(self.retired);
    }
}

#[cfg(any(target_os = "android", test))]
#[derive(Default)]
struct AndroidWindowAuthority {
    token: Option<String>,
    activity_id: Option<i64>,
    accepting: bool,
    drain: Option<tokio::sync::watch::Receiver<Option<Result<(), String>>>>,
}

pub struct Service {
    pub store: Arc<Store>,
    pub(crate) update_admission: crate::update_admission::AdmissionGate,
    catalog_cache: crate::catalog::CatalogCache,
    catalog_index: crate::catalog::CatalogIndex,
    editions: crate::edition_updates::EditionManager,
    edition_errors: Mutex<HashMap<String, String>>,
    #[cfg(target_os = "ios")]
    official_catalog: Mutex<crate::ios_background::OfficialCatalogRegistry>,
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    notifications: crate::notifications::NotificationManager,
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    notification_gui: AtomicBool,
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    notification_launch_gate: Mutex<()>,
    #[cfg(target_os = "android")]
    android_notifications: Arc<android_notifications_service::AndroidNotifications>,
    downloads: tokio::sync::OnceCell<DownloadManager>,
    localswarm: tokio::sync::OnceCell<LocalSwarmManager>,
    shared_library: tokio::sync::OnceCell<crate::shared_library::SharedLibraryClient>,
    private_sync: tokio::sync::OnceCell<Arc<private_sync_service::PrivateSyncManager>>,
    localswarm_admission: Mutex<LocalSwarmAdmission>,
    localswarm_sources: tokio::sync::Mutex<()>,
    pub(super) hotspot_use: tokio::sync::Mutex<()>,
    #[cfg(test)]
    localswarm_initialization_gate: Mutex<Option<(
        tokio::sync::oneshot::Sender<()>, std::sync::mpsc::Receiver<()>,
    )>>,
    maps: Mutex<Option<MapManager>>,
    torrents: Mutex<Option<Arc<TorrentManager>>>,
    cli_read_only: bool,
    /// Archive moves that replace library files. An update install refuses
    /// while this is non-zero so a restart cannot land mid-move. The guard is
    /// counted on every platform because the move command itself is.
    migration_active: std::sync::atomic::AtomicUsize,
    /// Windows only: user-requested library moves in their file-moving phase.
    /// An open archive pins its file there, so `archive()` refuses meanwhile.
    /// Narrower than `migration_active`, which also spans startup recovery.
    #[cfg(windows)]
    archive_moves: std::sync::atomic::AtomicUsize,
    privacy_generation: Arc<std::sync::atomic::AtomicU64>,
    privacy_policy_generation: std::sync::atomic::AtomicU64,
    privacy_gate: Arc<tokio::sync::RwLock<()>>,
    launch_locations: Mutex<Vec<Value>>,
    /// "Look up in Kiwix": only the latest external search request is kept, so
    /// a flood of launches can never accumulate work for the shell.
    #[cfg(any(desktop, test))]
    launch_search: Mutex<Option<Value>>,
    archives: Mutex<VecDeque<CachedArchive>>,
    #[cfg(test)]
    archive_open_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    searches: Mutex<HashMap<String, SearchSession>>,
    pdf_jobs: Mutex<HashMap<Uuid, (String, tokio_util::sync::CancellationToken)>>,
    pdf_idle: tokio::sync::Notify,
    integrity: crate::integrity::IntegrityManager,
    readers: tokio::sync::Mutex<HashMap<String, ReaderServer>>,
    sensors: Mutex<HashMap<String, SensorManager>>,
    shell_origins: Mutex<HashSet<String>>,
    imports: Mutex<Arc<ImportManager>>,
    spelling: Arc<SpellingManager>,
    stopping: AtomicBool,
    startup_warnings: Mutex<Vec<String>>,
    #[cfg(test)]
    mutation_test_gate: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
    closed_windows: Mutex<HashSet<String>>,
    #[cfg(any(target_os = "android", test))]
    android_window: Mutex<AndroidWindowAuthority>,
    #[cfg(any(target_os = "android", test))]
    android_window_gate: Arc<tokio::sync::RwLock<()>>,
    pub app: Mutex<Option<tauri::AppHandle>>,
    sharing: tokio::sync::Mutex<Option<crate::sharing::SharingServer>>,
    nearby: nearby_service::NearbyState,
    #[cfg(target_os = "android")]
    hotspot: Mutex<crate::hotspot::HotspotState>,
    #[cfg(desktop)]
    watcher: Mutex<Option<crate::watch::FolderWatch>>,
    #[cfg(desktop)]
    watch_failures: Arc<Mutex<crate::watch::FailureLog>>,
    /// Throttles the on-demand session snapshot so a forced Quit still has a
    /// recent restorable session without writing on every scroll tick.
    #[cfg(desktop)]
    session_snapshot_at: Mutex<Option<std::time::Instant>>,
    /// Windows created by the explicit restore command whose shell has not yet
    /// hydrated. Their empty first session save must not erase the restored tabs.
    #[cfg(desktop)]
    restored_windows: Mutex<HashSet<String>>,
    /// Native window labels whose shell has committed a usable first snapshot.
    /// The startup watchdog reads this to decide whether to paint diagnostics.
    ready_frontends: Mutex<HashSet<String>>,
    #[cfg(mobile)]
    pub(crate) mobile: crate::mobile::MobileState,
    speech: mobile_speech::SpeechManager,
    speech_prepare: tokio::sync::Mutex<()>,
    /// Offline-library agent connector (MCP relay channel).
    #[cfg(desktop)]
    agent: agent_connector::AgentConnector,
    /// Desktop photo chooser for archive capture inputs.
    #[cfg(desktop)]
    photo: photo_capture::PhotoCapture,
    /// Offline Kokoro model and playback. Available on every platform; the
    /// platform voice path remains the fallback until the user downloads it.
    kokoro: Arc<kokoro::KokoroManager>,
}

/// Counts an archive move for the lifetime of the guard. The updater reads the
/// same counter before installing, so it never restarts the process while the
/// library is being replaced.
struct MigrationActivity<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> MigrationActivity<'a> {
    fn begin(counter: &'a std::sync::atomic::AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for MigrationActivity<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing string argument: {key}"))
}
fn strings(args: &Value, key: &str) -> Result<Vec<String>, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(json!([])))
        .map_err(|e| format!("Invalid {key}: {e}"))
}
fn now() -> String {
    crate::timestamps::now()
}
fn value<T: serde::Serialize>(input: T) -> Result<Value, String> {
    serde_json::to_value(input).map_err(|e| e.to_string())
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("Background operation stopped: {e}"))?
}

/// Remove listeners under the admission lock, then join every owned archive read.
/// Keep the lock until retirement finishes so a replacement cannot race teardown.
async fn retire_readers(
    readers: &mut HashMap<String, ReaderServer>,
    installation: Option<&str>,
) -> Result<(), String> {
    let retired: Vec<_> = if let Some(installation) = installation {
        let keys: Vec<_> = readers.iter()
            .filter(|(_, reader)| reader.installation_id == installation)
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter().filter_map(|key| readers.remove(&key)).collect()
    } else {
        readers.drain().map(|(_, reader)| reader).collect()
    };
    shutdown_readers(retired).await
}

async fn shutdown_readers(retired: Vec<ReaderServer>) -> Result<(), String> {
    let mut errors = Vec::new();
    for result in futures_util::future::join_all(
        retired.into_iter().map(ReaderServer::shutdown),
    ).await {
        if let Err(error) = result { errors.push(error); }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// What the reader asked to open. A stored location that cannot be decoded is
/// a navigation miss, not a fatal error: it resolves like a stale entry so the
/// user still lands on a readable page with the usual notice.
enum OpenRequest {
    Empty,
    Path(String),
    Unreadable,
    Random,
}

impl OpenRequest {
    fn decode(path: &str) -> Self {
        match crate::server::decode_location_path(path) {
            Ok(raw) if raw.is_empty() => Self::Empty,
            Ok(raw) => Self::Path(raw),
            Err(_) => Self::Unreadable,
        }
    }
}

struct OpenEntry {
    entry: crate::archive::EntryInfo,
    missing: bool,
    no_main_page: bool,
}

/// UI-facing outcome when an archive has nothing to read at all. The shell
/// classifies this as a quiet, recorded notice instead of a blocking band.
pub(crate) const NO_ARTICLES: &str = "This archive has no articles to open.";

/// Resolve the document to show without ever turning a navigation miss into a
/// blocking failure. A missing main page falls back to a random article; an
/// archive with neither has no readable document at all.
fn resolve_open_entry(
    archive: &Archive,
    request: &OpenRequest,
) -> Result<OpenEntry, String> {
    if matches!(request, OpenRequest::Random) {
        if !archive.has_articles()? {
            return Err(NO_ARTICLES.into());
        }
        return match archive.random_entry() {
            Ok(entry) => Ok(OpenEntry { entry, missing: false, no_main_page: false }),
            Err(error) if crate::archive::is_entry_not_found(&error) => {
                Err(NO_ARTICLES.into())
            }
            Err(error) => Err(error),
        };
    }
    let missing = match request {
        OpenRequest::Path(raw) => match archive.entry(raw) {
            Ok(entry) => {
                return Ok(OpenEntry { entry, missing: false, no_main_page: false })
            }
            Err(error) if crate::archive::is_entry_not_found(&error) => true,
            Err(error) => return Err(error),
        },
        OpenRequest::Unreadable => true,
        _ => false,
    };
    match archive.main_or_random_entry() {
        Ok((entry, random)) => Ok(OpenEntry { entry, missing, no_main_page: random }),
        Err(error) if crate::archive::is_entry_not_found(&error) => {
            Err(NO_ARTICLES.into())
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(desktop, test))]
struct RestorePlan {
    plans: Vec<(String, crate::store::WindowSession)>,
    skipped: Vec<String>,
}

/// Decide which saved windows the explicit restore fills.
///
/// An invoking window with no tabs adopts its own saved session, else the saved
/// main session, so a relaunch restores in place. Any other open window with no
/// tabs adopts its saved session too (the caller pushes it); only windows that
/// already hold reader tabs are left alone. Live tabs are never replaced.
#[cfg(any(desktop, test))]
fn plan_session_restore(
    saved: &crate::store::SessionSnapshot,
    live_tabs: &HashMap<String, usize>,
    live_main_tabs: usize,
    open: &HashSet<String>,
    label: &str,
) -> RestorePlan {
    let caller_tabs = live_tabs
        .get(label)
        .copied()
        .unwrap_or_else(|| if label == "main" { live_main_tabs } else { 0 });
    let main_tabs = live_tabs.get("main").copied().unwrap_or(live_main_tabs);
    let mut plans: Vec<(String, crate::store::WindowSession)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut main_planned = false;
    if caller_tabs == 0 {
        if let Some(own) = saved.windows.get(label).filter(|window| !window.tabs.is_empty() || window.native_tab.is_some()) {
            plans.push((label.to_owned(), own.clone()));
        } else if let Some(main) = saved.windows.get("main").filter(|window| !window.tabs.is_empty() || window.native_tab.is_some()) {
            plans.push((label.to_owned(), main.clone()));
            main_planned = true;
        }
    }
    for (window_label, session) in &saved.windows {
        if window_label == label || (main_planned && window_label == "main") {
            continue;
        }
        if session.tabs.is_empty() && session.native_tab.is_none() {
            skipped.push(window_label.clone());
        } else if !open.contains(window_label) {
            plans.push((window_label.clone(), session.clone()));
        } else if window_label == "main" && main_tabs == 0 {
            plans.push(("main".into(), session.clone()));
        } else if window_label == "main" {
            // Never replace live reader tabs: reopen the saved main session in
            // its own native window.
            plans.push((format!("window-{}", Uuid::new_v4()), session.clone()));
        } else if live_tabs.get(window_label).copied().unwrap_or(0) == 0 {
            // Open but empty: the saved tabs are pushed into it instead of the
            // restore silently doing nothing for that window.
            plans.push((window_label.clone(), session.clone()));
        } else {
            skipped.push(window_label.clone());
        }
    }
    RestorePlan { plans, skipped }
}

impl Service {
    pub fn launch_generation(&self) -> u64 {
        self.privacy_generation.load(Ordering::SeqCst)
    }

    #[cfg(any(desktop, target_os = "ios", test))]
    pub async fn queue_launch_locations(
        &self,
        target_window: &str,
        locations: Vec<store::TabLocation>,
        generation: u64,
    ) -> Result<(), String> {
        let _mode_guard = self.privacy_gate.read().await;
        self.ensure_running()?;
        self.ensure_window_active(target_window)?;
        self.ensure_generation(&json!({"privacyGeneration": generation}))?;
        let reference_revision = self.store.read()?.reference_revision;
        let mut queue = self
            .launch_locations
            .lock()
            .map_err(|_| "Launch queue unavailable")?;
        for location in locations {
            let mut location = value(location)?;
            location["launchId"] = json!(Uuid::new_v4().to_string());
            location["targetWindow"] = json!(target_window);
            location["referenceRevision"] = json!(reference_revision);
            queue.push(location);
        }
        Ok(())
    }

    /// Queue an external "Look up in Kiwix" request for `target_window`,
    /// replacing any request the shell has not acknowledged yet. The query was
    /// sanitised by `launch_search`; it is only ever used to prefill a search.
    #[cfg(any(desktop, test))]
    pub async fn queue_launch_search(
        &self,
        target_window: &str,
        query: String,
        generation: u64,
    ) -> Result<(), String> {
        let _mode_guard = self.privacy_gate.read().await;
        self.ensure_running()?;
        self.ensure_window_active(target_window)?;
        self.ensure_generation(&json!({"privacyGeneration": generation}))?;
        *self
            .launch_search
            .lock()
            .map_err(|_| "Launch search unavailable")? = Some(json!({
            "launchId": Uuid::new_v4().to_string(),
            "targetWindow": target_window,
            "query": query,
        }));
        Ok(())
    }

    /// Resolve external `zim://` links to ordinary launch locations.
    ///
    /// A link can only name an archive already present in the library; it never
    /// imports, downloads or resolves a filesystem path. Unknown archives,
    /// unavailable archives and malformed links reject the whole batch before
    /// anything is queued, so one bad link cannot partially open others.
    // Desktop links arrive only through macOS `RunEvent::Opened`.
    #[cfg(any(target_os = "macos", target_os = "ios", test))]
    pub async fn queue_zim_links(
        &self,
        target_window: &str,
        links: Vec<String>,
        generation: u64,
    ) -> Result<usize, String> {
        if links.is_empty() {
            return Ok(0);
        }
        if links.len() > 32 {
            return Err("Too many zim:// links in one request".into());
        }
        let mut locations = Vec::with_capacity(links.len());
        let count = links.len();
        for raw in links {
            let link = crate::zim_scheme::parse(&raw)?;
            let book = self.store.book(&link.book_id)?.ok_or_else(|| {
                format!(
                    "No archive in the library has ID {}; import it before opening this link",
                    link.book_id
                )
            })?;
            if !book.available {
                return Err(format!(
                    "Archive “{}” is unavailable; locate it in the library before opening this link",
                    book.metadata.title
                ));
            }
            let archive_path = link.path.as_deref().unwrap_or(&book.metadata.main_path);
            locations.push(store::TabLocation {
                book_id: book.metadata.id.clone(),
                path: format!("{}{}", encode_path(archive_path), link.suffix),
                title: book.metadata.title.clone(),
                scroll: 0.0,
            });
        }
        self.queue_launch_locations(target_window, locations, generation)
            .await?;
        Ok(count)
    }

    /// Mobile startup, plus the tests that open a profile directly. Desktop
    /// startup claims the profile before the builder (`new_checked` or
    /// `new_with_claimed_lock`), so it never takes this path.
    #[cfg(any(mobile, test))]
    pub fn new(data_dir: PathBuf, private: bool, read_only: bool) -> Result<Arc<Self>, String> {
        #[cfg(mobile)]
        let store = Arc::new(Store::new_mobile(data_dir, read_only)?);
        #[cfg(desktop)]
        let store = Arc::new(Store::new_with_mode(data_dir.clone(), read_only)?);
        Self::from_store(store, private, read_only)
    }

    /// Desktop startup that claims the profile itself. The busy outcome stays
    /// typed so the launch path can activate the owner instead of aborting.
    #[cfg(desktop)]
    pub fn new_checked(
        data_dir: PathBuf,
        private: bool,
        read_only: bool,
    ) -> Result<Arc<Self>, crate::store::StoreOpenError> {
        let store = Arc::new(Store::new_with_mode_checked(data_dir, read_only)?);
        Self::from_store(store, private, read_only).map_err(crate::store::StoreOpenError::Fatal)
    }

    /// Desktop startup that already claimed the profile lock before Tauri built
    /// a window. The service adopts that descriptor instead of locking again.
    #[cfg(all(desktop, unix))]
    pub fn new_with_claimed_lock(
        data_dir: PathBuf,
        private: bool,
        read_only: bool,
        lock_file: std::fs::File,
    ) -> Result<Arc<Self>, crate::store::StoreOpenError> {
        let store = Arc::new(Store::new_with_claimed_lock(
            data_dir,
            read_only,
            crate::portable::PortableProfile::detect()
                .map_err(crate::store::StoreOpenError::Fatal)?,
            lock_file,
        )?);
        Self::from_store(store, private, read_only).map_err(crate::store::StoreOpenError::Fatal)
    }

    fn from_store(store: Arc<Store>, private: bool, read_only: bool) -> Result<Arc<Self>, String> {
        let data_dir = store.data_dir().to_path_buf();
        if cfg!(debug_assertions) || std::env::var_os("KIWIX_NEARBY_DEBUG").is_some() {
            crate::nearby_log::set_path(data_dir.join("nearby-debug.log"));
        }
        store.set_runtime_mode(private, read_only)?;
        #[cfg(not(mobile))]
        crate::crash::set_read_only(store.read_only()?);
        // Persisted validation outcomes seed the crash reporter so a later
        // reader panic can name the archive's last known state without a new
        // filesystem read on the panic path.
        if let Ok(state) = store.read() {
            for book in &state.books {
                if let Some(record) = book.integrity.as_ref().filter(|record| record.matches(book)) {
                    crate::crash::note_archive_validation(
                        &book.metadata.id,
                        &book.installation_id,
                        match record.status {
                            crate::integrity::IntegrityStatus::Valid => "valid",
                            crate::integrity::IntegrityStatus::Invalid => "invalid",
                        },
                    );
                }
            }
        }
        let settings = store.settings()?;
        let load_warnings = store.load_warnings().to_vec();
        #[cfg(mobile)]
        // A quarantined state file is a user-visible notice: the profile is
        // empty until the original is restored. Other skipped records stay in
        // Troubleshooting only.
        let startup_warnings: Vec<String> = load_warnings.iter()
            .filter(|warning| *warning == crate::mobile_rebase::STATE_QUARANTINED)
            .cloned()
            .collect();
        #[cfg(not(mobile))]
        let startup_warnings: Vec<String> = Vec::new();
        if !load_warnings.is_empty() {
            // Recoverable record skips are recorded in Troubleshooting only.
            // They must not paint the blocking migration band on every launch.
            crate::diagnostics::record(
                &store,
                "application_operation",
                Some("Some saved items could not be read and were skipped"),
            );
        }
        let spelling = Arc::new(SpellingManager::new(data_dir.join("spelling")));
        spelling.set_writable(
            settings.spelling_enabled && !settings.private_mode && !settings.read_only,
        )?;
        let service = Arc::new(Self {
            update_admission: crate::update_admission::AdmissionGate::default(),
            catalog_cache: crate::catalog::CatalogCache::new(store.clone()),
            catalog_index: crate::catalog::CatalogIndex::new(store.clone()),
            editions: crate::edition_updates::EditionManager::new(),
            edition_errors: Mutex::new(HashMap::new()),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            notifications: crate::notifications::NotificationManager::new(store.clone()),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            notification_gui: AtomicBool::new(false),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            notification_launch_gate: Mutex::new(()),
            #[cfg(target_os = "android")]
            android_notifications: Arc::new(android_notifications_service::AndroidNotifications::default()),
            store,
            #[cfg(target_os = "ios")]
            official_catalog: Mutex::new(crate::ios_background::OfficialCatalogRegistry::default()),
            downloads: tokio::sync::OnceCell::new(),
            localswarm: tokio::sync::OnceCell::new(),
            shared_library: tokio::sync::OnceCell::new(),
            private_sync: tokio::sync::OnceCell::new(),
            localswarm_admission: Mutex::new(LocalSwarmAdmission {
                cancel: tokio_util::sync::CancellationToken::new(),
                retirements: 0,
                generation: 0,
            }),
            #[cfg(test)]
            localswarm_initialization_gate: Mutex::new(None),
            localswarm_sources: tokio::sync::Mutex::new(()),
            hotspot_use: tokio::sync::Mutex::new(()),
            maps: Mutex::new(None),
            torrents: Mutex::new(None),
            cli_read_only: read_only,
            migration_active: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(windows)]
            archive_moves: std::sync::atomic::AtomicUsize::new(0),
            privacy_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            privacy_policy_generation: std::sync::atomic::AtomicU64::new(1),
            privacy_gate: Arc::new(tokio::sync::RwLock::new(())),
            launch_locations: Mutex::new(Vec::new()),
            #[cfg(any(desktop, test))]
            launch_search: Mutex::new(None),
            archives: Mutex::new(VecDeque::new()),
            #[cfg(test)]
            archive_open_hook: Mutex::new(None),
            searches: Mutex::new(HashMap::new()),
            pdf_jobs: Mutex::new(HashMap::new()),
            pdf_idle: tokio::sync::Notify::new(),
            integrity: crate::integrity::IntegrityManager::default(),
            readers: tokio::sync::Mutex::new(HashMap::new()),
            sensors: Mutex::new(HashMap::new()),
            shell_origins: Mutex::new(HashSet::from([
                "tauri://localhost".into(),
                "http://tauri.localhost".into(),
                "https://tauri.localhost".into(),
                "http://127.0.0.1:1420".into(),
            ])),
            imports: Mutex::new(Arc::new(ImportManager::new()?)),
            spelling,
            stopping: AtomicBool::new(false),
            startup_warnings: Mutex::new(startup_warnings),
            #[cfg(test)]
            mutation_test_gate: Mutex::new(None),
            closed_windows: Mutex::new(HashSet::new()),
            #[cfg(any(target_os = "android", test))]
            android_window: Mutex::new(AndroidWindowAuthority::default()),
            #[cfg(any(target_os = "android", test))]
            android_window_gate: Arc::new(tokio::sync::RwLock::new(())),
            app: Mutex::new(None),
            sharing: tokio::sync::Mutex::new(None),
            nearby: nearby_service::NearbyState::default(),
            #[cfg(target_os = "android")]
            hotspot: Mutex::new(crate::hotspot::HotspotState::default()),
            #[cfg(desktop)]
            watcher: Mutex::new(None),
            #[cfg(desktop)]
            watch_failures: Arc::new(Mutex::new(crate::watch::FailureLog::default())),
            #[cfg(desktop)]
            session_snapshot_at: Mutex::new(None),
            #[cfg(desktop)]
            restored_windows: Mutex::new(HashSet::new()),
            ready_frontends: Mutex::new(HashSet::new()),
            #[cfg(mobile)]
            mobile: crate::mobile::MobileState::default(),
            speech: mobile_speech::SpeechManager::default(),
            speech_prepare: tokio::sync::Mutex::new(()),
            #[cfg(desktop)]
            agent: Default::default(),
            #[cfg(desktop)]
            photo: Default::default(),
            kokoro: Arc::new(kokoro::KokoroManager::new(&data_dir)),
        });
        #[cfg(mobile)]
        if !private && !read_only { service.kokoro.ensure_performance_probe(); }
        Ok(service)
    }
    pub fn mark_frontend_ready(&self, label: &str) -> Result<(), String> {
        self.ready_frontends
            .lock()
            .map_err(|_| "Frontend readiness state unavailable")?
            .insert(label.to_string());
        Ok(())
    }

    pub fn is_frontend_ready(&self, label: &str) -> bool {
        self.ready_frontends
            .lock()
            .map(|labels| labels.contains(label))
            .unwrap_or(true)
    }

    /// Keep startup problems visible without aborting the launch. The shell
    /// already renders this bounded list above the library.
    pub(crate) fn note_startup_warning(&self, message: String) {
        if let Ok(mut warnings) = self.startup_warnings.lock() {
            if warnings.contains(&message) {
                return;
            }
            if warnings.len() >= 32 {
                warnings.remove(0);
            }
            warnings.push(message);
        }
    }

    #[cfg(any(desktop, test))]
    pub fn allow_shell_origin(&self, origin: String) -> Result<(), String> {
        self.shell_origins
            .lock()
            .map_err(|_| "Origin registry unavailable")?
            .insert(origin);
        Ok(())
    }

    /// Exact origins of the readers currently open. The navigation policy uses
    /// these to allow archive frames without trusting arbitrary loopback ports.
    #[cfg(any(desktop, test))]
    pub fn reader_origins(&self) -> Vec<String> {
        self.readers
            .try_lock()
            .map(|readers| readers.values().map(|reader| reader.origin.clone()).collect())
            .unwrap_or_default()
    }
    // Lock order is window liveness, then Store. Storage/sensor commits retain
    // liveness through their Store transaction; never reacquire it inside one.
    fn active_window_guard(
        &self,
        label: &str,
    ) -> Result<std::sync::MutexGuard<'_, HashSet<String>>, String> {
        let closed = self
            .closed_windows
            .lock()
            .map_err(|_| "Window state unavailable")?;
        if closed.contains(label) {
            Err("This application window has closed".into())
        } else {
            Ok(closed)
        }
    }
    fn ensure_window_active(&self, label: &str) -> Result<(), String> {
        self.active_window_guard(label).map(|_| ())
    }
    #[cfg(mobile)]
    pub(crate) fn nearby_root_reload(self: &Arc<Self>) {
        let owners = self.nearby_cancel_window("main");
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = service.nearby_release_owners(owners).await {
                eprintln!("Nearby reload retirement failed: {error}");
            }
        });
    }
    /// The visible title bar is hidden, but AppKit's native tab bar and Window
    /// menu label a tab by its window title. Keep that label on the active
    /// article so a native tab group is distinguishable; the archive title is
    /// already on screen, so this persists no new information.
    #[cfg(desktop)]
    fn apply_window_title(&self, label: &str, surface: &str, tabs: &[Tab], active: Option<&str>) {
        if label != "main" && !label.starts_with("window-") {
            return;
        }
        let article = (surface == "reader")
            .then(|| active.and_then(|id| tabs.iter().find(|tab| tab.id == id)))
            .flatten()
            .filter(|tab| tab.book_id.is_some())
            .map(|tab| {
                tab.title
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(160)
                    .collect::<String>()
            })
            .filter(|title| !title.trim().is_empty());
        let title = article.unwrap_or_else(|| "Kiwix".into());
        let app = match self.app.lock() {
            Ok(app) => app.clone(),
            Err(_) => None,
        };
        let Some(window) = app.and_then(|app| app.get_webview_window(label)) else {
            return;
        };
        if window.title().ok().as_deref() != Some(title.as_str()) {
            let _ = window.set_title(&title);
        }
    }
    /// Keep the explicit-restore snapshot close to the live session. AppKit's
    /// `terminate:` Quit path bypasses `RunEvent::ExitRequested`, so waiting for
    /// the graceful Quit flush would lose the session on Cmd+Q. One write per
    /// second is enough for a restorable snapshot and far cheaper than the
    /// state transaction this follows.
    #[cfg(desktop)]
    fn refresh_session_snapshot(&self) {
        let now = std::time::Instant::now();
        let Ok(mut last) = self.session_snapshot_at.lock() else {
            return;
        };
        if last.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(1)) {
            return;
        }
        *last = Some(now);
        drop(last);
        let open: HashSet<String> = match self.app.lock() {
            Ok(app) => app
                .as_ref()
                .map(|app| {
                    app.webview_windows()
                        .into_keys()
                        .filter(|label| label == "main" || label.starts_with("window-"))
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => return,
        };
        if open.is_empty() {
            return;
        }
        if let Err(error) = self.store.save_session_snapshot(&open) {
            eprintln!("Saving the restorable session: {error}");
        }
    }
    /// Register a fresh document, or its retained identity on a newer Activity,
    /// only after the previous source's owned drain.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn android_open_window(&self, token: String, activity_id: i64) -> Result<(), String> {
        self.ensure_running()?;
        if Uuid::parse_str(&token).map_err(|_| "Invalid Android window incarnation")?.to_string() != token {
            return Err("Reopen the app to continue".into());
        }
        if activity_id <= 0 { return Err("Reopen the app to continue".into()); }
        let mut authority = self.android_window.lock().map_err(|_| "The app window is unavailable. Reopen the app to continue")?;
        if authority.token.as_deref() == Some(token.as_str())
            && authority.activity_id.is_some_and(|previous| activity_id <= previous)
        {
            return Err("This saved window is out of date. Close it and open a new window".into());
        }
        if authority.token.is_some() {
            let result = authority.drain.as_ref().and_then(|drain| drain.borrow().clone());
            match result {
                Some(Ok(())) => {}
                Some(Err(error)) => return Err(format!("Android window retirement failed: {error}")),
                None => return Err("The previous Android window has not drained".into()),
            }
        }
        self.closed_windows.lock().map_err(|_| "Window state unavailable")?.remove("main");
        authority.token = Some(token);
        authority.activity_id = Some(activity_id);
        authority.accepting = true;
        authority.drain = None;
        Ok(())
    }

    #[cfg(any(target_os = "android", test))]
    pub(crate) fn android_window_token(&self) -> Option<String> {
        self.android_window.lock().ok().and_then(|authority| authority.token.clone())
    }

    /// Native UI admission must already be closed for the retiring Activity.
    /// Close Rust admission before any await; the detached task owns the drain
    /// even when the lifecycle callback awaiting it is cancelled.
    #[cfg(any(target_os = "android", test))]
    pub(crate) async fn android_retire_window(self: Arc<Self>, token: &str) -> Result<(), String> {
        let mut drain = {
            let mut authority = self.android_window.lock().map_err(|_| "The app window is unavailable. Reopen the app to continue")?;
            if authority.token.as_deref() != Some(token) {
                return Err("This window is out of date. Close it and open a new window".into());
            }
            if let Some(drain) = &authority.drain {
                drain.clone()
            } else {
                authority.accepting = false;
                self.closed_windows.lock().map_err(|_| "Window state unavailable")?.insert("main".into());
                self.retire_searches(Some("main"));
                self.searches.lock().map_err(|_| "Search ownership unavailable")?.remove("main");
                if let Some(manager) = self.sensors.lock().map_err(|_| "Sensor manager unavailable")?.get("main") {
                    manager.deactivate();
                }
                #[cfg(mobile)]
                let speech = self.mobile_speech_block(mobile_speech::Scope::Window("main".into()));
                let (completion, drain) = tokio::sync::watch::channel(None);
                authority.drain = Some(drain.clone());
                let service = self.clone();
                tokio::spawn(async move {
                    let result = {
                        #[cfg(mobile)]
                        let _speech = speech;
                        let _lifetime = service.android_window_gate.write().await;
                        let _mode = service.privacy_gate.read().await;
                        // Internal cleanup bypasses external admission, which is
                        // already closed, and cannot reacquire the lifetime gate.
                        service.clone().dispatch_inner("close_window", json!({"__windowLabel":"main"}), None).await.map(|_| ())
                    };
                    let _ = completion.send_replace(Some(result));
                });
                drain
            }
        };
        loop {
            if let Some(result) = drain.borrow().clone() { return result; }
            drain.changed().await.map_err(|_| "Android window drain stopped before completion")?;
        }
    }

    /// Source identity is captured once by the source document, including its
    /// immutable per-WebView Activity generation, never from label-routed state.
    #[cfg(any(target_os = "android", test))]
    pub(crate) async fn android_dispatch(
        self: Arc<Self>,
        command: &str,
        args: Value,
        source: Option<String>,
        source_activity: Option<String>,
    ) -> Result<Value, String> {
        let lifetime = self.android_window_gate.clone().read_owned().await;
        {
            let authority = self.android_window.lock().map_err(|_| "The app window is unavailable. Reopen the app to continue")?;
            let activity_id = source_activity.as_deref().and_then(|id| id.parse::<i64>().ok());
            if !authority.accepting || source.is_none()
                || authority.token.as_deref() != source.as_deref()
                || activity_id.is_none() || authority.activity_id != activity_id
            {
                return Err("This window is out of date. Reopen the app to continue".into());
            }
            if args.get("__windowLabel").and_then(Value::as_str) != Some("main") {
                return Err("Open Kiwix from the app window to continue".into());
            }
            self.ensure_window_active("main")?;
        }
        self.ensure_running()?;
        if matches!(command, "save_pdf" | "mobile_save_pdf" | "share_pdf") {
            let job = self.admit_pdf(command, &args)?;
            // The accepted job is registered and owns the immutable composite
            // source identity before retirement can acquire the lifetime gate.
            drop(lifetime);
            return tokio::spawn(async move { self.export_pdf(args, job).await }).await
                .map_err(|error| format!("Android PDF operation stopped: {error}"))?;
        }
        let command = command.to_owned();
        tokio::spawn(async move {
            let _lifetime = lifetime;
            self.operation(&command, args).await
        }).await.map_err(|error| format!("Android application operation stopped: {error}"))?
    }

    fn ensure_generation(&self, args: &Value) -> Result<u64, String> {
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
        Ok(generation)
    }
    /// Signal accepted search work before waiting for its privacy read lease.
    pub(super) fn retire_searches(&self, window: Option<&str>) {
        let searches = self.searches.lock().unwrap_or_else(|error| error.into_inner());
        for (label, session) in searches.iter() {
            if window.is_none_or(|window| window == label) { session.cancel(); }
        }
    }
    fn search_control(&self, label: &str, command: &str, args: &Value) -> Result<Value, String> {
        let _window = if !matches!(command, "cancel_search" | "search_release") {
            Some(self.active_window_guard(label)?)
        } else { None };
        let mut searches = self.searches.lock().map_err(|_| "Search ownership unavailable")?;
        if command == "search_session" {
            if !searches.contains_key(label) && searches.len() >= 16 {
                return Err("Too many search windows".into());
            }
            let id = Uuid::new_v4().to_string();
            if let Some(old) = searches.insert(label.into(), SearchSession { id: id.clone(), owners: HashMap::new() }) {
                old.cancel();
            }
            return Ok(json!({"sessionId":id}));
        }
        let session_id = text(args, "sessionId")?;
        let session = searches.get_mut(label).filter(|session| session.id == session_id);
        let Some(session) = session else {
            return if matches!(command, "cancel_search" | "search_release") {
                Ok(json!({"status":"retired"}))
            } else {
                Err("This search is no longer available. Start a new search".into())
            };
        };
        if command == "search_owner" {
            if session.owners.len() >= 201 { return Err("Too many search consumers".into()); }
            let owner_id = Uuid::new_v4().to_string();
            session.owners.insert(owner_id.clone(), SearchOwner { request_id: 0, cancel: None });
            return Ok(json!({"ownerId":owner_id}));
        }
        let owner_id = text(args, "ownerId")?;
        if command == "search_release" {
            if let Some(owner) = session.owners.remove(owner_id) {
                if let Some(cancel) = owner.cancel { cancel.cancel(); }
                return Ok(json!({"status":"released"}));
            }
            return Ok(json!({"status":"retired"}));
        }
        let request_id = args.get("requestId").and_then(Value::as_u64).filter(|id| *id != 0)
            .ok_or("Missing search request identity")?;
        let Some(owner) = session.owners.get_mut(owner_id) else {
            return Ok(json!({"status":"retired"}));
        };
        if request_id < owner.request_id { return Ok(json!({"status":"superseded"})); }
        if let Some(cancel) = owner.cancel.take() { cancel.cancel(); }
        // A cancellation can arrive before its search IPC. Retain just this
        // owner's latest epoch so the delayed start cannot resurrect work.
        owner.request_id = request_id;
        Ok(json!({"status":"cancelled"}))
    }
    fn search_token(&self, label: &str, args: &Value) -> Result<tokio_util::sync::CancellationToken, String> {
        let _window = self.active_window_guard(label)?;
        self.ensure_generation(args)?;
        let mut searches = self.searches.lock().map_err(|_| "Search ownership unavailable")?;
        let session = searches.get_mut(label)
            .filter(|session| Some(session.id.as_str()) == args.get("sessionId").and_then(Value::as_str))
            .ok_or("This search is no longer available. Start a new search")?;
        let owner = session.owners.get_mut(text(args, "ownerId")?).ok_or("This search is no longer available. Start a new search")?;
        let request_id = args.get("requestId").and_then(Value::as_u64).filter(|id| *id != 0)
            .ok_or("Missing search request identity")?;
        if request_id <= owner.request_id { return Err("Search cancelled".into()); }
        if let Some(cancel) = owner.cancel.take() { cancel.cancel(); }
        let cancel = tokio_util::sync::CancellationToken::new();
        owner.request_id = request_id;
        owner.cancel = Some(cancel.clone());
        Ok(cancel)
    }
    pub fn sensor_manager(&self, label: &str) -> SensorManager {
        self.sensors
            .lock()
            .unwrap()
            .entry(label.into())
            .or_default()
            .clone()
    }
    /// Whether this build has an app-owned location provider (`native_location`
    /// in build.rs). Apple's CoreLocation and the Android plugin also need the
    /// running application.
    fn native_location_supported(&self) -> bool {
        if !cfg!(native_location) {
            false
        } else if cfg!(any(target_os = "macos", target_os = "ios", target_os = "android")) {
            self.app.lock().is_ok_and(|app| app.is_some())
        } else {
            true
        }
    }
    fn sensor_status(&self, manager: &SensorManager, session: &str) -> Value {
        self.sensor_status_for(manager, session, None)
    }
    /// An inactive session still reports the installation's remembered decision
    /// when the trusted shell names it. Otherwise a session that was merely
    /// deactivated would masquerade as an explicit denial and overwrite the UI.
    fn sensor_status_for(
        &self,
        manager: &SensorManager,
        session: &str,
        installation: Option<&str>,
    ) -> Value {
        let decision = |kind| match installation {
            Some(installation) if !manager.owns_session_id(session) => {
                manager.installation_decision(installation, kind)
            }
            _ => manager.decision(session, kind),
        };
        json!({"camera":decision(SensorKind::Camera),"location":decision(SensorKind::Location),"nativeLocation":self.native_location_supported()})
    }
    fn writable(&self) -> Result<(), String> {
        if self.store.read_only()? {
            Err("The application is in strict read-only mode".into())
        } else {
            Ok(())
        }
    }
    /// The side-effect-free admission checks of `save_settings`, returning the
    /// current and merged settings. `operation` runs them before it retires any
    /// work for a mode switch, so a save that will be refused (stale privacy
    /// generation, read-only rule, invalid value) cannot first cancel LocalSwarm,
    /// Nearby, private sync, integrity checks or PDF jobs. Dispatch runs them
    /// again under the privacy writer, which remains authoritative.
    pub(super) fn validate_settings_patch(&self, args: &Value) -> Result<(store::Settings, store::Settings), String> {
        let patch = args
            .get("settings")
            .and_then(Value::as_object)
            .ok_or("Missing settings object")?;
        if patch.contains_key("speechPreferences") {
            return Err("Speech preferences require a language-level edit".into());
        }
        if patch.contains_key("pinnedBookIds") {
            return Err("Pinned archives require the dedicated pin action".into());
        }
        if let Some(generation) = args.get("privacyGeneration") {
            if generation.as_u64() != Some(self.privacy_generation.load(Ordering::SeqCst)) {
                return Err("Reader preferences expired after a privacy transition".into());
            }
        }
        if self.cli_read_only {
            return Err("This profile was opened with --read-only; restart without that flag to change settings".into());
        }
        let previous = self.store.settings()?;
        if previous.read_only
            && !(patch.len() == 1 && patch.get("readOnly") == Some(&Value::Bool(false)))
        {
            return Err("Leave read-only mode before changing settings".into());
        }
        let mut merged = serde_json::to_value(&previous).map_err(|e| e.to_string())?;
        let object = merged.as_object_mut().ok_or("Settings are not an object")?;
        for (k, v) in patch {
            object.insert(k.clone(), v.clone());
        }
        let settings: store::Settings =
            serde_json::from_value(merged).map_err(|e| format!("Invalid settings: {e}"))?;
        settings.validate()?;
        #[cfg(any(mobile, test))]
        if settings.download_directory != previous.download_directory
            && crate::library_root::has_pending_operation(&self.store)?
        {
            return Err("Finish recovering the pending storage move before changing the download folder".into());
        }
        if settings.completion_notifications && !previous.completion_notifications {
            return Err("Enable completion notifications through the explicit permission action".into());
        }
        Ok((previous, settings))
    }
    fn normal_writable(&self) -> Result<(), String> {
        self.writable()?;
        if self.store.private_mode()? {
            return Err("This operation is disabled in private mode".into());
        }
        Ok(())
    }
    async fn synchronize_crash_policy(&self, read_only: bool) -> Result<(), String> {
        #[cfg(mobile)]
        { self.mobile_set_crash_capture_policy(read_only).await }
        #[cfg(not(mobile))]
        {
            crate::crash::set_read_only(read_only);
            Ok(())
        }
    }
    async fn download_manager(&self) -> Result<DownloadManager, String> {
        self.store.validate_portable_directory(self.store.data_dir())?;
        let manager = self
            .downloads
            .get_or_try_init(|| async move {
                self.writable()?;
                let settings = self.store.settings()?;
                let directory = self.store.data_dir().join("downloads");
                let store = self.store.clone();
                let manager_store = store.clone();
                let admission = self.update_admission.clone();
                let manager = blocking(move || DownloadManager::new(directory, manager_store, admission)).await?;
                manager.set_download_limit(settings.download_limit)?;
                #[cfg(target_os = "ios")]
                manager.reconcile_native().await?;
                if settings.auto_resume_downloads && !cfg!(target_os = "android")
                    && crate::transfer_policy::require_internet().is_ok() {
                    // A job that cannot auto-resume must not leave the cell
                    // uninitialized: the manager records the reason on that job
                    // and stays available to list, pause or cancel it.
                    if let Err(error) = manager.resume_interrupted().await {
                        crate::diagnostics::record(&store, "downloads", Some(&error));
                    }
                }
                Ok::<_, String>(manager)
            })
            .await?;
        Ok(manager.clone())
    }
    /// Run even offline and with auto-resume disabled: OS-owned receipts are
    /// reconciled independently of permission to initiate another transfer.
    #[cfg(target_os = "ios")]
    pub(crate) async fn bootstrap_ios_acquisition(&self) -> Result<(), String> {
        let _update = self.update_admission.admit().await;
        let _transfer = self.mobile.transfer_gate.lock().await;
        let _mode = self.privacy_gate.read().await;
        self.ensure_running()?;
        let settings = self.store.settings()?;
        crate::ios_background::set_policy(settings.private_mode, settings.read_only).await?;
        if !settings.private_mode && !settings.read_only {
            self.download_manager().await?;
        }
        Ok(())
    }
    async fn localswarm_manager(&self, expected_generation: Option<u64>) -> Result<LocalSwarmManager, String> {
        self.normal_writable()?;
        self.store.validate_portable_directory(self.store.data_dir())?;
        let (_, admission) = self.localswarm_admission_token(expected_generation)?;
        let manager = self.localswarm.get_or_try_init(|| async {
            let store = self.store.clone();
            let token = admission.clone();
            #[cfg(test)]
            let gate = self.localswarm_initialization_gate.lock().unwrap().take();
            blocking(move || {
                #[cfg(test)]
                if let Some((entered, release)) = gate {
                    let _ = entered.send(());
                    release.recv().map_err(|error| error.to_string())?;
                }
                LocalSwarmManager::new(store, token)
            }).await
        }).await?;
        #[cfg(target_os = "ios")]
        if !self.mobile_active() {
            return Err("Return to the foreground before starting a LocalSwarm operation".into());
        }
        manager.activate(&admission)?;
        let settings = self.store.settings()?;
        manager.limits(settings.download_limit, settings.upload_limit);
        Ok(manager.clone())
    }
    async fn shared_library_client(&self) -> Result<&crate::shared_library::SharedLibraryClient, String> {
        self.shared_library.get_or_try_init(|| async {
            let discovery = self.localswarm_manager(None).await?;
            Ok::<_, String>(crate::shared_library::SharedLibraryClient::new(self.store.clone(), discovery))
        }).await
    }
    fn localswarm_admission_token(&self, expected_generation: Option<u64>) -> Result<(u64, tokio_util::sync::CancellationToken), String> {
        let admission = self.localswarm_admission.lock().map_err(|_| "LocalSwarm admission unavailable")?;
        if expected_generation.is_some_and(|generation| generation != admission.generation) {
            return Err("Nearby sharing was stopped. Select the share or resume the download again.".into());
        }
        if admission.retirements != 0 || admission.cancel.is_cancelled() {
            return Err("Nearby sharing stopped. Turn it on again".into());
        }
        Ok((admission.generation, admission.cancel.child_token()))
    }
    fn retire_localswarm(&self) {
        self.retire_private_sync();
        self.nearby.cancel_all();
        {
            let mut admission = self.localswarm_admission.lock().unwrap();
            admission.retirements += 1;
            admission.generation += 1;
            admission.cancel.cancel();
        }
        if let Some(manager) = self.localswarm.get() { manager.interrupt(); }
        if let Some(client) = self.shared_library.get() { client.invalidate(); }
    }
    fn finish_localswarm_retirement(&self, retired: bool) -> Result<(), String> {
        let normal = !self.stopping.load(Ordering::SeqCst)
            && !self.store.private_mode()? && !self.store.read_only()?;
        let mut admission = self.localswarm_admission.lock().map_err(|_| "LocalSwarm admission unavailable")?;
        if retired { admission.retirements -= 1; }
        if normal && admission.retirements == 0 && admission.cancel.is_cancelled() {
            admission.cancel = tokio_util::sync::CancellationToken::new();
        }
        Ok(())
    }
    async fn localswarm_status(&self) -> Result<Value, String> {
        let mut status = match self.localswarm.get() {
            Some(manager) => manager.status().await?,
            None => json!({"discovery":false,"peers":[],"seeds":[],"error":null}),
        };
        let nearby = self.nearby_status()?;
        if let (Some(peers), Some(native)) = (status.get_mut("peers").and_then(Value::as_array_mut),
            nearby.get("peers").and_then(Value::as_array)) {
            peers.extend(native.iter().cloned());
        }
        if nearby.get("discovery").and_then(Value::as_bool) == Some(true) {
            status["discovery"] = json!(true);
        }
        status["nearby"] = nearby;
        Ok(status)
    }
    async fn localswarm_target(&self, args: &Value) -> Result<ConnectionTarget, String> {
        let host = text(args, "host")?;
        let port = args.get("port").and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok()).filter(|port| *port != 0)
            .ok_or("Invalid LocalSwarm port")?;
        let direct = self.store.settings()?.direct_connection;
        if args.get("discoveryId").is_some() {
            let id = text(args, "discoveryId")?;
            let manager = self.localswarm.get().ok_or("The selected LocalSwarm peer is no longer discovered; start discovery and choose it again")?;
            manager.peer_target(id, text(args, "swarmId")?, port, direct).await
        } else {
            Ok(ConnectionTarget::manual(host.to_string(), port).with_direct(direct))
        }
    }
    async fn localswarm_source_mutation(
        &self, book_id: Option<&str>,
    ) -> Result<(tokio::sync::MutexGuard<'_, ()>, Option<tokio::sync::MutexGuard<'_, ()>>), String> {
        // Exists before lazy manager construction, so a first seed cannot
        // slip between a "no manager" observation and the library mutation.
        let sources = self.localswarm_sources.lock().await;
        let manager = match self.localswarm.get() {
            Some(manager) => Some(manager.quiesce_sources(book_id).await?),
            None => None,
        };
        Ok((sources, manager))
    }
    fn map_manager(&self) -> Result<MapManager, String> {
        self.store.validate_portable_directory(self.store.data_dir())?;
        let mut manager = self.maps.lock().map_err(|_| "Map manager unavailable")?;
        if manager.is_none() {
            self.writable()?;
            *manager = Some(MapManager::new(
                self.store.data_dir().join("maps"),
                self.store.clone(),
            )?);
        }
        Ok(manager.as_ref().unwrap().clone())
    }
    fn torrent_manager(&self) -> Result<Arc<TorrentManager>, String> {
        self.store.validate_portable_directory(self.store.data_dir())?;
        let mut manager = self
            .torrents
            .lock()
            .map_err(|_| "Torrent manager unavailable")?;
        if manager.is_none() {
            self.writable()?;
            let created = TorrentManager::new(
                self.store.data_dir().to_path_buf(),
                self.store.settings()?.auto_resume_downloads && !cfg!(target_os = "android")
                    && crate::transfer_policy::require_internet().is_ok(),
                self.store.clone(),
            )?;
            *manager = Some(created);
        }
        Ok(manager.as_ref().unwrap().clone())
    }
    /// True when a saved book cannot be opened from this app's storage: a path
    /// outside managed storage, or a managed file that is no longer present.
    #[cfg(mobile)]
    fn mobile_archive_unusable(&self, book: &crate::store::Book) -> bool {
        if book.source_reference.is_some() {
            return !book.available;
        }
        if !crate::library_root::is_managed_path(&self.store, Path::new(&book.path)) {
            return true;
        }
        !std::fs::metadata(&book.path).is_ok_and(|metadata| metadata.is_file())
    }
    pub fn archive(&self, id: &str) -> Result<Archive, String> {
        let book = self.store.book(id)?.ok_or("Archive is not in the library")?;
        self.archive_for_book(book)
    }

    fn export_archive(&self, args: &Value) -> Result<(Archive, String), String> {
        let id = text(args, "bookId")?;
        let book = self.store.book(id)?.ok_or("Archive is not in the library")?;
        if !book.available || args.get("installationId")
            .is_some_and(|expected| expected.as_str() != Some(&book.installation_id))
        {
            return Err("The archive installation changed or is unavailable; open it again".into());
        }
        let installation = book.installation_id.clone();
        Ok((self.archive_for_book(book)?, installation))
    }

    fn with_export_installation<T>(&self, window: &str, book: &str, installation: &str,
        action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let _window = self.active_window_guard(window)?;
        self.store.with_current_installation(book, installation, action)
    }

    fn prepare_archive_export(&self, args: &Value, destination: &Path, mhtml: bool)
        -> Result<(crate::exports::PreparedOutput, String, String), String>
    {
        let (archive, installation) = self.export_archive(args)?;
        let entry = archive.entry(&crate::server::decode_location_path(text(args, "path")?)?)?;
        let (output, mime) = if mhtml {
            (crate::exports::prepare_mhtml(&archive, &entry.path, destination)?, "multipart/related".into())
        } else {
            (crate::exports::prepare_resource(&archive, &entry.path, destination)?, entry.mime)
        };
        Ok((output, mime, installation))
    }

    fn archive_for_book(&self, book: store::Book) -> Result<Archive, String> {
        let id = book.metadata.id.as_str();
        #[cfg(mobile)]
        if book.source_reference.is_none()
            && !crate::library_root::is_managed_path(&self.store, Path::new(&book.path))
        {
            return Err("This file is outside Kiwix's library storage. Import it into your library, then open it".into());
        }
        // Windows: an open archive pins its file, so a library move would be
        // refused part-way. New opens are refused (not queued) until the move's
        // file phase ends. Best effort: an open already past this check, or a
        // clone held elsewhere, makes that one book's move fail and roll back.
        #[cfg(windows)]
        self.refuse_during_archive_move()?;
        // Publish the pending slot before opening, not the native handle after
        // opening. Same-key callers then share one initialization without
        // holding the global LRU lock during provider/native work.
        let slot = {
            let mut cache = self
                .archives
                .lock()
                .map_err(|_| "Archive cache unavailable")?;
            if let Some(index) = cache.iter().position(|item| {
                item.id == id
                    && item.installation_id == book.installation_id
                    && item.path == book.path
            }) {
                let item = cache.remove(index).unwrap();
                let slot = item.archive.clone();
                cache.push_back(item);
                slot
            } else {
                let slot = Arc::new(OnceLock::new());
                if cache.len() >= 16 {
                    cache.pop_front();
                }
                cache.push_back(CachedArchive {
                    id: id.into(),
                    installation_id: book.installation_id.clone(),
                    path: book.path.clone(),
                    archive: slot.clone(),
                });
                slot
            }
        };
        #[cfg(test)]
        {
            // A per-Service rendezvous lets regressions hold every caller after
            // cache admission without serializing them on either cache lock.
            let hook = self.archive_open_hook.lock().unwrap().clone();
            if let Some(hook) = hook { hook(); }
        }
        let result = slot.get_or_init(|| {
            let retained = crate::archive_source::retain(&book)
                .map_err(|error| crate::provider_reference::archive_error_context(&book, error))?;
            let archive = retained.open().map_err(|e| {
                format!(
                    "Cannot open {}: {e}. Locate the archive to reconnect it.",
                    book.metadata.title
                )
            })?;
            if archive.metadata()?.id != id {
                return Err("Archive file identity changed; import it separately or locate the original archive".into());
            }
            Ok(archive)
        }).clone();
        if result.is_err() {
            // Share this attempt's error with its waiters, but permit a later
            // request to retry. Never remove a replacement slot after an
            // invalidation, nor republish a retired slot on successful open.
            let mut cache = self
                .archives
                .lock()
                .map_err(|_| "Archive cache unavailable")?;
            cache.retain(|item| !Arc::ptr_eq(&item.archive, &slot));
        }
        result
    }
    /// Windows: refuse work that opens archives while a library move is in
    /// its file phase (see `archive_moves`).
    #[cfg(windows)]
    fn refuse_during_archive_move(&self) -> Result<(), String> {
        if self.archive_moves.load(Ordering::SeqCst) > 0 {
            return Err("Archives are being moved; try again when the move finishes".into());
        }
        Ok(())
    }
    fn invalidate(&self, id: &str) {
        if let Ok(mut cache) = self.archives.lock() {
            cache.retain(|item| item.id != id);
        }
    }
    pub fn import_paths(&self, paths: Vec<String>, managed: bool) -> Result<Value, String> {
        self.writable()?;
        #[cfg(mobile)]
        let paths = crate::library_files::mobile_primary_archives(paths)?;
        let mut books = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let result = (|| {
                #[cfg(mobile)]
                crate::library_files::mobile_import_paths(self.store.data_dir(), std::iter::once(path.as_str()))?;
                // A manually chosen file is admitted only after the same
                // integrity check as a download. A corrupt copy must fail here,
                // not when the reader opens it.
                let book = load_book(&path, managed, true)?;
                self.register_books(std::slice::from_ref(&book), None)?;
                self.store
                    .book(&book.metadata.id)?
                    .ok_or_else(|| "Imported archive disappeared".to_string())
            })();
            match result {
                Ok(book) => books.push(book),
                Err(error) => errors.push(json!({"path":path,"error":error})),
            }
        }
        #[cfg(mobile)]
        if !books.is_empty() { self.mobile_apply_archive_backup()?; }
        Ok(json!({"books":books.iter().map(Book::shell_value).collect::<Vec<_>>(),"errors":errors}))
    }
    fn monitored_import_claims(&self) -> Result<MonitoredImportClaims, String> {
        // Read durable history once per scan without constructing a downloader,
        // torrent session or LocalSwarm node just to exclude its publications.
        // A live download manager contributes its in-memory ledger instead, and
        // the whole set is queried per file rather than rebuilt per file.
        Ok(MonitoredImportClaims {
            downloads: Some(match self.downloads.get() {
                Some(manager) => manager.monitored_paths()?,
                None => DownloadManager::saved_monitored_paths(&self.store)?,
            }),
            maps: if self.maps.lock().map_err(|_| "Map manager unavailable")?.is_none() {
                Some(MapManager::saved_monitored_paths(&self.store)?)
            } else { None },
            torrents: if self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.is_none() {
                Some(TorrentManager::saved_monitored_paths(&self.store)?)
            } else { None },
            localswarm: if self.localswarm.get().is_none() {
                Some(LocalSwarmManager::saved_monitored_paths(&self.store)?)
            } else { None },
        })
    }
    fn monitored_path_is_claimed(
        &self,
        path: &Path,
        saved: &MonitoredImportClaims,
    ) -> Result<bool, String> {
        // A manager may have initialized since the scan began. Its snapshot is
        // still the live ledger, and an unavailable claim set is an error, never
        // "unclaimed".
        if saved.downloads.as_ref()
            .map(|claims| claims.claims_monitored_path(path))
            .transpose()?
            .unwrap_or(false)
        { return Ok(true); }
        {
            let maps = self.maps.lock().map_err(|_| "Map manager unavailable")?;
            if match maps.as_ref() {
                Some(manager) => manager.claims_monitored_path(path)?,
                None => saved.maps.as_ref().map(|claims| claims.claims_monitored_path(path)).transpose()?.unwrap_or(false),
            } { return Ok(true); }
        }
        {
            let torrents = self.torrents.lock().map_err(|_| "Torrent manager unavailable")?;
            if match torrents.as_ref() {
                Some(manager) => manager.claims_monitored_path(path)?,
                None => saved.torrents.as_ref().map(|claims| claims.claims_monitored_path(path)).transpose()?.unwrap_or(false),
            } { return Ok(true); }
        }
        match self.localswarm.get() {
            Some(manager) => manager.claims_monitored_path(path),
            None => saved.localswarm.as_ref().map(|claims| claims.claims_monitored_path(path)).transpose().map(|claimed| claimed.unwrap_or(false)),
        }
    }
    fn active_torrent_share(&self, book_id: &str) -> Result<Option<String>, String> {
        let torrents = self
            .torrents
            .lock()
            .map_err(|_| "Torrent manager unavailable")?;
        match torrents.as_ref() {
            Some(manager) => manager.active_share_for_book(book_id),
            None => TorrentManager::saved_active_share(&self.store, book_id),
        }
    }
    fn import_monitored_paths(&self, paths: Vec<String>) -> Result<Value, String> {
        self.normal_writable()?;
        #[cfg(mobile)]
        let paths = crate::library_files::mobile_primary_archives(paths)?;
        let claims = self.monitored_import_claims()?;
        let mut books = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let result = (|| {
                // A path one of our own publishers still owns must never be
                // opened: the metadata read below is the first point where a
                // half-written archive could be adopted. Callers pass absolute
                // canonical paths, so the claim comparison is exact.
                if Path::new(&path).is_absolute()
                    && self.monitored_path_is_claimed(Path::new(&path), &claims)?
                {
                    return Ok(None);
                }
                #[cfg(mobile)]
                crate::library_files::mobile_import_paths(self.store.data_dir(), std::iter::once(path.as_str()))?;
                // Verify before admission: a monitored folder is just another
                // way for an unverified file to become a library entry.
                let book = load_book(&path, false, true)?;
                // Metadata loading proves the candidate already exists. Every
                // publisher retains its claim through final-file visibility, so
                // checking now also closes the scan-versus-rename window.
                if self.monitored_path_is_claimed(Path::new(&book.path), &claims)? {
                    return Ok(None);
                }
                // All manager locks have been released before taking Store's
                // transaction lock (publishers take their manager lock first).
                // Verification can outlive a bounded watcher drain at Quit:
                // never register a book once shutdown has begun.
                self.ensure_running()?;
                self.register_books(std::slice::from_ref(&book), None)?;
                self.store.book(&book.metadata.id)?
                    .map(Some)
                    .ok_or_else(|| "Imported archive disappeared".to_string())
            })();
            match result {
                Ok(Some(book)) => books.push(book),
                Ok(None) => {}
                Err(error) => errors.push(json!({"path":path,"error":error})),
            }
        }
        #[cfg(mobile)]
        if !books.is_empty() { self.mobile_apply_archive_backup()?; }
        Ok(json!({"books":books.iter().map(Book::shell_value).collect::<Vec<_>>(),"errors":errors}))
    }
    /// Start live monitoring of the configured folders. Called once from the
    /// background maintenance task; the periodic scan keeps running regardless.
    #[cfg(desktop)]
    pub(crate) fn start_folder_watch(self: &Arc<Self>) -> Result<(), String> {
        self.ensure_running()?;
        let mut watcher = self
            .watcher
            .lock()
            .map_err(|_| "Monitored folder state unavailable")?;
        if watcher.is_some() {
            return Ok(());
        }
        *watcher = Some(crate::watch::FolderWatch::start(
            self.store.clone(),
            Arc::downgrade(self),
            self.watch_failures.clone(),
        )?);
        Ok(())
    }
    /// Apply one settled observation from the watcher. Every path goes through
    /// the same policy gates and the same scan/import path as the periodic
    /// maintenance pass; this only decides what changed.
    #[cfg(desktop)]
    pub(crate) async fn apply_watch(
        self: &Arc<Self>,
        message: crate::watch::WatchMessage,
    ) -> Result<(), String> {
        self.ensure_running()?;
        let _admission = self.update_admission.admit().await;
        // Serialize with privacy/read-only cutovers exactly like the periodic
        // maintenance pass: an import accepted before a cutover finishes, and
        // one accepted after it sees the new policy.
        let _mode = self.privacy_gate.read().await;
        if self.store.read_only()? || self.store.private_mode()? {
            return Ok(());
        }
        let settings = self.store.settings()?;
        let directories = crate::watch::monitored_directories(&settings);
        if directories.is_empty() {
            return Ok(());
        }
        match message {
            crate::watch::WatchMessage::SettledFile(path) => {
                if !crate::watch::is_candidate(&path, settings.portable_mode)
                    || !directories
                        .iter()
                        .any(|directory| crate::watch::is_direct_child(&path, directory))
                {
                    return Ok(());
                }
                // The file may have been renamed away while the observation
                // waited; the directory reconciliation retires the entry.
                let Ok(path) = path.canonicalize() else {
                    return Ok(());
                };
                // A writer may have touched the file after the observation was
                // queued; a fresh modification time means a new event (and a
                // new stability window) is coming.
                if !crate::watch::is_settled(&path) {
                    return Ok(());
                }
                {
                    let failures = self
                        .watch_failures
                        .lock()
                        .map_err(|_| "Monitored folder state unavailable")?;
                    if failures.should_skip(&path) {
                        return Ok(());
                    }
                }
                if self.store.has_book_at_path(&path)? {
                    if let Ok(mut failures) = self.watch_failures.lock() {
                        failures.clear(&path);
                    }
                    return Ok(());
                }
                let service = self.clone();
                let candidate = path.to_string_lossy().into_owned();
                let result =
                    blocking(move || service.import_monitored_paths(vec![candidate])).await?;
                self.record_watch_result(&result)
            }
            crate::watch::WatchMessage::SettledDirectory(directory) => {
                if !directories
                    .iter()
                    .any(|candidate| crate::watch::same_path(candidate, &directory))
                {
                    return Ok(());
                }
                self.reconcile_watched_directory(directory, settings.portable_mode)
                    .await
            }
        }
    }
    #[cfg(desktop)]
    fn record_watch_result(&self, result: &Value) -> Result<(), String> {
        let mut failures = self
            .watch_failures
            .lock()
            .map_err(|_| "Monitored folder state unavailable")?;
        for book in result
            .get("books")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(path) = book.get("path").and_then(Value::as_str) {
                failures.clear(Path::new(path));
            }
        }
        for error in result
            .get("errors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(path) = error.get("path").and_then(Value::as_str) else {
                continue;
            };
            failures.record(Path::new(path));
            eprintln!(
                "Monitored folders: {path}: {}",
                error
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("import failed")
            );
        }
        Ok(())
    }
    /// Diff one monitored directory against the library, then import what
    /// appeared and retire what disappeared. The directory listing uses the
    /// same no-follow and extension rules as the periodic scan.
    #[cfg(desktop)]
    async fn reconcile_watched_directory(
        self: &Arc<Self>,
        directory: PathBuf,
        portable: bool,
    ) -> Result<(), String> {
        // A vanished or unreachable directory (unmounted share, revoked
        // permission) must never be read as "everything was deleted". Listing
        // runs off the runtime thread because a network mount can stall.
        let scanned = blocking(move || {
            let Ok(directory) = directory.canonicalize() else {
                return Ok(None);
            };
            Ok(scan_watched_directory(&directory, portable).map(|present| (directory, present)))
        })
        .await?;
        let Some((directory, present)) = scanned else {
            return Ok(());
        };
        let state = self.store.read()?;
        let known: Vec<Book> = state
            .books
            .iter()
            .filter(|book| {
                book.source_reference.is_none()
                    && crate::watch::is_direct_child(Path::new(&book.path), &directory)
            })
            .cloned()
            .collect();
        let open: HashSet<String> = state
            .tabs
            .iter()
            .filter_map(|tab| tab.book_id.clone())
            .chain(
                state
                    .windows
                    .values()
                    .flat_map(|window| window.tabs.iter().filter_map(|tab| tab.book_id.clone())),
            )
            .collect();
        drop(state);

        let mut imports = Vec::new();
        for path in &present {
            if known
                .iter()
                .any(|book| crate::watch::same_path(Path::new(&book.path), path))
            {
                continue;
            }
            if !crate::watch::is_settled(path) {
                continue;
            }
            let skipped = self
                .watch_failures
                .lock()
                .map_err(|_| "Monitored folder state unavailable")?
                .should_skip(path);
            if !skipped {
                imports.push(path.to_string_lossy().into_owned());
            }
        }
        if !imports.is_empty() {
            imports.sort();
            let service = self.clone();
            let result = blocking(move || service.import_monitored_paths(imports)).await?;
            self.record_watch_result(&result)?;
        }

        for book in known {
            if present
                .iter()
                .any(|path| crate::watch::same_path(path, Path::new(&book.path)))
            {
                continue;
            }
            // kiwix-desktop leaves a book that is open in a tab alone; removing
            // it here would close the reader under the user.
            if open.contains(&book.metadata.id) {
                continue;
            }
            self.retire_missing_watched_book(&book).await?;
        }
        Ok(())
    }
    /// Remove a library entry whose file vanished from a monitored folder.
    /// Nothing on disk is touched (there is nothing to delete) and the archive
    /// itself is not "owned" differently from a manual removal.
    #[cfg(desktop)]
    async fn retire_missing_watched_book(self: &Arc<Self>, book: &Book) -> Result<(), String> {
        let id = book.metadata.id.clone();
        let installation_id = book.installation_id.clone();
        let _sources = self.localswarm_source_mutation(Some(&id)).await?;
        if self.active_torrent_share(&id)?.is_some() {
            return Ok(());
        }
        let mut readers = self.readers.lock().await;
        let mut removed = false;
        self.store.update(|state| {
            let Some(existing) = state.books.iter().find(|existing| existing.metadata.id == id)
            else {
                return Ok(());
            };
            if existing.installation_id != installation_id {
                return Ok(());
            }
            // Re-check under the transaction: a reader may have opened the book
            // since the directory diff.
            if state
                .tabs
                .iter()
                .any(|tab| tab.book_id.as_deref() == Some(id.as_str()))
                || state.windows.values().any(|window| {
                    window
                        .tabs
                        .iter()
                        .any(|tab| tab.book_id.as_deref() == Some(id.as_str()))
                })
            {
                return Ok(());
            }
            state.books.retain(|existing| existing.metadata.id != id);
            state.reader_storage.remove(&installation_id);
            state.settings.pinned_book_ids.retain(|value| value != &id);
            removed = true;
            Ok(())
        })?;
        if removed {
            retire_readers(&mut readers, Some(&installation_id)).await?;
            self.invalidate(&id);
            self.forget_installation(&installation_id);
            self.notify_library_changed();
        }
        Ok(())
    }
    fn register_books(&self, books: &[Book], receipt: Option<&str>) -> Result<(), String> {
        let _new_receipt = self.store.update(|state| {
            for book in books {
                if state.edition_intents.iter().any(|intent| intent.destination == book.path)
                    || state.edition_transactions.iter().any(|transaction|
                        // A completed rollback retains history, not import
                        // authority over the preserved original after removal.
                        transaction.rolled_back_installation_id.is_none()
                            && (transaction.original.path == book.path || transaction.replacement.path == book.path
                                || transaction.rollback_book.as_ref().is_some_and(|saved| saved.path == book.path)))
                {
                    return Err("This file belongs to an archive edition decision. Review it in Downloads instead of importing it separately".into());
                }
            }
            // Canonical file paths are resolved before this transaction. Never
            // probe every installed path here: one unavailable NAS would block
            // all profile readers behind the transaction mutex.
            let validate_reuse = |existing: &str, incoming: &str| -> Result<(), String> {
                if receipt.is_some() || Path::new(existing) != Path::new(incoming) {
                    return Err("Another copy of this archive is already in your library. Open that copy, or remove it before adding this file".into());
                }
                Ok(())
            };
            if let [book] = books {
                if let Some(existing) = state.books.iter().find(|old| old.metadata.id == book.metadata.id) {
                    validate_reuse(&existing.path, &book.path)?;
                }
            } else if !books.is_empty() {
                let mut paths: HashMap<&str, &str> = state.books.iter()
                    .map(|book| (book.metadata.id.as_str(), book.path.as_str()))
                    .collect();
                for book in books {
                    if let Some(existing) = paths.get(book.metadata.id.as_str()) {
                        validate_reuse(existing, &book.path)?;
                    } else {
                        paths.insert(book.metadata.id.as_str(), book.path.as_str());
                    }
                }
            }
            for book in books {
                if let Some(existing) = state
                    .books
                    .iter_mut()
                    .find(|existing| existing.metadata.id == book.metadata.id)
                {
                    existing.path.clone_from(&book.path);
                    existing.available = book.available;
                    existing.metadata = book.metadata.clone();
                    existing.managed |= book.managed;
                } else {
                    state.books.push(book.clone());
                }
            }
            Ok(receipt.is_some_and(|receipt| state.consumed_acquisitions.insert(receipt.into())))
        })?;
        #[cfg(any(target_os = "ios", target_os = "macos", target_os = "android"))]
        if _new_receipt {
            if let Some(receipt) = receipt { self.notify_verified_admission(receipt, books); }
        }
        for book in books {
            self.invalidate(&book.metadata.id);
        }
        self.notify_library_changed();
        Ok(())
    }
    /// Admit a mobile intake batch, resolving archives that are already
    /// installed by their immutable archive identity.
    ///
    /// Re-sharing or re-opening an installed archive is success, not a UUID
    /// collision: the existing installation (and therefore its bookmarks and
    /// reader storage) is returned unchanged and the archive is never added a
    /// second time. Only genuinely new books reach [`Self::register_books`].
    /// A receipt is still consumed when the entire batch was already installed,
    /// so the native intake request can be acknowledged and does not stay
    /// pending forever. The second return value lists staging copies that
    /// resolved to an installed book and may be removed by the caller.
    #[cfg(any(mobile, test))]
    fn admit_incoming_books(
        &self,
        books: Vec<Book>,
        receipt: Option<&str>,
    ) -> Result<(Vec<Book>, Vec<String>), String> {
        let mut pending = Vec::with_capacity(books.len());
        let mut resolved = Vec::with_capacity(books.len());
        let mut duplicates = Vec::new();
        {
            let state = self.store.read()?;
            for book in books {
                match state
                    .books
                    .iter()
                    .find(|existing| existing.metadata.id == book.metadata.id)
                {
                    Some(existing) => {
                        if Path::new(&existing.path) != Path::new(&book.path) {
                            duplicates.push(book.path);
                        }
                        resolved.push(existing.clone());
                    }
                    None => {
                        pending.push(book.clone());
                        resolved.push(book);
                    }
                }
            }
        }
        if pending.is_empty() {
            if let Some(receipt) = receipt {
                self.store.update(|state| {
                    state.consumed_acquisitions.insert(receipt.into());
                    Ok(())
                })?;
            }
            return Ok((resolved, duplicates));
        }
        self.register_books(&pending, receipt)?;
        Ok((resolved, duplicates))
    }
    fn forget_installation(&self, installation_id: &str) {
        if let Err(error) = self.spelling.retire_installation(installation_id) {
            eprintln!("Stopping the removed installation's spelling index: {error}");
        }
        let _speech = self.mobile_speech_block(mobile_speech::Scope::Installation(installation_id.into()));
        if let Ok(managers) = self.sensors.lock() {
            for manager in managers.values() {
                manager.forget_installation(installation_id);
            }
        }
    }
    pub fn notify_library_changed(&self) {
        if let Ok(app) = self.app.lock() {
            if let Some(app) = app.as_ref() {
                use tauri::Emitter;
                let _ = app.emit("kiwix-library-changed", ());
            }
        }
    }
    async fn set_reader_restricted(
        &self,
        book_id: &str,
        restricted: bool,
        label: &str,
    ) -> Result<Value, String> {
        self.ensure_window_active(label)?;
        let key = format!("{label}:{book_id}");
        let readers = self.readers.lock().await;
        let reader = readers
            .get(&key)
            .ok_or("Open the archive before changing restricted mode")?;
        reader.set_restricted(restricted);
        Ok(json!({"restricted": reader.is_restricted()}))
    }

    #[allow(clippy::too_many_arguments)]
    async fn reader(
        self: &Arc<Self>,
        book_id: String,
        path: Option<String>,
        random: bool,
        parent_origin: String,
        fresh: bool,
        restricted: Option<bool>,
        label: &str,
    ) -> Result<Value, String> {
        #[cfg(mobile)]
        let _speech = if fresh {
            let retirement = self.mobile_speech_block(mobile_speech::Scope::Window(label.into()));
            self.sensor_manager(label).deactivate();
            Some(retirement)
        } else { None };
        // Desktop: a fresh reader for this window ends its Kokoro read-aloud.
        #[cfg(desktop)]
        let _speech = fresh.then(|| self.mobile_speech_block(mobile_speech::Scope::Window(label.into())));
        if !self
            .shell_origins
            .lock()
            .map_err(|_| "Origin registry unavailable")?
            .contains(&parent_origin)
        {
            return Err("Untrusted reader parent origin".into());
        }
        let service = self.clone();
        let id = book_id.clone();
        let path = path.unwrap_or_default();
        let (_, suffix) = split_location(&path);
        // A malformed stored location is a miss, not a dead end: it opens the
        // fallback document and the reader says so quietly.
        let request = OpenRequest::decode(&path);
        let suffix = suffix.to_string();
        let installation_id = self
            .store
            .book(&book_id)?
            .ok_or("Archive is not in the library")?
            .installation_id;
        let crash_id = book_id.clone();
        let crash_installation = installation_id.clone();
        let (entry, missing, no_main_page) = blocking(move || {
            let _reading = crate::crash::reading(&crash_id, &crash_installation, "read_article");
            let archive = service.archive(&id)?;
            if random {
                return resolve_open_entry(&archive, &OpenRequest::Random)
                    .map(|open| (open.entry, open.missing, open.no_main_page));
            }
            resolve_open_entry(&archive, &request)
                .map(|open| (open.entry, open.missing, open.no_main_page))
        })
        .await?;
        let key = format!("{label}:{book_id}");
        let mut readers = self.readers.lock().await;
        if fresh {
            if let Some(reader) = readers.remove(&key) { reader.shutdown().await?; }
        }
        self.ensure_window_active(label)?;
        if self
            .store
            .book(&book_id)?
            .is_none_or(|book| book.installation_id != installation_id)
        {
            return Err(
                "Archive installation changed while opening the reader; open it again".into(),
            );
        }
        if !readers.contains_key(&key) {
            let service = self.clone();
            let id = book_id.clone();
            let crash_id = book_id.clone();
            let crash_installation = installation_id.clone();
            let archive = blocking(move || {
                let _reading = crate::crash::reading(&crash_id, &crash_installation, "open_archive");
                service.archive(&id)
            }).await?;
            let server = ReaderServer::start(
                archive,
                book_id.clone(),
                installation_id,
                parent_origin.clone(),
                self.store.clone(),
                self.privacy_generation.clone(),
            )
            .await?;
            readers.insert(key.clone(), server);
        }
        let reader = readers.get(&key).unwrap();
        if let Some(restricted) = restricted {
            reader.set_restricted(restricted);
        }
        reader.set_parent_origin(parent_origin)?;
        // A recovered miss now shows the main document (or a random article when
        // the archive has no main page), so the requested article's
        // query/fragment no longer addresses what is on screen.
        let suffix = if random || missing || no_main_page { "" } else { &suffix };
        let path = format!("{}{suffix}", encode_path(&entry.path));
        Ok(
            json!({"bookId":book_id,"path":path,"title":entry.title,"mime":entry.mime,"missing":missing,"noMainPage":no_main_page,"restricted":reader.is_restricted(),"url":format!("{}{path}",reader.base_url),"origin":reader.origin,"baseUrl":reader.base_url,"retirementUrl":reader.retirement_url(),"retirementProof":reader.retirement_proof()}),
        )
    }
    pub async fn dispatch(self: Arc<Self>, command: &str, args: Value) -> Result<Value, String> {
        self.ensure_running()?;
        // A cancelled native/HTTP waiter only drops the JoinHandle. The accepted
        // operation continues owning its mode guard until all blocking work ends.
        if mutation_command(command) {
            let command = command.to_owned();
            return tokio::spawn(async move { self.operation(&command, args).await })
                .await
                .map_err(|error| format!("Application operation stopped: {error}"))?;
        }
        self.operation(command, args).await
    }
    pub(crate) fn ensure_running(&self) -> Result<(), String> {
        if self.stopping.load(Ordering::SeqCst) {
            Err("The application is shutting down".into())
        } else {
            Ok(())
        }
    }

    /// Run the once-a-day check under the same privacy read lease as ordinary
    /// commands. A privacy transition waits for the check instead of racing it.
    #[cfg(desktop)]
    pub(crate) async fn update_auto_check(self: &Arc<Self>) {
        let _mode_guard = self.privacy_gate.read().await;
        crate::update::automatic_check(self.clone()).await;
    }

    /// Why installing an update must wait. An update replaces the running
    /// application and restarts it, so a transfer writing into the profile or
    /// an archive move must not be interrupted. Paused and terminal transfers
    /// hold no write, and seeding only reads verified bytes.
    #[cfg(desktop)]
    pub(crate) async fn update_install_blocker(&self) -> Result<Option<String>, String> {
        if self.migration_active.load(Ordering::SeqCst) > 0 {
            return Ok(Some(
                "Kiwix is moving archives. Wait for it to finish before installing an update."
                    .into(),
            ));
        }
        let downloads = self.downloads.get().cloned();
        if let Some(manager) = downloads {
            if manager.has_running_work()? || manager
                .list()?
                .iter()
                .any(|job| crate::update::transfer_in_progress(&job.status))
            {
                return Ok(Some(
                    "A download is still in progress. Finish or cancel it before installing an update."
                        .into(),
                ));
            }
        }
        let torrents = self
            .torrents
            .lock()
            .map_err(|_| "Download state unavailable")?
            .clone();
        if let Some(manager) = torrents {
            if manager
                .list()
                .await
                .iter()
                .any(|job| crate::update::transfer_in_progress(&job.status))
            {
                return Ok(Some(
                    "A download is still in progress. Finish or cancel it before installing an update."
                        .into(),
                ));
            }
        }
        let localswarm = self.localswarm.get().cloned();
        if let Some(manager) = localswarm {
            if manager
                .list()?
                .iter()
                .any(|job| crate::update::transfer_in_progress(&job.status))
            {
                return Ok(Some(
                    "A download is still in progress. Finish or cancel it before installing an update."
                        .into(),
                ));
            }
        }
        let maps = self
            .maps
            .lock()
            .map_err(|_| "Map download state unavailable")?
            .clone();
        if let Some(manager) = maps {
            if manager
                .status()?
                .is_some_and(|job| crate::update::transfer_in_progress(&job.status))
            {
                return Ok(Some(
                    "A map download is still in progress. Finish or cancel it before installing an update."
                        .into(),
                ));
            }
        }
        Ok(None)
    }
    async fn operation(self: Arc<Self>, command: &str, args: Value) -> Result<Value, String> {
        if matches!(command, "save_pdf" | "mobile_save_pdf" | "share_pdf" | "print_pdf") {
            let job = self.admit_pdf(command, &args)?;
            return self.export_pdf(args, job).await;
        }
        #[cfg(any(target_os = "ios", target_os = "macos", target_os = "android"))]
        if command == "request_completion_notifications" {
            return self.request_completion_notifications(args).await;
        }
        #[cfg(target_os = "ios")]
        if command == "mobile_media_state" {
            return self.mobile_reader_media_state(args).await;
        }
        if command.starts_with("mobile_speech_") {
            let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main").to_string();
            // Native initialization can wait for system TTS. Neither Start nor
            // its ownership-only cancellation may retain the global mode lease.
            return self.mobile_speech_command(command, args, &label).await;
        }
        if command.starts_with("nearby_") {
            return self.nearby_command(command, args).await;
        }
        if command.starts_with("private_sync_") {
            // Private sync binds 0.0.0.0 and connects to LAN peers, so the
            // Android 17 permission must be admitted before its early return.
            // Status and confirm only create the manager when sync is enabled.
            #[cfg(target_os = "android")]
            if mobile_policy::local_network_command(command, &args)
                && (command == "private_sync_configure" || self.store.reading_sync_enabled()?)
            {
                self.normal_writable()?;
                if command == "private_sync_status" {
                    // Polled every two seconds: check first, prompt once. A
                    // denial must not hide the status controls; let the
                    // private-sync command report the manager error instead.
                    if let Err(error) = self.mobile_local_network_admit_passive().await {
                        crate::diagnostics::record(&self.store, "private-sync", Some(&error));
                    }
                } else {
                    self.mobile_local_network_admit().await?;
                }
            }
            return self.private_sync_command(command, args).await;
        }
        // Cosmetic and idempotent: the shell reports whether a reader document
        // is on screen in this window so the native under-page colour can
        // follow it. It owns no writable state, must not queue behind a privacy
        // writer or lifecycle transition, and carries nothing from the archive.
        // Only desktop has multiple webviews with a window registry; mobile
        // keeps the single window's backdrop from its own creation path.
        #[cfg(desktop)]
        if command == "reader_backdrop" {
            let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main").to_string();
            if let Some(reader) = args.get("reader").and_then(Value::as_bool) {
                crate::windows::set_reader_surface(&label, reader);
            }
            if let Some(app) = self.app.lock().map_err(|_| "Application handle unavailable")?.clone() {
                if let Some(window) = app.get_webview_window(&label) {
                    crate::windows::apply_under_page(&window, &self.store.settings()?);
                }
            }
            return Ok(json!({ "reader": crate::windows::has_reader_surface(&label) }));
        }
        #[cfg(mobile)]
        self.mobile_admit(command, &args)?;
        self.ensure_running()?;
        // Android 17 blocks LAN sockets until ACCESS_LOCAL_NETWORK is granted.
        // Ask before any discovery, browse, seed, share or transfer starts so
        // the native manager never races a denied socket. Read-only and private
        // mode refuse first, without showing a permission sheet.
        #[cfg(target_os = "android")]
        if mobile_policy::local_network_command(command, &args) {
            self.normal_writable()?;
            self.mobile_local_network_admit().await?;
        }
        // Ownership-only cancellation must never queue behind a privacy writer
        // that is itself waiting for the cancelled search's read lease.
        if matches!(command, "search_session" | "search_owner" | "search_release" | "cancel_search") {
            let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
            return self.search_control(label, command, &args);
        }
        let label = args
            .get("__windowLabel")
            .and_then(Value::as_str)
            .unwrap_or("main");
        let _speech_retirement = self.mobile_speech_lifecycle(command, &args, label)?;
        if command == "close_window" {
            self.closed_windows
                .lock()
                .map_err(|_| "Window state unavailable")?
                .insert(label.into());
            self.cancel_pdf_jobs(Some(label));
            self.retire_searches(Some(label));
            self.searches.lock().map_err(|_| "Search ownership unavailable")?.remove(label);
            #[cfg(desktop)]
            crate::windows::forget_window(label);
            if let Ok(mut ready) = self.ready_frontends.lock() {
                ready.remove(label);
            }
            self.nearby_release_window(label).await;
        } else if matches!(
            command,
            "save_session"
                | "restore_session"
                | "desktop_dialog_directory"
                | "remember_desktop_dialog_directory"
                | "record_visit"
                | "open_article"
                | "random_article"
                | "save_settings"
                | "sensor_activate"
                | "sensor_location"
                | "sensor_consent"
                | "sensor_revoke"
                | "sensor_reset"
        ) {
            self.ensure_window_active(label)?;
        }
        // Always take update admission before transfer/privacy locks. Updater
        // installation and newly admitted archive writers must not cross.
        #[cfg(desktop)]
        let update_installation = if command == "update_install" {
            Some(self.update_admission.install().await)
        } else { None };
        #[cfg(not(desktop))]
        let update_installation = None;
        let _update_admission = if matches!(command,
            "save_settings" | "import_archives" | "scan_folder" | "migrate_library"
            | "move_archives" | "recover_migrations" | "library_maintenance"
            | "downloads" | "transfer_history_notice" | "retained_download_files"
            | "kokoro_probe" | "kokoro_download" | "kokoro_delete"
            | "prepare_download_cleanup" | "retry_import" | "remove_book" | "relink_book"
            | "start_download" | "resume_download" | "pause_download" | "cancel_download"
            | "delete_download_files" | "remove_download_history" | "forget_download_record"
            | "torrent_seeding" | "share_torrent" | "torrent_job" | "torrent_metadata" | "torrent_limits"
            | "localswarm_discovery" | "localswarm_download" | "localswarm_seed" | "localswarm_status"
            | "localswarm_share_all" | "localswarm_stop_library"
            | "shared_library_browse" | "shared_library_download" | "sharing_start" | "sharing_stop"
            | "map_prepare" | "map_start" | "map_resume" | "map_status" | "map_pause" | "map_cancel"
            | "map_delete_download" | "map_remove_history" | "map_forget_recovery" | "map_clear_cache"
            | "mobile_library_root_set" | "library_files_relocate"
            | "edition_prepare" | "edition_download" | "edition_status" | "edition_commit"
            | "edition_cancel" | "edition_rollback"
        ) {
            Some(self.update_admission.admit().await)
        } else { None };
        #[cfg(mobile)]
        let _mobile_transfer_guard = if self.mobile_transfer_command(command, &args)
            || matches!(command, "pause_download" | "cancel_download" | "delete_download_files" | "torrent_seeding" | "share_torrent" | "localswarm_discovery" | "localswarm_stop" | "localswarm_stop_seed" | "localswarm_share_all" | "localswarm_stop_library" | "map_pause" | "map_cancel" | "map_delete_download" | "map_remove_history" | "sharing_start" | "sharing_stop" | "save_settings")
            || (command == "remove_book" && args.get("cleanupSource").is_some())
            || command == "prepare_download_cleanup"
            || matches!(command, "mobile_library_root_set" | "library_files_relocate")
            || matches!(command, "edition_cancel" | "edition_commit" | "edition_rollback")
        {
            Some(self.mobile.transfer_gate.lock().await)
        } else {
            None
        };
        // Serialize filesystem side effects with mode cutovers. Native location
        // waits own no writable resource: deactivation retires their requests, so
        // they must not block a writer (or cancellation queued behind that writer).
        // Never nest read guards behind Tokio's fair RwLock.
        // Retire cancellable LocalSwarm setup before waiting for its accepted
        // operation's read lease. The writer below joins all resulting work.
        if command == "save_settings" {
            // Refuse an invalid save before retiring anything below.
            self.ensure_running()?;
            self.validate_settings_patch(&args)?;
        }
        let retire_localswarm = command == "localswarm_stop" ||
            (command == "save_settings" && args.get("settings").is_some_and(|settings| {
                settings.get("privateMode") == Some(&Value::Bool(true))
                    || settings.get("readOnly") == Some(&Value::Bool(true))
            }));
        let (retire_private_work, retire_searches) = if command == "save_settings" {
            let previous = self.store.settings()?;
            if let Some(settings) = args.get("settings") {
                let policy = settings.get("privateMode").and_then(Value::as_bool)
                    .is_some_and(|value| value != previous.private_mode)
                    || settings.get("readOnly").and_then(Value::as_bool)
                        .is_some_and(|value| value != previous.read_only);
                let snippets = settings.get("showSearchSnippets").and_then(Value::as_bool)
                    .is_some_and(|value| value != previous.show_search_snippets);
                (policy, policy || snippets)
            } else { (false, false) }
        } else { (false, false) };
        if retire_localswarm { self.retire_localswarm(); }
        #[cfg(target_os = "ios")]
        if retire_localswarm {
            if let Err(error) = self.mobile_call("localTransferState", json!({"active": false})).await {
                crate::mobile::error(&self, "local-transfer-idle-release", error);
            }
            self.mobile.wake.notify_waiters();
        }
        if retire_searches { self.retire_searches(None); }
        if retire_private_work { self.retire_private_sync(); }
        if retire_private_work { self.integrity.retire().await; }
        if retire_private_work { self.cancel_pdf_jobs(None); }
        let _localswarm_retirement = if command == "save_settings" || command == "localswarm_stop" {
            Some(LocalSwarmRetirement { service: self.as_ref(), retired: retire_localswarm })
        } else { None };
        if retire_localswarm { self.nearby_retire().await?; }
        let _mode_writer = if matches!(command, "save_settings" | "localswarm_stop") {
            Some(self.privacy_gate.write().await)
        } else {
            None
        };
        if retire_private_work {
            self.integrity.cancel_and_wait(true).await?;
        }
        let _mode_guard = if matches!(command, "save_settings" | "localswarm_stop" | "sensor_location" | "catalog" | "catalog_facets" | "catalog_torrent" | "find_updates" | "map_recipe_metadata" | "map_prepare" | "shared_library_browse") {
            None
        } else {
            Some(self.privacy_gate.read().await)
        };
        self.ensure_running()?;
        #[cfg(mobile)]
        self.mobile_admit(command, &args)?;
        if matches!(
            command,
            "save_session"
                | "restore_session"
                | "desktop_dialog_directory"
                | "remember_desktop_dialog_directory"
                | "record_visit"
                | "open_article"
                | "random_article"
                | "save_settings"
                | "sensor_activate"
                | "sensor_location"
                | "sensor_consent"
                | "sensor_revoke"
                | "sensor_reset"
        ) {
            self.ensure_window_active(label)?;
        }
        #[cfg(mobile)]
        let network_admission = if (self.mobile_transfer_command(command, &args)
            && !matches!(command, "localswarm_share_all" | "shared_library_download"))
            || command == "localswarm_inspect"
            || command == "sharing_start"
        {
            let (shared_resume, local_resume) = if command == "resume_download" {
                match args.get("id").and_then(Value::as_str) {
                    Some(id) if !id.starts_with("localswarm:") && !id.starts_with("torrent:") => {
                        let manager = self.download_manager().await?;
                        (manager.is_shared_library(id)?, manager.has_local_native_resume(id).await?)
                    }
                    _ => (false, false),
                }
            } else { (false, false) };
            if local_resume || shared_resume {
                // Native receipts need no network; QUIC library grants enforce
                // their own exact joined/AP/native/LAN route instead of the
                // possibly unrelated default network.
                None
            } else {
                let internet = !(matches!(command, "localswarm_download" | "localswarm_seed" | "localswarm_inspect" | "localswarm_discovery" | "shared_library_browse" | "shared_library_download" | "sharing_start")
                    || shared_resume
                    || (command == "resume_download" && args.get("id").and_then(Value::as_str).is_some_and(|id| id.starts_with("localswarm:"))));
                Some((self.mobile_network_admit(internet).await?, internet))
            }
        } else { None };
        #[cfg(target_os = "android")]
        if self.mobile_transfer_command(command, &args) {
            self.normal_writable()?;
            if let Err(error) = self.mobile_transfer_begin(label, command).await {
                let _ = self.mobile_transfer_update_locked().await;
                return Err(error);
            }
        }
        #[cfg(mobile)]
        if let Some((generation, internet)) = network_admission {
            let validation = self.mobile_network_validate(generation, internet);
            #[cfg(target_os = "android")]
            if validation.is_err() {
                let _ = self.mobile_transfer_update_locked().await;
            }
            validation?;
        }
        // Catalog fetches release their own short cache leases during HTTP.
        // Retain only admission identity for the final response/diagnostic lease.
        let catalog_admission = if matches!(command, "map_recipe_metadata" | "map_prepare") {
            // Read-only metadata owns no profile/cache resources during HTTP or
            // estimation. Capture admission under a short lease, then recheck
            // before returning/recording diagnostics after any privacy cutover.
            let _admission_guard = self.privacy_gate.read().await;
            self.ensure_running()?;
            self.ensure_window_active(label)?;
            Some((label.to_owned(), Some(self.launch_generation())))
        } else {
            matches!(command, "catalog" | "catalog_facets" | "catalog_torrent").then(|| (
                label.to_owned(),
                args.get("privacyGeneration").and_then(Value::as_u64),
            ))
        };
        // Even a failed settings save may have retired the old reader origins.
        // Notify all shells before the writer is released, and discard queued
        // launches only when the effective privacy policy actually changed.
        let previous_policy = if command == "save_settings" {
            let policy = self.store.settings()?;
            Some((self.launch_generation(), policy.private_mode, policy.read_only))
        } else { None };
        let result = self.clone().dispatch_inner(command, args, update_installation).await;
        if let Some((epoch, private, read_only)) = previous_policy {
            if self.launch_generation() != epoch {
                if !self.store.settings().is_ok_and(|s| (s.private_mode, s.read_only) == (private, read_only)) {
                    self.launch_locations.lock().unwrap_or_else(|error| error.into_inner()).clear();
                    #[cfg(any(desktop, test))]
                    self.launch_search.lock().unwrap_or_else(|error| error.into_inner()).take();
                }
                if let Some(app) = self.app.lock().unwrap_or_else(|error| error.into_inner()).as_ref() {
                    use tauri::Emitter;
                    let _ = app.emit("kiwix-privacy-changed", ());
                }
            }
        }
        #[cfg(target_os = "android")]
        if _mobile_transfer_guard.is_some() && self.mobile.foreground_transfers.load(Ordering::SeqCst) {
            if let Err(error) = self.mobile_transfer_update_locked().await {
                crate::mobile::error(&self, "transfers", error);
                let service = self.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = service.mobile_pause_transfers().await {
                        crate::mobile::error(&service, "transfers", error);
                    }
                });
            }
        }
        #[cfg(target_os = "ios")]
        if _mobile_transfer_guard.is_some() {
            if command == "save_settings" && self.mobile_active() {
                match self.store.settings() {
                    Ok(settings) => if let Some(manager) = self.localswarm.get() {
                        manager.set_sharing_admission(!settings.private_mode && !settings.read_only).await;
                    },
                    Err(error) => {
                        if let Some(manager) = self.localswarm.get() { manager.set_sharing_admission(false).await; }
                        self.mobile.sharing_policy_retry.store(true, Ordering::SeqCst);
                        crate::mobile::error(&self, "local-sharing-admission", error);
                    }
                }
            }
            if let Err(error) = self.mobile_local_transfer_update_locked().await {
                crate::mobile::error(&self, "local-transfers", error);
            }
            self.mobile.wake.notify_waiters();
        }
        // Location and catalog waits hold no global lease. Their diagnostics
        // still need a short lease so privacy cutovers cannot race disk writes.
        let _diagnostic_guard = if matches!(command, "sensor_location" | "catalog" | "catalog_facets" | "catalog_torrent" | "find_updates" | "map_recipe_metadata" | "map_prepare") {
            Some(self.privacy_gate.read().await)
        } else {
            None
        };
        if let Some((label, generation)) = catalog_admission {
            self.ensure_running()?;
            self.ensure_window_active(&label)?;
            let generation = generation.ok_or("This window's session has expired. Reopen the window to continue")?;
            if generation != self.launch_generation() {
                return Err("Browsing session expired after a privacy transition".into());
            }
        }
        if !self.stopping.load(Ordering::SeqCst) && !matches!(command, "localswarm_status" | "localswarm_inspect" | "integrity_status" | "crash_report" | "crash_report_acknowledge" | "crash_report_delete" | "debug_crash") {
            crate::diagnostics::record(
                &self.store,
                command,
                result.as_ref().err().map(String::as_str),
            );
        }
        result
    }
    fn import_manager(&self) -> Result<Arc<ImportManager>, String> {
        self.imports
            .lock()
            .map(|manager| manager.clone())
            .map_err(|_| "Import manager unavailable".into())
    }
    #[cfg(desktop)]
    pub async fn imports_changed(&self) {
        if let Ok(manager) = self.import_manager() {
            manager.changed().notified().await;
        }
    }
    fn import_publication(
        &self, receipt: &str, paths: &[String],
    ) -> Option<Result<crate::acquisition::ImportPublication, String>> {
        let download = uuid::Uuid::parse_str(receipt).is_ok();
        if !download && !receipt.starts_with("torrent:") && !receipt.starts_with("localswarm:") {
            return None;
        }
        Some((|| {
            if paths.len() != 1 { return Err("An acquisition must identify exactly one archive for verification".into()); }
            let destination = &paths[0];
            if download {
                return self.downloads.get()
                    .ok_or_else(|| "Download ownership is unavailable; reopen Downloads and retry".to_string())?
                    .import_publication(receipt);
            }
            if receipt.starts_with("torrent:") {
                let (id, _) = receipt.rsplit_once(':').ok_or("Invalid torrent publication receipt")?;
                return self.torrent_manager()?.import_publication(id, destination);
            }
            match self.localswarm.get() {
                Some(manager) => manager.import_publication(receipt, destination),
                None => LocalSwarmManager::saved_import_publication(&self.store, receipt, destination),
            }
        })())
    }

    fn retry_import(&self, receipt: &str) -> Result<crate::imports::ImportStatus, String> {
        self.normal_writable()?;
        let imports = self.import_manager()?;
        let status = imports.status(receipt).ok_or("Unknown acquisition receipt")?;
        match self.import_publication(receipt, &status.paths) {
            Some(publication) => imports.retry_publication(receipt, publication),
            None => imports.retry(receipt),
        }
    }

    fn enqueue_import(&self, receipt: &str, paths: Vec<String>) -> Result<(), String> {
        self.enqueue_import_with_saved(receipt, paths, Ok(None))
    }

    fn enqueue_import_with_saved(
        &self, receipt: &str, paths: Vec<String>,
        saved: Result<Option<&crate::localswarm::SavedImportSnapshot>, &str>,
    ) -> Result<(), String> {
        self.normal_writable()?;
        if !self.store.acquisition_consumed(receipt)? {
            // Capacity is a per-row condition: a full verifier queue leaves
            // this job without an import state (the UI already says
            // "Preparing") and a later poll retries it. It must never abort
            // the whole Downloads list, or every row behind it disappears.
            let imports = self.import_manager()?;
            let admission = if imports.status(receipt).is_some() {
                // Polling an existing verification must not repeatedly load or
                // clone a potentially large producer manifest. Explicit Retry
                // refreshes authority; duplicate admission only checks paths.
                imports.enqueue(receipt, paths)
            } else {
                if !imports.has_admission_capacity()? { return Ok(()); }
                let publication = if receipt.starts_with("localswarm:") {
                    match saved {
                        Ok(Some(saved)) => Some(if paths.len() == 1 { saved.import_publication(receipt, &paths[0]) }
                            else { Err("An acquisition must identify exactly one archive for verification".into()) }),
                        Err(error) => Some(Err(error.to_owned())),
                        Ok(None) => self.import_publication(receipt, &paths),
                    }
                } else {
                    self.import_publication(receipt, &paths)
                };
                match publication {
                    Some(publication) => imports.enqueue_publication(receipt, paths, publication),
                    None => imports.enqueue(receipt, paths),
                }
            };
            match admission {
                Ok(_) | Err(EnqueueError::Full) => {}
                Err(EnqueueError::Rejected(error)) => return Err(error),
            }
        }
        Ok(())
    }
    // Only called by owned operations while the global mode read guard is held.
    fn commit_ready_imports(&self) -> Result<(), String> {
        self.ensure_running()?;
        self.normal_writable()?;
        let manager = self.import_manager()?;
        let ready_imports = manager.ready();
        // Decode saved Nearby history once per batch, outside the import mutex.
        // Its shared process lock is retained only through these short commits.
        let saved_localswarm = if self.localswarm.get().is_none()
            && ready_imports.iter().any(|ready| ready.receipt().starts_with("localswarm:")) {
            Some(LocalSwarmManager::saved_import_snapshot(&self.store))
        } else { None };
        for ready in ready_imports {
            // Map choices require asynchronous source/reader retirement gates;
            // never launder them through ordinary UUID-based registration.
            if ready.receipt().starts_with("map:") { continue; }
            let paths = manager.status(ready.receipt()).map(|status| status.paths).unwrap_or_default();
            if crate::edition_updates::EditionManager::owns_receipt(&self.store.read()?, ready.receipt(), &paths) { continue; }
            let registered = manager.with_ready_publications(&ready, |books, sources, publication| {
                if self.store.acquisition_consumed(ready.receipt())? {
                    return Ok(());
                }
                // HTTP/selected-library acquisitions use bare UUID receipts;
                // other producers have explicit map/torrent/localswarm prefixes.
                // Integrity validation alone must never adopt a replacement
                // path left behind by a completed acquisition before import.
                let receipt = ready.receipt();
                if uuid::Uuid::parse_str(receipt).is_ok() || receipt.starts_with("torrent:") || receipt.starts_with("localswarm:") {
                    if books.len() != 1 || sources.len() != 1 {
                        return Err("A download must have exactly one verified archive".into());
                    }
                    let publication = publication.ok_or("Download verification omitted its original publication authority")?;
                    // Recheck the worker's content-change identity after all
                    // producer checks, immediately before Store registration.
                    let commit = || {
                        ready.sources_unchanged()?;
                        self.register_books(books, Some(receipt))
                    };
                    if uuid::Uuid::parse_str(receipt).is_ok() {
                        return self.downloads.get().ok_or("Download ownership is unavailable; reopen Downloads and retry")?
                            .with_import_publication(receipt, &sources[0], publication, commit);
                    }
                    let destination = &books[0].path;
                    if receipt.starts_with("torrent:") {
                        let (id, _) = receipt.rsplit_once(':').ok_or("Invalid torrent publication receipt")?;
                        return self.torrent_manager()?.with_import_publication(id, destination, &sources[0], publication, commit);
                    }
                    return match self.localswarm.get() {
                        Some(manager) => manager.with_import_publication(receipt, destination, &sources[0], publication, commit),
                        None => match &saved_localswarm {
                            Some(Ok(Some(saved))) => saved.with_import_publication(receipt, destination, &sources[0], publication, commit),
                            Some(Err(error)) => Err(error.clone()),
                            _ => Err("Nearby download ownership is unavailable; retry the import".into()),
                        },
                    };
                }
                self.register_books(books, Some(ready.receipt()))
            })?;
            if registered { manager.ack(&ready); }
        }
        Ok(())
    }

    async fn reconcile_completed_map(&self) -> Result<(), String> {
        if self.store.private_mode()? || self.store.read_only()? { return Ok(()); }
        let initialized = self.maps.lock().map_err(|_| "Map manager unavailable")?.is_some();
        if !initialized && !self.store.data_dir().join("maps/map-job.json").is_file() { return Ok(()); }
        let maps = self.map_manager()?;
        maps.recover_terminal_publication()?;
        if let Some(job) = maps.status()?.filter(|job| job.status == "complete") {
            if let Some(path) = job.path {
                self.enqueue_import(&format!("map:{}", job.id), vec![path])?;
            }
        }
        self.commit_ready_map_imports().await
    }

    async fn commit_ready_map_imports(&self) -> Result<(), String> {
        self.ensure_running()?;
        self.normal_writable()?;
        let imports = self.import_manager()?;
        let maps = self.map_manager()?;
        let generation = self.privacy_policy_generation.load(Ordering::SeqCst);
        for ready in imports.ready() {
            let Some(id) = ready.receipt().strip_prefix("map:") else { continue };
            let (target, archive_id, decision_token) = maps.with_import_decision(id, generation, |decision|
                Ok((decision.and_then(|decision| decision.target()).cloned(),
                    decision.map(|decision| decision.archive_id().to_owned()),
                    decision.map(|decision| decision.token().to_owned())))).unwrap_or_default();
            let _source_guards = if let Some(old) = &target {
                match self.localswarm_source_mutation(Some(&old.metadata.id)).await {
                    Ok(guards) => Some(guards),
                    Err(error) => {
                        let _ = imports.with_ready(&ready, |_| Err(error));
                        continue;
                    }
                }
            } else { None };
            let mut readers = self.readers.lock().await;
            let registered = imports.with_ready(&ready, |books| {
                if self.store.acquisition_consumed(ready.receipt())? { return Ok(()); }
                maps.with_import_decision(id, generation, |decision| {
                    if decision.map(|decision| decision.token()) != decision_token.as_deref() {
                        return Err("Map choice changed while waiting for its installation gate; retry verification with the current explicit choice".into());
                    }
                    if let Some(decision) = decision {
                        let [book] = books else { return Err("A map receipt must contain exactly one verified archive".into()); };
                        self.store.update(|state| decision.register(state, book, ready.receipt()))
                    } else {
                        // Pre-upgrade completed outputs have no replacement
                        // authority and retain ordinary collision protection.
                        self.register_books(books, Some(ready.receipt()))
                    }
                })
            }).unwrap_or(false); // Another owned poll may already have acknowledged this ticket.
            if registered {
                imports.ack(&ready);
                if let Some(old) = &target {
                    retire_readers(&mut readers, Some(&old.installation_id)).await?;
                    self.forget_installation(&old.installation_id);
                    self.invalidate(&old.metadata.id);
                }
                // New installations never inherit an old cached archive handle.
                if let Some(id) = &archive_id {
                    self.invalidate(id);
                    #[cfg(any(target_os = "ios", target_os = "macos", target_os = "android"))]
                    if let Some(book) = self.store.book(id)? {
                        self.notify_verified_admission(ready.receipt(), std::slice::from_ref(&book));
                    }
                }
                self.notify_library_changed();
            }
        }
        Ok(())
    }
    async fn cleanup_publications(
        &self, kind: &str, id: &str,
    ) -> Result<Vec<(PathBuf, crate::torrent::CleanupEntry)>, String> {
        match kind {
            "map" => {
                let manager = self.map_manager()?;
                let id = id.to_owned();
                blocking(move || manager.terminal_owned_publications(&id)).await
            }
            "download" if id.starts_with("localswarm:") => {
                let manager = self.localswarm_manager(None).await?;
                let id = id.to_owned();
                blocking(move || manager.terminal_owned_publications(&id)).await
            }
            "download" if id.starts_with("torrent:") =>
                self.torrent_manager()?.terminal_owned_publications(id).await,
            "download" => self.download_manager().await?.terminal_owned_publications(id).await,
            _ => Err("Unknown download cleanup source".into()),
        }
    }
    fn import_status(&self, receipt: &str, paths: Vec<String>) -> Result<Option<Value>, String> {
        if self.store.acquisition_consumed(receipt)? {
            Ok(Some(
                json!({"receipt":receipt,"paths":paths,"state":"imported"}),
            ))
        } else {
            self.import_manager()?
                .status(receipt)
                .map(value)
                .transpose()
        }
    }
    fn decorate_import(
        &self,
        row: &mut Value,
        receipt: &str,
        paths: Vec<String>,
    ) -> Result<(), String> {
        if let Some(status) = self.import_status(receipt, paths)? {
            row["importState"] = status["state"].clone();
            if let Some(error) = status.get("error") {
                row["importError"] = error.clone();
            }
            row["imports"] = json!([status]);
        }
        Ok(())
    }

    async fn retained_download_jobs(self: &Arc<Self>) -> Result<Value, String> {
        self.normal_writable()?;
        let mut rows = Vec::new();
        let manager = self.download_manager().await?;
        for job in manager.retained_snapshots()? {
            let mut row = value(&job)?;
            self.decorate_import(&mut row, &job.id, vec![job.destination.clone()])?;
            rows.push(row);
        }
        for job in self.torrent_manager()?.retained_jobs().await {
            let mut row = value(&job)?;
            let mut imports = Vec::new();
            for path in &job.completed_paths {
                use sha2::Digest;
                let receipt = format!("{}:{}", job.id, hex::encode(sha2::Sha256::digest(path.as_bytes())));
                if let Some(status) = self.import_status(&receipt, vec![path.clone()])? {
                    imports.push(status);
                }
            }
            if !imports.is_empty() {
                row["importState"] = json!(if imports.iter().all(|entry| entry["state"] == "imported") {
                    "imported"
                } else {
                    "failed"
                });
                row["imports"] = json!(imports);
            }
            rows.push(row);
        }
        let local_jobs = match self.localswarm.get() {
            Some(manager) => manager.retained_jobs()?,
            None => LocalSwarmManager::saved_retained_jobs(&self.store)?,
        };
        for job in local_jobs {
            let mut row = value(&job)?;
            self.decorate_import(&mut row, &job.id, job.completed_paths)?;
            rows.push(row);
        }
        for job in self.map_manager()?.retained_jobs()? {
            let mut row = value(&job)?;
            self.decorate_import(&mut row, &format!("map:{}", job.id), job.completed_paths)?;
            rows.push(row);
        }
        // Deliberately do not enqueue or commit imports: these rows were
        // explicitly dismissed and are exposed only for retained-file cleanup.
        Ok(json!(rows))
    }
    async fn kokoro_status(self: &Arc<Self>) -> Result<Value, String> {
        // The probe writes its result into the profile; startup skips it in
        // private and read-only mode, and so does opening Settings.
        if !self.store.private_mode()? && !self.store.read_only()? {
            self.kokoro.ensure_performance_probe();
        }
        Ok(self.kokoro.status())
    }

    async fn kokoro_probe(self: &Arc<Self>) -> Result<Value, String> {
        self.normal_writable()?;
        self.kokoro.reprobe();
        Ok(self.kokoro.status())
    }

    async fn kokoro_download(self: &Arc<Self>) -> Result<Value, String> {
        self.normal_writable()?;
        crate::transfer_policy::require_internet()?;
        let cancel = self.kokoro.begin_download()?;
        let manager = self.kokoro.clone();
        let root = self.store.data_dir().to_path_buf();
        let app = self.app.lock().map_err(|_| "Application handle unavailable")?.clone();
        tauri::async_runtime::spawn(async move {
            let progress_manager = manager.clone();
            let progress_app = app.clone();
            let result = kokoro::download(&root, cancel, move |downloaded, total| {
                progress_manager.download_progress(downloaded, total);
                if let Some(app) = progress_app.as_ref() {
                    use tauri::Emitter;
                    let _ = app.emit("kiwix-speech-model", json!({
                        "state": "downloading", "downloaded": downloaded, "total": total,
                    }));
                }
            }).await;
            let failed = result.as_ref().err().cloned();
            let result = manager.finish_download(result);
            if result.is_ok() { manager.ensure_performance_probe(); }
            if let Some(app) = app.as_ref() {
                use tauri::Emitter;
                let _ = app.emit("kiwix-speech-model", json!({
                    "state": if result.is_ok() { "installed" } else { "error" },
                    "error": failed,
                    "voices": if result.is_ok() { kokoro::voice_entries() } else { Vec::<Value>::new() },
                }));
            }
        });
        Ok(json!({"state":"downloading"}))
    }

    async fn kokoro_cancel(&self) -> Result<Value, String> {
        self.kokoro.cancel_download();
        Ok(json!({"state":"cancelling"}))
    }

    async fn kokoro_delete(self: &Arc<Self>) -> Result<Value, String> {
        self.normal_writable()?;
        self.mobile_speech_model_deleted();
        self.kokoro.delete()?;
        Ok(json!({"state":"deleted"}))
    }

    async fn download_jobs(self: &Arc<Self>) -> Result<Value, String> {
        let admission_generation = self.localswarm_admission.lock()
            .map_err(|_| "LocalSwarm admission unavailable")?.generation;
        if self.store.read_only()? || self.store.private_mode()? {
            let mut jobs = Vec::new();
            // Read-only and private launches cannot quarantine a malformed
            // record, but an unreadable history still carries no rows. Treat
            // it as empty instead of turning a recoverable record into a
            // blocking transfer error.
            let saved_downloads = read_saved(&self.store, "downloads/downloads.json")
                .unwrap_or(Value::Null);
            if let Some(saved) = saved_downloads.as_object() {
                for row in saved.values() {
                    if row.get("historyDismissed") == Some(&Value::Bool(true)) { continue; }
                    let transport = DownloadManager::saved_transport(row);
                    let mut row = row.clone();
                    if let Some(row) = row.as_object_mut() {
                        row.insert("transport".into(), value(transport)?);
                    }
                    jobs.push(row);
                }
            }
            if let Some(saved) = read_saved(&self.store, "torrents/jobs.json")?
                .get("jobs")
                .and_then(Value::as_array)
            {
                jobs.extend(saved.iter()
                    .filter(|row| row.get("historyDismissed") != Some(&Value::Bool(true)))
                    .filter_map(|row| row.get("job").cloned()));
            }
            jobs.extend(LocalSwarmManager::saved_jobs(&self.store)?.into_iter().map(|job| {
                let mut row = json!(job);
                row["admissionGeneration"] = json!(admission_generation);
                row
            }));
            return Ok(json!(jobs));
        }
        self.commit_ready_imports()?;
        let mut jobs = Vec::new();
        let manager = self.download_manager().await?;
        self.reconcile_edition_jobs(&manager).await?;
        for job in manager.list_snapshots()? {
            if job.status == "complete" {
                self.enqueue_import(&job.id, vec![job.destination.clone()])?;
            }
            let mut row = value(&job)?;
            self.decorate_import(&mut row, &job.id, vec![job.destination.clone()])?;
            jobs.push(row);
        }
        for job in self.torrent_manager()?.list().await {
            let mut imports = Vec::new();
            for path in &job.completed_paths {
                use sha2::Digest;
                let receipt = format!(
                    "{}:{}",
                    job.id,
                    hex::encode(sha2::Sha256::digest(path.as_bytes()))
                );
                self.enqueue_import(&receipt, vec![path.clone()])?;
                if let Some(status) = self.import_status(&receipt, vec![path.clone()])? {
                    imports.push(status);
                }
            }
            let mut row = value(&job)?;
            if !imports.is_empty() {
                let errors: Vec<_> = imports
                    .iter()
                    .filter_map(|status| status["error"].as_str())
                    .collect();
                if !errors.is_empty() {
                    row["importError"] = json!(errors.join("\n"));
                }
                let state = if imports.iter().any(|status| status["state"] == "failed") {
                    "failed"
                } else if imports.iter().all(|status| status["state"] == "imported") {
                    "imported"
                } else if imports.iter().any(|status| status["state"] == "verifying") {
                    "verifying"
                } else if imports.iter().any(|status| status["state"] == "ready") {
                    "ready"
                } else {
                    "queued"
                };
                row["importState"] = json!(state);
                row["imports"] = json!(imports);
            }
            jobs.push(row);
        }
        let saved_localswarm = if self.localswarm.get().is_none() {
            LocalSwarmManager::saved_import_snapshot(&self.store)
        } else { Ok(None) };
        let localswarm_jobs = match self.localswarm.get() {
            Some(manager) => manager.list()?,
            None => match &saved_localswarm {
                Ok(Some(saved)) => saved.jobs(),
                Ok(None) => Vec::new(),
                // A manager can own its process lock before lazy initialization
                // publishes it. Preserve listing and show authority failure on
                // each affected import instead of hiding unrelated downloads.
                Err(_) => LocalSwarmManager::saved_jobs(&self.store)?,
            },
        };
        for job in localswarm_jobs {
            if job.status == "complete" {
                self.enqueue_import_with_saved(&job.id, job.completed_paths.clone(),
                    saved_localswarm.as_ref().map(Option::as_ref).map_err(String::as_str))?;
            }
            let mut row = value(&job)?;
            row["admissionGeneration"] = json!(admission_generation);
            self.decorate_import(&mut row, &job.id, job.completed_paths)?;
            jobs.push(row);
        }
        Ok(json!(jobs))
    }
    async fn sharing_status(&self) -> Value {
        self.sharing.lock().await.as_ref().map(|s|s.status()).unwrap_or(json!({"running":false,"scope":"selected","url":null,"urls":[],"port":null,"binding":null,"bindings":[],"books":[],"token":null,"bookIds":[],"catalogUrl":null,"catalogUrls":[],"libraryCount":0}))
    }
    /// A selected share pins its archive handles, so removal and relinking must
    /// stop it first. Library scope revalidates per request and needs no guard.
    /// Callers hold the slot across their whole mutation, never just this check.
    fn selected_share_serves(
        sharing: &Option<crate::sharing::SharingServer>,
        id: &str,
    ) -> bool {
        sharing
            .as_ref()
            .is_some_and(|server| server.serves_selected(id))
    }
    pub(super) async fn nearby_release_hotspot_if_unused(&self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            let _use = self.hotspot_use.lock().await;
            if self.nearby_uses_hotspot()
                || self.sharing.lock().await.as_ref().is_some_and(|server| server.uses_hotspot()) {
                return Ok(());
            }
            if let Some(manager) = self.localswarm.get() {
                if manager.library_descriptor().await.is_some_and(|descriptor| !descriptor.cancel.is_cancelled()) {
                    return Ok(());
                }
            }
            self.mobile_hotspot_stop().await?;
        }
        Ok(())
    }

    /// Retire the running share in place. The server stays in the slot until
    /// its listeners and workers have stopped, so `selected_share_serves`
    /// cannot report "not sharing" while a selected archive is still streamed.
    async fn stop_sharing(&self) -> Result<(), String> {
        let hotspot_use = self.hotspot_use.lock().await;
        let mut sharing = self.sharing.lock().await;
        let Some(server) = sharing.as_mut() else {
            return Ok(());
        };
        let hotspot = server.uses_hotspot();
        let shutdown = server.shutdown().await;
        *sharing = None;
        // Native hotspot retirement checks this same slot. Its release must
        // never run while we own the sharing mutex.
        drop(sharing);
        // The listener and the reservation are retired together. A listener
        // failure must not leave the hotspot live while the UI says sharing
        // stopped, so always run the release and report the first failure.
        drop(hotspot_use);
        let release = async {
            if hotspot { self.nearby_release_hotspot_if_unused().await } else { Ok(()) }
        };
        shutdown.and(release.await)
    }
    /// Terminal retirement attempts every stage; an error must not authorize exit.
    // Android owns process lifetime; its native Activity retirement stays live.
    // Explicit terminal shutdown remains available to desktop/iOS and tests.
    #[cfg(any(desktop, target_os = "ios", test))]
    pub async fn shutdown(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        self.stopping.store(true, Ordering::SeqCst);
        #[cfg(desktop)]
        self.agent_connector_stop();
        self.retire_private_sync();
        self.editions.retire_authorizations();
        if let Err(error) = self.nearby_retire().await { errors.push(error); }
        #[cfg(desktop)]
        {
            // Stop live folder work before any teardown: a settled observation
            // must not import or retire a book while readers are being joined.
            let watcher = self
                .watcher
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(mut watcher) = watcher {
                watcher.shutdown().await;
            }
        }
        #[cfg(target_os = "ios")]
        if let Err(error) = self.mobile_call("localTransferState", json!({"active": false})).await {
            crate::mobile::error(self, "transfer-idle-release", error);
        }
        #[cfg(mobile)]
        self.mobile.wake.notify_waiters();
        self.cancel_pdf_jobs(None);
        self.drain_pdf_jobs().await;
        let _speech = self.mobile_speech_block(mobile_speech::Scope::All);
        self.retire_searches(None);
        self.integrity.retire().await;
        #[cfg(target_os = "ios")]
        let (_mobile_serial, retirement_errors) = self.mobile_lock_lifecycle_after_retirement().await?;
        #[cfg(target_os = "ios")]
        errors.extend(retirement_errors);
        #[cfg(all(mobile, not(target_os = "ios")))]
        let _mobile_serial = self.mobile.serial.lock().await;
        #[cfg(mobile)]
        let _mobile_transfer = self.mobile.transfer_gate.lock().await;
        self.retire_localswarm();
        let _mode_guard = self.privacy_gate.write().await;
        if let Err(error) = self.integrity.cancel_and_wait(true).await {
            errors.push(format!("Stopping archive verification: {error}"));
        }
        let imports = match self.imports.lock() {
            Ok(manager) => manager.clone(),
            Err(error) => {
                errors.push("Import manager unavailable".into());
                error.into_inner().clone()
            }
        };
        if let Err(error) = imports.shutdown_and_wait().await {
            errors.push(format!("Stopping archive imports: {error}"));
        }
        let spelling = self.spelling.clone();
        if let Err(error) = blocking(move || spelling.set_writable(false)).await {
            errors.push(format!("Stopping spelling dictionary: {error}"));
        }
        {
            let sensors = self.sensors.lock().unwrap_or_else(|error| {
                errors.push("Sensor manager unavailable".into());
                error.into_inner()
            });
            for manager in sensors.values() { manager.deactivate(); }
        }
        if let Some(manager) = self.downloads.get() {
            if let Err(error) = manager.shutdown().await {
                errors.push(format!("Pausing downloads: {error}"));
            }
        }
        if let Some(manager) = self.localswarm.get() {
            if let Err(error) = manager.stop().await {
                errors.push(format!("Stopping LocalSwarm: {error}"));
            }
        }
        let maps = self.maps.lock().unwrap_or_else(|error| {
            errors.push("Map manager unavailable".into());
            error.into_inner()
        }).clone();
        if let Some(manager) = maps {
            if let Err(error) = manager.pause().await {
                errors.push(format!("Pausing maps: {error}"));
            }
        }
        #[cfg(target_os = "ios")]
        if let Err(error) = self.mobile_release_transfer_lifetime().await {
            crate::mobile::error(self, "transfer-background-release", error);
        }
        let torrents = self.torrents.lock().unwrap_or_else(|error| {
            errors.push("Torrent manager unavailable".into());
            error.into_inner()
        }).clone();
        if let Some(manager) = torrents {
            if let Err(error) = manager.shutdown().await {
                errors.push(format!("Stopping torrents: {error}"));
            }
        }
        {
            // Retire in place: the slot stays occupied until the listeners and
            // workers have stopped, so a concurrent removal cannot unlink an
            // archive that is still being streamed.
            let mut sharing = self.sharing.lock().await;
            if let Some(server) = sharing.as_mut() {
                if let Err(error) = server.shutdown().await {
                    errors.push(format!("Stopping archive sharing: {error}"));
                }
            }
            *sharing = None;
        }
        if let Err(error) = retire_readers(&mut *self.readers.lock().await, None).await {
            errors.push(format!("Stopping archive readers: {error}"));
        }
        self.archives.lock().unwrap_or_else(|error| {
            errors.push("Archive cache unavailable".into());
            error.into_inner()
        }).clear();
        #[cfg(mobile)]
        if let Err(error) = self.mobile_clear_external_books().await {
            errors.push(format!("Releasing session archive providers: {error}"));
        }
        #[cfg(target_os = "android")]
        if let Err(error) = self.mobile_shutdown_adapter().await {
            errors.push(format!("Stopping native transfer service: {error}"));
        }
        if let Err(error) = crate::native_pdf::retry_cleanup() {
            errors.push(error);
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
}
#[cfg(not(target_os = "android"))]
impl Service {
    /// Hotspot hosting is an Android LocalOnlyHotspot reservation; other
    /// platforms have no access-point link to bind a listener to.
    pub(crate) fn hotspot_binding_address(&self) -> Result<std::net::IpAddr, String> {
        Err("Sharing over a Wi-Fi hotspot is only available on Android".into())
    }

    pub(crate) fn mobile_hotspot_status(&self) -> Value {
        Value::Null
    }
}
fn mutation_command(command: &str) -> bool {
    #[cfg(test)]
    if command == "__test_blocking_mutation" {
        return true;
    }
    if command.starts_with("private_sync_") { return true; }
    #[cfg(mobile)]
    if command.starts_with("mobile_") {
        return !matches!(command, "mobile_lifecycle_state" | "mobile_validate_background" | "mobile_ack_entry");
    }
    matches!(
        command,
        "ack_launch_locations"
            | "ack_launch_search"
            | "snapshot"
            | "set_book_zoom"
            | "set_book_pinned"
            | "set_speech_preference"
            | "remember_desktop_dialog_directory"
            | "record_search"
            | "remove_search"
            | "import_archives"
            | "scan_folder"
            | "open_article"
            | "random_article"
            | "search"
            | "article_preview"
            | "book_illustration"
            | "spelling_status"
            | "export_article"
            | "relink_book"
            | "remove_book"
            | "save_session"
            | "record_visit"
            | "close_window"
            | "clear_history"
            | "remove_history_entry"
            | "save_bookmark"
            | "remove_bookmark"
            | "import_bookmarks"
            | "migrate_library"
            | "library_files_relocate"
            | "save_settings"
            | "downloads"
            | "kokoro_status"
            | "kokoro_probe"
            | "kokoro_download"
            | "kokoro_cancel"
            | "kokoro_delete"
            | "retained_download_files"
            | "prepare_download_cleanup"
            | "start_download"
            | "pause_download"
            | "resume_download"
            | "cancel_download"
            | "remove_download_history"
            | "delete_download_files"
            | "forget_download_record"
            | "localswarm_discovery"
            | "localswarm_inspect"
            | "localswarm_download"
            | "localswarm_seed"
            | "localswarm_stop_seed"
            | "localswarm_stop"
            | "torrent_seeding"
            | "share_torrent"
            | "torrent_limits"
            | "find_updates"
            | "edition_prepare" | "edition_download" | "edition_status"
            | "edition_commit" | "edition_cancel" | "edition_rollback"
            | "map_prepare"
            | "map_start"
            | "map_status"
            | "map_pause"
            | "map_resume"
            | "map_delete_download"
            | "map_cancel"
            | "map_clear_cache"
            | "map_remove_history"
            | "map_forget_recovery"
            | "sensor_activate"
            | "sensor_deactivate"
            | "sensor_consent"
            | "sensor_revoke"
            | "sensor_reset"
            | "sensor_location"
            | "sensor_cancel"
            | "sharing_start"
            | "sharing_stop"
            | "save_resource"
            | "save_mhtml"
            | "save_pdf"
            | "print_pdf"
            | "share_pdf"
            | "share_book"
            | "save_text"
            | "save_bytes"
            | "open_external"
            | "new_window"
            | "restore_session"
            | "retry_import"
            | "spelling_build"
            | "spelling_cancel"
            | "reveal_book"
            | "move_archives"
            | "recover_migrations"
            | "open_attachment"
            | "library_maintenance"
            | "reader_storage_write"
            | "check_integrity"
            | "diagnostics_integrity"
            | "cancel_integrity"
            | "update_check"
            | "update_install"
    )
}
/// Authorize the final native handoff, including requests from a dialog that
/// was opened before another window changed the external-link setting.
fn external_link_target(settings: &store::Settings, args: &Value) -> Result<url::Url, String> {
    if settings.external_links == "deny" {
        return Err("External links are disabled in Settings. This address has not been opened.".into());
    }
    if args.get("confirmed") != Some(&Value::Bool(true)) {
        return Err("Opening an external URL requires explicit confirmation".into());
    }
    let raw = text(args, "url")?;
    let url = url::Url::parse(raw).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "https" | "http" | "mailto")
        || !url.username().is_empty()
        || url.password().is_some()
        || raw.chars().any(char::is_control)
        || (matches!(url.scheme(), "http" | "https") && url.host_str().is_none_or(str::is_empty))
    {
        return Err("External URL scheme or embedded credentials are not permitted".into());
    }
    Ok(url)
}

const MAX_EXPORT: u64 = 16 * 1024 * 1024;
fn read_saved(store: &Store, relative: &str) -> Result<Value, String> {
    let path = store.data_dir().join(relative);
    let Some(file) = crate::acquisition::open_history(&path)? else { return Ok(Value::Null); };
    if file.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("Saved transfer state exceeds 16 MiB".into());
    }
    let mut saved = crate::acquisition::read_history_json(file, 16 * 1024 * 1024)
        .map_err(|e| format!("Invalid saved transfer state: {e}"))?
        .ok_or("Saved transfer state exceeds 16 MiB")?;
    #[cfg(mobile)]
    {
        let kind = match relative {
            "downloads/downloads.json" => crate::mobile_rebase::RecordKind::Downloads,
            "torrents/jobs.json" => crate::mobile_rebase::RecordKind::Torrents,
            "maps/map-job.json" => crate::mobile_rebase::RecordKind::Maps,
            _ => return Err("This saved transfer item cannot be read. Restart the app, then try again".into()),
        };
        store.mobile_rebase_saved_json(kind, &mut saved)?;
    }
    #[cfg(desktop)]
    if store.portable_mode() {
        let kind = match relative {
            "downloads/downloads.json" => crate::profile_paths::RecordKind::Downloads,
            "torrents/jobs.json" => crate::profile_paths::RecordKind::Torrents,
            "maps/map-job.json" => crate::profile_paths::RecordKind::Maps,
            _ => return Err("This saved transfer item cannot be read. Restart the app, then try again".into()),
        };
        store.portable_rebase_saved_json(kind, &mut saved)?;
    }
    Ok(saved)
}
/// List direct-child archive candidates without following symlinks. An
/// incomplete observation cannot authorize retiring any library installation.
#[cfg(desktop)]
fn scan_watched_directory(directory: &Path, portable: bool) -> Option<HashSet<PathBuf>> {
    let entries = std::fs::read_dir(directory).ok()?;
    collect_watched_entries(entries, portable)
}

#[cfg(desktop)]
fn collect_watched_entries(
    entries: impl IntoIterator<Item = std::io::Result<std::fs::DirEntry>>,
    portable: bool,
) -> Option<HashSet<PathBuf>> {
    let mut present = HashSet::new();
    for entry in entries {
        let entry = entry.ok()?;
        let file_type = entry.file_type().ok()?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if !crate::watch::is_candidate(&path, portable) {
            continue;
        }
        present.insert(path.canonicalize().ok()?);
    }
    Some(present)
}

fn scan(root: &Path, recursive: bool, paths: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(root).map_err(|e| format!("Scanning {}: {e}", root.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ty.is_dir() && recursive {
            scan(&path, true, paths)?;
        } else if ty.is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if name.ends_with(".zim") || name.ends_with(".zimaa") {
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
    Ok(())
}

mod dispatch;

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

#[cfg(all(test, not(mobile)))]
mod android_window_tests {
    use super::*;

    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("kiwix-android-window-test-{}", Uuid::new_v4())))
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[tokio::test]
    async fn source_identity_is_required_and_rechecked_after_the_lifetime_lease() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        let old = Uuid::new_v4().to_string();
        let args = json!({"__windowLabel":"main"});
        assert!(service.clone().android_dispatch("snapshot", args.clone(), Some(old.clone()), Some("1".into())).await.is_err());
        service.android_open_window(old.clone(), 1).unwrap();
        assert!(service.clone().android_dispatch("snapshot", args.clone(), None, Some("1".into())).await.is_err());
        assert!(service.clone().android_dispatch("snapshot", args.clone(), Some(old.clone()), None).await.is_err());
        assert!(service.clone().android_dispatch("snapshot", json!({"__windowLabel":"other"}), Some(old.clone()), Some("1".into())).await.is_err());
        service.clone().android_dispatch("snapshot", args.clone(), Some(old.clone()), Some("1".into())).await.unwrap();
        service.clone().android_retire_window(&old).await.unwrap();
        assert!(service.android_open_window(old.clone(), 1).is_err());

        // Queue a source-bound close behind a lease. The label can already name
        // the replacement by the time this old document's command gets its read.
        let writer = service.android_window_gate.write().await;
        let worker = service.clone();
        let source = old.clone();
        let stale = tokio::spawn(async move {
            worker.android_dispatch("close_window", json!({"__windowLabel":"main"}), Some(source), Some("1".into())).await
        });
        tokio::task::yield_now().await;
        let fresh = Uuid::new_v4().to_string();
        service.android_open_window(fresh.clone(), 2).unwrap();
        drop(writer);
        assert!(stale.await.unwrap().is_err());
        assert!(service.clone().android_retire_window(&old).await.is_err());
        assert_eq!(service.android_window_token(), Some(fresh.clone()));
        service.ensure_window_active("main").unwrap();
        service.clone().android_dispatch("snapshot", args, Some(fresh.clone()), Some("2".into())).await.unwrap();
        service.clone().android_retire_window(&fresh).await.unwrap();
        service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_waiters_do_not_release_accepted_work_or_abandon_the_drain() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        let old = Uuid::new_v4().to_string();
        service.android_open_window(old.clone(), 1).unwrap();
        let session = service.search_control("main", "search_session", &json!({})).unwrap();
        let owner = service.search_control("main", "search_owner", &session).unwrap();
        let search = service.search_token("main", &json!({
            "sessionId":session["sessionId"], "ownerId":owner["ownerId"],
            "privacyGeneration":service.launch_generation(), "requestId":1,
        })).unwrap();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        *service.mutation_test_gate.lock().unwrap() = Some((entered, blocked));
        let worker = service.clone();
        let source = old.clone();
        let caller = tokio::spawn(async move {
            worker.android_dispatch("__test_blocking_mutation", json!({"__windowLabel":"main"}), Some(source), Some("1".into())).await
        });
        started.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());

        let mut retirement = Box::pin(service.clone().android_retire_window(&old));
        assert!(futures_util::poll!(&mut retirement).is_pending());
        assert!(search.is_cancelled());
        assert!(service.closed_windows.lock().unwrap().contains("main"));
        let fresh = Uuid::new_v4().to_string();
        assert!(service.android_open_window(fresh.clone(), 2).is_err());
        // Dropping the lifecycle waiter must leave its already-started drain
        // alive and closed until the cancelled IPC's blocking work completes.
        drop(retirement);
        release.send(()).unwrap();
        service.clone().android_retire_window(&old).await.unwrap();
        assert_eq!(std::fs::read(profile.0.join("owned-mutation")).unwrap(), b"finished");
        service.android_open_window(fresh.clone(), 2).unwrap();
        assert!(service.clone().android_dispatch("close_window", json!({"__windowLabel":"main"}), Some(old), Some("1".into())).await.is_err());
        service.ensure_window_active("main").unwrap();
        service.clone().android_retire_window(&fresh).await.unwrap();
        service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn retained_uuid_requires_draining_and_a_new_source_activity_generation() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        let token = Uuid::new_v4().to_string();
        let first = 9_007_199_254_740_993_i64;
        let second = first + 1;
        service.android_open_window(token.clone(), first).unwrap();
        assert!(service.android_open_window(token.clone(), second).is_err());
        service.clone().android_retire_window(&token).await.unwrap();
        assert!(service.android_open_window(token.clone(), first).is_err());
        assert!(service.android_open_window(token.clone(), first - 1).is_err());

        let writer = service.android_window_gate.write().await;
        let worker = service.clone();
        let source = token.clone();
        let stale = tokio::spawn(async move {
            worker.android_dispatch(
                "close_window", json!({"__windowLabel":"main"}),
                Some(source), Some(first.to_string()),
            ).await
        });
        tokio::task::yield_now().await;
        service.android_open_window(token.clone(), second).unwrap();
        drop(writer);
        assert!(stale.await.unwrap().is_err());
        assert!(service.clone().android_dispatch(
            "snapshot", json!({"__windowLabel":"main"}),
            Some(token.clone()), Some(first.to_string()),
        ).await.is_err());
        service.ensure_window_active("main").unwrap();
        service.clone().android_dispatch(
            "snapshot", json!({"__windowLabel":"main"}),
            Some(token.clone()), Some(second.to_string()),
        ).await.unwrap();
        service.clone().android_retire_window(&token).await.unwrap();
        service.shutdown().await.unwrap();
    }
}

fn preview_excerpt(document: &crate::exports::html::Html) -> String {
    use crate::exports::html;
    document.elements()
        .filter(|element| element.has_name("p"))
        .find_map(|paragraph| {
            let text = html::text(&paragraph);
            let excerpt = preview_text(std::iter::once(text.as_str()));
            (!excerpt.is_empty()).then_some(excerpt)
        })
        .unwrap_or_else(|| preview_text(std::iter::once(html::text(&document.root_element()).as_str())))
}

fn preview_text<'a>(texts: impl Iterator<Item = &'a str>) -> String {
    let mut output = String::new();
    let mut count = 0usize;
    let mut space = true;
    'outer: for text in texts {
        for character in text.chars() {
            if character.is_whitespace() {
                if !space {
                    output.push(' ');
                    count += 1;
                    space = true;
                }
            } else if !character.is_control() {
                output.push(character);
                count += 1;
                space = false;
            }
            if count >= 2048 {
                break 'outer;
            }
        }
    }
    if output.ends_with(' ') {
        output.pop();
    }
    output
}

fn preview_thumbnail(archive: &Archive, path: &str, document: &crate::exports::html::Html) -> Option<String> {
    use crate::exports::html;
    // A synthetic origin is only a URL resolver; it is never fetched.
    let origin = url::Url::parse("https://archive.invalid/").ok()?;
    let page = origin.join(&encode_path(path)).ok()?;
    // The first base[href]: any <base> with an un-namespaced href.
    let href = document.elements().filter(|element| element.has_name("base")).find_map(|base| {
        html::attr(&html::element(&base), "href").map(str::to_owned)
    });
    let base = match href {
        Some(href) if href.len() <= 16384 => page.join(&href).ok()?,
        Some(_) => return None,
        None => page,
    };
    for image in document.elements().filter(|element| element.has_name("img")).take(8) {
        let image = html::element(&image);
        for source in ["src", "data-src"].into_iter().filter_map(|name| html::attr(&image, name)) {
            if source.trim().is_empty() || source.len() > 16384 {
                continue;
            }
            let Ok(url) = base.join(source) else { continue };
            if url.origin() != origin.origin() || !url.username().is_empty() || url.password().is_some() {
                continue;
            }
            let Ok(path) = crate::server::decode_location_path(url.path()) else { continue };
            let Ok(entry) = archive.entry(&path) else { continue };
            if entry.size > 256 * 1024 {
                continue;
            }
            let Ok(bytes) = archive.read(&entry.path, 0, entry.size) else { continue };
            if let Some(value) = crate::catalog::raster_data_url(&bytes) {
                return Some(value);
            }
        }
    }
    None
}
