// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Read-only archive origins. No privileged application command is exposed here.
//! Retirement closes network IO and joins archive workers before releasing providers.
use crate::archive::{is_html_mime, Archive, EntryLookup, MAX_READ_LENGTH};
use crate::store::Store;
#[cfg(any(desktop, test))]
use axum::response::IntoResponse;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::Response,
    routing::any,
    Router,
};
use futures_util::{future::poll_fn, stream, FutureExt};
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    io,
    net::SocketAddr,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, Weak},
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use uuid::Uuid;

const BRIDGE: &str = include_str!("../../src/reader-bridge.js");
const STORAGE_BRIDGE: &str = include_str!("../../src/storage-bridge.js");

pub fn encode_path(path: &str) -> String {
    encode_path_with(path, NON_ALPHANUMERIC)
}

fn encode_path_with(path: &str, charset: &'static percent_encoding::AsciiSet) -> String {
    // Keep a canonical archive path inside this origin, including malformed or
    // unusual paths beginning with '/'. Never produce a scheme-relative URL.
    let leading = path.bytes().take_while(|byte| *byte == b'/').count();
    let mut encoded = String::with_capacity(path.len());
    for _ in 0..leading {
        encoded.push_str("%2F");
    }
    for (index, part) in path[leading..].split('/').enumerate() {
        if index > 0 {
            encoded.push('/');
        }
        encoded.extend(utf8_percent_encode(part, charset));
    }
    encoded
}

/// Split an application location into its encoded path and its optional
/// archive-authored suffix (`?query`, `#fragment`, or both).
///
/// A location is always `encode_path(libzim_path)` followed by that suffix, so a
/// literal `?` or `#` inside the libzim path is `%3F`/`%23` here and never looks
/// like a delimiter. Producers that hold a raw libzim path must encode it with
/// [`encode_path`] before building a location; splitting the raw path is what
/// made a `?`-title open the prefix article.
pub fn split_location(path: &str) -> (&str, &str) {
    let offset = path.find(['?', '#']).unwrap_or(path.len());
    (&path[..offset], &path[offset..])
}

/// Reject a `%` that does not introduce exactly two hexadecimal digits.
///
/// `percent_decode_str` passes malformed escapes through unchanged, which would
/// silently resolve a broken location as if its bytes were literal path text.
fn valid_percent_encoding(path: &str) -> bool {
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len()
            || !bytes[index + 1].is_ascii_hexdigit()
            || !bytes[index + 2].is_ascii_hexdigit()
        {
            return false;
        }
        index += 3;
    }
    true
}

/// Decode one percent-encoded archive path (already stripped of any location
/// suffix) exactly once.
fn decode_percent_path(path: &str) -> Result<String, String> {
    if !valid_percent_encoding(path) {
        return Err("Archive location contains a malformed percent escape".into());
    }
    percent_decode_str(path)
        .decode_utf8()
        .map(|path| path.into_owned())
        .map_err(|_| "Archive location is not valid UTF-8".into())
}

/// Application locations are URL-encoded archive-relative paths plus an optional
/// query/fragment. Decode only the path, exactly once, at the libzim boundary.
/// Leading `/` separators are not archive path text. Malformed escapes and
/// non-UTF-8 bytes are rejected instead of being resolved as a different
/// (prefix) entry.
pub fn decode_location_path(location: &str) -> Result<String, String> {
    let (path, _) = split_location(location);
    decode_percent_path(path.trim_start_matches('/'))
}
pub fn normalize_location(location: &str) -> Result<String, String> {
    let (_, suffix) = split_location(location);
    Ok(format!(
        "{}{suffix}",
        encode_path(&decode_location_path(location)?)
    ))
}

#[derive(Clone)]
struct ArchiveState {
    archive: Weak<Archive>,
    id: String,
    installation_id: String,
    active: Arc<std::sync::atomic::AtomicBool>,
    restricted: Arc<std::sync::atomic::AtomicBool>,
    origin: String,
    host: String,
    token: String,
    retirement_proof: String,
    parent_origin: Arc<Mutex<String>>,
    store: Arc<Store>,
    privacy_generation: Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    workers: Arc<ArchiveWorkers>,
    cancellation: CancellationToken,
}

pub struct ReaderServer {
    pub origin: String,
    pub base_url: String,
    pub installation_id: String,
    token: String,
    retirement_proof: String,
    restricted: Arc<std::sync::atomic::AtomicBool>,
    parent_origin: Arc<Mutex<String>>,
    storage: Arc<ReaderStorage>,
    workers: Arc<ArchiveWorkers>,
    cancellation: CancellationToken,
    server: Option<JoinHandle<Result<(), String>>>,
}

impl Drop for ReaderServer {
    fn drop(&mut self) {
        // Only signal here: the supervisor closes worker admission and joins
        // cleanup without synchronously waiting on any worker or registry lock.
        self.storage
            .active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.cancellation.cancel();
    }
}

/// Own blocking archive operations independently of their request futures.
///
/// Closing and registration share a lock. A cancelled drain leaves every handle
/// in the registry so another owner can still join libzim work before teardown.
#[derive(Default)]
pub(crate) struct ArchiveWorkers {
    state: Mutex<ArchiveWorkerState>,
    draining: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct ArchiveWorkerState {
    closed: bool,
    tasks: JoinSet<()>,
    failures: Vec<String>,
}

impl ArchiveWorkerState {
    fn completed(&mut self, result: Result<(), tokio::task::JoinError>) {
        if let Err(error) = result {
            self.failures.push(format!("Archive worker failed: {error}"));
        }
    }
}

impl ArchiveWorkers {
    fn lock(&self) -> MutexGuard<'_, ArchiveWorkerState> {
        self.state.lock().unwrap_or_else(|poisoned| {
            let mut state = poisoned.into_inner();
            state.closed = true;
            state.failures.push("Archive worker registry lock poisoned".into());
            self.state.clear_poison();
            state
        })
    }

    pub(crate) async fn run<T, F>(&self, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        self.spawn(None, work).await
    }

    /// Like `run`, but marks the blocking worker as serving one archive so a
    /// panic can record the reader phase and that archive's validation result.
    pub(crate) async fn run_for<T, F>(
        &self,
        archive_id: &str,
        installation_id: &str,
        operation: &str,
        work: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        self.spawn(
            Some((
                archive_id.to_owned(),
                installation_id.to_owned(),
                operation.to_owned(),
            )),
            work,
        )
        .await
    }

    async fn spawn<T, F>(&self, context: Option<(String, String, String)>, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self.lock();
            if state.closed {
                return Err("Reading stopped. Reload the article".into());
            }
            while let Some(result) = state.tasks.try_join_next() {
                state.completed(result);
            }
            state.tasks.spawn_blocking(move || {
                let _reading = context.as_ref().map(|(archive_id, installation_id, operation)| {
                    crate::crash::reading(archive_id, installation_id, operation)
                });
                let result = work();
                // A disconnected consumer does not transfer worker ownership.
                let _ = sender.send(result);
            });
        }
        receiver.await.map_err(|_| "Archive worker stopped".to_string())?
    }

    pub(crate) fn close(&self) {
        self.lock().closed = true;
    }

    pub(crate) async fn drain(&self) -> Result<(), String> {
        self.close();
        let _draining = self.draining.lock().await;
        poll_fn(|cx| {
            let mut state = self.lock();
            loop {
                match state.tasks.poll_join_next(cx) {
                    Poll::Ready(Some(result)) => state.completed(result),
                    Poll::Ready(None) => return Poll::Ready(()),
                    Poll::Pending => return Poll::Pending,
                }
            }
        })
        .await;
        let state = self.lock();
        if state.failures.is_empty() {
            Ok(())
        } else {
            Err(state.failures.join("; "))
        }
    }
}

/// Axum's graceful shutdown alone can wait forever for a stalled HTTP consumer.
/// Cancel every pending socket operation as well as stopping listener admission.
pub(crate) struct ReaderListener {
    listener: TcpListener,
    cancellation: CancellationToken,
}

impl ReaderListener {
    pub(crate) fn new(listener: TcpListener, cancellation: CancellationToken) -> Self {
        Self { listener, cancellation }
    }
}

