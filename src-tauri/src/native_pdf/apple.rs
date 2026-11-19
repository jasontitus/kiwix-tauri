// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Authority, Commit, Destination, Files, Phase, Presentation};
use std::{ffi::{c_char, CString}, path::PathBuf, sync::{Arc, atomic::AtomicBool}};
#[cfg(target_os = "macos")]
use std::{ffi::{c_void, CStr}, path::Path, sync::atomic::{AtomicU64, Ordering}, time::Duration};
#[cfg(target_os = "macos")]
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
#[path = "apple_callbacks.rs"]
mod callbacks;
use callbacks::CALLBACKS;
type Admit = extern "C" fn(u64) -> bool;
type Complete = extern "C" fn(u64, u8);
unsafe extern "C" {
    fn kiwix_pdf_start(id: u64, bootstrap: *const c_char, article: *const c_char, output: *const c_char, width: u32, height: u32, authority: Admit, complete: Complete);
    fn kiwix_pdf_cancel(id: u64);
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn kiwix_pdf_print_start(id: u64, path: *const c_char, title: *const c_char, context: *mut c_void, authority: extern "C" fn(*mut c_void) -> bool, complete: extern "C" fn(*mut c_void, *const c_char)) -> bool;
    fn kiwix_pdf_print_cancel(id: u64);
}
extern "C" fn admitted(id: u64) -> bool { CALLBACKS.admitted(id) }
extern "C" fn complete(id: u64, status: u8) {
    let Some(callback) = CALLBACKS.take(id) else { return; };
    let _ = callback.sender.send(render_result(status));
}

// These ABI values match PdfRenderStatus in native/pdf.mm. Unknown values
// are errors so a mismatched provider cannot accidentally authorize output.
fn render_result(status: u8) -> Result<(), String> {
    let error = match status {
        0 => return Ok(()),
        1 => "The live article PDF document could not load",
        2 => "The live article PDF renderer terminated",
        3 => "Article PDF authority expired",
        4 => "Article PDF exceeds the 512-page limit",
        5 => "Cannot create the live article PDF output",
        6 => "Live article PDF capture requires iOS 14 or macOS 11 or later",
        7 => "WebKit could not capture a bounded live article PDF page",
        8 => "WebKit returned an invalid live article PDF page",
        9 => "WebKit returned invalid live article PDF dimensions",
        10 => "Native PDF output exceeds the 64 MiB limit",
        11 => "Invalid live article capture dimensions",
        12 => "Article PDF export timed out",
        13 => "Article PDF export was cancelled",
        _ => "Unknown native Apple PDF export error",
    };
    Err(error.into())
}

struct NativeRequest(u64);
impl Drop for NativeRequest {
    fn drop(&mut self) {
        if CALLBACKS.revoke(self.0) {
            // SAFETY: this ID identifies the native renderer started below.
            unsafe { kiwix_pdf_cancel(self.0); }
        }
    }
}
// Shared platform-provider boundary: keep native rendering, handoff authority,
// temporary-file ownership and cancellation explicit across Apple/desktop/Android.
#[allow(clippy::too_many_arguments)]
pub async fn render(_app: tauri::AppHandle, bootstrap: String, article: String, _secret: String, output: PathBuf, _presentation: Presentation, _destination: Destination, width: u32, height: u32, authority: Authority, _commit: Commit, _handed_off: Arc<AtomicBool>, files: Files, cancel: CancellationToken) -> Result<(), String> {
    let bootstrap = CString::new(bootstrap).map_err(|_| "Invalid PDF bootstrap URL")?;
    let article = CString::new(article).map_err(|_| "Invalid PDF article URL")?;
    let output = CString::new(output.to_str().ok_or("PDF output path is not UTF-8")?).map_err(|_| "Invalid PDF output path")?;
    let (id, receiver) = CALLBACKS.register(authority, files)?;
    let _request = NativeRequest(id);
    // SAFETY: the bridge copies these C strings before returning. The request
    // owns its registered ID and the callbacks have the declared C ABI.
    unsafe { kiwix_pdf_start(id, bootstrap.as_ptr(), article.as_ptr(), output.as_ptr(), width, height, admitted, complete); }
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
