// SPDX-License-Identifier: GPL-3.0-or-later
//! Synchronous retained-file creation/publication hooks. iOS copies the actual
//! NSURL backup-key backing attribute onto the caller's descriptor, never its path.
//! Android backup eligibility is fixed by manifest/XML, never a runtime toggle.
use std::path::Path;

#[cfg(target_os = "ios")]
static INCLUDE_ARCHIVES: std::sync::RwLock<bool> = std::sync::RwLock::new(false);

/// Set before worker admission and synchronously with settings commit. The write lock
/// waits for any native attribute operation, so an old grant cannot land after revocation.
// The retained-file hook module is shared with desktop; settings call this only
// from the native mobile initialization/commit boundary.
#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) fn set_archive_backup(include: bool) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    { *INCLUDE_ARCHIVES.write().map_err(|_| "Archive backup policy unavailable")? = include; }
    #[cfg(target_os = "android")]
    if include { return Err("Android archive backup is excluded by static platform rules".into()); }
    #[cfg(not(target_os = "ios"))]
    let _ = include;
    Ok(())
}

/// Borrowed descriptors must remain open, with their existing capability/mutation
/// lease held, until the synchronous hook returns. No hook opens a replacement file.
#[cfg(target_os = "ios")]
pub(crate) trait BackupFile: std::os::fd::AsRawFd {}
#[cfg(target_os = "ios")]
impl<T: std::os::fd::AsRawFd + ?Sized> BackupFile for T {}
#[cfg(not(target_os = "ios"))]
pub(crate) trait BackupFile {}
#[cfg(not(target_os = "ios"))]
impl<T: ?Sized> BackupFile for T {}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    safe fn kiwix_mobile_archive_backup(
        fd: i32, path: crate::ffi::CPath<'_>, include: i32, admit: i32,
    ) -> i32;
    safe fn kiwix_mobile_archive_admitted(fd: i32, path: crate::ffi::CPath<'_>) -> i32;
    safe fn kiwix_mobile_exclude_unadmitted_payload(fd: i32, path: crate::ffi::CPath<'_>) -> i32;
}

/// Invoke after exclusive owned-file creation, BEFORE writing or allocating payload bytes.
/// The path is only an app-owned capability/identity check; mutation uses `file` alone.
pub(crate) fn exclude_payload(file: &(impl BackupFile + ?Sized), path: &Path) -> Result<(), String> {
    apply(file, path, false, false)
}

/// Reconciliation may race publication under a privacy read lease. Recheck
/// admission inside the SAME native attribute-write transaction as publishers;
/// a stale negative scan must not revoke the publisher's newer inclusion.
#[cfg(target_os = "ios")]
pub(crate) fn exclude_unadmitted_payload(file: &(impl BackupFile + ?Sized), path: &Path) -> Result<(), String> {
    let path = native_path(path)?;
    // SAFETY: the caller keeps this descriptor open through the synchronous
    // Swift call; `file` may be borrowed, so this code does not claim ownership.
    // `path` is a live NUL-terminated C string read synchronously.
    if kiwix_mobile_exclude_unadmitted_payload(file.as_raw_fd(), crate::ffi::CPath::new(&path)) != 0 {
        return Err("iOS could not conditionally exclude the owned payload".into());
    }
    Ok(())
}

/// Invoke only on a verified, published archive leaf (or a retained Store-admitted
/// archive). Records descriptor-bound durable admission before applying current policy.
/// Never pass provider originals, directories, preferences, credentials or partials.
pub(crate) fn archive_policy(file: &(impl BackupFile + ?Sized), path: &Path) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        let include = INCLUDE_ARCHIVES.read().map_err(|_| "Archive backup policy unavailable")?;
        apply(file, path, *include, true)
    }
    #[cfg(not(target_os = "ios"))]
    { let _ = (file, path); Ok(()) }
}

/// Admission is inode-bound publication evidence, not a filename/staging heuristic.
/// It survives history deletion and same-filesystem archive moves.
#[cfg(target_os = "ios")]
pub(crate) fn admitted(file: &(impl BackupFile + ?Sized), path: &Path) -> Result<bool, String> {
    let path = native_path(path)?;
    // SAFETY: the caller keeps this descriptor open through the synchronous
    // Swift call; `file` may be borrowed, so this code does not claim ownership.
    // `path` is a live NUL-terminated C string read synchronously.
    match kiwix_mobile_archive_admitted(file.as_raw_fd(), crate::ffi::CPath::new(&path)) {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("iOS could not inspect owned archive admission".into()),
    }
}

#[cfg(target_os = "ios")]
fn native_path(path: &Path) -> Result<std::ffi::CString, String> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "Archive backup path contains NUL".into())
}

fn apply(
    file: &(impl BackupFile + ?Sized), path: &Path, include: bool, admit: bool,
) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        let path = native_path(path)?;
        // SAFETY: the caller keeps this descriptor open through the synchronous
        // Swift call; `file` may be borrowed, so this code does not claim ownership.
        // `path` is a live NUL-terminated C string read synchronously.
        if kiwix_mobile_archive_backup(file.as_raw_fd(), crate::ffi::CPath::new(&path), i32::from(include), i32::from(admit)) != 0 {
            return Err("iOS could not apply the owned archive backup policy".into());
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = (file, path, include, admit);
    Ok(())
}