impl axum::serve::Listener for ReaderListener {
    type Io = ReaderConnection;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, address) = axum::serve::Listener::accept(&mut self.listener).await;
        (
            ReaderConnection {
                stream: Some(stream),
                cancelled: Box::pin(self.cancellation.clone().cancelled_owned()),
            },
            address,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

pub(crate) struct ReaderConnection {
    stream: Option<TcpStream>,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl ReaderConnection {
    fn stream(&mut self, cx: &mut Context<'_>) -> io::Result<Pin<&mut TcpStream>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            drop(self.stream.take());
        }
        self.stream
            .as_mut()
            .map(Pin::new)
            .ok_or_else(|| io::Error::new(io::ErrorKind::ConnectionAborted, "Reading stopped. Reload the article"))
    }
}

impl AsyncRead for ReaderConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.get_mut().stream(cx)?.poll_read(cx, buffer)
    }
}

impl AsyncWrite for ReaderConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut().stream(cx)?.poll_write(cx, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().stream(cx)?.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.cancelled.as_mut().poll(cx).is_ready() {
            drop(this.stream.take());
            return Poll::Ready(Ok(()));
        }
        match this.stream.as_mut() {
            Some(stream) => Pin::new(stream).poll_shutdown(cx),
            None => Poll::Ready(Ok(())),
        }
    }
}

/// The native owner survives iframe navigation/removal. Only the authenticated
/// service can submit batches; archive HTTP origins expose no write endpoint.
struct ReaderStorage {
    installation_id: String,
    active: Arc<std::sync::atomic::AtomicBool>,
    store: Arc<Store>,
    privacy_generation: Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    documents: Mutex<HashMap<String, (u64, String)>>,
}

#[derive(Debug)]
pub(crate) enum ReaderStorageError {
    Retired,
    Rejected(String),
    Retryable(String),
}
impl std::fmt::Display for ReaderStorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retired => formatter.write_str("Reader privacy session expired"),
            Self::Rejected(message) | Self::Retryable(message) => formatter.write_str(message),
        }
    }
}
impl From<&str> for ReaderStorageError {
    fn from(message: &str) -> Self {
        Self::Rejected(message.into())
    }
}
impl From<String> for ReaderStorageError {
    fn from(message: String) -> Self {
        Self::Rejected(message)
    }
}

impl ReaderStorage {
    fn write_batch(&self, batch: &Value) -> Result<(), ReaderStorageError> {
        let object = batch.as_object().ok_or("Invalid storage mutation batch")?;
        if object.len() != 4 {
            return Err("Invalid storage mutation batch fields".into());
        }
        let uuid = |field| {
            batch
                .get(field)
                .and_then(Value::as_str)
                .filter(|id| Uuid::parse_str(id).is_ok())
                .ok_or_else(|| format!("Invalid storage {field}"))
        };
        let id = uuid("id")?;
        let document = uuid("documentId")?;
        let sequence = batch
            .get("sequence")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0 && *value <= 9_007_199_254_740_991)
            .ok_or("Invalid storage mutation sequence")?;
        let operations = batch
            .get("operations")
            .and_then(Value::as_array)
            .filter(|operations| !operations.is_empty() && operations.len() <= 10000)
            .ok_or("Invalid storage mutations")?;
        if storage_json_bytes(batch)? > 2 * 1024 * 1024 {
            return Err("Storage mutation batch is too large".into());
        }
        let mut documents = self.documents.lock().map_err(|_| {
            ReaderStorageError::Retryable("Storage mutation queue unavailable".into())
        })?;
        // Hold the sequence registry across the transaction so concurrent callers
        // cannot replay a clear or overtake an uncommitted predecessor.
        let mut failure = ReaderStorageError::Retryable("Storage persistence unavailable".into());
        self.store
            .update(|state| {
                if !self.active.load(std::sync::atomic::Ordering::SeqCst)
                    || self.generation
                        != self
                            .privacy_generation
                            .load(std::sync::atomic::Ordering::SeqCst)
                {
                    failure = ReaderStorageError::Retired;
                    return Err("Reader privacy session expired".into());
                }
                failure = ReaderStorageError::Rejected("Storage mutation rejected".into());
                if state.settings.read_only {
                    return Err("Reader storage is read-only".into());
                }
                if let Some((last, last_id)) = documents.get(document) {
                    if sequence < *last || (sequence == *last && id == last_id) {
                        failure =
                            ReaderStorageError::Retryable("Storage persistence unavailable".into());
                        return Ok(false);
                    }
                    if sequence != last + 1 {
                        return Err("Storage mutation sequence is out of order".into());
                    }
                } else if sequence != 1 || documents.len() >= 4096 {
                    return Err("Unknown or exhausted storage document stream".into());
                }
                let mut saved = state
                    .reader_storage
                    .remove(&self.installation_id)
                    .unwrap_or_else(|| json!({"localStorage":{},"cookies":[]}));
                apply_storage_mutations(&mut saved, operations)?;
                state
                    .reader_storage
                    .insert(self.installation_id.clone(), saved);
                // Validation has completed. A subsequent Store failure is durable IO,
                // not a malformed stream, and must keep the original batch retryable.
                failure = ReaderStorageError::Retryable("Storage persistence unavailable".into());
                Ok(true)
            })
            .map(|changed| {
                if changed {
                    documents.insert(document.into(), (sequence, id.into()));
                }
            })
            .map_err(|message| {
                // Runtime read-only/durability gates may reject before invoking
                // the closure. Consult the authoritative generation again rather
                // than classifying an English error string.
                if !self.active.load(std::sync::atomic::Ordering::SeqCst)
                    || self.generation
                        != self
                            .privacy_generation
                            .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return ReaderStorageError::Retired;
                }
                match failure {
                    ReaderStorageError::Retired => ReaderStorageError::Retired,
                    ReaderStorageError::Rejected(_) => ReaderStorageError::Rejected(message),
                    ReaderStorageError::Retryable(_) => ReaderStorageError::Retryable(message),
                }
            })
    }
}

/// Measure the wire quota without allocating another potentially MiB-sized copy.
fn storage_json_bytes(value: &Value) -> Result<usize, String> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|e| e.to_string())?;
    Ok(counter.0)
}

impl ReaderServer {
    pub async fn start(
        archive: Archive,
        id: String,
        installation_id: String,
        parent_origin: String,
        store: Arc<Store>,
        privacy_generation: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("Cannot start offline archive listener: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        // A new single-label hostname guarantees a new browser storage/permission
        // origin even if the OS reuses a port. Installation-scoped compatibility
        // storage restores deliberate preferences; metadata UUIDs are untrusted.
        let nonce = Uuid::new_v4().simple().to_string();
        let host = format!(
            "{}-{}.localhost:{port}",
            id.to_ascii_lowercase(),
            &nonce[..16]
        );
        let origin = format!("http://{host}");
        let token = Uuid::new_v4().simple().to_string();
        let retirement_proof = Uuid::new_v4().simple().to_string();
        let parent_origin = Arc::new(Mutex::new(parent_origin));
        let generation = privacy_generation.load(std::sync::atomic::Ordering::SeqCst);
        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let restricted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let workers = Arc::new(ArchiveWorkers::default());
        let cancellation = CancellationToken::new();
        let storage = Arc::new(ReaderStorage {
            installation_id: installation_id.clone(),
            active: active.clone(),
            store: store.clone(),
            privacy_generation: privacy_generation.clone(),
            generation,
            documents: Mutex::new(HashMap::new()),
        });
        // Router and response bodies hold only weak references. Axum may signal
        // connection completion just before its task-local service is destroyed.
        let archive = Arc::new(archive);
        let state = ArchiveState {
            archive: Arc::downgrade(&archive),
            id,
            installation_id: installation_id.clone(),
            active,
            origin: origin.clone(),
            host,
            token: token.clone(),
            retirement_proof: retirement_proof.clone(),
            restricted: restricted.clone(),
            parent_origin: parent_origin.clone(),
            store,
            privacy_generation,
            generation,
            workers: workers.clone(),
            cancellation: cancellation.clone(),
        };
        let app = Router::new()
            .fallback(any(archive_request))
            .layer(DefaultBodyLimit::max(1024 * 1024))
            .with_state(state);
        let listener = ReaderListener {
            listener,
            cancellation: cancellation.clone(),
        };
        let serving_workers = workers.clone();
        let serving_cancellation = cancellation.clone();
        let serving_active = storage.active.clone();
        // This supervisor also owns cleanup when ReaderServer is only dropped or
        // its explicit shutdown future is cancelled. Never abort blocking reads.
        let server = tokio::spawn(async move {
            let serving = AssertUnwindSafe(async {
                axum::serve(listener, app)
                    .with_graceful_shutdown(serving_cancellation.clone().cancelled_owned())
                    .await
            })
            .catch_unwind()
            .await;
            serving_active.store(false, std::sync::atomic::Ordering::SeqCst);
            serving_workers.close();
            serving_cancellation.cancel();
            let serving = match serving {
                Ok(result) => result.map_err(|error| format!("Offline archive server failed: {error}")),
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown panic");
                    Err(format!("Offline archive server panicked: {message}"))
                }
            };
            let workers = serving_workers.drain().await;
            drop(archive);
            match (serving, workers) {
                (Ok(()), result) | (result, Ok(())) => result,
                (Err(server), Err(workers)) => Err(format!("{server}; {workers}")),
            }
        });
        Ok(Self {
            base_url: format!("{origin}/"),
            origin,
            installation_id,
            token,
            retirement_proof,
            restricted,
            parent_origin,
            storage,
            workers,
            cancellation,
            server: Some(server),
        })
    }

