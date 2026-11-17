// SPDX-License-Identifier: GPL-3.0-or-later
//! Advisory whole-file locks and volume space queries.
//!
//! This replaces the `fs2` crate, which has had no release since 2018. It keeps
//! fs2's exact system calls so lock semantics and error codes are unchanged:
//! `flock(2)` on Unix (including Android and iOS), `LockFileEx` over the whole
//! file on Windows, and `statvfs(3)` for space on Unix.
//!
//! `std::fs::File::try_lock` (Rust 1.89) is not a substitute yet: through at
//! least Rust 1.97 it returns `Unsupported` on Android, where these locks guard
//! profile, download and archive ownership.

use std::fs::File;
use std::io;
use std::path::Path;

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
        flock(self, libc::LOCK_EX | libc::LOCK_NB)
    }

    fn try_flock_shared(&self) -> io::Result<()> {
        flock(self, libc::LOCK_SH | libc::LOCK_NB)
    }
}

#[cfg(unix)]
fn flock(file: &File, operation: libc::c_int) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: the descriptor is owned by `file`, which outlives the call.
    if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
impl FileExt for File {
    fn try_flock_exclusive(&self) -> io::Result<()> {
        use windows::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};
        lock_file(self, LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)
    }

    fn try_flock_shared(&self) -> io::Result<()> {
        use windows::Win32::Storage::FileSystem::LOCKFILE_FAIL_IMMEDIATELY;
        lock_file(self, LOCKFILE_FAIL_IMMEDIATELY)
    }
}

#[cfg(windows)]
fn lock_file(file: &File, flags: windows::Win32::Storage::FileSystem::LOCK_FILE_FLAGS) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::LockFileEx;
    use windows::Win32::System::IO::OVERLAPPED;

    let mut overlapped = OVERLAPPED::default();
    // SAFETY: the handle is owned by `file`, which outlives the call, and the
    // zeroed OVERLAPPED (offset 0) is only used for this synchronous request.
    // The range covers the whole file, as fs2 did.
    unsafe { LockFileEx(HANDLE(file.as_raw_handle()), flags, None, u32::MAX, u32::MAX, &mut overlapped) }
        .map_err(win32_error)
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
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    // SAFETY: `statvfs` is plain old data and the call fully initializes it on
    // success; `path` is a valid NUL-terminated string for the call.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Field widths differ by platform; the casts are widening where needed.
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    let (fragment, available, total) = (stat.f_frsize as u64, stat.f_bavail as u64, stat.f_blocks as u64);
    Ok(VolumeSpace { available: fragment.saturating_mul(available), total: fragment.saturating_mul(total) })
}

#[cfg(windows)]
fn volume_space(path: &Path) -> io::Result<VolumeSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumePathNameW};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
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
