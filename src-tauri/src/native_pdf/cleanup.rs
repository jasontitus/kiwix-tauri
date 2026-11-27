// SPDX-License-Identifier: GPL-3.0-or-later
//! Scratch lifetime is independent of application authority. A late native
//! producer or response body may retain only this lease, never publication.
#![forbid(unsafe_code)]
use std::{collections::HashSet, fs, path::{Path, PathBuf}, sync::{Arc, LazyLock, Mutex}};
pub type Report = Arc<dyn Fn(String) + Send + Sync>;
static PENDING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
/// Scratch directories a lease in this process still owns. A lease has no upper
/// bound (a system print dialog can stay open for hours), so `sweep_stale` must
/// never judge these by age.
static LIVE: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
/// Longer than any export another running copy of the application can still be
/// working on: rendering is cut off after 16 minutes, and the macOS print panel
/// that follows a 90-second render after another 16 minutes plus 20 seconds.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(60 * 60);
struct Owner { directory: PathBuf, report: Report, _admission: Option<tokio::sync::OwnedSemaphorePermit> }
#[derive(Clone)]
pub(crate) struct Files(Arc<Owner>);
impl Files {
    pub(crate) fn new(directory: PathBuf, report: Report, admission: Option<tokio::sync::OwnedSemaphorePermit>) -> Self {
        LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(directory.clone());
        Self(Arc::new(Owner { directory, report, _admission: admission }))
    }
    pub(crate) fn directory(&self) -> &Path { &self.0.directory }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.remove();
        // Only after a failed removal has been recorded in PENDING, so the
        // directory is never unowned in between.
        LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&self.directory);
    }
}
impl Owner {
    fn remove(&self) {
        match fs::remove_dir_all(&self.directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // Windows: the per-job WebView2 browser process releases its
            // profile files asynchronously after the controller closes, so an
            // immediate removal meets a sharing violation. Retry off this
            // thread (often a UI or async worker) before reporting.
            #[cfg(windows)]
            Err(error) if retryable(&error) => {
                // Record it first: an orderly shutdown during the retry still
                // finds it in PENDING and makes (and reports) its own attempt.
                PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(self.directory.clone());
                let directory = self.directory.clone();
                let report = self.report.clone();
                let retry = std::thread::Builder::new().name("pdf-cleanup-retry".into())
                    .spawn(move || retry_until_released(directory, report));
                if retry.is_err() { fail(&self.directory, &self.report, error); }
            }
            Err(error) => fail(&self.directory, &self.report, error),
        }
    }
}
fn fail(directory: &Path, report: &Report, error: std::io::Error) {
    // Retain the exact owned path and reporting responsibility. A cleanup
    // failure is never silently reclassified as success.
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(directory.to_path_buf());
    report(format!("Could not remove private article PDF temporary files: {error}"));
}
/// Access denied / delete pending (5), sharing (32) or lock (33) violations,
/// or a directory a lingering browser helper refilled (145).
#[cfg(windows)]
fn retryable(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33 | 145))
}
/// Bounded (about 18 s) retry while WebView2 releases the per-job profile.
/// The directory is already in PENDING; success removes it from there.
#[cfg(windows)]
fn retry_until_released(directory: PathBuf, report: Report) {
    let mut delay = std::time::Duration::from_millis(100);
    let mut last = None;
    for _ in 0..10 {
        std::thread::sleep(delay);
        delay = (delay * 2).min(std::time::Duration::from_secs(3));
        match fs::remove_dir_all(&directory) {
            Ok(()) => return released(&directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return released(&directory),
            Err(error) if retryable(&error) => last = Some(error),
            Err(error) => return fail(&directory, &report, error),
        }
    }
    if let Some(error) = last { fail(&directory, &report, error); }
}
#[cfg(windows)]
fn released(directory: &Path) {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(directory);
}
/// `pdf-<uuid>`, exactly as `OwnedPdf::create` names its scratch directory.
fn scratch_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| name.strip_prefix("pdf-")
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .is_some_and(|id| name == format!("pdf-{id}")))
}
/// Remove scratch that a killed or crashed process left in the private
/// article-PDF directory: it holds the captured article and its PDF, private
/// browsing included, and nothing else would ever delete it.
///
/// Only real directories directly inside `root` named exactly `pdf-<uuid>` are
/// considered, never one a lease in this process still owns or whose removal is
/// already pending, and never one changed within `STALE_AFTER` (another running
/// copy of the application may be using it). Nothing is followed: a symlink is
/// left alone, and removal does not traverse links inside the tree. A failed
/// removal is reported and retained in PENDING like any other.
pub(crate) fn sweep_stale(root: &Path, report: &Report) {
    sweep_stale_at(root, report, std::time::SystemTime::now());
}
fn sweep_stale_at(root: &Path, report: &Report, now: std::time::SystemTime) {
    // Two exports may start together; one sweep at a time, so neither reports
    // the other's half-removed tree as a failure.
    static SWEEP: Mutex<()> = Mutex::new(());
    let _sweep = SWEEP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !fs::symlink_metadata(root).is_ok_and(|metadata| metadata.is_dir()) { return; }
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        if !scratch_name(&entry.file_name()) { continue; }
        let directory = root.join(entry.file_name());
        // `symlink_metadata` describes the entry itself, so a link is not a directory.
        let Ok(metadata) = fs::symlink_metadata(&directory) else { continue };
        let stale = metadata.is_dir() && metadata.modified().ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_AFTER);
        if !stale { continue; }
        // Leases are keyed by their unique name, however `root` was spelled.
        let owned = |set: &Mutex<HashSet<PathBuf>>| set.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter().any(|path| path.file_name() == Some(entry.file_name().as_os_str()));
        if owned(&LIVE) || owned(&PENDING) { continue; }
        match fs::remove_dir_all(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => fail(&directory, report, error),
        }
    }
}
/// Called by the application's existing orderly-shutdown/profile cleanup path.
/// Removes only paths recorded in PENDING; `sweep_stale` is the one scan.
#[cfg(any(desktop, target_os = "ios", test))]
pub fn retry_cleanup() -> Result<(), String> {
    let pending = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    let mut failures = Vec::new();
    for directory in pending {
        if let Err(error) = fs::remove_dir_all(&directory) {
            if error.kind() == std::io::ErrorKind::NotFound { continue; }
            failures.push(format!("Could not remove private article PDF temporary files: {error}"));
            PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(directory);
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_native_admission_survives_waiter_until_last_file_lease() {
        let directory = std::env::temp_dir().join(format!("kiwix-pdf-lease-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = admission.clone().try_acquire_owned().unwrap();
        let waiter = Files::new(directory.clone(), Arc::new(|error| panic!("{error}")), Some(permit));
        let native_producer = waiter.clone();
        drop(waiter);
        assert!(admission.try_acquire().is_err());
        drop(native_producer);
        assert!(admission.try_acquire().is_ok());
        assert!(!directory.exists());
    }

    #[test]
    fn stale_sweep_removes_only_abandoned_pdf_scratch() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let report: Report = Arc::new(|error| panic!("{error}"));
        let scratch = |name: String| {
            let directory = root.join(name);
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("live-document"), b"<p>private article</p>").unwrap();
            fs::create_dir(directory.join("profile")).unwrap();
            fs::write(directory.join("profile").join("article.pdf"), b"%PDF-").unwrap();
            directory
        };
        let abandoned = scratch(format!("pdf-{}", uuid::Uuid::new_v4()));
        let leased = scratch(format!("pdf-{}", uuid::Uuid::new_v4()));
        let lease = Files::new(leased.clone(), report.clone(), None);
        // Names this module never creates, however similar.
        let unrelated = [
            scratch("documents".into()),
            scratch(format!("pdf-{}", uuid::Uuid::new_v4().simple())),
            scratch(format!("pdf-{}", uuid::Uuid::new_v4()).to_uppercase()),
            scratch(format!("pdf-{}.bak", uuid::Uuid::new_v4())),
        ];
        let file = root.join(format!("pdf-{}", uuid::Uuid::new_v4()));
        fs::write(&file, b"a file, not scratch").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep"), b"outside the scratch root").unwrap();
        #[cfg(unix)]
        let link = {
            let link = root.join(format!("pdf-{}", uuid::Uuid::new_v4()));
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            std::os::unix::fs::symlink(outside.path(), abandoned.join("escape")).unwrap();
            link
        };
        let survivors = |context: &str| {
            assert!(leased.join("live-document").exists(), "{context}: live lease");
            for directory in &unrelated { assert!(directory.join("live-document").exists(), "{context}: {directory:?}"); }
            assert!(file.exists(), "{context}: plain file");
            assert_eq!(fs::read(outside.path().join("keep")).unwrap(), b"outside the scratch root", "{context}");
            #[cfg(unix)]
            assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "{context}: symlink");
        };
        let now = std::time::SystemTime::now();
        // Another running copy may still be exporting into a recent directory.
        sweep_stale_at(&root, &report, now + STALE_AFTER - std::time::Duration::from_secs(60));
        assert!(abandoned.join("live-document").exists());
        survivors("recent");
        sweep_stale_at(&root, &report, now + STALE_AFTER + std::time::Duration::from_secs(60));
        assert!(!abandoned.exists(), "abandoned scratch and the article in it are removed");
        survivors("stale");
        drop(lease);
        assert!(!leased.exists());
    }
}