    fn stop(&self) {
        self.storage
            .active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.workers.close();
        self.cancellation.cancel();
    }

    pub(crate) async fn shutdown(mut self) -> Result<(), String> {
        self.stop();
        match self.server.take() {
            Some(server) => match server.await {
                Ok(result) => result,
                Err(error) => {
                    // Also drain after a supervisor failure; no failed join may
                    // authorize releasing providers beneath running libzim work.
                    let failure = format!("Offline archive shutdown task failed: {error}");
                    match self.workers.drain().await {
                        Ok(()) => Err(failure),
                        Err(workers) => Err(format!("{failure}; {workers}")),
                    }
                }
            },
            None => self.workers.drain().await,
        }
    }

    pub fn retirement_url(&self) -> String {
        format!("{}/.kiwix/{}/retire.html", self.origin, self.token)
    }

    pub fn retirement_proof(&self) -> &str {
        &self.retirement_proof
    }

    pub fn set_parent_origin(&self, origin: String) -> Result<(), String> {
        *self
            .parent_origin
            .lock()
            .map_err(|_| "Reader origin lock unavailable")? = origin;
        Ok(())
    }

    /// Session-scoped. A restricted reader keeps its archive origin and bridge
    /// but serves documents whose only executable script is the injected,
    /// per-response nonce bootstrap; archive scripts, forms, workers and
    /// network requests are denied by CSP.
    pub fn set_restricted(&self, restricted: bool) {
        self.restricted
            .store(restricted, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_restricted(&self) -> bool {
        self.restricted.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn write_storage_batch(&self, batch: &Value) -> Result<(), ReaderStorageError> {
        self.storage.write_batch(batch)
    }
}

async fn archive_request(
    State(state): State<ArchiveState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let cancellation = state.cancellation.clone();
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => error(StatusCode::GONE, "Reading stopped. Reload the article"),
        response = archive_request_inner(state, method, uri, headers) => response,
    }
}

/// Resolve an archive path exactly first. Only a recoverable entry-not-found
/// miss may retry the one-step trailing-slash variant, preserving distinct
/// `path` and `path/` entries and all other archive failures.
///
/// Serving uses [`crate::archive::EntryLookup`] under one retained archive
/// acquisition; this copy remains the direct regression for that policy.
#[cfg(test)]
fn lookup_entry_with_slash_fallback<T>(
    path: &str,
    mut lookup: impl FnMut(&str) -> Result<T, String>,
) -> Result<T, String> {
    match lookup(path) {
        Ok(value) => Ok(value),
        Err(error) if crate::archive::is_entry_not_found(&error) => {
            let alternate = if path.is_empty() {
                None
            } else if let Some(trimmed) = path.strip_suffix('/') {
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            } else {
                Some(format!("{path}/"))
            };
            let Some(alternate) = alternate else { return Err(error); };
            match lookup(&alternate) {
                Ok(value) => Ok(value),
                Err(alternate_error) if crate::archive::is_entry_not_found(&alternate_error) => Err(error),
                Err(alternate_error) => Err(alternate_error),
            }
        }
        Err(error) => Err(error),
    }
}

async fn archive_request_inner(
    state: ArchiveState,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if !state.active.load(std::sync::atomic::Ordering::SeqCst) {
        return error(StatusCode::GONE, "Reading stopped. Reload the article");
    }
    if headers.get("host").and_then(|h| h.to_str().ok()) != Some(state.host.as_str()) {
        return error(StatusCode::FORBIDDEN, "Unknown archive host");
    }
    // The browser adds this forbidden request header for service-worker scripts.
    // A worker must never synthesize documents without our CSP/permission policy.
    if headers.contains_key("service-worker")
        || headers
            .get("sec-fetch-dest")
            .is_some_and(|h| h == "serviceworker")
    {
        return error(
            StatusCode::FORBIDDEN,
            "Archive-controlled service workers are not supported",
        );
    }
    if let Some(origin) = headers.get("origin").and_then(|h| h.to_str().ok()) {
        if origin != state.origin {
            return error(StatusCode::FORBIDDEN, "Foreign archive origin");
        }
    }
    let prefix = format!("/.kiwix/{}/", state.token);
    if let Some(resource) = uri.path().strip_prefix(&prefix) {
        return internal_request(state, method, resource, uri.query(), headers).await;
    }
    if method != Method::GET && method != Method::HEAD {
        return error(StatusCode::METHOD_NOT_ALLOWED, "Archives are read-only");
    }
    let path = match decode_percent_path(uri.path().trim_start_matches('/')) {
        Ok(path) => path,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Archive URL is not valid UTF-8"),
    };
    let archive = state.archive.clone();
    let lookup_path = path.clone();
    let cancellation = state.cancellation.clone();
    let archive_id = state.id.clone();
    let token = state.token.clone();
    let query = uri.query().map(str::to_owned);
    let request_headers = headers.clone();
    let request_method = method.clone();
    let document_request = is_document_request(&headers);
    // Resolve the entry, choose its response shape, and read every selected
    // byte inside one archive lock acquisition. Redirects and HEAD/304/416
    // decisions read nothing, and no path is looked up twice.
    let served = state
        .workers
        .run_for(&state.id, &state.installation_id, "read_article", move || {
            let archive = archive.upgrade().ok_or("Reading stopped. Reload the article")?;
            let lookup = if lookup_path.is_empty() {
                EntryLookup::MainOrRandom
            } else {
                EntryLookup::ExactOrTrailingSlash(&lookup_path)
            };
            archive.with_entry(lookup, &cancellation, |entry, reader| {
                let redirect = if lookup_path.is_empty() {
                    Some(format!("/{}", encode_path(&entry.path)))
                } else if entry.path != lookup_path {
                    Some(format!(
                        "/{0}{1}",
                        encode_path(&entry.path),
                        query
                            .as_deref()
                            .map(|q| format!("?{q}"))
                            .unwrap_or_default()
                    ))
                } else {
                    None
                };
                if let Some(target) = redirect {
                    return Ok(ServedEntry {
                        entry: entry.clone(),
                        index: reader.index(),
                        plan: ServedPlan::Redirect(target),
                    });
                }
                let plan = if is_html_mime(&entry.mime) {
                    // A large article must stay readable: inject the bridge
                    // from the first bounded prefix and stream the rest in
                    // 1 MiB archive ranges instead of refusing the document.
                    if entry.size > MAX_READ_LENGTH {
                        ServedPlan::HtmlPrefix(reader.read_range(0, entry.size.min(HTML_PREFIX))?)
                    } else {
                        ServedPlan::HtmlWhole(reader.read_range(0, entry.size)?)
                    }
                } else if document_request {
                    let mime = entry.mime.split(';').next().unwrap_or("");
                    if (mime.starts_with("text/")
                        || matches!(mime, "application/json" | "application/xml"))
                        && entry.size <= MAX_READ_LENGTH
                    {
                        match reader.read_range(0, entry.size) {
                            Ok(bytes) => ServedPlan::ResourceText(bytes),
                            Err(message) => ServedPlan::ResourceUnreadable(message),
                        }
                    } else {
                        ServedPlan::ResourceFallback
                    }
                } else {
                    ServedPlan::Raw(raw_plan(
                        reader,
                        entry,
                        &request_headers,
                        &request_method,
                        &archive_id,
                        &token,
                    )?)
                };
                Ok(ServedEntry {
                    entry: entry.clone(),
                    index: reader.index(),
                    plan,
                })
            })
        })
        .await;
    let ServedEntry { entry, index, plan } = match served {
        Ok(served) => served,
        Err(message) if message == "Archive worker stopped" => {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Archive worker stopped");
        }
        Err(message) => {
            // A document navigation must still reach the reader bridge: an
            // uninstrumented 404 leaves the frame inert until the shell's
            // watchdog fires. Redirect to the fallback document with the miss
            // marker so the shell can say so quietly.
            if crate::archive::is_entry_not_found(&message) && document_request {
                return missing_document(&state, method).await;
            }
            return archive_failure(&state, "read_article", &message);
        }
    };
    match plan {
        ServedPlan::Redirect(target) => Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header("Location", target)
            .body(Body::empty())
            .unwrap(),
        ServedPlan::HtmlWhole(bytes) => {
            if std::str::from_utf8(&bytes).is_err() {
                return reader_error_document(
                    &state,
                    method,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "This page could not be opened.",
                    "The article is not valid UTF-8 HTML.",
                );
            }
            let nonce = restricted_nonce(&state);
            let injection = bridge_injection(&state, &entry.mime, nonce.as_deref());
            let size = bytes.len() + injection.len();
            archive_response(
                &state,
                StatusCode::OK,
                "text/html; charset=utf-8",
                size as u64,
                None,
                None,
                nonce.as_deref(),
                if method == Method::HEAD {
                    Body::empty()
                } else {
                    injected_html_body(bytes, injection)
                },
            )
        }
        ServedPlan::HtmlPrefix(first) => {
            stream_html_document(state, entry, index, first, method).await
        }
        ServedPlan::ResourceText(bytes) => {
            resource_document(state, entry, method, Some(bytes))
        }
        ServedPlan::ResourceUnreadable(message) => {
            crate::diagnostics::record(&state.store, "read_article", Some(&message));
            reader_error_document(
                &state,
                method,
                StatusCode::UNPROCESSABLE_ENTITY,
                "This page could not be opened.",
                "The archive could not read this entry.",
            )
        }
        ServedPlan::ResourceFallback => resource_document(state, entry, method, None),
        ServedPlan::Raw(plan) => raw_response(&state, entry, index, method, plan),
        ServedPlan::RejectHtml => {
            error(StatusCode::BAD_REQUEST, "HTML documents must use the instrumented reader")
        }
    }
}

/// Browser metadata for a navigation that renders a document. Other
/// destinations (media, scripts, fetches) get plain status responses.
fn is_document_request(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-dest")
        .is_some_and(|value| value == "iframe" || value == "document")
}

/// Status mapping for a failed entry lookup. A missing entry is the archive's
/// content being absent; anything else is the archive failing to be read, which
/// must not masquerade as a 404.
pub(crate) fn archive_failure_status(message: &str) -> (StatusCode, &'static str) {
    if crate::archive::is_entry_not_found(message) {
        (StatusCode::NOT_FOUND, "Entry not found")
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "The archive could not read this entry",
        )
    }
}

fn archive_failure(state: &ArchiveState, operation: &str, message: &str) -> Response {
    let (status, body) = archive_failure_status(message);
    if status != StatusCode::NOT_FOUND {
        // Keep the real diagnostic without leaking libzim text to the document.
        crate::diagnostics::record(&state.store, operation, Some(message));
    }
    error(status, body)
}

/// The shell recognises this query marker on the fallback document and shows a
/// quiet navigation notice, then keeps the marker out of the tab location.
pub(crate) const MISS_MARKER: &str = "kiwix-miss=1";

/// A navigation to an absent entry still needs an instrumented document. The
/// fallback document is served through a marked redirect; when the archive has
/// no readable document at all, a bridge-bearing error page keeps the frame
/// responsive instead of leaving it inert.
async fn missing_document(state: &ArchiveState, method: Method) -> Response {
    let archive = state.archive.clone();
    let fallback = state
        .workers
        .run_for(&state.id, &state.installation_id, "read_article", move || {
            archive
                .upgrade()
                .ok_or("Reading stopped. Reload the article")?
                .main_or_random_entry()
        })
        .await;
    match fallback {
        Ok((entry, _)) => Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(
                "Location",
                format!("/{}?{MISS_MARKER}", encode_path(&entry.path)),
            )
            .body(Body::empty())
            .unwrap(),
        Err(message) if crate::archive::is_entry_not_found(&message) => {
            reader_error_document(
                state,
                method,
                StatusCode::NOT_FOUND,
                "This article is not in this archive.",
                "This archive has no articles to open.",
            )
        }
        Err(message) => archive_failure(state, "read_article", &message),
    }
}

fn restricted_nonce(state: &ArchiveState) -> Option<String> {
    state
        .restricted
        .load(std::sync::atomic::Ordering::SeqCst)
        .then(|| Uuid::new_v4().simple().to_string())
}

fn bridge_injection(state: &ArchiveState, mime: &str, nonce: Option<&str>) -> String {
    let prefix = format!("/.kiwix/{}/", state.token);
    let parent = state
        .parent_origin
        .lock()
        .map(|parent| parent.clone())
        .unwrap_or_default();
    // Reader appearance travels with the document so the bridge can paint the
    // first frame already themed. Waiting for the shell's post-`ready`
    // appearance message painted the archive's own light background and default
    // type on every navigation.
    let appearance = state.store.settings().map(|settings| {
        let mut attributes = String::new();
        if ["light", "sepia", "dark"].contains(&settings.reader_theme.as_str()) {
            attributes.push_str(&format!(
                " data-reader-theme=\"{}\"",
                escape_attribute(&settings.reader_theme)
            ));
        }
        if ["comfortable", "full"].contains(&settings.reader_width.as_str()) {
            attributes.push_str(&format!(
                " data-reader-width=\"{}\"",
                escape_attribute(&settings.reader_width)
            ));
        }
        if ["serif", "sans"].contains(&settings.font_family.as_str()) {
            attributes.push_str(&format!(
                " data-reader-font-family=\"{}\"",
                escape_attribute(&settings.font_family)
            ));
        }
        if (10.0..=48.0).contains(&settings.font_size) {
            attributes.push_str(&format!(
                " data-reader-font-size=\"{}\"",
                escape_attribute(&settings.font_size.to_string())
            ));
        }
        attributes
    });
    let appearance = appearance.unwrap_or_default();
    let nonce = nonce
        .map(|nonce| format!(" nonce=\"{}\"", escape_attribute(nonce)))
        .unwrap_or_default();
    format!(
        "<script src=\"{prefix}storage.js\"{nonce}></script><script src=\"{prefix}bridge.js\" data-parent-origin=\"{}\" data-entry-mime=\"{}\"{appearance}{nonce}></script>",
        escape_attribute(&parent),
        escape_attribute(mime)
    )
}

/// Share the already-read article buffer across the injection boundary instead
/// of allocating and copying the complete document a second time.
fn injected_html_body(bytes: Vec<u8>, injection: String) -> Body {
    let insert = html_head_offset(&bytes);
    let bytes = bytes::Bytes::from(bytes);
    Body::from_stream(stream::iter([
        bytes.slice(..insert),
        bytes::Bytes::from(injection),
        bytes.slice(insert..),
    ].map(Ok::<_, io::Error>)))
}

/// Bootstrap before executable archive markup, retaining the leading doctype
/// and its parsing mode. A raw `<head` search can match inside archive scripts.
/// The same bounded, allocation-free scan serves buffered and streamed HTML.
fn html_head_offset(bytes: &[u8]) -> usize {
    fn space(byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
    }
    fn keyword(bytes: &[u8], index: &mut usize, word: &[u8]) -> bool {
        if bytes.get(*index..*index + word.len()).is_some_and(|part| part.eq_ignore_ascii_case(word)) {
            *index += word.len();
            true
        } else { false }
    }
    fn root_end(bytes: &[u8], mut index: usize) -> Option<usize> {
        if !keyword(bytes, &mut index, b"<html") ||
            !bytes.get(index).is_some_and(|byte| space(*byte) || matches!(byte, b'/' | b'>')) {
            return None;
        }
        // Tag attribute states: before/name/after-name/before-value,
        // single-quoted/double-quoted/unquoted/after-quoted/self-closing.
        // A quote in an unquoted value is literal, not a delimiter.
        let mut state = 0;
        while let Some(byte) = bytes.get(index).copied() {
            index += 1;
            if byte == b'>' && !matches!(state, 4 | 5) { return Some(index); }
            state = match (state, byte) {
                (4, b'\'') | (5, b'"') => 7,
                (4 | 5, _) => state,
                (3, b'\'') => 4,
                (3, b'"') => 5,
                (3, byte) if space(byte) => 3,
                (3, _) => 6,
                (6, byte) => if space(byte) { 0 } else { 6 },
                (1 | 2, b'=') => 3,
                (1 | 2, byte) if space(byte) => 2,
                (_, byte) if space(byte) => 0,
                (_, b'/') => 8,
                _ => 1,
            };
        }
        None
    }
    let bytes = &bytes[..bytes.len().min(65536)];
    let mut index = if bytes.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    loop {
        // Plain text cannot execute; keep it before bootstrap too so a later
        // authored root tag can merge its attributes before appearance runs.
        let text_start = index;
        while bytes.get(index).is_some_and(|byte| *byte != b'<') { index += 1; }
        if index == bytes.len() {
            // The bounded prefix ends inside this text run. Injecting at the
            // truncation could split a UTF-8 sequence or an entity, so keep
            // the whole run after the scripts instead.
            return text_start;
        }
        if !bytes[index..].starts_with(b"<!--") {
            if bytes[index..].starts_with(b"<!") || bytes[index..].starts_with(b"<?") {
                // HTML doctypes terminate at the first `>` even inside a
                // quoted public/system identifier (an abrupt, quirks ending).
                // Other leading declarations are HTML bogus comments.
                if let Some(end) = bytes[index..].iter().position(|byte| *byte == b'>') {
                    index += end + 1;
                    continue;
                }
            }
            break;
        }
        let start = index;
        index += 4;
        // HTML comment start/start-dash/comment/end-dash/end/end-bang states.
        // Abrupt `<!-->` and `--!>` endings must not conceal a following script.
        let mut state = 0;
        let mut closed = false;
        while let Some(byte) = bytes.get(index).copied() {
            index += 1;
            if byte == b'>' && matches!(state, 0 | 1 | 4 | 5) { closed = true; break; }
            state = match (state, byte) {
                (0, b'-') => 1,
                (1, b'-') | (3, b'-') | (4, b'-') => 4,
                (4, b'!') => 5,
                (_, b'-') => 3,
                _ => 2,
            };
        }
        if !closed { return start; }
    }
    // Let the authored root attributes exist before appearance initialization;
    // otherwise an implicit root style would discard a later authored style.
    root_end(bytes, index).unwrap_or(index)
}

/// Stream an oversized HTML article: the instrumented prefix is sent first, the
/// remainder follows in 1 MiB archive ranges so memory stays bounded.
async fn stream_html_document(
    state: ArchiveState,
    entry: crate::archive::EntryInfo,
    index: u32,
    first: Vec<u8>,
    method: Method,
) -> Response {
    const CHUNK: u64 = 1024 * 1024;
    let nonce = restricted_nonce(&state);
    let injection = bridge_injection(&state, &entry.mime, nonce.as_deref());
    let prefix_length = entry.size.min(HTML_PREFIX);
    if first.len() as u64 != prefix_length {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Truncated archive document",
        );
    }
    let insert = html_head_offset(&first);
    let mut prefix = Vec::with_capacity(first.len() + injection.len());
    prefix.extend_from_slice(&first[..insert]);
    prefix.extend_from_slice(injection.as_bytes());
    prefix.extend_from_slice(&first[insert..]);
    let total = entry.size + injection.len() as u64;
    let stream = stream::try_unfold(
        (
            state.archive.clone(),
            state.workers.clone(),
            state.cancellation.clone(),
            state.id.clone(),
            state.installation_id.clone(),
            index,
            prefix_length,
            entry.size - prefix_length,
            Some(prefix),
        ),
        |(archive, workers, cancellation, archive_id, installation_id, index, offset, remaining, prefix)| async move {
            if let Some(bytes) = prefix {
                return Ok::<_, io::Error>(Some((
                    bytes,
                    (archive, workers, cancellation, archive_id, installation_id, index, offset, remaining, None),
                )));
            }
            if remaining == 0 {
                return Ok(None);
            }
            let count = remaining.min(CHUNK);
            let a = archive.clone();
            let cancel = cancellation.clone();
            let bytes = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Reading stopped. Reload the article"));
                }
                bytes = workers.run_for(&archive_id, &installation_id, "read_article", move || {
                    a.upgrade()
                        .ok_or("Reading stopped. Reload the article")?
                        .read_index_cancellable(index, offset, count, &cancel)
                }) => {
                    bytes.map_err(io::Error::other)?
                }
            };
            if bytes.len() as u64 != count {
                return Err(io::Error::other("Truncated archive document"));
            }
            Ok(Some((
                bytes,
                (archive, workers, cancellation, archive_id, installation_id, index, offset + count, remaining - count, None),
            )))
        },
    );
    let mut response = archive_response(
        &state,
        StatusCode::OK,
        "text/html; charset=utf-8",
        total,
        None,
        None,
        nonce.as_deref(),
        if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from_stream(stream)
        },
    );
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

