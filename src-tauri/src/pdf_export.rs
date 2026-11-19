// SPDX-License-Identifier: GPL-3.0-or-later
//! Accepted PDF jobs own only immutable identity and cancellation while native
//! loading/system UI runs. Short commits use Mode -> Android -> Window -> Store.
use super::*;
use crate::native_pdf::{Authority, Commit, Outcome, Phase};
#[cfg(not(target_os = "android"))]
use tauri::Manager;
#[cfg(target_os = "ios")]
use std::{future::Future, pin::Pin};
use tokio_util::sync::CancellationToken;

fn same_live_document(base: &str, location: &str, actual: &str) -> Result<bool, String> {
    fn identity(mut url: url::Url) -> url::Url {
        // Missing-entry recovery is an actual live URL marker, not part of the
        // persisted archive location supplied by the shell's export action.
        if url.query_pairs().any(|(key, _)| key == "kiwix-miss") {
            let query = url.query().unwrap_or("").split('&').filter(|part| {
                !url::form_urlencoded::parse(part.as_bytes()).next()
                    .is_some_and(|(key, _)| key == "kiwix-miss")
            }).collect::<Vec<_>>().join("&");
            url.set_query(if query.is_empty() { None } else { Some(&query) });
        }
        url
    }
    let expected = url::Url::parse(&format!("{base}{}", location.trim_start_matches('/')))
        .map_err(|_| "Invalid live article URL")?;
    let actual = url::Url::parse(actual).map_err(|_| "Invalid live article URL")?;
    let actual = identity(actual);
    let expected = identity(expected);
    // Reader tabs use encode_path's strict percent encoding; archive-authored
    // navigation URLs may spell unreserved characters literally. Compare the
    // exact decoded libzim identity once, not serialized URL path spelling.
    Ok(actual.scheme() == expected.scheme()
        && actual.host_str() == expected.host_str()
        && actual.port_or_known_default() == expected.port_or_known_default()
        && actual.username().is_empty() && actual.password().is_none()
        && crate::server::decode_location_path(actual.path())? == crate::server::decode_location_path(expected.path())?
        && actual.query() == expected.query() && actual.fragment() == expected.fragment())
}

#[derive(Clone)]
struct Identity {
    label: String, book: String, installation: String, generation: u64, policy: u64,
    reader_origin: Option<String>,
    #[cfg(target_os = "android")]
    android: (String, i64),
}
pub(super) struct PdfJob {
    id: Uuid, identity: Arc<Identity>, cancel: CancellationToken,
    /// Share jobs keep the rendered PDF for the system sheet instead of
    /// publishing it to a user-selected destination.
    share: bool,
    /// Print jobs present the captured article in the platform print panel.
    print: bool,
    service: std::sync::Weak<Service>,
}
impl Drop for PdfJob {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(service) = self.service.upgrade() {
            service.pdf_jobs.lock().unwrap_or_else(|error| error.into_inner()).remove(&self.id);
            service.pdf_idle.notify_waiters();
        }
    }
}

