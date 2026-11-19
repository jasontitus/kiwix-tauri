// SPDX-License-Identifier: GPL-3.0-or-later
//! System share-sheet handoff. The renderer never supplies a shareable path:
//! article PDFs come from the existing native renderer and archives come from
//! the library store. Every shared byte is either caller-owned scratch (deleted
//! when the sheet finishes) or a file the library already owns.
use super::*;
#[cfg(target_os = "macos")]
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        collections::HashMap,
        ffi::{c_char, c_void, CStr, CString},
        path::Path,
        sync::{atomic::{AtomicU64, Ordering}, LazyLock, Mutex},
    };
    use tokio::sync::oneshot;
    use tokio_util::sync::CancellationToken;
    type Completion = extern "C" fn(*mut c_void, *const c_char);
    unsafe extern "C" {
        fn kiwix_share_present(token: u64, path: *const c_char, context: *mut c_void, complete: Completion);
        fn kiwix_share_cancel(token: u64);
    }
    type ShareCompletion = oneshot::Sender<Result<(), String>>;
    static SHARES: LazyLock<Mutex<HashMap<u64, ShareCompletion>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    extern "C" fn complete(context: *mut c_void, error: *const c_char) {
        let token = context as usize as u64;
        let sender = SHARES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&token);
        let result = if error.is_null() {
            Ok(())
        } else {
            // SAFETY: the native completion passes either NULL or a NUL-terminated
            // UTF-8 message valid for this callback; it is copied before returning.
            Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned())
        };
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    }
    struct CancelOnDrop(u64);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            // SAFETY: the token is the opaque integer this request registered;
            // native only looks it up and never dereferences it as a pointer.
            unsafe { kiwix_share_cancel(self.0) };
        }
    }
    pub async fn present_file(path: &Path, cancel: CancellationToken) -> Result<(), String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let token = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = CString::new(path.to_str().ok_or("Shared path is not UTF-8")?)
            .map_err(|_| "Invalid shared path")?;
        let (sender, receiver) = oneshot::channel();
        SHARES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(token, sender);
        // Drop always retires the native request, including task cancellation.
        let _guard = CancelOnDrop(token);
        // SAFETY: `path` is a live NUL-terminated CString and native copies it into
        // an NSString before dispatching. The context carries only the integer
        // token, and native calls `complete` at most once for this token.
        unsafe {
            kiwix_share_present(token, path.as_ptr(), token as usize as *mut c_void, complete);
        }
        tokio::select! {
            result = receiver => result.map_err(|_| "Native share sheet stopped".to_string())?,
            _ = cancel.cancelled() => Err("Sharing was cancelled".into()),
        }
    }
}
#[cfg(target_os = "macos")]
pub(super) use macos::present_file;

impl Service {
    /// Share a library archive through the platform share sheet. The path is
    /// resolved from the trusted store, never from renderer-supplied input.
    pub(super) async fn share_book(self: Arc<Self>, label: &str, args: Value) -> Result<Value, String> {
        self.writable()?;
        self.ensure_window_active(label)?;
        self.ensure_generation(&args)?;
        let id = text(&args, "bookId")?.to_string();
        let book = self.store.book(&id)?.ok_or("The archive is no longer in the library")?;
        if !book.available {
            return Err("The archive file is missing; relocate it before sharing".into());
        }
        let path = PathBuf::from(&book.path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("Reading the archive to share: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("Only a regular archive file can be shared".into());
        }
        #[cfg(target_os = "macos")]
        {
            // No window guard is held across the sheet: closing the window must
            // not wait for a user-completed share.
            let cancel = CancellationToken::new();
            present_file(&path, cancel).await?;
            Ok(Value::Null)
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("The archive name is not UTF-8")?
                .to_string();
            // The native adapter copies into its own export scratch before the
            // system sheet appears, so the library file is never mutated.
            self.mobile_export(&path, "application/x-zim", &name, true, true).await?;
            Ok(Value::Null)
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let _ = &path;
            Err("System sharing is unavailable on this platform".into())
        }
    }
}
