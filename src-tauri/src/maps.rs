// SPDX-License-Identifier: GPL-3.0-or-later
//! StreetZim build-index/1 and recipe/2 composition, ported from kiwix-apple.
//! Only complete, verified files are published; library import belongs to the caller.
//!
//! Network sources are HTTPS-only (including redirects), with exact 206 range
//! semantics and bounded bodies. Network parts persist by masked SHA-256/length;
//! every reuse rechecks their hash. Pause journals an owned, synced prefix;
//! Resume rechecks its identity, prefix hash and exact recipe before appending.
//! Owned output reservations and inodes are journaled before composition writes.
//! Cleanup first captures
//! an exact journaled entry in a private capability, then checks inode and size;
//! mismatches are restored exclusively or retained at a reported recovery path.
//! Local base/other-language candidates are opened read-only.
//! Native FAT/FAT32 limits apply to the final output and individual cache chunks,
//! not to source archive sizes or the aggregate cache-space requirement.
//! `cancel` waits for cleanup; publication commits completion even if cancellation
//! arrives immediately afterward. `status` retains the last terminal job across
//! restarts. Only `complete`/`path` is eligible for caller-owned archive validation
//! and import; an import failure must not delete that verified output.
//! Completion/path and removal of publication recovery are one durable checkpoint;
//! failed completion saves retain the publication identity for restart recovery.
//! Standalone catalog work is owned through policy retirement, including joined
//! JSON parsing/validation, even when the caller drops its command future.
//! Catalog admission retains its denial revision from before registration/spawn;
//! reconciliation joins all registered catalogs before acknowledging retirement.
//! Cancellation reports durable terminal checkpoint failures, including late callers.
//! Completion also binds the ambient output path to the verified open handle
//! and recorded directory identity. Failed publication checks preserve recovery
//! identities without deleting potentially unowned output. Uncommitted publication
//! checkpoints are reclaimable; unresolved records follow later jobs as visible
//! recovery warnings rather than blocking unrelated compositions. Fresh publication
//! also holds the library reservation guard, so even a missing installed book's
//! path cannot be filled with another acquisition's bytes.
//! Low-space admission runs after recipe metadata and cache accounting, before
//! any output reservation or source payload. The selected destination volume
//! includes the output plus missing cache bytes on that volume; a separate cache
//! volume is confirmed independently. A failed job retains the structured
//! warning for explicit retry of the same request with its opaque acknowledgement.
//! Replacing the current job retains earlier completed destinations as durable
//! automatic-monitor exclusions only, never as publication or import authority.
#![forbid(unsafe_code)]
use base64::{engine::general_purpose::STANDARD, Engine};
use crate::map_chunk_cache::{MapChunkCache, MAX_BYTES as MAX_CACHE_BYTES};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata};
use futures_util::{FutureExt, StreamExt};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{watch, OnceCell},
};
use url::Url;
use uuid::Uuid;

mod partials;
#[allow(unused_imports)]
pub(crate) use self::partials::*;
mod recipe;
#[allow(unused_imports)]
pub(crate) use self::recipe::*;
mod manager;
#[allow(unused_imports)]
pub(crate) use self::manager::*;
mod compose;
#[allow(unused_imports)]
pub(crate) use self::compose::*;

