// SPDX-License-Identifier: GPL-3.0-or-later
use crate::native_pdf::{Authority, Commit, Destination, Files, Phase, Presentation};
use std::{path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use tauri::Manager;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use webview2_com::{Microsoft::Web::WebView2::Win32::{ICoreWebView2_2, ICoreWebView2_7, ICoreWebView2_13, ICoreWebView2_16, ICoreWebView2Environment5, COREWEBVIEW2_PRINT_STATUS_SUCCEEDED}, BrowserProcessExitedEventHandler, NavigationCompletedEventHandler, PrintCompletedHandler, PrintToPdfCompletedHandler, CoTaskMemPWSTR};
use windows::{core::{BOOL, HSTRING, Interface, PWSTR}, Win32::Foundation::HWND};
#[path = "windows_print.rs"]
mod printing;
type Reply = Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>;
fn finish(reply: &Reply, result: Result<(), String>) { if let Ok(mut reply) = reply.lock() { if let Some(sender) = reply.take() { let _ = sender.send(result); } } }
/// `Files` captured by WebView2 handlers or UI-thread closures. Its last
/// reference can drop inside a COM Release on the UI thread, where cleanup
/// (blocking directory removal, library-change events that re-enter the
/// runtime) must not run: a panic there cannot unwind and aborts the app.
struct OffUiFiles(Option<Files>);
impl OffUiFiles {
    fn new(files: Files) -> Self { Self(Some(files)) }
    fn share(&self) -> Self { Self(self.0.clone()) }
}
impl Drop for OffUiFiles {
    fn drop(&mut self) {
        if let Some(files) = self.0.take() {
            // If no thread can be spawned the closure, and the files, drop here.
            let _ = std::thread::Builder::new().name("pdf-cleanup".into()).spawn(move || drop(files));
        }
    }
}
struct NativeJob { app: tauri::AppHandle, label: String, files: Files, dialog: Arc<printing::Dialog> }
impl Drop for NativeJob {
    fn drop(&mut self) {
        // Win32's modal dialog pumps its own loop; cancel it directly rather
        // than waiting for a queued WebView callback to regain the UI thread.
        self.dialog.cancel();
        if let Some(window) = self.app.get_webview_window(&self.label) {
            let destroy = window.clone(); let files = OffUiFiles::new(self.files.clone());
            let _ = window.with_webview(move |platform| {
                // Force controller closure, not a cancellable window-close
                // event. The process-exit callback owns profile cleanup.
                unsafe { let _ = platform.controller().Close(); }
                let _ = destroy.destroy();
                drop(files);
            });
        }
    }
}
// Shared platform-provider boundary; authority, commit, retained files and
// cancellation remain independent of untrusted article/presentation data.
#[allow(clippy::too_many_arguments)]
pub async fn render(app: tauri::AppHandle, bootstrap: String, article: String, _secret: String, output: PathBuf, _presentation: Presentation, destination: Destination, width: u32, _height: u32, authority: Authority, commit: Commit, handed_off: Arc<AtomicBool>, files: Files, cancel: CancellationToken) -> Result<(), String> {
    let label = format!("pdf-render-{}", uuid::Uuid::new_v4().simple());
    let dialog = Arc::new(printing::Dialog::default());
    let _job = NativeJob { app: app.clone(), label: label.clone(), files: files.clone(), dialog: dialog.clone() };
    // Every copy that moves to the UI thread is off-UI on its final drop,
    // including the setup error paths below.
    let files = OffUiFiles::new(files);
    let (sender, receiver) = oneshot::channel();
    let reply = Arc::new(Mutex::new(Some(sender)));
    let setup_reply = reply.clone(); let app_on_main = app.clone();
    let setup_cancel = cancel.clone();
    app.run_on_main_thread(move || {
        let setup = (|| -> Result<(), String> {
            // A job cancelled before this ran has already looked for (and not
            // found) its window; never build one nothing would destroy.
            if setup_cancel.is_cancelled() { return Err("Article PDF export was cancelled".into()); }
            authority(Phase::BeforeHandoff)?;
            let allowed_bootstrap = bootstrap.clone(); let allowed_article = article.clone();
            // A private per-job user-data root prevents incognito contexts from
            // accidentally sharing even transient loopback-cookie namespaces.
            let profile = output.parent().ok_or("Missing PDF scratch directory")?.join("webview");
            let window = tauri::WebviewWindowBuilder::new(&app_on_main, &label, tauri::WebviewUrl::External("about:blank".parse().unwrap()))
                .visible(false).skip_taskbar(true).inner_size(f64::from(width), 1123.0)
                .incognito(true).data_directory(profile)
                .on_navigation(move |url| url.as_str() == "about:blank" || url.as_str() == allowed_bootstrap || url.as_str() == allowed_article)
                .build().map_err(|e| format!("Creating the native PDF view: {e}"))?;
            let native_reply = setup_reply.clone();
            window.with_webview(move |platform| {
                let start = (|| -> windows::core::Result<()> { unsafe {
                    let core = platform.controller().CoreWebView2()?;
                    let mut owner = HWND::default();
                    platform.controller().ParentWindow(&mut owner)?;
                    let print_files = files.share();
                    let environment = core.cast::<ICoreWebView2_2>()?.Environment()?.cast::<ICoreWebView2Environment5>()?;
                    let profile_files = std::cell::RefCell::new(Some(files));
                    let mut exited = 0;
                    environment.add_BrowserProcessExited(&BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
                        // This is the native acknowledgement that per-job
                        // profile handles are no longer in use.
                        profile_files.borrow_mut().take();
                        Ok(())
                    })), &mut exited)?;
                    // Wry's incognito flag silently falls back on old runtimes.
                    // Verify the real profile before loading ANY archive bytes.
                    let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
                    let mut private = BOOL::default();
                    profile.IsInPrivateModeEnabled(&mut private)?;
                    if !private.as_bool() { finish(&native_reply, Err("The WebView2 runtime cannot create a private article PDF view".into())); return Ok(()); }
                    let settings = core.Settings()?;
                    settings.SetIsScriptEnabled(false)?;
                    settings.SetAreDevToolsEnabled(false)?;
                    settings.SetAreDefaultScriptDialogsEnabled(false)?;
                    let once = AtomicBool::new(false);
                    let completed_reply = native_reply.clone();
                    let mut token = 0;
                    core.add_NavigationCompleted(&NavigationCompletedEventHandler::create(Box::new(move |webview, args| {
                        let result = (|| -> windows::core::Result<()> {
                            let (Some(webview), Some(args)) = (webview, args) else { return Ok(()); };
                            let mut source = PWSTR::null(); webview.Source(&mut source)?;
                            let source = CoTaskMemPWSTR::from(source).to_string();
                            if source != article { return Ok(()); }
                            let mut success = BOOL::default(); args.IsSuccess(&mut success)?;
                            if !success.as_bool() { finish(&completed_reply, Err("WebView2 could not load the article for PDF export".into())); return Ok(()); }
                            if once.swap(true, Ordering::SeqCst) { return Ok(()); }
                            if let Err(error) = authority(Phase::BeforeHandoff) { finish(&completed_reply, Err(error)); return Ok(()); }
                            if destination == Destination::Print {
                                // The native dialog only selects settings. No
                                // archive bytes reach a printer until the short
                                // authority commit following user confirmation.
                                let printer = match printing::choose(&webview, owner, &dialog, width) {
                                    Ok(Some(settings)) => settings,
                                    Ok(None) => { finish(&completed_reply, Ok(())); return Ok(()); },
                                    Err(error) => { finish(&completed_reply, Err(error)); return Ok(()); },
                                };
                                let print_reply = completed_reply.clone();
                                let print_lifetime = print_files.share();
                                let callback = PrintCompletedHandler::create(Box::new(move |status, result| {
                                    let _ = &print_lifetime;
                                    finish(&print_reply, status.map_err(|error| format!("Native printing failed: {error}"))
                                        .and_then(|_| if result == COREWEBVIEW2_PRINT_STATUS_SUCCEEDED { Ok(()) } else { Err(format!("The Windows print spool rejected the job ({})", result.0)) }));
                                    Ok(())
                                }));
                                let print = webview.cast::<ICoreWebView2_16>()?;
                                if let Err(error) = commit(&mut || {
                                    handed_off.store(true, Ordering::SeqCst);
                                    print.Print(&printer, &callback).map_err(|error| format!("Sending the article to the Windows printer: {error}"))
                                }) { finish(&completed_reply, Err(error)); }
                                return Ok(());
                            }
                            let output = HSTRING::from(output.as_os_str());
                            let pdf_reply = completed_reply.clone();
                            let callback = PrintToPdfCompletedHandler::create(Box::new(move |status, success| {
                                finish(&pdf_reply, status.map_err(|e| format!("Native PDF export failed: {e}")).and_then(|_| if success { Ok(()) } else { Err("WebView2 did not save the article PDF".into()) }));
                                Ok(())
                            }));
                            let settings = match printing::pdf_settings(&webview, width) {
                                Ok(settings) => settings,
                                Err(error) => { finish(&completed_reply, Err(error)); return Ok(()); },
                            };
                            webview.cast::<ICoreWebView2_7>()?.PrintToPdf(&output, &settings, &callback)?;
                            Ok(())
                        })();
                        if let Err(error) = result { finish(&completed_reply, Err(format!("Native PDF export failed: {error}"))); }
                        Ok(())
                    })), &mut token)?;
                    core.Navigate(&HSTRING::from(bootstrap))?;
                    Ok(())
                } })();
                if let Err(error) = start { finish(&native_reply, Err(format!("A current InPrivate-capable WebView2 runtime is required for PDF export: {error}"))); }
            }).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(error) = setup { finish(&setup_reply, Err(error)); }
    }).map_err(|e| e.to_string())?;
    tokio::select! { result = receiver => result.map_err(|_| "Native WebView2 PDF provider stopped")?, _ = cancel.cancelled() => Err("Article PDF export was cancelled".into()) }
}
