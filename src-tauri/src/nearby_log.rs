#![forbid(unsafe_code)]
//! Bounded Nearby diagnostics that survive the iOS console boundary.
//!
//! stderr reaches logcat on Android, but a UI-test-launched iOS app has no
//! captured stderr. The same lines are appended to a small file in the app
//! data directory so `devicectl device copy from` can retrieve them.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};

const LIMIT: u64 = 512 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();
static FILE: LazyLock<Mutex<Option<std::fs::File>>> = LazyLock::new(|| Mutex::new(None));
static WRITTEN: AtomicU64 = AtomicU64::new(0);
static WRITES: AtomicU64 = AtomicU64::new(0);

pub fn set_path(path: PathBuf) {
    let _ = PATH.set(path);
}

pub fn log(arguments: std::fmt::Arguments<'_>) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let line = format!("{stamp} {arguments}\n");
    eprint!("{line}");
    let Some(path) = PATH.get() else { return };
    let Ok(mut file) = FILE.lock() else {
        return;
    };
    // The iOS Swift writer shares this path and rotates by renaming it. Check
    // periodically whether the path now names a different inode, and reopen
    // rather than appending forever to the rotated-away file.
    if WRITES.fetch_add(1, Ordering::Relaxed).is_multiple_of(64) {
        if let Some(open) = file.as_ref() {
            let replaced = match (std::fs::metadata(path), open.metadata()) {
                (Ok(path), Ok(open)) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt as _;
                        path.dev() != open.dev() || path.ino() != open.ino()
                    }
                    #[cfg(not(unix))]
                    {
                        path.len() != open.len()
                    }
                }
                _ => true,
            };
            if replaced {
                *file = None;
            }
        }
    }
    if file.is_none() {
        *file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok();
        if let Some(open) = file.as_ref() {
            WRITTEN.store(
                open.metadata().map(|meta| meta.len()).unwrap_or(0),
                Ordering::Relaxed,
            );
        }
    }
    let Some(open) = file.as_mut() else { return };
    let written = WRITTEN.load(Ordering::Relaxed);
    if written < LIMIT && open.write_all(line.as_bytes()).is_ok() {
        WRITTEN.store(written.saturating_add(line.len() as u64), Ordering::Relaxed);
    }
}

#[macro_export]
macro_rules! nearby_log {
    ($($argument:tt)*) => {
        $crate::nearby_log::log(format_args!($($argument)*))
    };
}