#[cfg(target_os = "ios")]
struct PickerRetirement { service: std::sync::Weak<Service>, token: String, active: bool }
#[cfg(target_os = "ios")]
impl Drop for PickerRetirement {
    fn drop(&mut self) {
        if !self.active { return; }
        if let Some(service) = self.service.upgrade() {
            let token = self.token.clone();
            tauri::async_runtime::spawn(async move {
                let _ = service.mobile_call("cancelPdfExport", json!({"pdfAuthority":token})).await;
            });
        }
    }
}
impl Service {
    pub(super) fn cancel_pdf_jobs(&self, window: Option<&str>) {
        let jobs = self.pdf_jobs.lock().unwrap_or_else(|error| error.into_inner());
        for (label, cancel) in jobs.values() {
            if window.is_none_or(|window| window == label) { cancel.cancel(); }
        }
    }
    #[cfg(any(desktop, target_os = "ios", test))]
    pub(super) async fn drain_pdf_jobs(&self) {
        loop {
            let wake = self.pdf_idle.notified();
            if self.pdf_jobs.lock().unwrap_or_else(|error| error.into_inner()).is_empty() { return; }
            wake.await;
        }
    }
    pub(super) fn admit_pdf(self: &Arc<Self>, command: &str, args: &Value) -> Result<PdfJob, String> {
        let share = command == "share_pdf";
        let print = command == "print_pdf";
        #[cfg(mobile)]
        { if !matches!(command, "mobile_save_pdf" | "share_pdf") { return Err("Use the native mobile PDF export action".into()); } self.mobile_admit(command, args)?; }
        #[cfg(desktop)]
        if !matches!(command, "save_pdf" | "share_pdf" | "print_pdf") { return Err("Mobile PDF export is unavailable on this platform".into()); }
        // Every PDF command writes or hands off archive bytes: strict read-only
        // mode blocks save, share and print alike. Private mode is allowed.
        self.writable()?;
        self.ensure_running()?;
        let _mode = self.privacy_gate.try_read().map_err(|_| "The browsing policy is changing; try PDF export again")?;
        let generation = self.ensure_generation(args)?;
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main").to_string();
        if label != "main" && !label.starts_with("window-") { return Err("Open the article in the main window to export a PDF".into()); }
        #[cfg(target_os = "android")]
        let android = self.android_window.lock().map_err(|_| "The app window is unavailable. Reopen the app to continue")?;
        #[cfg(target_os = "android")]
        let source = if android.accepting {
            (android.token.clone().ok_or("This window is out of date. Reopen the app to continue")?, android.activity_id.ok_or("This window is out of date. Reopen the app to continue")?)
        } else { return Err("This window is out of date. Reopen the app to continue".into()); };
        let _window = self.active_window_guard(&label)?;
        let book = text(args, "bookId")?.to_string();
        let installation = text(args, "installationId")?.to_string();
        let identity = Arc::new(Identity { label: label.clone(), book: book.clone(), installation: installation.clone(), generation,
            policy: self.privacy_policy_generation.load(Ordering::SeqCst),
            reader_origin: None,
            #[cfg(target_os = "android")]
            android: source,
        });
        self.store.with_current_installation(&book, &installation, || {
            let id = Uuid::new_v4(); let cancel = CancellationToken::new();
            // Register before admission guards are released: a concurrent close
            // or policy writer cannot miss this newly accepted native job.
            self.pdf_jobs.lock().map_err(|_| "PDF export could not start. Try again")?.insert(id, (label, cancel.clone()));
            Ok(PdfJob { id, identity, cancel, share, print, service: Arc::downgrade(self) })
        })
    }
    fn pdf_commit<T>(&self, identity: &Identity, phase: Phase, cancel: &CancellationToken, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let _mode = self.privacy_gate.try_read().map_err(|_| "Your privacy settings changed during PDF export. Start the export again")?;
        self.pdf_check_locked(identity, phase, cancel, action)
    }
    fn pdf_check_locked<T>(&self, identity: &Identity, phase: Phase, cancel: &CancellationToken, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.ensure_running()?;
        if cancel.is_cancelled() { return Err("Article PDF export was cancelled".into()); }
        if self.privacy_policy_generation.load(Ordering::SeqCst) != identity.policy
            || (matches!(phase, Phase::BeforeHandoff) && self.privacy_generation.load(Ordering::SeqCst) != identity.generation)
        { return Err("Your privacy settings changed during PDF export. Start the export again".into()); }
        #[cfg(target_os = "android")]
        let android = self.android_window.lock().map_err(|_| "The app window is unavailable. Reopen the app to continue")?;
        #[cfg(target_os = "android")]
        if !android.accepting || android.token.as_deref() != Some(identity.android.0.as_str()) || android.activity_id != Some(identity.android.1) {
            return Err("The window you started this from is out of date. Reopen the app and try again".into());
        }
        let _reader = if matches!(phase, Phase::BeforeHandoff) {
            if let Some(origin) = &identity.reader_origin {
                let readers = self.readers.try_lock()
                    .map_err(|_| "The live article reader is changing; export it again")?;
                let current = readers.get(&format!("{}:{}", identity.label, identity.book));
                if current.is_none_or(|reader| reader.origin != *origin || reader.installation_id != identity.installation) {
                    return Err("The live article reader has retired; capture it again".into());
                }
                Some(readers)
            } else { None }
        } else { None };
        let _window = self.active_window_guard(&identity.label)?;
        self.store.with_current_installation(&identity.book, &identity.installation, action)
    }
    async fn bind_pdf_reader(&self, job: &mut PdfJob, args: &Value, snapshot: &crate::native_pdf::Snapshot) -> Result<(), String> {
        let readers = self.readers.lock().await;
        let reader = readers.get(&format!("{}:{}", job.identity.label, job.identity.book))
            .filter(|reader| reader.installation_id == job.identity.installation)
            .ok_or("The article was closed or changed. Open it again before exporting")?;
        if !same_live_document(&reader.base_url, text(args, "path")?, &snapshot.document_url)? {
            return Err("The live article changed during capture; export it again".into());
        }
        Arc::get_mut(&mut job.identity).ok_or("The PDF capture identity is already in use")?
            .reader_origin = Some(reader.origin.clone());
        Ok(())
    }
    pub(super) async fn export_pdf(self: Arc<Self>, args: Value, mut job: PdfJob) -> Result<Value, String> {
        let app = self.app.lock().map_err(|_| "Application handle unavailable")?.clone()
            .ok_or("Article PDF export requires a native application window")?;
        #[cfg(target_os = "macos")]
        let print_window = if job.print {
            Some(app.get_webview_window(&job.identity.label)
                .ok_or("The window you started printing from has closed")?)
        } else { None };
        let name = text(&args, "suggestedName")?.to_string();
        if name.is_empty() || name.len() > 240 || !name.to_ascii_lowercase().ends_with(".pdf")
            || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')) {
            return Err("Invalid PDF filename suggestion".into());
        }
        let snapshot: crate::native_pdf::Snapshot = serde_json::from_value(args.get("liveSnapshot").cloned()
            .ok_or("Capture the current article before exporting a PDF")?)
            .map_err(|error| format!("Kiwix could not capture the current article: {error}"))?;
        snapshot.validate()?;
        let location = crate::server::decode_location_path(text(&args, "path")?)?;
        self.bind_pdf_reader(&mut job, &args, &snapshot).await?;
        let source = self.clone(); let identity = job.identity.clone(); let cancellation = job.cancel.clone();
        blocking(move || {
            source.pdf_commit(&identity, Phase::BeforeHandoff, &cancellation, || Ok(()))?;
            let archive = source.archive(&identity.book)?;
            let entry = archive.entry(&location)?;
            if !crate::archive::is_html_mime(&entry.mime) { return Err("Article PDF export requires an HTML article".into()); }
            source.pdf_commit(&identity, Phase::BeforeHandoff, &cancellation, || Ok(()))
        }).await?;
        #[cfg(target_os = "android")]
        let cache = {
            use tauri_plugin_kiwix_mobile::MobileExt;
            // Android's /data/data and /data/user/0 can be bind aliases rather
            // than symlinks. Use the same native root as the export validator.
            let metadata = app.kiwix_mobile().call("storageRoot", json!({})).await?;
            let path = std::path::PathBuf::from(metadata.get("cachePath").and_then(Value::as_str)
                .ok_or("Native Android cache root is missing")?);
            if !path.is_absolute() || !path.is_dir() {
                return Err("Native Android cache root must be an existing absolute directory".into());
            }
            path
        };
        #[cfg(not(target_os = "android"))]
        let cache = app.path().app_cache_dir().map_err(|error| format!("Native PDF temporary storage is unavailable: {error}"))?;
        let prepare = cache.join("article-pdf");
        let root = blocking(move || {
            let mut builder = std::fs::DirBuilder::new(); builder.recursive(true);
            #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
            builder.create(&prepare).map_err(|error| format!("Creating PDF temporary storage: {error}"))?;
            let metadata = std::fs::symlink_metadata(&prepare).map_err(|error| error.to_string())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() { return Err("Unsafe PDF temporary storage directory".into()); }
            // Normalize only this trusted producer; caller source validation
            // continues to reject aliases, traversal and symlinks.
            std::fs::canonicalize(&prepare).map_err(|error| format!("Resolving PDF temporary storage: {error}"))
        }).await?;
        let weak = Arc::downgrade(&self); let identity = job.identity.clone(); let cancellation = job.cancel.clone();
        let authority: Authority = Arc::new(move |phase| {
            let service = weak.upgrade().ok_or("PDF export stopped because the app is closing")?;
            // Observation never blocks a mode writer. Only the final short
            // handoff/publication below acquires a mode read lease.
            service.pdf_check_locked(&identity, phase, &cancellation, || Ok(()))
        });
        #[cfg(target_os = "macos")]
        let print_authority = authority.clone();
        let weak = Arc::downgrade(&self); let identity = job.identity.clone(); let cancellation = job.cancel.clone();
        let commit: Commit = Arc::new(move |action| weak.upgrade().ok_or("PDF export stopped because the app is closing")?
            .pdf_commit(&identity, Phase::BeforeHandoff, &cancellation, action));
        let weak = Arc::downgrade(&self);
        let report: crate::native_pdf::Report = Arc::new(move |message| {
            eprintln!("Kiwix PDF cleanup: {message}");
            if let Some(service) = weak.upgrade() {
                if let Ok(mut warnings) = service.startup_warnings.lock() {
                    if warnings.len() < 64 { warnings.push(message.chars().take(1024).collect()); }
                }
                service.notify_library_changed();
            }
        });
        let destination = if job.share { crate::native_pdf::Destination::Share }
            else if job.print && !cfg!(target_os = "macos") { crate::native_pdf::Destination::Print }
            else { crate::native_pdf::Destination::Save };
        let presentation = crate::native_pdf::Presentation {
            name: name.clone(),
            #[cfg(target_os = "android")]
            labels: self.mobile_labels(if job.share { "Share" } else { "Save export" })?,
        };
        let outcome = crate::native_pdf::render(app.clone(), snapshot, root, presentation, destination, authority, commit.clone(), report, job.cancel.clone()).await?;
        match outcome {
            Outcome::SystemPrintOpened => Ok(json!({"systemPrintOpened":true})),
            Outcome::SystemShareOpened => Ok(json!({"systemShareOpened":true})),
            #[cfg(target_os = "android")]
            Outcome::Cancelled => Ok(json!({"cancelled":true})),
            Outcome::Pdf(pdf) if job.print => {
                // macOS prints the validated PDF from its originating window.
                // Other desktop providers already handed it off to system UI.
                #[cfg(target_os = "macos")]
                {
                    let window = print_window.ok_or("The window you started printing from has closed")?;
                    crate::native_pdf::print_pdf(window, &pdf, &name, print_authority, commit, job.cancel.clone()).await?;
                    drop(pdf);
                    // The panel closing does not prove paper was produced: the
                    // user may have cancelled or chosen Save as PDF.
                    Ok(json!({"printPanelClosed":true}))
                }
                #[cfg(not(target_os = "macos"))]
                { let _ = pdf; Err("The native print provider did not complete its system print operation".into()) }
            }
            Outcome::Pdf(pdf) if job.share => {
                #[cfg(target_os = "macos")]
                {
                    let path = pdf.path().to_path_buf();
                    // Recheck the accepted privacy/window identity immediately
                    // before a system share sheet receives archive bytes.
                    let service = self.clone(); let identity = job.identity.clone(); let cancel = job.cancel.clone();
                    blocking(move || service.pdf_commit(&identity, Phase::BeforeHandoff, &cancel, || Ok(()))).await?;
                    super::share::present_file(&path, job.cancel.clone()).await?;
                    drop(pdf);
                    Ok(Value::Null)
                }
                #[cfg(target_os = "ios")]
                { self.present_pdf(pdf, &name, commit, job.cancel.clone(), true).await }
                #[cfg(target_os = "android")]
                {
                    let _ = pdf;
                    Err("Kiwix could not share the article PDF. Try sharing it again".into())
                }
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                { let _ = pdf; Err("System sharing is unavailable on this platform".into()) }
            }
            Outcome::Pdf(pdf) => {
                #[cfg(desktop)]
                {
                    let destination = PathBuf::from(text(&args, "destination")?);
                    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
                    let prepared = blocking(move || crate::exports::prepare_pdf(pdf.path(), &destination)).await?;
                    let service = self.clone(); let identity = job.identity.clone(); let cancel = job.cancel.clone();
                    blocking(move || service.pdf_commit(&identity, Phase::BeforeHandoff, &cancel, ||
                        prepared.publish(overwrite))).await?;
                    Ok(Value::Null)
                }
                #[cfg(target_os = "ios")]
                { self.present_pdf(pdf, &name, commit, job.cancel.clone(), false).await }
                #[cfg(target_os = "android")]
                { let _ = pdf; Err("Android PDF export must use the native system print UI".into()) }
            }
        }
    }
    /// Present a rendered PDF through the iOS document picker (save) or the
    /// system share sheet (share), under the one-use native commit authority.
    #[cfg(target_os = "ios")]
    async fn present_pdf(self: &Arc<Self>, pdf: crate::native_pdf::OwnedPdf, name: &str, commit: Commit, cancel: CancellationToken, share: bool) -> Result<Value, String> {
        let admission = crate::native_pdf::admit_export(commit)?;
        let labels = self.mobile_labels(if share { "Share" } else { "Save export" })?;
        let payload = json!({"path":pdf.path(),"mimeType":"application/pdf","suggestedName":name,"open":share,"pdfAuthority":admission.token(),"labels":labels});
        let mut retirement = PickerRetirement { service: Arc::downgrade(self), token: admission.token().to_string(), active: true };
        let export = self.mobile_documents("exportFile", payload);
        tokio::pin!(export);
        let result = tokio::select! {
            result = &mut export => result,
            _ = cancel.cancelled() => {
                retire_pdf_picker(self, admission.token(), export.as_mut()).await;
                Err("Article PDF export was cancelled".into())
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(90)) => {
                retire_pdf_picker(self, admission.token(), export.as_mut()).await;
                Err("The native PDF export picker timed out".into())
            },
        };
        retirement.active = false;
        let result = result?;
        result.get("cancelled").and_then(Value::as_bool).ok_or("The native PDF picker returned no completion state")?;
        drop(pdf);
        Ok(Value::Null)
    }
}

