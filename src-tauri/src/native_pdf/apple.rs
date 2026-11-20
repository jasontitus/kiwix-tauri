// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Authority, Commit, Destination, Files, Presentation};
#[cfg(target_os = "macos")]
use super::Phase;
use crate::apple_requests::Registry;
use std::{ffi::{c_char, CString}, path::PathBuf, sync::{Arc, LazyLock, atomic::AtomicBool}};
#[cfg(target_os = "macos")]
use {super::OwnedPdf, crate::apple_requests::ResultReceiver, std::{ffi::c_void, time::Duration}};
use tokio_util::sync::CancellationToken;
static CALLBACKS: LazyLock<Registry<Files>> = LazyLock::new(Registry::default);
type Admit = extern "C" fn(u64) -> bool;
type Complete = extern "C" fn(u64, u8);
unsafe extern "C" {
    fn kiwix_pdf_start(id: u64, bootstrap: *const c_char, article: *const c_char, output: *const c_char, width: u32, height: u32, authority: Admit, complete: Complete);
    safe fn kiwix_pdf_cancel(id: u64);
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn kiwix_pdf_print_start(id: u64, host: *mut c_void, path: *const c_char, title: *const c_char, authority: Admit, complete: extern "C" fn(u64, u8, i32)) -> bool;
    safe fn kiwix_pdf_print_cancel(id: u64);
}
extern "C" fn admitted(id: u64) -> bool { CALLBACKS.admitted(id) }
extern "C" fn complete(id: u64, status: u8) {
    let Some(callback) = CALLBACKS.take(id) else { return; };
    callback.finish(render_result(status));
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
            // The native cancel only looks up this ID and is a no-op once the
            // renderer has finished, so it is declared safe at the ABI.
            kiwix_pdf_cancel(self.0);
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
/// The native operation has its own scratch lease through AppKit completion.
/// Discarding the Rust waiter revokes authority and requests cancellation.
#[cfg(target_os = "macos")]
pub async fn print(window: tauri::WebviewWindow, pdf: &OwnedPdf, title: &str, authority: Authority, commit: Commit, cancel: CancellationToken) -> Result<(), String> {
    let path = CString::new(pdf.path().to_str().ok_or("PDF print path is not UTF-8")?).map_err(|_| "Invalid PDF print path")?;
    let title = CString::new(title).map_err(|_| "Invalid PDF print title")?;
    let (id, receiver) = CALLBACKS.register(Arc::new(move |_| commit(&mut || Ok(()))), pdf.files.clone())?;
    let request = PrintRequest(id);
    let start = ScheduledPrint(Some(id));
    let origin = window.clone();
    window.run_on_main_thread(move || start.begin(origin, path, title))
        .map_err(|_| "Could not schedule the system print panel")?;
    wait_for_print(request, receiver, authority, cancel).await
}

#[cfg(target_os = "macos")]
async fn wait_for_print(request: PrintRequest, mut receiver: ResultReceiver, authority: Authority, cancel: CancellationToken) -> Result<(), String> {
    let mut checks = tokio::time::interval(Duration::from_millis(250));
    checks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let deadline = tokio::time::sleep(Duration::from_secs(16 * 60));
    tokio::pin!(deadline);
    let mut error = loop {
        tokio::select! {
            result = &mut receiver => return result.map_err(|_| "Native Apple print provider stopped".to_string())?,
            _ = cancel.cancelled() => break "Article PDF printing was cancelled".to_string(),
            _ = checks.tick() => {
                if let Err(error) = authority(Phase::AfterHandoff) { break error; }
            }
            _ = &mut deadline => break "The system print dialog timed out".to_string(),
        }
    };
    request.cancel();
    if tokio::time::timeout(Duration::from_secs(20), receiver).await.is_err() {
        error.push_str(". Close the system print dialog to finish cancellation.");
    }
    Err(error)
}
#[cfg(target_os = "macos")]
struct ScheduledPrint(Option<u64>);
#[cfg(target_os = "macos")]
impl ScheduledPrint {
    fn begin(mut self, origin: tauri::WebviewWindow, path: CString, title: CString) {
        let Some(id) = self.0 else { return; };
        if !CALLBACKS.admitted(id) {
            print_complete(id, 1, 0);
            return;
        }
        let Ok(host) = origin.ns_window() else {
            print_complete(id, 4, 0);
            return;
        };
        // SAFETY: this runs on the AppKit thread with the originating window
        // live. The bridge takes a strong window reference and copies the
        // strings. It accepts only a registered ID and the matching callbacks.
        if unsafe { kiwix_pdf_print_start(id, host, path.as_ptr(), title.as_ptr(), admitted, print_complete) } {
            self.0 = None;
        } else {
            print_complete(id, 9, 0);
        }
    }
}
#[cfg(target_os = "macos")]
impl Drop for ScheduledPrint {
    fn drop(&mut self) {
        if let Some(id) = self.0 {
            drop(CALLBACKS.take(id));
        }
    }
}

// Keep these values in sync with PdfPrintStatus in native/print.mm.
#[cfg(target_os = "macos")]
extern "C" fn print_complete(id: u64, status: u8, system_error: i32) {
    let Some(callback) = CALLBACKS.take(id) else { return; };
    let result = match status {
        0 => Ok(()),
        1 => Err("Article PDF print authority expired".into()),
        2 => Err("Cannot open the rendered article PDF for printing".into()),
        3 => Err("Cannot create the native print operation".into()),
        4 => Err("Activate the original article window and try printing again".into()),
        5 => Err("Close the open dialog in the article window before printing".into()),
        6 => Err("Article PDF printing was cancelled".into()),
        7 => Err("The system print dialog timed out".into()),
        8 => Err(format!("The native print operation failed ({system_error})")),
        10 => Err("Close the current system print dialog before printing another article".into()),
        _ => Err("The native print panel could not be opened".into()),
    };
    callback.finish(result);
}
#[cfg(target_os = "macos")]
struct PrintRequest(u64);
#[cfg(target_os = "macos")]
impl PrintRequest {
    fn cancel(&self) {
        if CALLBACKS.revoke(self.0) {
            // The native cancel only looks up this ID and is a no-op once the
            // print request has finished, so it is declared safe at the ABI.
            kiwix_pdf_print_cancel(self.0);
        }
    }
}
#[cfg(target_os = "macos")]
impl Drop for PrintRequest { fn drop(&mut self) { self.cancel(); } }

#[cfg(all(test, target_os = "macos"))]
mod print_tests {
    use super::*;

    #[tokio::test]
    async fn withdrawn_scheduled_print_releases_its_lease_and_returns_an_error() {
        let directory = tempfile::tempdir().unwrap().keep();
        let files = Files::new(directory.clone(), Arc::new(|error| panic!("{error}")), None);
        let authority: Authority = Arc::new(|_| Ok(()));
        let (id, receiver) = CALLBACKS.register(authority.clone(), files).unwrap();
        drop(ScheduledPrint(Some(id)));
        let result = wait_for_print(PrintRequest(id), receiver, authority, CancellationToken::new()).await;
        assert_eq!(result, Err("Native Apple print provider stopped".into()));
        assert!(!directory.exists());
    }
}
