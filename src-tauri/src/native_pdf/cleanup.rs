// SPDX-License-Identifier: GPL-3.0-or-later
//! Scratch lifetime is independent of application authority. A late native
//! producer or response body may retain only this lease, never publication.
use std::{collections::HashSet, fs, path::{Path, PathBuf}, sync::{Arc, LazyLock, Mutex}};
pub type Report = Arc<dyn Fn(String) + Send + Sync>;
static PENDING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
struct Owner { directory: PathBuf, report: Report, _admission: Option<tokio::sync::OwnedSemaphorePermit> }
#[derive(Clone)]
pub(crate) struct Files(Arc<Owner>);
impl Files {
    pub(crate) fn new(directory: PathBuf, report: Report, admission: Option<tokio::sync::OwnedSemaphorePermit>) -> Self {
        Self(Arc::new(Owner { directory, report, _admission: admission }))
    }
    pub(crate) fn directory(&self) -> &Path { &self.0.directory }
}
impl Drop for Owner {
    fn drop(&mut self) {
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
/// Called by the application's existing orderly-shutdown/profile cleanup path.
/// Never scans arbitrary directories or removes a path it did not create.
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
}
