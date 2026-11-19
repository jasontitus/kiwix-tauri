// SPDX-License-Identifier: GPL-3.0-or-later
//! Advisory whole-file locks and volume space queries.
//!
//! This replaces the `fs2` crate, which has had no release since 2018. It keeps
//! the platform lock semantics and contention error codes that callers expect:
//! `flock(2)` on Unix (including Android and iOS), `LockFileEx` over the whole
//! file on Windows, and `statvfs(3)` for space on Unix.
//!
//! `std::fs::File::try_lock` (Rust 1.89) is not a substitute yet: through at
//! least Rust 1.97 it returns `Unsupported` on Android, where these locks guard
//! profile, download and archive ownership.

#![cfg_attr(unix, forbid(unsafe_code))]

use std::fs::File;
use std::io;
use std::path::Path;

/// Create missing profile or transfer directories with owner-only Unix access.
/// Existing directories keep their permissions, including user-selected destinations.
/// Callers retain responsibility for validating ownership and mutation authority.
pub(crate) fn create_private_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// This process's effective user ID: the principal every ownership check in
/// this crate compares a file's owner against.
///
/// `geteuid(2)` has no pointer, lifetime or ownership precondition, so rustix
/// exposes it as a safe function; this keeps the raw `libc` call out of the
/// callers that gate profile, identity and relocation directories.
#[cfg(unix)]
pub(crate) fn effective_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Non-blocking advisory locks on an open file. The lock is released when the
/// last handle to the open file description is closed.
///
/// The names deliberately differ from std's inherent `File::try_lock_shared`
/// and `File::try_lock`: an inherent method wins over a trait method in method
/// syntax, and std's versions are `Unsupported` on Android.
pub trait FileExt {
    /// Takes an exclusive lock or fails immediately with the OS contention
    /// error (`EWOULDBLOCK` on Unix, `ERROR_LOCK_VIOLATION` on Windows).
    fn try_flock_exclusive(&self) -> io::Result<()>;
    /// Takes a shared lock or fails immediately with the OS contention error.
    fn try_flock_shared(&self) -> io::Result<()>;
}

/// Whether a `try_flock_*` failure means another holder owns the lock, as
/// opposed to a real I/O or filesystem failure. Unix reports `EWOULDBLOCK`
/// (`ErrorKind::WouldBlock`); Windows `LockFileEx` reports
/// `ERROR_LOCK_VIOLATION` (33), which std does not map to `WouldBlock`.
/// Same availability as its only caller, `store::lock_failure`.
#[cfg(any(not(mobile), test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
pub fn is_lock_contention(error: &io::Error) -> bool {
    #[cfg(windows)]
    if error.raw_os_error() == Some(33) {
        return true;
    }
    error.kind() == io::ErrorKind::WouldBlock
}

#[cfg(unix)]
impl FileExt for File {
    fn try_flock_exclusive(&self) -> io::Result<()> {
        rustix::fs::flock(self, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(Into::into)
    }

    fn try_flock_shared(&self) -> io::Result<()> {
        rustix::fs::flock(self, rustix::fs::FlockOperation::NonBlockingLockShared)
            .map_err(Into::into)
    }
}

#[cfg(windows)]
impl FileExt for File {
    fn try_flock_exclusive(&self) -> io::Result<()> {
        self.try_lock().map_err(windows_lock_error)
    }

    fn try_flock_shared(&self) -> io::Result<()> {
        self.try_lock_shared().map_err(windows_lock_error)
    }
}

#[cfg(windows)]
fn windows_lock_error(error: std::fs::TryLockError) -> io::Error {
    match error {
        std::fs::TryLockError::WouldBlock => io::Error::from_raw_os_error(33),
        std::fs::TryLockError::Error(error) => error,
    }
}

/// Bytes available to unprivileged callers on the volume holding `path`.
pub fn available_space(path: impl AsRef<Path>) -> io::Result<u64> {
    Ok(volume_space(path.as_ref())?.available)
}

/// Total size of the volume holding `path`.
pub fn total_space(path: impl AsRef<Path>) -> io::Result<u64> {
    Ok(volume_space(path.as_ref())?.total)
}

struct VolumeSpace {
    available: u64,
    total: u64,
}

#[cfg(unix)]
fn volume_space(path: &Path) -> io::Result<VolumeSpace> {
    use std::os::unix::ffi::OsStrExt;
    if path.as_os_str().as_bytes().contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"));
    }
    let stat = rustix::fs::statvfs(path)?;
    Ok(VolumeSpace {
        available: stat.f_frsize.saturating_mul(stat.f_bavail),
        total: stat.f_frsize.saturating_mul(stat.f_blocks),
    })
}