pub const DEFAULT_CATALOG_URL: &str = "https://streetzim.web.app/build/recipes/index.json";
pub const DEFAULT_PROXY_URL: &str = "https://streetzim-build-proxy.tiltastech.workers.dev";
const MAX_JSON: usize = 32 * 1024 * 1024;
const MAX_STATE: usize = 64 * 1024 * 1024;
const MAX_CHUNK: u64 = 16 * 1024 * 1024;
// Metadata estimation is pre-consent work: cap expanded reads independently of
// JSON size. Current production California/en uses 942; the largest recipe JSON
// within MAX_JSON (Germany/nl, 33,390,256 bytes) uses 2,871.
const MAX_ESTIMATE_CHUNK_READS: u64 = 100_000;
const CANCELLED: &str = "Map composition cancelled";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapCatalog {
    pub format: String,
    pub regions: Vec<MapRegion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapRegion {
    pub id: String,
    pub title: String,
    pub base: MapBase,
    pub menu: Vec<MapLanguage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapBase {
    pub file: String,
    pub size: u64,
    pub download: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapLanguage {
    pub lang: String,
    pub label: String,
    pub name: String,
    pub recipe: String,
    #[serde(default)]
    pub size: Option<u64>,
    pub download_bytes: u64,
    pub md5: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRecipeMetadata {
    pub region_id: String,
    pub language: String,
    pub name: String,
    pub md5: String,
    pub uuid: String,
    pub version: String,
    pub assembled_bytes: u64,
    /// Source payload with an empty cache and no reusable local base.
    pub cold_download_bytes: u64,
    /// Source payload with every base chunk supplied by a verified local map.
    pub reusable_base_download_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPreparation {
    pub token: String,
    pub region_id: String,
    pub language: String,
    pub name: String,
    pub md5: String,
    pub version: String,
    pub installed: Option<crate::store::Book>,
    pub same_version: bool,
}

/// Durable evidence of the original choice, not authority after a process restart.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MapDecision {
    token: String,
    session: String,
    /// Native privacy-policy epoch; ordinary reader/background retirement is separate.
    generation: u64,
    catalog_url: String,
    proxy_url: String,
    destination: String,
    region_id: String,
    language: String,
    name: String,
    md5: String,
    uuid: String,
    size: u64,
    target: Option<crate::store::Book>,
}

impl MapDecision {
    pub(crate) fn target(&self) -> Option<&crate::store::Book> { self.target.as_ref() }
    pub(crate) fn archive_id(&self) -> &str { &self.uuid }
    pub(crate) fn token(&self) -> &str { &self.token }

    pub(crate) fn validate(&self, state: &crate::store::State) -> Result<(), String> {
        if state.settings.map_catalog_url != self.catalog_url
            || state.settings.map_proxy_url != self.proxy_url
            || state.settings.download_directory != self.destination
            || state.settings.private_mode || state.settings.read_only
        {
            return Err("Map settings changed; select the map again before downloading or replacing it".into());
        }
        let mut matching = state.books.iter().filter(|book|
            book.metadata.name == self.name || book.metadata.id == self.uuid);
        match (matching.next(), matching.next(), &self.target) {
            (None, None, None) => Ok(()),
            (Some(current), None, Some(expected))
                if current.installation_id == expected.installation_id
                    && current.metadata.id == expected.metadata.id
                    && current.metadata.name == expected.metadata.name
                    && current.metadata.date == expected.metadata.date
                    && current.metadata.size == expected.metadata.size
                    && current.path == expected.path
                    && current.source_reference == expected.source_reference => Ok(()),
            _ => Err("The installed map changed; select the map again before replacing it".into()),
        }
    }

    /// Called only inside Store's isolated atomic update, after verified import.
    pub(crate) fn register(
        &self, state: &mut crate::store::State, book: &crate::store::Book, receipt: &str,
    ) -> Result<(), String> {
        self.validate(state)?;
        if book.metadata.id != self.uuid || book.metadata.name != self.name
            || book.metadata.size != self.size || book.source_reference.is_some()
        {
            return Err("Verified map does not match the authorized recipe".into());
        }
        if let Some(old) = &self.target {
            let old_id = &old.metadata.id;
            let new_id = &book.metadata.id;
            let mut replacement = book.clone();
            replacement.added_at.clone_from(&old.added_at);
            let index = state.books.iter().position(|entry|
                entry.installation_id == old.installation_id).ok_or("Map installation changed")?;
            state.books[index] = replacement;
            for bookmark in &mut state.bookmarks {
                if &bookmark.book_id == old_id { bookmark.book_id.clone_from(new_id); }
            }
            for entry in &mut state.history {
                if &entry.book_id == old_id { entry.book_id.clone_from(new_id); }
            }
            for tab in state.tabs.iter_mut().chain(state.windows.values_mut().flat_map(|window| &mut window.tabs)) {
                if tab.book_id.as_ref() == Some(old_id) { tab.book_id = Some(new_id.clone()); }
                for location in tab.back.iter_mut().chain(&mut tab.forward) {
                    if &location.book_id == old_id { location.book_id.clone_from(new_id); }
                }
            }
            // Do not grant the new archive its predecessor's reader credentials
            // or sharing consent. Browsing references alone move to the new UUID.
            state.reader_storage.remove(&old.installation_id);
            state.settings.sharing_book_ids.retain(|id| id != old_id);
            state.settings.remap_pinned_book(old_id, new_id);
        } else {
            state.books.push(book.clone());
        }
        state.consumed_acquisitions.insert(receipt.into());
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapJob {
    pub id: String,
    pub status: String,
    /// When this map download was started (RFC3339), so Downloads can list the
    /// most recent first. Absent in older job journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub region_id: String,
    pub language: String,
    pub written: u64,
    pub total: u64,
    pub incoming: u64,
    pub incoming_total: u64,
    pub error: Option<String>,
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks_total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks_ready: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks_downloaded: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) decision: Option<MapDecision>,
    #[serde(default)]
    pub resumable: bool,
    #[serde(default)]
    pub can_remove_from_list: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resume: Option<MapResume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    published: Option<PublishedMap>,
    /// Native publication receipts survive replacement of this singleton job.
    /// Monitored paths and imported replacement targets never create receipts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retained_publications: Vec<RetainedMap>,
    /// Dismiss visible history without discarding cache, recovery or path claims.
    #[serde(default)]
    pub history_dismissed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owned_partial: Option<OwnedPartial>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recovery_records: Vec<OwnedPartial>,
    /// Prior visible destinations survive replacement of this single-job status.
    /// These names are exclusions only; they never authorize recovery or import.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    monitored_paths: Vec<PathBuf>,
}

#[cfg(target_os = "ios")]
pub(crate) fn backup_evidence(store: &crate::store::Store) -> Result<Vec<(PathBuf, crate::library_files::BackupEvidence)>, String> {
    let saved: Option<MapJob> = crate::library_files::backup_record(
        store, &["maps", "map-job.json"], 128 * 1024 * 1024,
        crate::mobile_rebase::RecordKind::Maps,
    )?;
    let mut candidates = Vec::new();
    if let Some(job) = &saved {
        for retained in &job.retained_publications {
            let published = &retained.published;
            if published.record.capture.is_none() {
                candidates.push((
                    published.record.destination.join(&published.record.name),
                    crate::library_files::BackupEvidence {
                        size: published.size,
                        identity: published.record.identity.as_ref().map(|identity| (identity.device, identity.inode)),
                        sha256: published.sha256,
                    },
                ));
            }
        }
    }
    if let Some(job) = saved.filter(|job| job.status == "complete" && job.written == job.total) {
        if let Some(path) = job.path {
            let path = PathBuf::from(path);
            let identity = job.owned_partial.as_ref().and_then(|record| {
                let publication = record.publication.as_ref()?;
                let identity = record.identity.as_ref()?;
                (record.destination.join(&publication.name) == path && publication.size == job.total)
                    .then_some((identity.device, identity.inode))
            });
            // Old completed jobs dropped their inode checkpoint. Such an
            // unmarked path is exclusion-only until verified library admission.
            candidates.push((path, crate::library_files::BackupEvidence {
                size: job.total, identity, sha256: None,
            }));
        }
    }
    Ok(candidates)
}
impl MapJob {
    fn publication(&self, id: &str) -> Option<&PublishedMap> {
        if self.id == id { self.published.as_ref() }
        else { self.retained_publications.iter().find(|saved| saved.id == id).map(|saved| &saved.published) }
    }

    fn retain_current_path(&mut self) {
        if let Some(path) = self.path.take() {
            let path = PathBuf::from(path);
            if !self.monitored_paths.contains(&path) {
                self.monitored_paths.push(path);
            }
        }
    }

    fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {
        if let Some(claimed) = &self.path {
            if crate::acquisition::monitored_path_matches(path, Path::new(claimed))? {
                return Ok(true);
            }
        }
        for claimed in &self.monitored_paths {
            if crate::acquisition::monitored_path_matches(path, claimed)? {
                return Ok(true);
            }
        }
        for record in self.owned_partial.iter().chain(&self.recovery_records) {
            if let Some(publication) = &record.publication {
                if crate::acquisition::monitored_child_matches(
                    path, &record.destination, Path::new(&publication.name),
                )? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Remove only the recovery sentence from the visible error, keeping any
    /// other failure (for example a low-disk warning) intact.
    fn clear_recovery_warning(&mut self) {
        const PREFIX: &str =
            "Some earlier map downloads could not be cleaned up. The files were kept here: ";
        let Some(error) = self.error.take() else { return };
        if let Some(mut value) = error
            .strip_prefix("LOW_DISK_SPACE:")
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .filter(serde_json::Value::is_object)
        {
            if let Some(object) = value.as_object_mut() {
                object.remove("recoveryWarning");
            }
            self.error = Some(format!("LOW_DISK_SPACE:{value}"));
            return;
        }
        self.error = match error.find(PREFIX) {
            Some(index) => {
                let before = error[..index].trim_end_matches([',', ';', ' ']);
                (!before.is_empty()).then(|| before.to_string())
            }
            None => Some(error),
        };
    }

    fn append_recovery_warning(&mut self) {
        use std::fmt::Write;
        if self.recovery_records.is_empty() {
            return;
        }
        let error = self.error.take().unwrap_or_default();
        let mut warning = String::new();
        warning.push_str("Some earlier map downloads could not be cleaned up. The files were kept here: ");
        for (index, record) in self.recovery_records.iter().enumerate() {
            if index != 0 {
                warning.push_str(", ");
            }
            let name = record
                .publication
                .as_ref()
                .map_or(record.name.as_str(), |saved| saved.name.as_str());
            let _ = write!(warning, "{}", record.destination.join(name).display());
        }
        self.error = Some(
            if let Some(mut value) = error
                .strip_prefix("LOW_DISK_SPACE:")
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .filter(serde_json::Value::is_object)
            {
                value["recoveryWarning"] = warning.into();
                format!("LOW_DISK_SPACE:{value}")
            } else if error.is_empty() {
                warning
            } else {
                format!("{error}; {warning}")
            },
        );
    }
}



#[cfg(test)]
type BeforeJsonParse = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;
#[cfg(test)]
type BeforePublish = Mutex<Option<Box<dyn FnOnce(&Path) + Send>>>;

/// Start of the current phase and completed (phase name, elapsed microseconds) samples.
#[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
type EvaluationPhaseTimings = (std::time::Instant, Vec<(String, u128)>);
/// Hashed bytes, elapsed microseconds, optional thread CPU microseconds, digest calls.
#[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
type EvaluationHashMetrics = (u64, u128, Option<u128>, u64);

#[derive(Clone)]
struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Sender<Option<Result<(), String>>>,
    /// 0 running, 1 retain/pause, 2 explicit revoke/cancel (wins over pause).
    retirement: Arc<std::sync::atomic::AtomicU8>,
    space_identity: Option<Arc<String>>,
    space_acknowledgements: Arc<MapSpaceAcknowledgements>,
    /// Deterministic capacity for admission tests; `None` reads the volume.
    #[cfg(test)]
    available_bytes: Option<u64>,
    #[cfg(test)]
    before_json_parse: BeforeJsonParse,
    #[cfg(test)]
    pause_after: Option<u64>,
    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    evaluation_cancel_after: Option<u64>,
    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    evaluation_phases: Arc<Mutex<EvaluationPhaseTimings>>,
    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    evaluation_hash: Arc<Mutex<EvaluationHashMetrics>>,
    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    evaluation_sources: Arc<HashMap<String, Arc<fs::File>>>,
}

impl Control {
    fn new() -> Self {
        Self {
            cancel: watch::channel(false).0,
            done: watch::channel(None).0,
            retirement: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            space_identity: None,
            space_acknowledgements: Arc::new(MapSpaceAcknowledgements::default()),
            #[cfg(test)]
            available_bytes: None,
            #[cfg(test)]
            before_json_parse: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            pause_after: None,
            #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
            evaluation_cancel_after: None,
            #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
            evaluation_phases: Arc::new(Mutex::new((std::time::Instant::now(), Vec::new()))),
            #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
            evaluation_hash: Arc::new(Mutex::new((0, 0, Some(0), 0))),
            #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
            evaluation_sources: Arc::new(HashMap::new()),
        }
    }
    fn request_pause(&self) {
        let _ = self.retirement.compare_exchange(0, 1,
            std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
        self.cancel.send_replace(true);
    }
    fn is_pausing(&self) -> bool {
        self.retirement.load(std::sync::atomic::Ordering::SeqCst) == 1
    }
    fn check(&self) -> Result<(), String> {
        if *self.cancel.borrow() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
    #[cfg(test)]
    fn available_bytes(&self) -> Option<u64> {
        self.available_bytes
    }
    #[cfg(not(test))]
    fn available_bytes(&self) -> Option<u64> {
        None
    }
    #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
    fn evaluation_mark(&self, name: &str) {
        if let Ok(mut phases) = self.evaluation_phases.lock() {
            let elapsed = phases.0.elapsed().as_micros();
            phases.1.push((name.into(), elapsed));
            phases.0 = std::time::Instant::now();
        }
    }
    fn chunk_sha256(&self, bytes: &[u8]) -> String {
        #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
        let (start, cpu) = (std::time::Instant::now(), crate::cpu_time::thread_cpu_time_micros());
        let digest = Sha256::digest(bytes);
        #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
        if let Ok(mut metrics) = self.evaluation_hash.lock() {
            metrics.0 += bytes.len() as u64;
            metrics.1 += start.elapsed().as_micros();
            let elapsed = crate::cpu_time::thread_cpu_time_micros().zip(cpu).map(|(end, start)| end.saturating_sub(start));
            metrics.2 = metrics.2.zip(elapsed).map(|(total, elapsed)| total + elapsed);
            metrics.3 += 1;
        }
        hex::encode(digest)
    }
    async fn cancelled(&self) {
        let mut rx = self.cancel.subscribe();
        let _ = rx.wait_for(|value| *value).await;
    }
}

struct ManagerState {
    job: Option<MapJob>,
    active: Option<Arc<Control>>,
    // A resumed job keeps its ID. Distinguish its worker from earlier
    // admissions even after both workers have retired.
    admission: u64,
    retirement_error: Option<String>,
    preparation: Option<MapDecision>,
    /// A cache clear owns the private parts directory while it runs. The state
    /// lock is released for that filesystem work, so job admission must see the
    /// reservation instead of the lock.
    cache_reserved: bool,
}
struct Inner {
    cache: PathBuf,
    store: Arc<crate::store::Store>,
    state_path: PathBuf,
    client: OnceCell<reqwest::Client>,
    state: Mutex<ManagerState>,
    session: String,
    active_work: std::sync::atomic::AtomicBool,
    /// Native expiry callbacks never wait behind state persistence.
    pause_control: Mutex<Option<Arc<Control>>>,
    #[cfg(test)]
    before_publish: BeforePublish,
    #[cfg(test)]
    before_publish_commit: BeforePublish,
    #[cfg(test)]
    before_worker: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Inner {
    async fn client(&self, control: &Control) -> Result<&reqwest::Client, String> {
        crate::transfer_policy::require_internet()?;
        control.check()?;
        tokio::select! {
            biased;
            _ = control.cancelled() => Err(CANCELLED.into()),
            result = self.client.get_or_try_init(|| async { checked_client() }) => result,
        }
    }
}

#[derive(Clone)]
pub struct MapManager {
    inner: Arc<Inner>,
}

fn checked_client() -> Result<reqwest::Client, String> {
    // One retained client per process: standalone catalog/recipe commands and
    // every manager clone share its connector pool instead of paying a new
    // TCP/TLS handshake per fetch. Cloning a client shares that pool.
    static CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| {
        reqwest::Client::builder()
            .user_agent(format!(
                "kiwix-workbench/{} ({})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS
            ))
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 8 {
                    return attempt.error("Too many StreetZim redirects");
                }
                if checked_url(attempt.url().as_str()).is_err() {
                    return attempt.error("Unsafe StreetZim redirect");
                }
                attempt.follow()
            }))
            .build()
            .map_err(|e| e.to_string())
    });
    let client: &Result<reqwest::Client, String> = &CLIENT;
    client.clone()
}

/// Fetch and validate a catalog without creating a manager, cache, or job state.
pub async fn fetch_catalog(url: &str) -> Result<MapCatalog, String> {
    let revision = crate::transfer_policy::admission_revision()?;
    let client = checked_client()?;
    let url = url.to_owned();
    let control = Arc::new(Control::new());
    let worker_control = control.clone();
    let result = CATALOGS.spawn(
        revision,
        control.clone(),
        async move { fetch_catalog_controlled(&client, &url, &worker_control).await },
    )?;
    let _cancel_on_drop = CatalogCancellation(control);
    result.await.map_err(|_| "Catalog worker ended without a result".to_string())?
}

/// Read only catalog/recipe JSON; never open source payload, cache or job state.
pub async fn fetch_recipe_metadata(
    url: &str,
    region_id: &str,
    language: &str,
) -> Result<MapRecipeMetadata, String> {
    let revision = crate::transfer_policy::admission_revision()?;
    let client = checked_client()?;
    let url = url.to_owned();
    let region_id = region_id.to_owned();
    let language = language.to_owned();
    let control = Arc::new(Control::new());
    let worker_control = control.clone();
    let result = CATALOGS.spawn(revision, control.clone(), async move {
        let catalog = fetch_catalog_controlled(&client, &url, &worker_control).await?;
        let region = catalog.regions.into_iter().find(|region| region.id == region_id)
            .ok_or("Selected map region is not in the catalog")?;
        let version = region.menu.iter().find(|version| version.lang == language)
            .ok_or("Selected map language is not offered")?;
        let recipe: Recipe = fetch_json(
            &client, recipe_url(&checked_url(&url)?, &version.recipe)?, &worker_control,
        ).await?;
        if !recipe.matches_catalog(&region, version) {
            return Err("Recipe does not match the selected catalog version".into());
        }
        let name = version.name.clone();
        let md5 = version.md5.clone();
        tokio::task::spawn_blocking(move || {
            recipe.validate(&worker_control)?;
            Ok::<_, String>(MapRecipeMetadata {
                region_id, language, name, md5,
                uuid: recipe.uuid.clone(), version: recipe.output_stem().into(),
                assembled_bytes: recipe.size,
                cold_download_bytes: recipe.download_estimate(false, &worker_control)?,
                reusable_base_download_bytes: recipe.download_estimate(true, &worker_control)?,
            })
        }).await.map_err(|error| format!("Recipe metadata worker failed: {error}"))?
    })?;
    let _cancel_on_drop = CatalogCancellation(control);
    result.await.map_err(|_| "Recipe metadata worker ended without a result".to_string())?
}

#[derive(Default)]
struct CatalogWorkers {
    controls: HashMap<Uuid, Arc<Control>>,
    tasks: tokio::task::JoinSet<()>,
    failure: Option<String>,
}

#[derive(Default)]
struct CatalogRegistry {
    workers: Mutex<CatalogWorkers>,
    #[cfg(any(mobile, test))]
    retirement: tokio::sync::Mutex<()>,
}

static CATALOGS: LazyLock<Arc<CatalogRegistry>> =
    LazyLock::new(|| Arc::new(CatalogRegistry::default()));

impl CatalogRegistry {
    fn spawn<T: Send + 'static>(
        self: &Arc<Self>,
        revision: u64,
        control: Arc<Control>,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<T, String>>, String> {
        let mut workers = self.workers.lock().map_err(|_| "Catalog ownership lock poisoned")?;
        crate::transfer_policy::require_internet_since(revision)?;
        while let Some(result) = workers.tasks.try_join_next() {
            if let Err(error) = result {
                workers.failure = Some(format!("Catalog worker failed: {error}"));
            }
        }
        if let Some(error) = workers.failure.as_ref() {
            return Err(error.clone());
        }
        let id = Uuid::new_v4();
        let (completed, result) = tokio::sync::oneshot::channel();
        // Register before spawn under the same ownership lock used by retirement.
        workers.controls.insert(id, control.clone());
        let registry = self.clone();
        workers.tasks.spawn(async move {
            let result = async {
                crate::transfer_policy::require_internet_since(revision)?;
                control.check()?;
                catalog_until_denied(
                    &control, work, crate::transfer_policy::internet_denied_since(revision),
                ).await
            }.await;
            if let Ok(mut workers) = registry.workers.lock() {
                workers.controls.remove(&id);
            }
            let _ = completed.send(result);
        });
        Ok(result)
    }

    #[cfg(any(mobile, test))]
    async fn retire(&self) -> Result<(), String> {
        let _retirement = self.retirement.lock().await;
        let mut tasks = {
            let mut workers = self.workers.lock().map_err(|_| "Catalog ownership lock poisoned")?;
            for control in std::mem::take(&mut workers.controls).into_values() {
                control.cancel.send_replace(true);
            }
            std::mem::take(&mut workers.tasks)
        };
        let mut failure = None;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                failure = Some(format!("Catalog worker failed during retirement: {error}"));
            }
        }
        let mut workers = self.workers.lock().map_err(|_| "Catalog ownership lock poisoned")?;
        if let Some(error) = failure {
            // A panic can bypass joining nested blocking work. A later retirement
            // must not silently acknowledge that unproven completion.
            workers.failure = Some(error);
        }
        workers.failure.clone().map_or(Ok(()), Err)
    }
}

/// Cancel and join every registered standalone catalog before policy acknowledgement.
#[cfg(mobile)]
pub(crate) async fn retire_catalogs() -> Result<(), String> {
    // Retirement itself remains owned if its caller disappears during a join.
    tokio::spawn(async { CATALOGS.retire().await })
        .await
        .map_err(|error| format!("Catalog retirement worker failed: {error}"))?
}

struct CatalogCancellation(Arc<Control>);

impl Drop for CatalogCancellation {
    fn drop(&mut self) {
        self.0.cancel.send_replace(true);
    }
}

async fn catalog_until_denied<T>(
    control: &Control,
    work: impl Future<Output = Result<T, String>>,
    denied: impl Future<Output = ()>,
) -> Result<T, String> {
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = denied => {
            control.cancel.send_replace(true);
            // Cancelling drops active request/body futures through Control;
            // blocking JSON/validation workers are cooperatively stopped and joined.
            let _ = work.await;
            Err("MOBILE_NETWORK_BLOCKED".into())
        }
        result = &mut work => result,
    }
}

async fn fetch_catalog_controlled(
    client: &reqwest::Client,
    url: &str,
    control: &Control,
) -> Result<MapCatalog, String> {
    crate::transfer_policy::require_internet()?;
    control.check()?;
    let index = checked_url(url)?;
    let catalog: MapCatalog = fetch_json(client, index.clone(), control).await?;
    let control = control.clone();
    tokio::task::spawn_blocking(move || {
        control.check()?;
        catalog.validate(&index, &control)?;
        control.check()?;
        Ok(catalog)
    })
    .await
    .map_err(|e| format!("Catalog validation worker failed: {e}"))?
}




/// Android-only evaluation enters the same validator, bounded chunk reader,
/// journal, cancellation cleanup and atomic publisher as ordinary map jobs.
#[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
pub(crate) mod evaluation {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    const MAX_EVALUATION_SIZE: u64 = 8 * 1024 * 1024 * 1024;

    pub(crate) struct Prepared {
        manager: MapManager,
        control: Control,
        recipe: Recipe,
        destination: PathBuf,
        pub(crate) sentinel: Option<PathBuf>,
    }

    pub(crate) async fn prepare(
        root: &Path,
        recipe_bytes: &[u8],
        candidates: Vec<PathBuf>,
        expected_size: u64,
        mode: &str,
    ) -> Result<Prepared, String> {
        let mut recipe: Recipe = serde_json::from_slice(recipe_bytes).map_err(|e| e.to_string())?;
        if recipe.size != expected_size || recipe.size <= 16 || recipe.size > MAX_EVALUATION_SIZE {
            return Err("Evaluation recipe output size differs from bounded manifest".into());
        }
        let source_bytes = std::iter::once(&recipe.base).chain(recipe.packs.values())
            .try_fold(0u64, |total, source| total.checked_add(source.size))
            .ok_or("Evaluation source size overflow")?;
        let source_budget = expected_size.checked_mul(2)
            .and_then(|size| size.checked_add(32 * 1024 * 1024))
            .ok_or("Evaluation source budget overflow")?;
        if source_bytes > source_budget {
            return Err("Evaluation staged sources exceed bounded workload budget".into());
        }
        let mut control = Control::new();
        let masks = recipe.validate(&control)?;
        let store = Arc::new(crate::store::Store::new_with_mode(root.join("profile"), false)?);
        let manager = MapManager::new(root.join("state"), store)?;
        // Fixture preparation retains read-only raw sources, not whole-source
        // copies in the map cache. Misses enter the normal range/mask/SHA path.
        let mut sources = HashMap::new();
        let mut warm_remaining = if mode == "warm" { MAX_CACHE_BYTES } else { 0 };
        let mut warmed = HashSet::new();
        let mut warm_done = mode != "warm";
        let mut cache = MapChunkCache::open(&manager.inner.cache, || control.check())?;
        let mut source_keys: Vec<_> = recipe.packs.keys().collect();
        source_keys.sort_unstable();
        for (key, source) in std::iter::once(("base".to_owned(), &recipe.base))
            .chain(source_keys.into_iter().map(|key| (format!("pack:{key}"), &recipe.packs[key])))
        {
            let path = candidates.iter().find(|path| path.file_name()
                .and_then(|name| name.to_str()) == Some(source.file.as_str()))
                .ok_or_else(|| format!("Missing staged source {}", source.file))?;
            let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
            if file.metadata().map_err(|error| error.to_string())?.len() != source.size {
                return Err("Staged source size mismatch".into());
            }
            for (index, expected) in source.sha256.iter().enumerate() {
                let start = index as u64 * source.chunk;
                let len = source.chunk.min(source.size - start);
                if warm_done { break; }
                if len > MAX_CACHE_BYTES { continue; }
                let name = format!("{expected}-{len}");
                if warmed.contains(&name) { continue; }
                if len > warm_remaining || warmed.len() == crate::map_chunk_cache::MAX_ENTRIES {
                    warm_done = true;
                    break;
                }
                let mut bytes = vec![0u8; len as usize];
                file.seek(SeekFrom::Start(start)).map_err(|error| error.to_string())?;
                file.read_exact(&mut bytes).map_err(|error| error.to_string())?;
                masks.get(&key).ok_or("Missing staged mask")?
                    .apply(&mut bytes, start, &control)?;
                if hex::encode(Sha256::digest(&bytes)) != *expected {
                    return Err("Staged source SHA-256 mismatch".into());
                }
                let inner = manager.inner.clone();
                cache.store(&name, Arc::new(bytes),
                    move || check_capacity(0, len, &inner.cache, &inner.cache)).await?;
                warm_remaining -= len;
                warmed.insert(name);
            }
            sources.insert(key, Arc::new(file));
        }
        control.evaluation_sources = Arc::new(sources);
        let destination = root.join("output");
        fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        let sentinel = if mode == "collision" {
            let path = destination.join(format!("{}.zim", recipe.output_stem()));
            let mut file = fs::OpenOptions::new().write(true).create_new(true)
                .open(&path).map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut file, b"evaluation-owned-existing-output")
                .map_err(|e| e.to_string())?;
            Some(path)
        } else { None };
        if mode == "corruption" {
            // A syntactically valid, wrong final checksum must fail after
            // writing real source bytes, never publish an apparently valid ZIM.
            recipe.md5 = if recipe.md5 == "0".repeat(32) { "1".repeat(32) } else { "0".repeat(32) };
        }
        if mode == "cancel" { control.evaluation_cancel_after = Some(1); }
        let job = MapJob {
            id: "android-evaluation".into(), status: "validating".into(), created_at: None,
            region_id: "synthetic".into(), language: "evaluation".into(),
            written: 0, total: recipe.size, incoming: 0, incoming_total: 0,
            error: None, path: None, owned_partial: None, recovery_records: Vec::new(),
            blocks_total: None, blocks_ready: None, blocks_downloaded: None, decision: None,
            resumable: false, resume: None, published: None,
            can_remove_from_list: false,
            monitored_paths: Vec::new(),
            retained_publications: Vec::new(),
            history_dismissed: false,
        };
        persist(&manager.inner.state_path, &job)?;
        {
            let mut state = manager.inner.state.lock().map_err(|_| "Evaluation state poisoned")?;
            state.job = Some(job);
            state.active = Some(Arc::new(control.clone()));
        }
        Ok(Prepared { manager, control, recipe, destination, sentinel })
    }

    impl Prepared {
        pub(crate) async fn compose(self) -> (Result<PathBuf, String>, Option<MapJob>, Vec<(String, u128)>, serde_json::Value) {
            let url = checked_url("https://android-evaluation.invalid/source").expect("constant HTTPS URL");
            if let Ok(mut phases) = self.control.evaluation_phases.lock() {
                phases.0 = std::time::Instant::now();
                phases.1.clear();
            }
            let result = self.manager.compose("android-evaluation", &self.control,
                self.recipe, url.as_str(), &url, &self.destination, Vec::new()).await;
            self.control.evaluation_mark("publication_or_failure");
            let phases = self.control.evaluation_phases.lock().map(|phases| phases.1.clone()).unwrap_or_default();
            let hash = self.control.evaluation_hash.lock().map(|metrics| serde_json::json!({
                "bytes":metrics.0, "elapsedUs":metrics.1, "threadCpuUs":metrics.2, "digestCalls":metrics.3
            })).unwrap_or(serde_json::Value::Null);
            self.manager.finish("android-evaluation", &self.control, result.clone());
            (result, self.manager.status().ok().flatten(), phases, hash)
        }
    }
}

#[cfg(test)]
#[path = "maps_tests.rs"]
mod tests;
