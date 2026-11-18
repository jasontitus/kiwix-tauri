// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Local byte/structure verification, not publisher authentication. One retained
//! worker survives view navigation; no queue, network, or diagnostic log writes.
use crate::{library_files, store::{Book, Store}};
#[cfg(test)]
use crate::archive::Archive;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::{Arc, Mutex}, time::UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified_ns: String,
    changed: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentIdentity(Vec<FileIdentity>);
impl ContentIdentity {
    pub(crate) fn from_files(files: &[(String, std::fs::File)]) -> Result<Self, String> {
        Self::from_handles(files.iter().map(|(_, file)| file))
    }

    pub(crate) fn from_handles<'a>(
        files: impl ExactSizeIterator<Item = &'a std::fs::File>,
    ) -> Result<Self, String> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let mut parts = Vec::with_capacity(files.len());
        for file in files {
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            #[cfg(unix)]
            let (device, inode) = (metadata.dev(), metadata.ino());
            #[cfg(not(unix))]
            let (device, inode) = {
                use cap_fs_ext::MetadataExt;
                let capability = cap_std::fs::Metadata::from_file(file).map_err(|e| e.to_string())?;
                (capability.dev(), capability.ino())
            };
            let modified = metadata.modified().map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
            #[cfg(unix)]
            let changed = {
                format!("{}:{}", metadata.ctime(), metadata.ctime_nsec())
            };
            #[cfg(not(unix))]
            let changed = metadata.created().map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos().to_string();
            parts.push(FileIdentity { device, inode,
                size: metadata.len(), modified_ns: modified.to_string(), changed });
        }
        Ok(Self(parts))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntegrityStatus { Valid, Invalid }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrityRecord {
    pub status: IntegrityStatus,
    pub checked_at: String,
    pub installation_id: String,
    pub identity: ContentIdentity,
}
impl IntegrityRecord {
    pub(crate) fn matches(&self, book: &Book) -> bool {
        self.installation_id == book.installation_id
            && library_files::archive_content_identity(book).is_ok_and(|identity| identity == self.identity)
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookResult {
    pub book_id: String,
    pub installation_id: String,
    pub status: String,
    pub checked_at: Option<String>,
    pub persisted: bool,
    #[serde(skip)]
    authority: Option<Arc<IntegrityRecord>>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookTarget {
    pub book_id: String,
    pub installation_id: String,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub running: bool,
    pub phase: String,
    pub book_id: Option<String>,
    pub completed: usize,
    pub total: usize,
    pub bytes_checked: u64,
    pub total_bytes: u64,
    pub targets: Vec<BookTarget>,
    pub results: Vec<BookResult>,
}
struct Worker {
    cancel: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}
#[derive(Default)]
pub struct IntegrityManager {
    operation: tokio::sync::Mutex<Option<Worker>>,
    job: Arc<Mutex<Job>>,
}
impl IntegrityManager {
    /// None means all currently available archives, processed sequentially.
    pub async fn start(&self, store: Arc<Store>, target: Option<(String, String)>) -> Result<Job, String> {
        let mut operation = self.operation.try_lock()
            .map_err(|_| "An archive integrity check is already running")?;
        if operation.as_ref().is_some_and(|worker| !worker.handle.is_finished()) {
            return Err("An archive integrity check is already running".into());
        }
        if let Some(worker) = operation.take() {
            worker.handle.await.map_err(|_| "Integrity worker unavailable")?;
        }
        let state = store.read()?;
        let books: Vec<_> = if let Some((id, installation_id)) = target {
            let book = state.books.into_iter().find(|book| book.metadata.id == id)
                .ok_or("Archive not in library")?;
            if book.installation_id != installation_id {
                return Err("Archive installation changed".into());
            }
            vec![book]
        } else {
            state.books.into_iter().filter(|book| book.available).collect()
        };
        let persistent = !state.settings.private_mode && !state.settings.read_only;
        let initial = Job { running: true, phase: "opening".into(), total: books.len(),
            targets: books.iter().map(|book| BookTarget { book_id: book.metadata.id.clone(),
                installation_id: book.installation_id.clone() }).collect(), ..Job::default() };
        *self.job.lock().map_err(|_| "Integrity state unavailable")? = initial.clone();
        let job = self.job.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let handle = tokio::task::spawn_blocking(move || {
            for book in books {
                if token.is_cancelled() { break; }
                if let Ok(mut job) = job.lock() {
                    job.book_id = Some(book.metadata.id.clone());
                    job.phase = "opening".into();
                    job.bytes_checked = 0;
                    job.total_bytes = 0;
                }
                let result = verify(&store, &book, persistent, &token, &job);
                if token.is_cancelled() { break; }
                let result = result.unwrap_or_else(|_| BookResult {
                    book_id: book.metadata.id.clone(), status: "unavailable".into(),
                    installation_id: book.installation_id.clone(),
                    checked_at: None, persisted: false, authority: None,
                });
                if let Ok(mut job) = job.lock() {
                    job.completed += 1;
                    job.results.push(result);
                }
            }
            if let Ok(mut job) = job.lock() {
                job.running = false;
                job.phase = if token.is_cancelled() { "cancelled" } else { "complete" }.into();
                job.book_id = None;
            }
        });
        *operation = Some(Worker { cancel, handle });
        Ok(initial)
    }
    pub fn status(&self, store: &Store) -> Result<Job, String> {
        let mut snapshot = self.job.lock().map_err(|_| "Integrity state unavailable")?.clone();
        let state = store.read()?;
        // Installation ids are unique per installed archive; index them once
        // instead of scanning the library for every result on every poll.
        let books: HashMap<&str, &Book> = state
            .books
            .iter()
            .map(|book| (book.installation_id.as_str(), book))
            .collect();
        for result in &mut snapshot.results {
            if let Some(record) = &result.authority {
                if !books
                    .get(record.installation_id.as_str())
                    .is_some_and(|book| record.matches(book))
                {
                    result.status = "unknown".into();
                    result.checked_at = None;
                    result.persisted = false;
                }
            }
        }
        Ok(snapshot)
    }
    /// Whether a running check still includes this book. Windows deletion is
    /// refused meanwhile, since the check holds the archive open.
    /// A book already checked in this run is no longer held, so only a
    /// pending or current target counts.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn running_for(&self, book_id: &str) -> Result<bool, String> {
        let job = self.job.lock().map_err(|_| "Integrity state unavailable")?;
        Ok(job.running
            && job.targets.iter().any(|target| target.book_id == book_id)
            && !job.results.iter().any(|result| result.book_id == book_id))
    }
    /// Whether any check is running (Windows library moves refuse meanwhile).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn running(&self) -> Result<bool, String> {
        Ok(self.job.lock().map_err(|_| "Integrity state unavailable")?.running)
    }
    /// Signal before waiting for the policy writer; join under that writer to
    /// catch any admission that held an earlier policy read lease.
    pub async fn retire(&self) {
        if let Some(worker) = self.operation.lock().await.as_ref() { worker.cancel.cancel(); }
    }
    /// Called under the service's policy writer. Joining precedes any policy
    /// write; clearing also prevents volatile private results escaping cutover.
    pub async fn cancel_and_wait(&self, clear: bool) -> Result<(), String> {
        let mut operation = self.operation.lock().await;
        if let Some(worker) = operation.take() {
            worker.cancel.cancel();
            worker.handle.await.map_err(|_| "Integrity worker unavailable")?;
        }
        if clear { *self.job.lock().map_err(|_| "Integrity state unavailable")? = Job::default(); }
        Ok(())
    }
}
fn verify(store: &Store, book: &Book, persistent: bool, cancel: &CancellationToken,
    job: &Mutex<Job>) -> Result<BookResult, String> {
    let pinned = crate::archive_source::retain(book)?;
    let identity = pinned.identity()?;
    if cancel.is_cancelled() { return Err("Integrity check cancelled".into()); }
    let archive = pinned.open()?;
    if archive.metadata_cancellable(cancel)?.id != book.metadata.id { return Err("Archive installation changed".into()); }
    let valid = archive.check_integrity_cancellable(cancel, |checked, total| {
        if let Ok(mut job) = job.lock() {
            job.phase = if checked < total { "checksum" } else { "structure" }.into();
            job.bytes_checked = checked.min(total);
            job.total_bytes = total;
        }
    })?;
    if cancel.is_cancelled() { return Err("Integrity check cancelled".into()); }
    if identity != pinned.identity()?
        || identity != library_files::archive_content_identity(book)? {
        return Err("Archive changed during integrity check".into());
    }
    let record = IntegrityRecord { status: if valid { IntegrityStatus::Valid } else { IntegrityStatus::Invalid },
        checked_at: crate::store::now(), installation_id: book.installation_id.clone(), identity };
    let persisted = if persistent { store.commit_integrity(book, &record, cancel)? } else { false };
    // Never publish a result for a relinked installation, even in volatile mode.
    let current = store.book(&book.metadata.id)?.ok_or("Archive no longer in library")?;
    if current.path != book.path || current.installation_id != book.installation_id || !record.matches(&current) {
        return Err("Archive installation changed".into());
    }
    // Only a result that survives the installation/identity check is published,
    // so a later reader crash cannot inherit a stale archive's outcome.
    crate::crash::note_archive_validation(
        &book.metadata.id,
        &book.installation_id,
        if valid { "valid" } else { "invalid" },
    );
    Ok(BookResult { book_id: book.metadata.id.clone(),
        installation_id: book.installation_id.clone(),
        status: if valid { "valid" } else { "invalid" }.into(), checked_at: Some(record.checked_at.clone()),
        persisted, authority: Some(Arc::new(record)) })
}
/// Cheap identity guards, never a new checksum claim. No filenames, UUIDs,
/// installation identifiers, titles, or article activity leave this summary.
pub fn summary(store: &Store) -> Result<serde_json::Value, String> {
    let state = store.read()?;
    let mut valid = 0;
    let mut invalid = 0;
    let archives: Vec<_> = state.books.iter().enumerate().map(|(index, book)| {
        let record = book.integrity.as_ref().filter(|record| record.matches(book));
        match record.map(|record| record.status) {
            Some(IntegrityStatus::Valid) => valid += 1,
            Some(IntegrityStatus::Invalid) => invalid += 1,
            None => {}
        }
        serde_json::json!({"archive":index+1,
            "status":match record.map(|record| record.status) {
                Some(IntegrityStatus::Valid) => "valid", Some(IntegrityStatus::Invalid) => "invalid", None => "unknown"
            },
            "checkedAt":record.and_then(|record| crate::timestamps::normalized(&record.checked_at)),
            "persisted":record.is_some()})
    }).collect();
    Ok(serde_json::json!({"valid":valid,"invalid":invalid,"unknown":state.books.len()-valid-invalid,
        "archiveNumbering":"library-order","archives":archives,
        "methods":["retained-byte-md5","archive-structure"],"publisherAuthenticity":false}))
}
pub fn book_statuses(store: &Store) -> Result<Vec<BookResult>, String> {
    Ok(store.read()?.books.iter().map(|book| {
        let record = book.integrity.as_ref().filter(|record| record.matches(book));
        BookResult { book_id: book.metadata.id.clone(),
            installation_id: book.installation_id.clone(),
            status: match record.map(|record| record.status) {
                Some(IntegrityStatus::Valid) => "valid", Some(IntegrityStatus::Invalid) => "invalid", None => "unknown"
            }.into(), checked_at: record.map(|record| record.checked_at.clone()), persisted: record.is_some(),
            authority: None }
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kiwix-integrity-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn store(&self) -> Arc<Store> { Arc::new(Store::new(self.0.join("profile")).unwrap()) }
        fn book(&self) -> Book {
            let path = self.0.join("book.zim");
            fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &path).unwrap();
            crate::imports::load_book(path.to_str().unwrap(), false, false).unwrap()
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn install(store: &Store, book: &Book) {
        store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    }
    fn check(store: &Store, book: &Book) -> BookResult {
        verify(store, book, true, &CancellationToken::new(), &Mutex::new(Job::default())).unwrap()
    }
    async fn finish(manager: &IntegrityManager) {
        let worker = manager.operation.lock().await.take().unwrap();
        worker.handle.await.unwrap();
    }
    #[test]
    fn real_valid_and_checksum_corrupt_results_survive_restart() {
        for corrupt in [false, true] {
            let profile = Profile::new();
            let store = profile.store();
            let book = profile.book();
            install(&store, &book);
            if corrupt {
                let mut bytes = fs::read(&book.path).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                fs::write(&book.path, bytes).unwrap();
            }
            let result = check(&store, &book);
            assert_eq!(result.status, if corrupt { "invalid" } else { "valid" });
            assert!(result.persisted);
            drop(store);
            let restarted = profile.store();
            let status = book_statuses(&restarted).unwrap();
            assert_eq!(status[0].status, result.status);
            assert_eq!(status[0].checked_at, result.checked_at);
            assert!(!result.checked_at.unwrap().is_empty());
        }
    }
    #[test]
    fn persisted_report_never_exports_untrusted_timestamp_text() {
        let profile = Profile::new();
        let store = profile.store();
        let book = profile.book();
        install(&store, &book);
        check(&store, &book);
        store.update(|state| {
            state.books[0].integrity.as_mut().unwrap().checked_at = "/private/path?token=secret".into();
            Ok(())
        }).unwrap();
        let report = summary(&store).unwrap();
        assert_eq!(report["archives"][0]["status"], "valid");
        assert!(report["archives"][0]["checkedAt"].is_null());
        assert!(!report.to_string().contains("secret"));
        assert!(!report.to_string().contains(&book.metadata.id));
        assert!(!report.to_string().contains(&book.installation_id));
    }
    #[test]
    fn replacement_and_relink_cannot_inherit_or_commit_verified_status() {
        let profile = Profile::new();
        let store = profile.store();
        let book = profile.book();
        install(&store, &book);
        check(&store, &book);
        let record = store.book(&book.metadata.id).unwrap().unwrap().integrity.unwrap();
        let replacement = profile.0.join("replacement.zim");
        fs::copy(&book.path, &replacement).unwrap();
        fs::rename(&replacement, &book.path).unwrap();
        assert_eq!(book_statuses(&store).unwrap()[0].status, "unknown");
        assert!(store.commit_integrity(&book, &record, &CancellationToken::new()).is_err());
        check(&store, &book);
        let current_record = store.book(&book.metadata.id).unwrap().unwrap().integrity.unwrap();
        store.update(|state| {
            state.books[0].installation_id = crate::store::new_installation_id();
            Ok(())
        }).unwrap();
        assert_eq!(book_statuses(&store).unwrap()[0].status, "unknown");
        assert!(store.commit_integrity(&book, &current_record, &CancellationToken::new()).is_err());
    }
    #[tokio::test]
    async fn protected_checks_are_real_volatile_and_policy_clear_retires_results() {
        for private in [false, true] {
            let profile = Profile::new();
            let store = profile.store();
            let book = profile.book();
            install(&store, &book);
            let state_path = store.data_dir().join("state.json");
            let original = fs::read(&state_path).unwrap();
            let modified = fs::metadata(&state_path).unwrap().modified().unwrap();
            store.set_runtime_mode(private, !private).unwrap();
            let manager = IntegrityManager::default();
            manager.start(store.clone(), Some((book.metadata.id.clone(), book.installation_id.clone()))).await.unwrap();
            finish(&manager).await;
            let result = manager.status(&store).unwrap().results.remove(0);
            assert_eq!(result.status, "valid");
            assert!(!result.persisted);
            assert!(store.book(&book.metadata.id).unwrap().unwrap().integrity.is_none());
            assert_eq!(fs::read(&state_path).unwrap(), original);
            assert_eq!(fs::metadata(&state_path).unwrap().modified().unwrap(), modified);
            assert!(!store.data_dir().join("diagnostics.jsonl").exists());
            manager.cancel_and_wait(true).await.unwrap();
            assert!(manager.status(&store).unwrap().results.is_empty());
        }
    }
    #[test]
    fn cancellation_and_unavailable_io_never_record_corruption() {
        let profile = Profile::new();
        let store = profile.store();
        let book = profile.book();
        install(&store, &book);
        let original = fs::read(store.data_dir().join("state.json")).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(verify(&store, &book, true, &cancel, &Mutex::new(Job::default())).is_err());
        fs::remove_file(&book.path).unwrap();
        assert!(verify(&store, &book, true, &CancellationToken::new(), &Mutex::new(Job::default())).is_err());
        assert!(store.book(&book.metadata.id).unwrap().unwrap().integrity.is_none());
        assert_eq!(fs::read(store.data_dir().join("state.json")).unwrap(), original);
    }
    #[tokio::test]
    async fn diagnostic_run_reports_actual_outcomes_and_no_paths() {
        let profile = Profile::new();
        let store = profile.store();
        let book = profile.book();
        install(&store, &book);
        let second = profile.0.join("second.zim");
        let mut bytes = fs::read(&book.path).unwrap();
        bytes[8..24].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        let checksum = u64::from_le_bytes(bytes[72..80].try_into().unwrap()) as usize;

        let digest = md5::compute(&bytes[..checksum]).0;
        bytes[checksum..checksum + 16].copy_from_slice(&digest);
        fs::write(&second, &bytes).unwrap();
        let second_book = crate::imports::load_book(second.to_str().unwrap(), false, false).unwrap();
        install(&store, &second_book);
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&second, bytes).unwrap();
        let manager = IntegrityManager::default();
        manager.start(store.clone(), None).await.unwrap();
        finish(&manager).await;
        let job = manager.status(&store).unwrap();
        assert!(!job.running);
        assert_eq!((job.completed, job.total), (2, 2));
        assert_eq!(job.results[0].status, "valid");
        assert_eq!(job.results[1].status, "invalid");
        let report = crate::diagnostics::report(&store).unwrap();
        assert_eq!(report["integrity"]["valid"], 1);
        assert_eq!(report["integrity"]["publisherAuthenticity"], false);
        let run = crate::diagnostics::integrity_run(&job);
        assert_eq!(run["outcomes"]["valid"], 1);
        assert_eq!(run["outcomes"]["invalid"], 1);
        assert_eq!(run["archives"][0]["status"], "valid");
        assert_eq!(run["archives"][1]["status"], "invalid");
        assert_eq!(run["archives"][0]["archive"], 1);
        assert!(run["archives"][0]["checkedAt"].is_string());
        assert_eq!(run["archives"][0]["persisted"], true);
        assert_eq!(report["integrity"]["archives"][0]["status"], "valid");
        assert!(!run.to_string().contains(&book.installation_id));
        assert!(!report.to_string().contains(&book.metadata.id));
        assert!(!run.to_string().contains(&book.path));
        assert!(!run.to_string().contains(&book.metadata.id));
        assert!(!report.to_string().contains(&book.path));
        let mut changed = fs::read(&book.path).unwrap();
        *changed.last_mut().unwrap() ^= 1;
        fs::write(&book.path, changed).unwrap();
        assert_eq!(manager.status(&store).unwrap().results[0].status, "unknown");
        assert_eq!(book_statuses(&store).unwrap()[0].status, "unknown");
    }
    #[tokio::test]
    async fn manual_admission_rejects_relinked_installation_before_start() {
        let profile = Profile::new();
        let store = profile.store();
        let book = profile.book();
        install(&store, &book);
        let manager = IntegrityManager::default();
        assert!(manager.start(store.clone(), Some((book.metadata.id.clone(), "stale-installation".into()))).await.is_err());
        assert!(manager.status(&store).unwrap().targets.is_empty());
        let initial = manager.start(store.clone(), Some((book.metadata.id.clone(), book.installation_id.clone()))).await.unwrap();
        assert_eq!(initial.targets.len(), 1);
        assert_eq!(initial.targets[0].installation_id, book.installation_id);
        finish(&manager).await;
    }
    #[tokio::test]
    async fn concurrent_start_is_rejected_instead_of_queued() {
        let profile = Profile::new();
        let store = profile.store();
        let manager = IntegrityManager::default();
        let admission = manager.operation.lock().await;
        assert!(manager.start(store, None).await.is_err());
        drop(admission);
    }
    #[test]
    fn cancellation_during_real_retained_checksum_is_not_invalid() {
        let profile = Profile::new();
        let book = profile.book();
        let pinned = library_files::retain_archive_files(&book).unwrap();
        let archive = Archive::open_files(&pinned.path, &pinned.files).unwrap();
        let cancel = CancellationToken::new();
        let mut previous = 0;
        let result = archive.check_integrity_cancellable(&cancel, |checked, total| {
            assert!(checked >= previous && checked <= total);
            previous = checked;
            if checked > 0 { cancel.cancel(); }
        });
        assert_eq!(result.unwrap_err(), "Integrity check cancelled");
        assert!(archive.check_integrity().unwrap());
    }
    #[tokio::test]
    async fn service_policy_cutover_clears_volatile_authority_without_check_writes() {
        use serde_json::json;
        for mode in ["privateMode", "readOnly"] {
            let profile = Profile::new();
            let book = profile.book();
            let service = crate::service::Service::new(profile.0.join("profile"), false, false).unwrap();
            install(&service.store, &book);
            service.clone().dispatch("save_settings", json!({
                "settings":{(mode):true},"privacyGeneration":1
            })).await.unwrap();
            let state_path = service.store.data_dir().join("state.json");
            let original = fs::read(&state_path).unwrap();
            let modified = fs::metadata(&state_path).unwrap().modified().unwrap();
            service.clone().dispatch("check_integrity", json!({
                "id":book.metadata.id,"installationId":book.installation_id,"privacyGeneration":2
            })).await.unwrap();
            let status = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let status = service.clone().dispatch("integrity_status", json!({"privacyGeneration":2})).await.unwrap();
                    if status["job"]["running"] == false { break status; }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            }).await.unwrap();
            assert_eq!(status["job"]["results"][0]["status"], "valid");
            assert_eq!(status["job"]["results"][0]["persisted"], false);
            assert_eq!(fs::read(&state_path).unwrap(), original);
            assert_eq!(fs::metadata(&state_path).unwrap().modified().unwrap(), modified);
            service.clone().dispatch("save_settings", json!({
                "settings":{(mode):false},"privacyGeneration":2
            })).await.unwrap();
            let status = service.clone().dispatch("integrity_status", json!({"privacyGeneration":3})).await.unwrap();
            assert_eq!(status["job"]["results"], json!([]));
            assert_eq!(status["books"][0]["status"], "unknown");
            assert!(service.clone().dispatch("check_integrity", json!({
                "id":book.metadata.id,"installationId":book.installation_id,"privacyGeneration":2
            })).await.is_err());
            service.shutdown().await.unwrap();
        }
    }
}