/// An instrumented page for a document navigation that has no readable
/// article. It loads the same bridge and storage scripts so the shell reaches
/// `ready`, shows its own quiet notice, and can navigate away normally.
fn reader_error_document(
    state: &ArchiveState,
    method: Method,
    status: StatusCode,
    title: &str,
    message: &str,
) -> Response {
    let nonce = restricted_nonce(state);
    let injection = bridge_injection(state, "text/html", nonce.as_deref());
    let title = escape_attribute(title);
    let message = escape_attribute(message);
    let document = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>{injection}<style>body{{margin:24px;font:16px/1.6 system-ui}}</style></head><body><h1>{title}</h1><p>{message}</p></body></html>"
    );
    let size = document.len() as u64;
    let mut response = archive_response(
        state,
        status,
        "text/html; charset=utf-8",
        size,
        None,
        None,
        nonce.as_deref(),
        if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(document)
        },
    );
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

const RAW_CHUNK: u64 = 1024 * 1024;
const HTML_PREFIX: u64 = 64 * 1024;

/// What the serving decision selected while the archive lock was held.
enum ServedPlan {
    Redirect(String),
    HtmlWhole(Vec<u8>),
    HtmlPrefix(Vec<u8>),
    ResourceText(Vec<u8>),
    ResourceUnreadable(String),
    ResourceFallback,
    Raw(RawPlan),
    RejectHtml,
}

