// SPDX-License-Identifier: GPL-3.0-or-later
use crate::native_pdf::{Authority, Commit, Destination, Files, Phase, Presentation};
use gtk::prelude::*;
use gtk::glib::translate::{from_glib_full, from_glib_none, ToGlibPtr};
use webkit2gtk::{NavigationPolicyDecisionExt, PolicyDecisionExt, PrintOperationExt, SettingsExt, URIRequestExt, WebViewExt};
use std::{cell::{Cell, RefCell}, collections::HashMap, ffi::{c_char, c_void, CStr, CString}, path::PathBuf, rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
type Reply = Rc<RefCell<Option<oneshot::Sender<Result<(), String>>>>>;
struct Job { window: gtk::OffscreenWindow, view: webkit2gtk::WebView, operation: Option<webkit2gtk::PrintOperation>, dialog: Option<gtk::Dialog>, reply: Reply }
thread_local! { static JOBS: RefCell<HashMap<String, Job>> = RefCell::new(HashMap::new()); }
fn finish(reply: &Reply, result: Result<(), String>) { if let Some(sender) = reply.borrow_mut().take() { let _ = sender.send(result); } }
fn retire(id: &str) {
    let job = JOBS.with(|jobs| jobs.borrow_mut().remove(id));
    if let Some(job) = job {
        if let Some(dialog) = job.dialog {
            dialog.response(gtk::ResponseType::Cancel);
            // SAFETY: retire runs on the GTK main thread; removing the job
            // prevents this map from retiring it twice, but a title-bar close
            // may already have destroyed the dialog. This is a best-effort
            // destroy and no Rust reference to the widget is kept afterwards.
            unsafe { dialog.destroy(); }
        }
        job.view.stop_loading();
        // SAFETY: the removed job owns this offscreen window, and destruction
        // runs on the GTK main thread before the job's fields drop; the call is
        // the job's single explicit destroy attempt.
        unsafe { job.window.destroy(); }
        finish(&job.reply, Err("Article PDF export was cancelled".into()));
    }
}
// GtkPrinter is Unix-printing API, not wrapped by gtk-rs's core GTK crate.
// Match the backend type, not its translated display name. An absent printer
// setting would select the default PHYSICAL printer, which is never allowed.
unsafe extern "C" {
    fn gtk_enumerate_printers(callback: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32, data: *mut c_void, destroy: Option<unsafe extern "C" fn(*mut c_void)>, wait: i32);
    fn gtk_printer_get_backend(printer: *mut c_void) -> *mut gtk::glib::gobject_ffi::GTypeInstance;
    fn gtk_printer_get_name(printer: *mut c_void) -> *const c_char;
    fn gtk_print_unix_dialog_new(title: *const c_char, parent: *mut gtk::ffi::GtkWindow) -> *mut gtk::ffi::GtkWidget;
    fn gtk_print_unix_dialog_get_settings(dialog: *mut c_void) -> *mut gtk::ffi::GtkPrintSettings;
    fn gtk_print_unix_dialog_get_page_setup(dialog: *mut c_void) -> *mut gtk::ffi::GtkPageSetup;
    fn gtk_print_unix_dialog_set_page_setup(dialog: *mut c_void, setup: *mut gtk::ffi::GtkPageSetup);
}
fn file_printer() -> Result<String, String> {
    unsafe extern "C" fn select(printer: *mut c_void, data: *mut c_void) -> i32 {
        // SAFETY: GTK passes a live printer that owns this backend for the
        // duration of the synchronous enumeration; the callback runs on the
        // thread that called gtk_enumerate_printers.
        let backend = unsafe { gtk_printer_get_backend(printer) };
        if backend.is_null() { return 0; }
        // SAFETY: backend is a non-null live GTypeInstance owned by the
        // printer, and GLib returns an interned type name with process lifetime.
        let name = unsafe { gtk::glib::gobject_ffi::g_type_name_from_instance(backend) };
        if name.is_null() { return 0; }
        // SAFETY: name is a non-null NUL-terminated interned string owned by
        // GLib; the bytes are borrowed only for this comparison.
        let kind = unsafe { CStr::from_ptr(name) }.to_bytes();
        if kind != b"GtkPrintBackendFile" && kind != b"GtkPrintBackendFileBuiltin" { return 0; }
        // SAFETY: printer is the live object GTK supplied to this callback, so
        // its name stays valid at least until the callback returns.
        let name = unsafe { gtk_printer_get_name(printer) };
        if name.is_null() { return 0; }
        // SAFETY: data is the caller's &mut Option<String>, kept alive for the
        // synchronous enumeration; only this callback writes it, on this thread.
        unsafe { *(data as *mut Option<String>) = Some(CStr::from_ptr(name).to_string_lossy().into_owned()); }
        1
    }
    let mut printer: Option<String> = None;
    // SAFETY: `printer` outlives the call because `wait` is nonzero and
    // enumeration completes before returning; GTK may retain `data` in its
    // state, but no destructor is registered and `select` runs on this thread.
    unsafe { gtk_enumerate_printers(select, (&mut printer as *mut Option<String>).cast(), None, 1); }
    printer.ok_or_else(|| "GTK's native PDF file-print backend is unavailable; no printer was selected".into())
}
struct NativeJob { app: tauri::AppHandle, id: String, files: Files }
impl Drop for NativeJob { fn drop(&mut self) { let id = self.id.clone(); let files = self.files.clone(); let _ = self.app.run_on_main_thread(move || { retire(&id); drop(files); }); } }
// Shared platform-provider boundary; authority, commit, retained files and
// cancellation remain independent of untrusted article/presentation data.
#[allow(clippy::too_many_arguments)]
pub async fn render(app: tauri::AppHandle, bootstrap: String, article: String, _secret: String, output: PathBuf, presentation: Presentation, destination: Destination, width: u32, _height: u32, authority: Authority, commit: Commit, handed_off: Arc<AtomicBool>, files: Files, cancel: CancellationToken) -> Result<(), String> {
    let name = presentation.name;
    let id = uuid::Uuid::new_v4().to_string();
    let _job = NativeJob { app: app.clone(), id: id.clone(), files: files.clone() };
    let (sender, receiver) = oneshot::channel();
    app.run_on_main_thread(move || {
        let reply = Rc::new(RefCell::new(Some(sender)));
        if let Err(error) = authority(Phase::BeforeHandoff) { finish(&reply, Err(error)); return; }
        let printer = if destination == Destination::Print { None } else {
            match file_printer() { Ok(printer) => Some(printer), Err(error) => { finish(&reply, Err(error)); return; } }
        };
        let context = webkit2gtk::WebContext::new_ephemeral();
        let view = webkit2gtk::WebView::with_context(&context);
        let settings = webkit2gtk::Settings::new();
        settings.set_enable_javascript(false);
        settings.set_enable_page_cache(false);
        view.set_settings(&settings);
        let window = gtk::OffscreenWindow::new();
        window.set_default_size(width as i32, 1123);
        window.add(&view); window.show_all();
        let allowed_bootstrap = bootstrap.clone(); let allowed_article = article.clone();
        view.connect_decide_policy(move |_, decision, kind| {
            if kind == webkit2gtk::PolicyDecisionType::NavigationAction || kind == webkit2gtk::PolicyDecisionType::NewWindowAction {
                let allowed = decision.downcast_ref::<webkit2gtk::NavigationPolicyDecision>()
                    .and_then(|decision| decision.navigation_action()).and_then(|action| action.request()).and_then(|request| request.uri())
                    .is_some_and(|url| kind != webkit2gtk::PolicyDecisionType::NewWindowAction && (url.as_str() == allowed_bootstrap || url.as_str() == allowed_article));
                if !allowed { decision.ignore(); return true; }
            }
            false
        });
        let failed_reply = reply.clone();
        view.connect_load_failed(move |_, _, _, error| { finish(&failed_reply, Err(format!("Native PDF resource load failed: {error}"))); true });
        let started = Rc::new(Cell::new(false));
        let loaded_reply = reply.clone(); let print_id = id.clone();
        view.connect_load_changed(move |view, event| {
            if event != webkit2gtk::LoadEvent::Finished || view.uri().as_deref() != Some(article.as_str()) || started.replace(true) { return; }
            if let Err(error) = authority(Phase::BeforeHandoff) { finish(&loaded_reply, Err(error)); return; }
            let settings = gtk::PrintSettings::new();
            if let Some(printer) = &printer {
                let uri = match url::Url::from_file_path(&output) { Ok(uri) => uri, Err(_) => { finish(&loaded_reply, Err("Invalid native PDF scratch path".into())); return; } };
                settings.set_printer(printer);
                settings.set("output-uri", Some(uri.as_str()));
                settings.set("output-file-format", Some("pdf"));
            }
            let page = gtk::PageSetup::new();
            page.set_paper_size(&gtk::PaperSize::new(Some("iso_a4")));
            page.set_top_margin(36.0, gtk::Unit::Points); page.set_bottom_margin(36.0, gtk::Unit::Points);
            page.set_left_margin(36.0, gtk::Unit::Points); page.set_right_margin(36.0, gtk::Unit::Points);
            settings.set_scale((page.page_width(gtk::Unit::Points) * 96.0 / 72.0 / f64::from(width)).min(1.0) * 100.0);
            let operation = webkit2gtk::PrintOperation::new(view);
            operation.set_print_settings(&settings); operation.set_page_setup(&page);
            let failure = Rc::new(RefCell::new(None::<String>));
            let failed = failure.clone();
            operation.connect_failed(move |_, error| { *failed.borrow_mut() = Some(format!("Native PDF export failed: {error}")); });
            let finished_reply = loaded_reply.clone();
            let finished_files = RefCell::new(Some(files.clone()));
            operation.connect_finished(move |_| {
                finish(&finished_reply, failure.borrow_mut().take().map_or(Ok(()), Err));
                finished_files.borrow_mut().take();
            });
            JOBS.with(|jobs| { if let Some(job) = jobs.borrow_mut().get_mut(&print_id) { job.operation = Some(operation.clone()); } });
            if destination == Destination::Print {
                let title = match CString::new(name.as_str()) { Ok(title) => title, Err(_) => { finish(&loaded_reply, Err("Invalid print title".into())); return; } };
                // SAFETY: this runs on the GTK main thread; title is NUL-
                // terminated and outlives the call (GTK copies it); a null
                // parent is permitted, and the result is checked before use.
                let widget = unsafe { gtk_print_unix_dialog_new(title.as_ptr(), std::ptr::null_mut()) };
                if widget.is_null() { finish(&loaded_reply, Err("GTK could not open the system print dialog".into())); return; }
                // SAFETY: widget is the non-null, newly created dialog;
                // from_glib_none takes its own strong reference so the dialog
                // owns the widget from here on.
                let dialog: gtk::Dialog = unsafe { from_glib_none(widget.cast()) };
                let pointer = widget.cast::<c_void>();
                // SAFETY: `pointer` is the live dialog owned by `dialog`; GTK
                // takes its own strong reference to the same page setup, so the
                // Rust wrapper may drop without freeing it while the dialog lives.
                unsafe { gtk_print_unix_dialog_set_page_setup(pointer, page.to_glib_none().0); }
                let print_operation = operation.clone(); let print_reply = loaded_reply.clone();
                let print_commit = commit.clone(); let print_handed_off = handed_off.clone();
                dialog.connect_response(move |dialog, response| {
                    dialog.hide();
                    if response != gtk::ResponseType::Ok { finish(&print_reply, Ok(())); return; }
                    let pointer = dialog.as_ptr().cast::<c_void>();
                    // SAFETY: the response handler runs on the GTK main thread
                    // while the dialog is alive; the getter returns a new
                    // settings reference that the caller owns.
                    let settings = unsafe { gtk_print_unix_dialog_get_settings(pointer) };
                    // SAFETY: the dialog is alive for this callback, and the
                    // returned page setup is borrowed from it, so it stays
                    // valid while the dialog lives.
                    let page = unsafe { gtk_print_unix_dialog_get_page_setup(pointer) };
                    if settings.is_null() || page.is_null() { finish(&print_reply, Err("The system print settings are unavailable".into())); return; }
                    // SAFETY: settings was checked non-null and is a full
                    // reference from the getter, so from_glib_full adopts it.
                    let settings: gtk::PrintSettings = unsafe { from_glib_full(settings) };
                    if settings.printer().is_none_or(|printer| printer.is_empty()) { finish(&print_reply, Err("No system printer was selected".into())); return; }
                    // SAFETY: page was checked non-null and is borrowed from
                    // the live dialog; from_glib_none adds its own reference.
                    let page: gtk::PageSetup = unsafe { from_glib_none(page) };
                    let fit = (page.page_width(gtk::Unit::Points) * 96.0 / 72.0 / f64::from(width)).min(1.0);
                    if !fit.is_finite() || fit <= 0.0 { finish(&print_reply, Err("Invalid printer page dimensions".into())); return; }
                    settings.set_scale(settings.scale() * fit);
                    print_operation.set_print_settings(&settings); print_operation.set_page_setup(&page);
                    let result = print_commit(&mut || {
                        print_handed_off.store(true, Ordering::SeqCst);
                        print_operation.print();
                        Ok(())
                    });
                    if let Err(error) = result { finish(&print_reply, Err(error)); }
                });
                JOBS.with(|jobs| { if let Some(job) = jobs.borrow_mut().get_mut(&print_id) { job.dialog = Some(dialog.clone()); } });
                dialog.show_all();
            } else { operation.print(); }
        });
        JOBS.with(|jobs| { jobs.borrow_mut().insert(id, Job { window, view: view.clone(), operation: None, dialog: None, reply }); });
        view.load_uri(&bootstrap);
    }).map_err(|e| e.to_string())?;
    tokio::select! { result = receiver => result.map_err(|_| "Native GTK PDF provider stopped")?, _ = cancel.cancelled() => Err("Article PDF export was cancelled".into()) }
}
