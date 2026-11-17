// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Authority, Commit, Destination, Files, Phase, Presentation};
use std::{ffi::{c_char, c_void, CStr, CString}, path::PathBuf, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};
#[cfg(target_os = "macos")]
use std::{path::Path, time::Duration};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
unsafe extern "C" {
    fn kiwix_pdf_start(id: u64, bootstrap: *const c_char, article: *const c_char, output: *const c_char, width: u32, height: u32, context: *mut c_void, authority: extern "C" fn(*mut c_void) -> bool, complete: extern "C" fn(*mut c_void, *const c_char));
    fn kiwix_pdf_cancel(id: u64);
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn kiwix_pdf_print_start(id: u64, path: *const c_char, title: *const c_char, context: *mut c_void, authority: extern "C" fn(*mut c_void) -> bool, complete: extern "C" fn(*mut c_void, *const c_char)) -> bool;
    fn kiwix_pdf_print_cancel(id: u64);
}
struct Callback { authority: Authority, sender: oneshot::Sender<Result<(), String>>, _files: Files }
extern "C" fn admitted(context: *mut c_void) -> bool {
    // Native owns the callback context until its exactly-once completion.
    let callback = unsafe { &*(context as *const Callback) };
    (callback.authority)(Phase::BeforeHandoff).is_ok()
}
extern "C" fn complete(context: *mut c_void, error: *const c_char) {
    let callback = unsafe { Box::from_raw(context as *mut Callback) };
    let result = if error.is_null() { Ok(()) } else { Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned()) };
    let _ = callback.sender.send(result);
}
struct NativeRequest(u64);
impl Drop for NativeRequest { fn drop(&mut self) { unsafe { kiwix_pdf_cancel(self.0); } } }
// Shared platform-provider boundary: keep native rendering, handoff authority,
// temporary-file ownership and cancellation explicit across Apple/desktop/Android.
#[allow(clippy::too_many_arguments)]
pub async fn render(_app: tauri::AppHandle, bootstrap: String, article: String, _secret: String, output: PathBuf, _presentation: Presentation, _destination: Destination, width: u32, height: u32, authority: Authority, _commit: Commit, _handed_off: Arc<AtomicBool>, files: Files, cancel: CancellationToken) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let bootstrap = CString::new(bootstrap).map_err(|_| "Invalid PDF bootstrap URL")?;
    let article = CString::new(article).map_err(|_| "Invalid PDF article URL")?;
    let output = CString::new(output.to_str().ok_or("PDF output path is not UTF-8")?).map_err(|_| "Invalid PDF output path")?;
    let (sender, receiver) = oneshot::channel();
    let context = Box::into_raw(Box::new(Callback { authority, sender, _files: files }));
    unsafe { kiwix_pdf_start(id, bootstrap.as_ptr(), article.as_ptr(), output.as_ptr(), width, height, context.cast(), admitted, complete); }
    let _request = NativeRequest(id);
    tokio::select! {
        result = receiver => result.map_err(|_| "Native Apple PDF provider stopped")?,
        _ = cancel.cancelled() => Err("Article PDF export was cancelled".into()),
    }
}
/// Present an already-rendered, validated article PDF through the real macOS
/// print panel. The caller keeps the private temporary file alive until this
/// future settles, so completion and cancellation both end before cleanup.
#[cfg(target_os = "macos")]
pub async fn print(pdf: &Path, title: &str, authority: Authority, cancel: CancellationToken) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = CString::new(pdf.to_str().ok_or("PDF print path is not UTF-8")?).map_err(|_| "Invalid PDF print path")?;
    let title = CString::new(title).map_err(|_| "Invalid PDF print title")?;
    let observe = authority.clone();
    let (sender, mut receiver) = oneshot::channel();
    let context = Box::into_raw(Box::new(PrintCallback { authority, sender }));
    if !unsafe { kiwix_pdf_print_start(id, path.as_ptr(), title.as_ptr(), context.cast(), print_admitted, print_complete) } {
        drop(unsafe { Box::from_raw(context) });
        return Err("The native print panel could not be opened".into());
    }
    let _request = PrintRequest(id);
    let mut checks = tokio::time::interval(Duration::from_millis(250));
    checks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let deadline = tokio::time::sleep(Duration::from_secs(16 * 60));
    tokio::pin!(deadline);
    let outcome = loop {
        tokio::select! {
            result = &mut receiver => break result.map_err(|_| "Native Apple print provider stopped".to_string()),
            _ = cancel.cancelled() => break Err("Article PDF printing was cancelled".to_string()),
            _ = checks.tick() => {
                if let Err(error) = observe(Phase::AfterHandoff) { break Err(error); }
            }
            _ = &mut deadline => break Err("The system print dialog timed out".to_string()),
        }
    };
    match outcome {
        Ok(result) => result?,
        Err(error) => {
            unsafe { kiwix_pdf_print_cancel(id); }
            // Wait for the native panel to settle before the caller drops (and
            // removes) the PDF it is still presenting.
            let _ = tokio::time::timeout(Duration::from_secs(20), receiver).await;
            return Err(error);
        }
    }
    Ok(())
}
#[cfg(target_os = "macos")]
struct PrintCallback { authority: Authority, sender: oneshot::Sender<Result<(), String>> }
#[cfg(target_os = "macos")]
extern "C" fn print_admitted(context: *mut c_void) -> bool {
    let callback = unsafe { &*(context as *const PrintCallback) };
    (callback.authority)(Phase::BeforeHandoff).is_ok()
}
#[cfg(target_os = "macos")]
extern "C" fn print_complete(context: *mut c_void, error: *const c_char) {
    let callback = unsafe { Box::from_raw(context as *mut PrintCallback) };
    let result = if error.is_null() { Ok(()) } else { Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned()) };
    let _ = callback.sender.send(result);
}
#[cfg(target_os = "macos")]
struct PrintRequest(u64);
#[cfg(target_os = "macos")]
impl Drop for PrintRequest { fn drop(&mut self) { unsafe { kiwix_pdf_print_cancel(self.0); } } }
