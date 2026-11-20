// SPDX-License-Identifier: GPL-3.0-or-later
//! Native live-article PDF export. Providers receive only a bounded, sanitized
//! frame snapshot in an isolated origin, never reader cookies or shell DOM.
use live::Document;
use axum::{body::{Body, Bytes}, extract::State, http::{HeaderMap, Method, StatusCode, Uri}, response::{IntoResponse, Response}, routing::any, Router};
use futures_util::StreamExt;
use tokio::io::AsyncReadExt;
use std::{collections::HashMap, fs, io::Read, path::{Path, PathBuf}, sync::{Arc, LazyLock, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
mod cleanup;
mod live;
pub use live::Snapshot;
mod presentation;
pub use presentation::Presentation;
#[cfg(any(desktop, target_os = "ios", test))]
pub use cleanup::retry_cleanup;
pub use cleanup::Report;
pub(crate) use cleanup::Files;
#[cfg(test)]
#[path = "native_pdf/origin_tests.rs"]
mod origin_tests;
#[cfg(target_os = "ios")]
mod handoff;
#[cfg(target_os = "ios")]
pub use handoff::admit_export;

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "native_pdf/apple.rs"]
mod provider;
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[path = "native_pdf/desktop.rs"]
mod provider;
#[cfg(target_os = "android")]
#[path = "native_pdf/android.rs"]
mod provider;
/// macOS-only: present an already-rendered article PDF in the system print panel.
#[cfg(target_os = "macos")]
pub use provider::print as print_pdf;

#[derive(Clone, Copy)]
pub enum Phase { BeforeHandoff, AfterHandoff }
/// The caller captures window, installation and privacy authority, not mutable
/// renderer arguments. AfterHandoff may ignore reader-background retirement,
/// but MUST still reject policy changes, installation replacement and closure.
pub type Authority = Arc<dyn Fn(Phase) -> Result<(), String> + Send + Sync>;
/// Execute the small irreversible native handoff while the caller holds its
/// Mode -> Window -> Store retirement guards. The action must not reenter Rust.
pub type Commit = Arc<dyn Fn(&mut dyn FnMut() -> Result<(), String>) -> Result<(), String> + Send + Sync>;
pub enum Outcome {
    Pdf(OwnedPdf),
    // Apple returns an owned PDF for its separate native handoff; other providers
    // report system UI through this common caller-facing outcome.
    #[cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]
    SystemPrintOpened,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    SystemShareOpened,
    #[cfg(target_os = "android")]
    Cancelled,
}
/// Where the caller intends to deliver the captured article.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Destination { Save, Share, Print }
pub struct OwnedPdf {
    files: Files,
    path: PathBuf,
    #[cfg(target_os = "macos")]
    validated: Option<(fs::File, crate::integrity::ContentIdentity)>,
}
#[cfg(any(desktop, target_os = "ios", test))]
impl OwnedPdf { pub fn path(&self) -> &Path { &self.path } }

impl OwnedPdf {
    fn create(root: &Path, report: Report, admission: tokio::sync::OwnedSemaphorePermit) -> Result<Self, String> {
        if crate::fs_ext::available_space(root).map_err(|e| format!("Checking PDF temporary storage: {e}"))? < 80 * 1024 * 1024 {
            return Err("Not enough temporary storage for article PDF export".into());
        }
        let directory = root.join(format!("pdf-{}", Uuid::new_v4()));
        #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
        builder.create(&directory).map_err(|e| format!("Creating private PDF directory: {e}"))?;
        let files = Files::new(directory.clone(), report, Some(admission));
        #[cfg(target_os = "ios")]
        {
            use std::os::unix::ffi::OsStrExt;
            unsafe extern "C" { fn kiwix_pdf_exclude_scratch(path: *const std::ffi::c_char) -> bool; }
            let path = std::ffi::CString::new(directory.as_os_str().as_bytes()).map_err(|_| "Invalid PDF scratch directory")?;
            // SAFETY: `path` is a live NUL-terminated C string for the call;
            // native reads it synchronously and retains no pointer.
            if !unsafe { kiwix_pdf_exclude_scratch(path.as_ptr()) } {
                return Err("iOS could not exclude PDF temporary storage from backup".into());
            }
        }
        Ok(Self {
            path: directory.join("article.pdf"), files,
            #[cfg(target_os = "macos")]
            validated: None,
        })
    }
    #[cfg(target_os = "macos")]
    fn open_output(&self) -> Result<fs::File, String> {
        use rustix::fs::{open, Mode, OFlags};
        open(&self.path, OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK, Mode::empty())
            .map(fs::File::from).map_err(|error| format!("Opening the private article PDF: {error}"))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn validated_file(&self) -> Result<&fs::File, String> {
        self.validated.as_ref().map(|(file, _)| file).ok_or_else(|| "The article PDF was not validated".into())
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn share_source_unchanged(&self) -> Result<(), String> {
        use crate::integrity::ContentIdentity;
        let (held, expected) = self.validated.as_ref().ok_or("The article PDF was not validated")?;
        let named = self.open_output()?;
        if ContentIdentity::from_handles(std::iter::once(held))? != *expected
            || ContentIdentity::from_handles(std::iter::once(&named))? != *expected
        {
            return Err("The article PDF changed before sharing. Export it again".into());
        }
        Ok(())
    }

    fn validate(&mut self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| format!("Reading native PDF: {e}"))?;
        if !metadata.is_file() || metadata.len() < 8 || metadata.len() > 64 * 1024 * 1024 {
            return Err("Native PDF is missing, invalid, or exceeds the 64 MiB output limit".into());
        }
        #[cfg(target_os = "macos")]
        let mut file = self.open_output()?;
        #[cfg(not(target_os = "macos"))]
        let mut file = fs::File::open(&self.path).map_err(|e| e.to_string())?;
        let held = file.metadata().map_err(|e| format!("Reading private PDF metadata: {e}"))?;
        if !held.is_file() || held.len() < 8 || held.len() > 64 * 1024 * 1024 {
            return Err("The native provider did not produce a usable PDF document".into());
        }
        #[cfg(target_os = "macos")]
        let identity = {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if held.nlink() != 1 { return Err("The article PDF has an unexpected file link".into()); }
            file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
            let identity = crate::integrity::ContentIdentity::from_handles(std::iter::once(&file))?;
            let size = file.metadata().map_err(|e| e.to_string())?.len();
            if !(8..=64 * 1024 * 1024).contains(&size) {
                return Err("The native provider did not produce a usable PDF document".into());
            }
            identity
        };
        let mut header = [0; 5];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if &header != b"%PDF-" { return Err("The native provider did not produce a PDF document".into()); }
        #[cfg(all(unix, not(target_os = "macos")))] { use std::os::unix::fs::PermissionsExt; file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?; }
        #[cfg(target_os = "macos")]
        {
            self.validated = Some((file, identity));
            self.share_source_unchanged()?;
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod share_identity_tests {
    use super::*;

    #[test]
    fn validated_pdf_rejects_path_replacement_before_sharing() {
        let root = tempfile::tempdir().unwrap();
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = admission.try_acquire_owned().unwrap();
        let mut pdf = OwnedPdf::create(root.path(), Arc::new(|error| panic!("{error}")), permit).unwrap();
        fs::write(pdf.path(), b"%PDF-fixture").unwrap();
        pdf.validate().unwrap();
        pdf.share_source_unchanged().unwrap();
        fs::rename(pdf.path(), pdf.path().with_extension("previous")).unwrap();
        fs::write(pdf.path(), b"%PDF-fixture").unwrap();
        assert!(pdf.share_source_unchanged().is_err());
    }
}

#[derive(Default)]
struct ResourceFailure {
    failed: AtomicBool,
    first: Mutex<Option<String>>,
}
impl ResourceFailure {
    fn record(&self, category: &str, path: &str) {
        let mut first = self.first.lock().unwrap_or_else(|error| error.into_inner());
        if first.is_none() {
            // Bootstrap paths carry native-only credentials. Diagnostics expose
            // only bounded logical archive paths, never that capability URL.
            let logical = if path.starts_with("/.kiwix-pdf-") {
                "<article bootstrap>".to_string()
            } else { path.chars().take(240).collect() };
            *first = Some(format!("A required archive resource could not be rendered ({category}: {logical}); no PDF was saved"));
        }
        self.failed.store(true, Ordering::SeqCst);
    }
    fn message(&self) -> String {
        self.first.lock().unwrap_or_else(|error| error.into_inner()).clone()
            .unwrap_or_else(|| "A required archive resource could not be rendered; no PDF was saved".into())
    }
    fn prefer_error(&self, native_error: String) -> String {
        if self.failed.load(Ordering::SeqCst) { self.message() } else { native_error }
    }
}

#[derive(Clone)]
struct OriginState {
    host: String, cookie: String, secret: String, bootstrap: String, root: String,
    resources: Arc<HashMap<String, (String, PathBuf, u64)>>,
    streams: Arc<tokio::sync::Semaphore>,
    files: Files,
    started: Arc<AtomicBool>, failure: Arc<ResourceFailure>, cancel: CancellationToken,
}
struct Origin { state: OriginState, url: String, _task: tokio::task::JoinHandle<()> }
impl Drop for Origin {
    fn drop(&mut self) {
        // The supervisor drains independently, retaining its Files lease.
        // Cancellation-aware sockets close stalled consumers; aborting only
        // the supervisor would leave Axum connection tasks orphaned.
        self.state.cancel.cancel();
    }
}
impl Origin {
    async fn start(document: Document, cancel: CancellationToken, files: Files) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| e.to_string())?;
        let host = listener.local_addr().map_err(|e| e.to_string())?.to_string();
        let id = Uuid::new_v4().simple().to_string();
        let secret = Uuid::new_v4().simple().to_string();
        let bootstrap = format!("/.kiwix-pdf-{id}-{secret}");
        let state = OriginState { host, cookie: format!("kiwix_pdf_{id}"), secret, bootstrap,
            root: document.root, resources: Arc::new(document.resources),
            streams: Arc::new(tokio::sync::Semaphore::new(4)),
            files,
            started: Arc::new(AtomicBool::new(false)), failure: Arc::new(ResourceFailure::default()), cancel };
        let url = format!("http://{}{}", state.host, state.bootstrap);
        let router = Router::new().fallback(any(serve)).with_state(state.clone());
        let stop = state.cancel.clone();
        let listener = crate::server::ReaderListener::new(listener, stop.clone());
        let failure = state.failure.clone();
        let task = tokio::spawn(async move {
            if axum::serve(listener, router).with_graceful_shutdown(stop.cancelled_owned()).await.is_err() {
                failure.record("native origin stopped", "<article origin>");
            }
        });
        Ok(Self { state, url, _task: task })
    }
}
async fn serve(State(state): State<OriginState>, method: Method, headers: HeaderMap, uri: Uri) -> Response {
    if state.cancel.is_cancelled() || method != Method::GET || headers.get("host").and_then(|h| h.to_str().ok()) != Some(state.host.as_str()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = if uri.path() == state.bootstrap && !state.started.swap(true, Ordering::SeqCst) {
        let mut response = StatusCode::SEE_OTHER.into_response();
        response.headers_mut().insert("location", state.root.parse().expect("encoded archive path"));
        response.headers_mut().insert("set-cookie", format!("{}={}; Path=/; HttpOnly; SameSite=Strict", state.cookie, state.secret).parse().expect("UUID cookie"));
        response
    } else {
        let cookie = format!("{}={}", state.cookie, state.secret);
        let authenticated = headers.get("x-kiwix-pdf").and_then(|h| h.to_str().ok()) == Some(state.secret.as_str())
            || headers.get("cookie").and_then(|h| h.to_str().ok()).is_some_and(|cookies| cookies.split(';').any(|part| part.trim() == cookie));
        if !authenticated { return StatusCode::FORBIDDEN.into_response(); }
        let identity = match crate::server::decode_location_path(uri.path()) {
            Ok(identity) => identity,
            Err(_) => {
                state.failure.record("invalid archive resource path", uri.path());
                return StatusCode::BAD_REQUEST.into_response();
            }
        };
        match state.resources.get(&identity) {
            Some((mime, path, size)) => {
                let permit = tokio::select! {
                    result = state.streams.clone().acquire_owned() => match result {
                        Ok(permit) => permit,
                        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                    },
                    _ = state.cancel.cancelled() => return StatusCode::GONE.into_response(),
                };
                let file = match tokio::fs::File::open(path).await {
                    Ok(file) => file,
                    Err(_) => { state.failure.record("The article image could not be read. Try the export again", uri.path()); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
                };
                let failure = state.failure.clone();
                let files = state.files.clone();
                let data = tokio_util::io::ReaderStream::with_capacity(file.take(*size), 64 * 1024)
                    .map(move |result| { if result.is_err() { failure.record("The article image stopped loading. Try the export again", "<article image>"); } result })
                    .chain(futures_util::stream::once(async move { drop(permit); drop(files); Ok::<Bytes, std::io::Error>(Bytes::new()) }));
                let mut response = Body::from_stream(data).into_response();
                response.headers_mut().insert("content-length", size.to_string().parse().unwrap());
                match mime.parse() { Ok(value) => { response.headers_mut().insert("content-type", value); }, Err(_) => { state.failure.record("invalid resource media type", uri.path()); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); } }
                response
            }
            None => { state.failure.record("missing archive resource", uri.path()); StatusCode::NOT_FOUND.into_response() }
        }
    };
    let out = response.headers_mut();
    out.insert("cache-control", "no-store, max-age=0".parse().unwrap());
    out.insert("referrer-policy", "no-referrer".parse().unwrap());
    out.insert("x-content-type-options", "nosniff".parse().unwrap());
    // Live assets are self-contained. The isolated renderer cannot fetch the
    // reader origin, application DOM, archive endpoints or external network.
    out.insert("content-security-policy", "default-src 'none'; script-src 'none'; style-src 'unsafe-inline' data:; img-src data:; font-src data:; media-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-same-origin".parse().unwrap());
    response
}

/// Finite application ownership, including Android's system-preview adapter.
/// Android owns the destination/spool and can retain bytes already handed off;
/// this API does not claim to retract them or report final PDF publication.
// Export boundary carries independently captured presentation, authority/commit
// guards, cleanup reporting and cancellation; none may be inferred from the DOM.
#[allow(clippy::too_many_arguments)]
pub async fn render(app: tauri::AppHandle, snapshot: Snapshot, scratch_root: PathBuf, presentation: Presentation, destination: Destination, authority: Authority, commit: Commit, report: Report, parent_cancel: CancellationToken) -> Result<Outcome, String> {
    // Only Apple platforms and Android implement a system share handoff.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if destination == Destination::Share {
        return Err("System sharing is unavailable on this platform".into());
    }
    static ADMISSION: LazyLock<Arc<tokio::sync::Semaphore>> = LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));
    let permit = ADMISSION.clone().try_acquire_owned().map_err(|_| "Another article PDF export is still running or finishing")?;
    authority(Phase::BeforeHandoff)?;
    let system_ui = destination == Destination::Print || cfg!(target_os = "android");
    let expires = tokio::time::Instant::now() + Duration::from_secs(if system_ui { 16 * 60 } else { 90 });
    let cancel = parent_cancel.child_token();
    struct CancelOnDrop(CancellationToken);
    impl Drop for CancelOnDrop { fn drop(&mut self) { self.0.cancel(); } }
    let _cancel_on_drop = CancelOnDrop(cancel.clone());
    let read_cancel = cancel.clone();
    // Snapshot capture has already completed in the authenticated live frame.
    // No archive reload can erase interactive state between capture and export.
    let mut owned = OwnedPdf::create(&scratch_root, report, permit)?;
    let snapshot_files = owned.files.clone();
    let width = snapshot.width;
    let height = snapshot.height;
    let mut worker = tokio::task::spawn_blocking(move || snapshot.document(snapshot_files.directory(), &read_cancel));
    let mut capture_checks = tokio::time::interval(Duration::from_millis(250));
    let document = loop {
        tokio::select! {
            result = &mut worker => break result.map_err(|e| format!("PDF resource worker stopped: {e}"))??,
            _ = cancel.cancelled() => {
                let _ = worker.await;
                return Err("Article PDF export was cancelled".into());
            }
            _ = capture_checks.tick() => {
                let error = if tokio::time::Instant::now() >= expires {
                    Some("Article PDF export timed out".to_string())
                } else { authority(Phase::BeforeHandoff).err() };
                if let Some(error) = error {
                    cancel.cancel();
                    // Join accepted libzim work before retiring its archive.
                    let _ = worker.await;
                    return Err(error);
                }
            }
        }
    };
    authority(Phase::BeforeHandoff)?;
    let origin = Origin::start(document, cancel.clone(), owned.files.clone()).await?;
    let handed_off = Arc::new(AtomicBool::new(false));
    let future = provider::render(app, origin.url.clone(), format!("http://{}{}", origin.state.host, origin.state.root), origin.state.secret.clone(), owned.path.clone(), presentation, destination, width, height, authority.clone(), commit, handed_off.clone(), owned.files.clone(), cancel.clone());
    tokio::pin!(future);
    let deadline = tokio::time::sleep_until(expires);
    tokio::pin!(deadline);
    let mut checks = tokio::time::interval(Duration::from_millis(250));
    let result = loop {
        tokio::select! {
            result = &mut future => break result,
            _ = cancel.cancelled() => break Err("Article PDF export was cancelled".into()),
            _ = &mut deadline => break Err("Article PDF export timed out".into()),
            _ = checks.tick() => {
                let phase = if handed_off.load(Ordering::SeqCst) { Phase::AfterHandoff } else { Phase::BeforeHandoff };
                if let Err(error) = authority(phase) { break Err(error); }
                if origin.state.failure.failed.load(Ordering::SeqCst) { break Err(origin.state.failure.message()); }
            }
        }
    };
    // A native fetcher can reject as soon as it receives HTTP failure. Keep
    // the already-recorded precise archive failure instead of losing it to a
    // generic platform error; sample before cancellation changes socket state.
    let result = result.map_err(|error| origin.state.failure.prefer_error(error));
    cancel.cancel();
    #[cfg(target_os = "android")]
    let completion = result?;
    #[cfg(not(target_os = "android"))]
    result?;
    if origin.state.failure.failed.load(Ordering::SeqCst) { return Err(origin.state.failure.message()); }
    #[cfg(target_os = "android")]
    if matches!(completion, provider::Completion::Cancelled) {
        drop(owned);
        return Ok(Outcome::Cancelled);
    }
    #[cfg(target_os = "android")]
    if destination == Destination::Share { drop(owned); return Ok(Outcome::SystemShareOpened); }
    // Print providers own the spool and report only system-UI completion, not
    // that paper or a user-selected PDF destination was actually produced.
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "android"))]
    if destination == Destination::Print || (cfg!(target_os = "android") && destination == Destination::Save) {
        drop(owned);
        return Ok(Outcome::SystemPrintOpened);
    }
    authority(Phase::BeforeHandoff)?;
    owned.validate()?;
    Ok(Outcome::Pdf(owned))
}