/// Bound on the native picker cancellation and drain after the export stopped
/// waiting for it. A wedged provider must not hold this job (and its rendered
/// private temporary file) open.
#[cfg(target_os = "ios")]
const PICKER_CLEANUP: std::time::Duration = std::time::Duration::from_secs(5);

/// Best-effort retirement of a native picker the export no longer waits for.
/// The cancellation call and the pending export share one short timeout; when
/// the provider does not answer, the export future (and its IPC reply) is
/// dropped as soon as this returns.
#[cfg(target_os = "ios")]
async fn retire_pdf_picker<F: Future<Output = Result<Value, String>>>(
    service: &Arc<Service>,
    token: &str,
    mut export: Pin<&mut F>,
) {
    let cleanup = async {
        let _ = service
            .mobile_call("cancelPdfExport", json!({"pdfAuthority": token}))
            .await;
        let _ = export.as_mut().await;
    };
    let _ = tokio::time::timeout(PICKER_CLEANUP, cleanup).await;
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    struct Profile(PathBuf);
    impl Drop for Profile {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn setup() -> (Profile, Arc<Service>, Book) { setup_fixture("small.zim") }
    fn setup_fixture(fixture: &str) -> (Profile, Arc<Service>, Book) {
        let profile = Profile(std::env::temp_dir().join(format!("kiwix-pdf-identity-{}", Uuid::new_v4())));
        let service = Service::new(profile.0.clone(), true, false).unwrap();
        let book = crate::imports::load_book(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture).to_str().unwrap(), false, false,
        ).unwrap();
        service.store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
        (profile, service, book)
    }
    fn args(service: &Service, book: &Book) -> Value {
        json!({"bookId":book.metadata.id,"installationId":book.installation_id,
            "privacyGeneration":service.privacy_generation.load(Ordering::SeqCst),"__windowLabel":"main"})
    }
    #[test]
    fn pdf_commit_guards_mode_window_and_private_export_without_a_long_lease() {
        let (_profile, service, book) = setup();
        let job = service.admit_pdf("save_pdf", &args(&service, &book)).unwrap();
        assert!(service.privacy_gate.try_write().is_ok(), "admission must release its mode lease");
        service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel, || {
            assert!(service.privacy_gate.try_write().is_err());
            assert!(matches!(service.closed_windows.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
            Ok(())
        }).unwrap();
        let writer = service.privacy_gate.try_write().unwrap();
        assert!(service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel,
            || -> Result<(), String> { panic!("busy native UI handoff must fail closed") }).is_err());
        drop(writer);
        drop(job);
        assert!(service.pdf_jobs.lock().unwrap().is_empty());
    }
    #[test]
    fn print_pdf_admission_reuses_the_same_job_ownership() {
        let (_profile, service, book) = setup();
        let job = service.admit_pdf("print_pdf", &args(&service, &book)).unwrap();
        assert!(job.print && !job.share);
        assert_eq!(service.pdf_jobs.lock().unwrap().len(), 1);
        drop(job);
        assert!(service.pdf_jobs.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn live_pdf_accepts_real_open_article_and_redirected_browser_path_spellings() {
        let (_profile, service, book) = setup_fixture("main_path.zim");
        let opened = service.clone().dispatch("open_article", json!({
            "bookId":book.metadata.id,"parentOrigin":"tauri://localhost","__windowLabel":"main"
        })).await.unwrap();
        let location = format!("{}?q=a%20b#section%23literal", opened["path"].as_str().unwrap());
        let opened = service.clone().dispatch("open_article", json!({
            "bookId":book.metadata.id,"path":location,"parentOrigin":"tauri://localhost","__windowLabel":"main"
        })).await.unwrap();
        let mut browser_url = url::Url::parse(opened["url"].as_str().unwrap()).unwrap();
        // Archive-authored links leave '.' literal; the Reader tab canonicalizes
        // it back to %2E. Exercise the actual server with that browser spelling.
        browser_url.set_path(&browser_url.path().replace("%2E", "."));
        let client = reqwest::Client::builder().no_proxy()
            .resolve(browser_url.host_str().unwrap(), std::net::SocketAddr::from(([127, 0, 0, 1], browser_url.port().unwrap())))
            .timeout(std::time::Duration::from_secs(10)).build().unwrap();
        let response = client.get(browser_url.clone()).header("Sec-Fetch-Dest", "iframe").send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let mut actual = response.url().clone();
        actual.set_fragment(browser_url.fragment());
        let mut snapshot = crate::native_pdf::Snapshot { html: response.text().await.unwrap(), width: 390, height: 800, document_url: actual.to_string() };
        let mut request = args(&service, &book);
        request["path"] = opened["path"].clone();
        let mut job = service.admit_pdf("save_pdf", &request).unwrap();
        service.bind_pdf_reader(&mut job, &request, &snapshot).await.unwrap();
        service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel, || Ok(())).unwrap();
        for changed in [
            { let mut url = actual.clone(); url.set_query(Some("q=other")); url },
            { let mut url = actual.clone(); url.set_fragment(Some("other")); url },
            { let mut url = actual.clone(); url.set_port(Some(if url.port() == Some(65535) { 65534 } else { url.port().unwrap() + 1 })).unwrap(); url },
            { let mut url = actual.clone(); url.set_path(&url.path().replace("%3F", "%253F")); url },
            { let mut url = actual.clone(); url.set_username("archive").unwrap(); url },
        ] {
            snapshot.document_url = changed.to_string();
            assert!(service.bind_pdf_reader(&mut job, &request, &snapshot).await.is_err(), "{changed}");
        }
        // A live missing-entry navigation really redirects to the main article
        // with kiwix-miss. Use that final response URL, not a hand-built base.
        let missing = url::Url::parse(opened["origin"].as_str().unwrap()).unwrap().join("/A/Missing.html").unwrap();
        let redirected = client.get(missing).header("Sec-Fetch-Dest", "iframe").send().await.unwrap();
        assert_eq!(redirected.status(), reqwest::StatusCode::OK);
        assert!(redirected.url().query_pairs().any(|(key, _)| key == "kiwix-miss"));
        request["path"] = json!(crate::server::encode_path(&crate::server::decode_location_path(redirected.url().path()).unwrap()));
        snapshot.document_url = redirected.url().to_string();
        snapshot.html = redirected.text().await.unwrap();
        service.bind_pdf_reader(&mut job, &request, &snapshot).await.unwrap();
        service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel, || Ok(())).unwrap();
        drop(job);
        service.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn live_pdf_pins_the_real_reader_instance_until_native_commit() {
        let (_profile, service, book) = setup();
        let opened = service.clone().dispatch("open_article", json!({
            "bookId":book.metadata.id,"parentOrigin":"tauri://localhost","__windowLabel":"main"
        })).await.unwrap();
        let snapshot = crate::native_pdf::Snapshot { html: "<p>live</p>".into(), width: 390, height: 800,
            document_url: opened["url"].as_str().unwrap().into() };
        let mut request = args(&service, &book); request["path"] = opened["path"].clone();
        let mut job = service.admit_pdf("save_pdf", &request).unwrap();
        service.bind_pdf_reader(&mut job, &request, &snapshot).await.unwrap();
        // Confirmation holds this current-reader lease through the small
        // irreversible handoff, excluding concurrent listener replacement.
        service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel, || {
            assert!(service.readers.try_lock().is_err()); Ok(())
        }).unwrap();
        let fresh = service.clone().dispatch("open_article", json!({
            "bookId":book.metadata.id,"parentOrigin":"tauri://localhost","__windowLabel":"main","freshOrigin":true
        })).await.unwrap();
        assert_ne!(opened["origin"], fresh["origin"]);
        assert!(service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel,
            || -> Result<(), String> { panic!("A retired reader cannot hand off archive bytes") }).is_err());
        assert!(service.bind_pdf_reader(&mut job, &request, &snapshot).await.is_err());
        // OS-owned handoffs retain existing background-retirement semantics,
        // but policy and installation authority still cannot be renewed.
        service.pdf_commit(&job.identity, Phase::AfterHandoff, &job.cancel, || Ok(())).unwrap();
        service.privacy_generation.fetch_add(1, Ordering::SeqCst);
        assert!(service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel, || Ok(())).is_err());
        service.privacy_generation.fetch_sub(1, Ordering::SeqCst);
        service.store.update(|state| { state.books[0].installation_id = crate::store::new_installation_id(); Ok(()) }).unwrap();
        assert!(service.pdf_commit(&job.identity, Phase::AfterHandoff, &job.cancel, || Ok(())).is_err());
        drop(job);
        service.shutdown().await.unwrap();
    }
    #[test]
    fn pdf_handoff_distinguishes_background_epoch_from_explicit_policy_and_cancellation() {
        let (_profile, service, book) = setup();
        let job = service.admit_pdf("save_pdf", &args(&service, &book)).unwrap();
        service.privacy_generation.fetch_add(1, Ordering::SeqCst);
        assert!(service.pdf_commit(&job.identity, Phase::BeforeHandoff, &job.cancel, || Ok(())).is_err());
        service.pdf_commit(&job.identity, Phase::AfterHandoff, &job.cancel, || Ok(())).unwrap();
        service.privacy_policy_generation.fetch_add(1, Ordering::SeqCst);
        assert!(service.pdf_commit(&job.identity, Phase::AfterHandoff, &job.cancel, || Ok(())).is_err());
        service.privacy_policy_generation.fetch_sub(1, Ordering::SeqCst);
        job.cancel.cancel();
        assert!(service.pdf_commit(&job.identity, Phase::AfterHandoff, &job.cancel, || Ok(())).is_err());
    }
    #[test]
    fn pdf_source_replacement_and_untrusted_renderer_cannot_regain_export_authority() {
        let (_profile, service, book) = setup();
        let job = service.admit_pdf("save_pdf", &args(&service, &book)).unwrap();
        service.store.update(|state| { state.books[0].installation_id = crate::store::new_installation_id(); Ok(()) }).unwrap();
        assert!(service.pdf_commit(&job.identity, Phase::AfterHandoff, &job.cancel, || Ok(())).is_err());
        let current = service.store.book(&book.metadata.id).unwrap().unwrap();
        let mut request = args(&service, &current);
        request["__windowLabel"] = json!("pdf-render-untrusted");
        assert!(service.admit_pdf("save_pdf", &request).is_err());
        service.cancel_pdf_jobs(Some("main"));
        assert!(job.cancel.is_cancelled());
    }
}