#[cfg(windows)]
fn volume_space(path: &Path) -> io::Result<VolumeSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumePathNameW};

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"));
    }
    wide.push(0);
    // As fs2 did, query the volume root: it accepts file paths and yields UNC
    // share roots with the trailing separator GetDiskFreeSpaceExW requires. The
    // root is at most the input plus that separator (`wide` already counts the
    // NUL); fs2 used a fixed 261.
    let mut root = vec![0u16; (wide.len() + 1).max(261)];
    // SAFETY: `wide` is NUL-terminated and `root` is a writable buffer whose
    // length the binding passes to the call; both outlive it.
    unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut root) }.map_err(win32_error)?;
    let (mut available, mut total) = (0u64, 0u64);
    // SAFETY: `root` was NUL-terminated by the call above and outlives this
    // call; the out pointers refer to live locals.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut available), Some(&mut total), None) }
        .map_err(win32_error)?;
    Ok(VolumeSpace { available, total })
}

/// Recovers the raw Win32 code so callers see the same `raw_os_error()` that
/// fs2 reported, rather than the wrapped HRESULT.
#[cfg(windows)]
pub(crate) fn win32_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        io::Error::from_raw_os_error(code as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_creation_preserves_existing_folder_modes() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("kiwix-private-dir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(root.as_path(), std::fs::Permissions::from_mode(0o775)).unwrap();
        let nested = root.as_path().join("download").join("state");
        create_private_dir_all(&nested).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&nested), 0o700);
        assert_eq!(mode(nested.parent().unwrap()), 0o700);
        assert_eq!(mode(root.as_path()), 0o775);
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir_all(&nested).unwrap();
        assert_eq!(mode(&nested), 0o755);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn exclusive_lock_excludes_other_open_files_until_dropped() {
        let dir = std::env::temp_dir().join(format!("kiwix-fs-ext-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lock");
        let open = || std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path).unwrap();

        let holder = open();
        holder.try_flock_exclusive().unwrap();
        let contender = open();
        let busy = contender.try_flock_exclusive().unwrap_err();
        #[cfg(unix)]
        assert_eq!(busy.kind(), io::ErrorKind::WouldBlock);
        #[cfg(windows)]
        assert_eq!(busy.raw_os_error(), Some(33)); // ERROR_LOCK_VIOLATION
        assert!(is_lock_contention(&busy), "{busy}");
        assert!(!is_lock_contention(&io::Error::from_raw_os_error(5)));
        assert!(FileExt::try_flock_shared(&contender).is_err());
        drop(holder);
        contender.try_flock_exclusive().unwrap();
        drop(contender);

        let first = open();
        let second = open();
        FileExt::try_flock_shared(&first).unwrap();
        FileExt::try_flock_shared(&second).unwrap();
        assert!(open().try_flock_exclusive().is_err());
        drop((first, second));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn volume_query_rejects_an_embedded_path_terminator() {
        let path = std::env::temp_dir().join("archive\0trailing");
        for error in [available_space(&path).unwrap_err(), total_space(&path).unwrap_err()] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(error.to_string(), "path contains a NUL byte");
        }
    }

    #[test]
    fn space_queries_report_the_volume_and_reject_missing_paths() {
        let dir = std::env::temp_dir();
        let total = total_space(&dir).unwrap();
        let available = available_space(&dir).unwrap();
        assert!(total > 0);
        assert!(available <= total);
        // On Windows, GetVolumePathNameW resolves a missing path to its volume
        // root and succeeds, as it did under fs2.
        #[cfg(unix)]
        assert!(available_space(dir.join(format!("missing-{}", uuid::Uuid::new_v4()))).is_err());
    }
}