struct ServedEntry {
    entry: crate::archive::EntryInfo,
    /// Exact resolved entry index. Streaming uses this instead of the public
    /// path so a same-path entry in another namespace cannot be substituted.
    index: u32,
    plan: ServedPlan,
}

enum RawPlan {
    NotModified {
        etag: String,
    },
    Unsatisfiable {
        size: u64,
    },
    Body {
        etag: String,
        status: StatusCode,
        start: u64,
        length: u64,
        content_range: Option<String>,
        first: Vec<u8>,
    },
}

/// Decide the raw resource response from metadata and bytes read under the
/// same archived-item acquisition. No second entry lookup or range read.
fn raw_plan(
    reader: &crate::archive::RetainedItem<'_>,
    entry: &crate::archive::EntryInfo,
    headers: &HeaderMap,
    method: &Method,
    archive_id: &str,
    token: &str,
) -> Result<RawPlan, String> {
    let etag = entry_validator(archive_id, token, &entry.path, entry.size);
    // A client that already holds this exact representation keeps its copy
    // instead of re-streaming a media entry it cannot use.
    if headers
        .get("if-none-match")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|value| if_none_match_matches(value, &etag))
    {
        return Ok(RawPlan::NotModified { etag });
    }
    // A range is only meaningful while the client's validator still names this
    // representation. A stale `If-Range` means the client's cached prefix is
    // from different bytes, so the whole current entry is sent instead of a
    // slice that would splice two representations together.
    let requested = headers.get("range").and_then(|h| h.to_str().ok());
    let requested = match headers.get("if-range").and_then(|h| h.to_str().ok()) {
        Some(validator) if !if_range_matches(validator, &etag) => None,
        _ => requested,
    };
    let range = match requested
        .map(|r| parse_range(r, entry.size))
        .transpose()
    {
        Ok(range) => range,
        Err(_) => {
            return Ok(RawPlan::Unsatisfiable { size: entry.size });
        }
    };
    let (start, length, status) = range
        .map(|(s, l)| (s, l, StatusCode::PARTIAL_CONTENT))
        .unwrap_or((0, entry.size, StatusCode::OK));
    let content_range =
        range.map(|_| format!("bytes {start}-{}/{}", start + length - 1, entry.size));
    let first = if *method == Method::HEAD || length == 0 {
        Vec::new()
    } else {
        let count = length.min(RAW_CHUNK);
        let first = reader.read_range(start, count)?;
        if first.len() as u64 != count {
            return Err("Truncated archive resource".into());
        }
        first
    };
    Ok(RawPlan::Body {
        etag,
        status,
        start,
        length,
        content_range,
        first,
    })
}

fn raw_response(
    state: &ArchiveState,
    entry: crate::archive::EntryInfo,
    index: u32,
    method: Method,
    plan: RawPlan,
) -> Response {
    let (etag, status, start, length, content_range, first) = match plan {
        RawPlan::NotModified { etag } => return not_modified(&etag),
        RawPlan::Unsatisfiable { size } => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", format!("bytes */{size}"))
                .body(Body::empty())
                .unwrap();
        }
        RawPlan::Body {
            etag,
            status,
            start,
            length,
            content_range,
            first,
        } => (etag, status, start, length, content_range, first),
    };
    let mime = if entry.mime.starts_with("text/") && !entry.mime.contains("charset=") {
        format!("{}; charset=utf-8", entry.mime)
    } else {
        entry.mime.clone()
    };
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        let prefix = (!first.is_empty()).then_some(first);
        let consumed = prefix.as_ref().map_or(0, |bytes| bytes.len() as u64);
        let stream = stream::try_unfold(
            (
                state.archive.clone(),
                state.workers.clone(),
                state.cancellation.clone(),
                state.id.clone(),
                state.installation_id.clone(),
                index,
                start + consumed,
                length - consumed,
                prefix,
            ),
            |(archive, workers, cancellation, archive_id, installation_id, index, offset, remaining, prefix)| async move {
                if let Some(bytes) = prefix {
                    return Ok::<_, std::io::Error>(Some((
                        bytes,
                        (archive, workers, cancellation, archive_id, installation_id, index, offset, remaining, None),
                    )));
                }
                if remaining == 0 {
                    return Ok(None);
                }
                let count = remaining.min(RAW_CHUNK);
                let a = archive.clone();
                let cancel = cancellation.clone();
                let bytes = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Reading stopped. Reload the article"));
                    }
                    bytes = workers.run_for(&archive_id, &installation_id, "read_resource", move || {
                        a.upgrade()
                            .ok_or("Reading stopped. Reload the article")?
                            .read_index_cancellable(index, offset, count, &cancel)
                    }) => {
                        bytes.map_err(io::Error::other)?
                    }
                };
                if bytes.len() as u64 != count {
                    return Err(std::io::Error::other("Truncated archive resource"));
                }
                Ok(Some((
                    bytes,
                    (archive, workers, cancellation, archive_id, installation_id, index, offset + count, remaining - count, None),
                )))
            },
        );
        Body::from_stream(stream)
    };
    archive_response(
        state,
        status,
        &mime,
        length,
        content_range.as_deref(),
        Some(&etag),
        None,
        body,
    )
}

async fn internal_request(
    state: ArchiveState,
    method: Method,
    resource: &str,
    query: Option<&str>,
    headers: HeaderMap,
) -> Response {
    if resource == "retire.html" {
        return retirement_document(&state, method, query, &headers);
    }
    if let Some(path) = resource.strip_prefix("raw/") {
        if !matches!(method, Method::GET | Method::HEAD) {
            return error(
                StatusCode::METHOD_NOT_ALLOWED,
                "Archive resources are read-only",
            );
        }
        let path = match decode_percent_path(path) {
            Ok(path) => path,
            Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid resource path"),
        };
        let archive = state.archive.clone();
        let cancellation = state.cancellation.clone();
        let archive_id = state.id.clone();
        let token = state.token.clone();
        let request_method = method.clone();
        // One acquisition resolves the exact entry and either rejects HTML or
        // reads the selected raw range; the archive cannot change in between.
        let served = state
            .workers
            .run_for(&state.id, &state.installation_id, "read_resource", move || {
                let archive = archive
                    .upgrade()
                    .ok_or("Reading stopped. Reload the article")?;
                archive.with_entry(EntryLookup::Exact(&path), &cancellation, |entry, reader| {
                    let plan = if is_html_mime(&entry.mime) {
                        ServedPlan::RejectHtml
                    } else {
                        ServedPlan::Raw(raw_plan(
                            reader,
                            entry,
                            &headers,
                            &request_method,
                            &archive_id,
                            &token,
                        )?)
                    };
                    Ok(ServedEntry {
                        entry: entry.clone(),
                        index: reader.index(),
                        plan,
                    })
                })
            })
            .await;
        let ServedEntry { entry, index, plan } = match served {
            Ok(served) => served,
            Err(message) if message == "Archive worker stopped" => {
                return error(StatusCode::INTERNAL_SERVER_ERROR, "Archive worker stopped");
            }
            Err(message) => return archive_failure(&state, "read_resource", &message),
        };
        return match plan {
            ServedPlan::Raw(plan) => raw_response(&state, entry, index, method, plan),
            ServedPlan::RejectHtml => error(
                StatusCode::BAD_REQUEST,
                "HTML documents must use the instrumented reader",
            ),
            _ => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Archive resource plan was not readable",
            ),
        };
    }
    match (method.as_str(), resource) {
        ("GET", "bridge.js") => Response::builder()
            .header("Content-Type", "application/javascript; charset=utf-8")
            .header("Cache-Control", "no-store")
            .header("Cross-Origin-Resource-Policy", "same-origin")
            .body(Body::from(BRIDGE))
            .unwrap(),
        ("GET", "storage.js") => {
            if !state.active.load(std::sync::atomic::Ordering::SeqCst)
                || state.generation
                    != state
                        .privacy_generation
                        .load(std::sync::atomic::Ordering::SeqCst)
            {
                return error(StatusCode::GONE, "Reader privacy session expired");
            }
            let (values, read_only) = match state.store.reader_storage_snapshot(&state.installation_id) {
                Ok(snapshot) => snapshot,
                Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
            };
            let parent = match state.parent_origin.lock() {
                Ok(parent) => parent.clone(),
                Err(_) => {
                    return error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Reader origin unavailable",
                    )
                }
            };
            let bootstrap = json!({"bookId":state.id,"parentOrigin":parent,"privacyGeneration":state.generation,"values":values,"readOnly":read_only});
            let script = format!("window.__kiwixStorageBootstrap={bootstrap};\n{STORAGE_BRIDGE}");
            Response::builder()
                .header("Content-Type", "application/javascript; charset=utf-8")
                .header("Cache-Control", "no-store")
                .header("Cross-Origin-Resource-Policy", "same-origin")
                .body(Body::from(script))
                .unwrap()
        }
        _ => error(StatusCode::NOT_FOUND, "Unknown reader resource"),
    }
}

/// A same-origin navigation commits only after the outgoing document's pagehide
/// handlers finish. Its trusted acknowledgement fences those final storage posts
/// without allowing any archive script to execute in the replacement document.
fn retirement_document(
    state: &ArchiveState,
    method: Method,
    query: Option<&str>,
    headers: &HeaderMap,
) -> Response {
    // Fetch Metadata prevents fetch/XHR/script reads. The shell-only ancestor
    // policy below is equally important: an archive-local descendant must not
    // embed this same-origin proof document and read its DOM.
    if headers
        .get("sec-fetch-mode")
        .and_then(|value| value.to_str().ok())
        != Some("navigate")
        || headers
            .get("sec-fetch-dest")
            .and_then(|value| value.to_str().ok())
            != Some("iframe")
    {
        return error(
            StatusCode::FORBIDDEN,
            "This link must be opened inside Kiwix",
        );
    }
    if method != Method::GET && method != Method::HEAD {
        return error(
            StatusCode::METHOD_NOT_ALLOWED,
            "This link can only be opened, not changed",
        );
    }
    let challenge = query.and_then(|query| query.strip_prefix("retire="));
    if !challenge.is_some_and(|id| {
        id.len() == 36
            && id.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                }
            })
    }) {
        return error(StatusCode::BAD_REQUEST, "This link is out of date. Reload the article");
    }
    let parent = match state.parent_origin.lock() {
        Ok(parent) => parent.clone(),
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reader origin unavailable",
            )
        }
    };
    // JSON quoting alone does not protect an inline script from </script>.
    let parent_origin = parent.clone();
    let parent = json!(parent)
        .to_string()
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let nonce = Uuid::new_v4().simple().to_string();
    let proof = &state.retirement_proof;
    let document = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Reader handoff</title><script nonce=\"{nonce}\">\"use strict\";{{const query=location.search;const id=query.slice(8);if(/^\\?retire=[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/.test(query))parent.postMessage({{type:\"kiwix-storage-retired\",id,proof:\"{proof}\"}},{parent});}}</script>"
    );
    Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Content-Length", document.len())
        .header(
            "Content-Security-Policy",
            format!("default-src 'none'; script-src 'nonce-{nonce}'; base-uri 'none'; form-action 'none'; frame-ancestors {parent_origin}"),
        )
        .header(
            "Permissions-Policy",
            "accelerometer=(), ambient-light-sensor=(), camera=(), display-capture=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), usb=(), xr-spatial-tracking=()",
        )
        .header("Cache-Control", "no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .header("Cross-Origin-Resource-Policy", "same-origin")
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(document)
        })
        .unwrap()
}
fn apply_storage_mutations(saved: &mut Value, operations: &[Value]) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock predates cookie expiration epoch")?
        .as_millis() as f64;
    if !saved.is_object() {
        *saved = json!({});
    }
    {
        let object = saved.as_object_mut().ok_or("Invalid saved storage")?;
        if !object.get("localStorage").is_some_and(Value::is_object) {
            object.insert("localStorage".into(), json!({}));
        }
    }
    let mut cookies: HashMap<String, Value> = saved
        .get("cookies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let expires = c.get("expires")?.as_f64()?;
            if !expires.is_finite() || expires <= now {
                return None;
            }
            Some((
                format!("{}\n{}", c.get("path")?.as_str()?, c.get("name")?.as_str()?),
                c.clone(),
            ))
        })
        .collect();
    for operation in operations {
        let object = operation.as_object().ok_or("Invalid storage mutation")?;
        let fields: &[&str] = match operation.get("kind").and_then(Value::as_str) {
            Some("local-set") => &["kind", "key", "value"],
            Some("local-remove" | "cookie-remove") => &["kind", "key"],
            Some("local-clear") => &["kind"],
            Some("cookie-set") => &["kind", "key", "cookie"],
            _ => return Err("Unknown storage mutation".into()),
        };
        if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
            return Err("Invalid storage mutation fields".into());
        }
        let key = || {
            operation
                .get("key")
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 8192)
                .ok_or("Invalid storage key")
        };
        match operation.get("kind").and_then(Value::as_str) {
            Some("local-set") => {
                let value = operation
                    .get("value")
                    .and_then(Value::as_str)
                    .filter(|value| value.len() <= 1024 * 1024)
                    .ok_or("Invalid local storage value")?;
                let local = saved
                    .get_mut("localStorage")
                    .and_then(Value::as_object_mut)
                    .ok_or("Invalid saved storage")?;
                local.insert(key()?.into(), json!(value));
            }
            Some("local-remove") => {
                let local = saved
                    .get_mut("localStorage")
                    .and_then(Value::as_object_mut)
                    .ok_or("Invalid saved storage")?;
                local.remove(key()?);
            }
            Some("local-clear") => {
                let local = saved
                    .get_mut("localStorage")
                    .and_then(Value::as_object_mut)
                    .ok_or("Invalid saved storage")?;
                local.clear();
            }
            Some("cookie-set") => {
                let cookie = operation.get("cookie").ok_or("Missing cookie")?;
                let fields = cookie.as_object().ok_or("Invalid cookie")?;
                if fields.len() != 4
                    || ["name", "value", "path", "expires"]
                        .iter()
                        .any(|field| !fields.contains_key(*field))
                {
                    return Err("Invalid cookie fields".into());
                }
                let name = cookie
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| name.len() <= 4096)
                    .ok_or("Invalid cookie name")?;
                let path = cookie
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|s| s.starts_with('/') && s.len() <= 8192)
                    .ok_or("Invalid cookie path")?;
                cookie
                    .get("value")
                    .and_then(Value::as_str)
                    .filter(|value| value.len() <= 4096 && name.len() + value.len() <= 4096)
                    .ok_or("Invalid cookie value")?;
                let expires = cookie
                    .get("expires")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite())
                    .ok_or("Invalid cookie expiry")?;
                if key()? != format!("{path}\n{name}") {
                    return Err("Cookie identity mismatch".into());
                }
                // A queued cookie can expire before its accepted batch reaches
                // native persistence. Match the synchronous mirror's live set.
                if expires > now {
                    cookies.insert(key()?.into(), cookie.clone());
                } else {
                    cookies.remove(key()?);
                }
            }
            Some("cookie-remove") => {
                cookies.remove(key()?);
            }
            _ => return Err("Unknown storage mutation".into()),
        }
    }
    let object = saved.as_object_mut().ok_or("Invalid saved storage")?;
    object.retain(|key, _| key == "localStorage" || key == "cookies");
    object.insert("cookies".into(), Value::Array(cookies.into_values().collect()));
    if storage_json_bytes(saved)? > 1024 * 1024 {
        return Err("Archive storage quota exceeded".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn archive_response(
    state: &ArchiveState,
    status: StatusCode,
    mime: &str,
    size: u64,
    range: Option<&str>,
    etag: Option<&str>,
    nonce: Option<&str>,
    body: Body,
) -> Response {
    let parent = state
        .parent_origin
        .lock()
        .map(|parent| parent.clone())
        .unwrap_or_default();
    // An explicit network origin (rather than `self` or blob:) limits supported
    // child documents to server-instrumented archive HTML. srcdoc/about:blank
    // inherit CSP but are not replay documents and receive no persistence shim.
    let policy = match nonce {
        Some(nonce) => format!("default-src 'self' data: blob:; script-src 'nonce-{nonce}'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; frame-src 'none'; frame-ancestors 'self' {parent}; worker-src 'none'; object-src 'none'; form-action 'none'; base-uri 'self'; manifest-src 'none'"),
        None => format!("default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob:; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' data: blob:; connect-src 'self'; frame-src {}; frame-ancestors 'self' {parent}; worker-src 'self' blob:; object-src 'none'; form-action 'self'; base-uri 'self'", state.origin),
    };
    // Archives supply MIME metadata. Refuse malformed header bytes rather than
    // terminating the request, and keep sniffing disabled on the fallback.
    let content_type = HeaderValue::from_str(mime).ok()
        .filter(|value| value.to_str().is_ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Content-Length", size)
        .header("Accept-Ranges", "bytes")
        .header("Content-Security-Policy", policy)
        .header(
            "Permissions-Policy",
            "camera=(self), geolocation=(), microphone=(), display-capture=()",
        )
        .header("Referrer-Policy", "same-origin")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-DNS-Prefetch-Control", "off")
        .header("Cross-Origin-Resource-Policy", "same-origin")
        .header(
            "Cache-Control",
            if mime.starts_with("text/html") { "no-store" } else { "private, max-age=3600" },
        );
    if let Some(range) = range {
        builder = builder.header("Content-Range", range);
    }
    if let Some(etag) = etag {
        builder = builder.header("ETag", etag);
    }
    builder.body(body).unwrap_or_else(|_| {
        error(StatusCode::INTERNAL_SERVER_ERROR, "Could not prepare the archive response")
    })
}

/// A strong validator for one archive entry in this reader session.
///
/// Every open publishes a new origin, host and token, and the archive handle
/// retains the exact file, so an entry's bytes cannot change while this server
/// lives. The validator therefore only has to keep representations distinct.
fn entry_validator(archive_id: &str, token: &str, path: &str, size: u64) -> String {
    let mut digest = Sha256::new();
    for part in [
        archive_id.as_bytes(),
        token.as_bytes(),
        path.as_bytes(),
        &size.to_le_bytes(),
    ] {
        digest.update(part);
        digest.update([0]);
    }
    format!("\"{}\"", hex::encode(&digest.finalize()[..16]))
}

/// `If-None-Match` uses weak comparison and treats `*` as any representation.
fn if_none_match_matches(value: &str, etag: &str) -> bool {
    let value = value.trim();
    value == "*"
        || value.split(',').any(|candidate| {
            let candidate = candidate.trim();
            let candidate = candidate.strip_prefix("W/").unwrap_or(candidate);
            candidate == etag
        })
}

/// `If-Range` requires strong comparison. This server emits no dates, so only
/// its own exact entity tag can satisfy the client's cached prefix.
fn if_range_matches(value: &str, etag: &str) -> bool {
    value.trim() == etag
}

/// A validated cache hit keeps its copy: no media bytes are streamed again.
fn not_modified(etag: &str) -> Response {
    Response::builder()
        .status(StatusCode::NOT_MODIFIED)
        .header("ETag", etag)
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "private, max-age=3600")
        .body(Body::empty())
        .unwrap()
}

fn error(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
        .header("Cache-Control", "no-store")
        .body(Body::from(message.to_string()))
        .unwrap()
}
fn escape_attribute(text: &str) -> String {
    // One pass with a pre-sized output. The old chained `replace` calls
    // allocated a full copy per entity, so a 16 MiB text resource could hold
    // several simultaneous full-size copies while serving one document.
    let mut escaped = String::with_capacity(text.len().saturating_mul(6));
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

pub fn parse_range(header: &str, size: u64) -> Result<(u64, u64), String> {
    let spec = header
        .strip_prefix("bytes=")
        .ok_or("Unsupported range unit")?;
    if spec.contains(',') || size == 0 {
        return Err("Only one nonempty byte range is supported".into());
    }
    let (start, end) = spec.split_once('-').ok_or("Invalid range")?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| "Invalid suffix")?;
        if suffix == 0 {
            return Err("Empty suffix".into());
        }
        let count = suffix.min(size);
        return Ok((size - count, count));
    }
    let start = start.parse::<u64>().map_err(|_| "Invalid start")?;
    if start >= size {
        return Err("Range outside resource".into());
    }
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().map_err(|_| "Invalid end")?.min(size - 1)
    };
    if end < start {
        return Err("Inverted range".into());
    }
    Ok((start, end - start + 1))
}

#[cfg(any(desktop, test))]
#[derive(rust_embed::RustEmbed)]
#[folder = "../dist/"]
struct Frontend;

#[cfg(any(desktop, test))]
#[derive(Clone)]
struct PreviewState {
    service: Arc<crate::service::Service>,
    token: String,
    host: String,
    origin: String,
}

#[cfg(any(desktop, test))]
pub async fn start_preview(
    service: Arc<crate::service::Service>,
    port: u16,
) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    // The privileged shell must never inherit an untrusted archive's origin,
    // including when a previous loopback port is reused by the operating system.
    let host = format!("kiwix-shell-{}.localhost:{port}", Uuid::new_v4().simple());
    let origin = format!("http://{host}");
    let token = Uuid::new_v4().simple().to_string();
    service.allow_shell_origin(origin.clone())?;
    let state = PreviewState {
        service,
        token: token.clone(),
        host,
        origin: origin.clone(),
    };
    let router = Router::new()
        .fallback(any(preview_request))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            eprintln!("Preview server: {e}");
        }
    });
    Ok(format!("{origin}/#token={token}"))
}

#[cfg(any(desktop, test))]
async fn preview_request(
    State(state): State<PreviewState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let mut hosts = headers.get_all("host").iter();
    if hosts.next().and_then(|v| v.to_str().ok()) != Some(state.host.as_str())
        || hosts.next().is_some()
    {
        return error(StatusCode::FORBIDDEN, "Unknown preview host");
    }
    if headers.contains_key("service-worker")
        || headers
            .get_all("sec-fetch-dest")
            .iter()
            .any(|v| v.as_bytes().eq_ignore_ascii_case(b"serviceworker"))
    {
        return error(
            StatusCode::FORBIDDEN,
            "Shell service workers are not supported",
        );
    }
    let mut origins = headers.get_all("origin").iter();
    let origin = origins.next();
    if origin.is_some_and(|v| v.to_str().ok() != Some(state.origin.as_str()))
        || origins.next().is_some()
    {
        return error(StatusCode::FORBIDDEN, "Foreign preview origin");
    }
    if uri.path() == "/api" {
        // Browser POSTs from the shell carry Origin. Missing, opaque, malformed,
        // duplicate, and foreign origins must never reach privileged dispatch.
        if origin.is_none() {
            return error(StatusCode::FORBIDDEN, "Preview origin required");
        }
        let mut authorizations = headers.get_all("authorization").iter();
        if method != Method::POST
            || authorizations.next().and_then(|v| v.to_str().ok())
                != Some(format!("Bearer {}", state.token).as_str())
            || authorizations.next().is_some()
        {
            return error(StatusCode::UNAUTHORIZED, "Preview authorization required");
        }
        let value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid command JSON"),
        };
        let Some(command) = value.get("command").and_then(Value::as_str) else {
            return error(StatusCode::BAD_REQUEST, "Missing command");
        };
        // The headless shell has one window and one shell origin. Never let an
        // API caller spoof a window label or name a different reader parent.
        let mut args = value.get("args").cloned().unwrap_or(json!({}));
        if let Some(object) = args.as_object_mut() {
            object.remove("__windowLabel");
            if let Some(parent) = object.get("parentOrigin") {
                if parent.as_str() != Some(state.origin.as_str()) {
                    return error(StatusCode::BAD_REQUEST, "Untrusted reader parent origin");
                }
            }
        }
        return match state
            .service
            .clone()
            .dispatch(command, args)
            .await
        {
            Ok(result) => axum::Json(json!({"result":result})).into_response(),
            Err(message) => (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"error":crate::user_error::plain(&message)})),
            )
                .into_response(),
        };
    }
    if method != Method::GET && method != Method::HEAD {
        return error(
            StatusCode::METHOD_NOT_ALLOWED,
            "Read-only application assets",
        );
    }
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Frontend::get(path) {
        Some(file)=>Response::builder().header("Content-Type",mime_guess::from_path(path).first_or_octet_stream().as_ref()).header("Cache-Control","no-store").header("X-Content-Type-Options","nosniff").header("Content-Security-Policy","default-src 'self'; script-src 'self'; worker-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; frame-src http://*.localhost:*; frame-ancestors 'none'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'").header("Permissions-Policy","microphone=()").header("Referrer-Policy","no-referrer").body(if method==Method::HEAD {Body::empty()}else{Body::from(file.data.into_owned())}).unwrap(),
        None=>error(StatusCode::NOT_FOUND,"Application asset not found"),
    }
}

fn resource_fallback(title: &str, source: &str, path: &str, read_only: bool) -> String {
    if read_only {
        format!("<h1>{title}</h1><p>{}</p>", escape_attribute(path))
    } else {
        format!(
            "<h1>{title}</h1><a href=\"{source}\" download>{}</a>",
            escape_attribute(path)
        )
    }
}
/// Binary navigation still needs a bridge-bearing document so history, Back,
/// title, export, and subsequent navigation remain owned by the trusted shell.
fn resource_document(
    state: ArchiveState,
    entry: crate::archive::EntryInfo,
    method: Method,
    bytes: Option<Vec<u8>>,
) -> Response {
    let prefix = format!("/.kiwix/{}/", state.token);
    let resource = format!("{prefix}raw/{}", encode_path(&entry.path));
    let source = escape_attribute(&resource);
    let title = escape_attribute(&entry.title);
    let mime = entry.mime.split(';').next().unwrap_or("");
    let content = if mime.starts_with("image/") {
        format!("<img src=\"{source}\" alt=\"{title}\">")
    } else if mime.starts_with("video/") {
        format!("<video controls src=\"{source}\"></video>")
    } else if mime.starts_with("audio/") {
        format!("<audio controls src=\"{source}\"></audio>")
    } else if (mime.starts_with("text/") || matches!(mime, "application/json" | "application/xml"))
        && entry.size <= MAX_READ_LENGTH
    {
        let Some(bytes) = bytes else {
            return reader_error_document(
                &state,
                method,
                StatusCode::UNPROCESSABLE_ENTITY,
                "This page could not be opened.",
                "The archive could not read this entry.",
            );
        };
        format!(
            "<pre>{}</pre>",
            escape_attribute(&String::from_utf8_lossy(&bytes))
        )
    } else {
        // Sandboxed webviews cannot reliably host native PDF/EPUB plugins.
        // Keep navigation instrumented; trusted shell controls provide Save/Open.
        let read_only = match state.store.read_only() {
            Ok(read_only) => read_only,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
        };
        resource_fallback(&title, &source, &entry.path, read_only)
    };
    let nonce = restricted_nonce(&state);
    let injection = bridge_injection(&state, &entry.mime, nonce.as_deref());
    let document=format!("<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>{injection}<style>body{{margin:24px;font:16px/1.6 system-ui}}img,video{{display:block;max-width:100%;max-height:85vh;margin:auto}}audio{{width:100%}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}</style></head><body>{content}</body></html>");
    archive_response(
        &state,
        StatusCode::OK,
        "text/html; charset=utf-8",
        document.len() as u64,
        None,
        None,
        nonce.as_deref(),
        if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(document)
        },
    )
}
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;


