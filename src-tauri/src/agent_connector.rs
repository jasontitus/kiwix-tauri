//! Offline-library agent connector: the app side of `kiwix-mcp`.
//!
//! While Settings → Agent connector is on (never in private mode), the app
//! serves three read-only tools over a same-user channel to the `kiwix-mcp`
//! relay, which speaks MCP to the agent. See docs/AGENT-CONNECTOR.md and
//! docs/PLATFORM-INTEGRATIONS.md.
//!
//! Limits: at most two calls at once (a call keeps its slot until its native
//! work really returns), an app-wide rate of 60 calls a minute with a burst
//! of 10, a 10-second budget per call, at most 16 connected relays, and
//! bounded inputs and outputs. Stopping the connector (turning it off, private
//! mode, shutdown) cancels in-flight work at its next checkpoint.
//! Archive-derived strings are untrusted: hidden markup and invisible or
//! reordering characters are removed, and errors returned to an agent are
//! fixed sentences that never carry local file paths.

use super::Service;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use unicode_normalization::UnicodeNormalization;

const MAX_QUERY_CHARS: usize = 512;
const MAX_LIMIT: u64 = 20;
const MAX_OFFSET: u64 = 200;
const MAX_ARTICLE_CHARS: u64 = 40_000;
/// Above the longest text a 1 MiB read can yield plus one page, so a
/// returned `nextOffset` is always accepted.
const MAX_ARTICLE_OFFSET: u64 = 1_200_000;
const ARTICLE_READ_BYTES: u64 = 1024 * 1024;
const CALL_BUDGET: Duration = Duration::from_secs(10);
const CONCURRENT_CALLS: usize = 2;
const BURST: f64 = 10.0;
const PER_SECOND: f64 = 1.0;
const OFF: &str = "The Kiwix agent connector is off. Turn it on in Kiwix Settings.";
const CANNOT_OPEN: &str = "Kiwix cannot open this archive right now. Call list_archives and try another.";

#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Activity {
    day: u64,
    calls_today: u64,
    last_used: Option<u64>,
    error: Option<String>,
}

struct Running {
    generation: u64,
    cancel: CancellationToken,
}

pub(crate) struct AgentConnector {
    running: Mutex<Option<Running>>,
    generation: std::sync::atomic::AtomicU64,
    transport: tokio::sync::Mutex<()>,
    bucket: Mutex<(f64, Instant)>,
    permits: Arc<tokio::sync::Semaphore>,
    activity: Mutex<Activity>,
}

impl Default for AgentConnector {
    fn default() -> Self {
        Self {
            running: Mutex::new(None),
            generation: Default::default(),
            transport: Default::default(),
            bucket: Mutex::new((BURST, Instant::now())),
            permits: Arc::new(tokio::sync::Semaphore::new(CONCURRENT_CALLS)),
            activity: Mutex::new(Activity::default()),
        }
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

impl AgentConnector {
    fn admit(&self) -> Result<(), String> {
        let mut bucket = self.bucket.lock().map_err(|_| "Agent connector unavailable")?;
        let now = Instant::now();
        bucket.0 = (bucket.0 + now.duration_since(bucket.1).as_secs_f64() * PER_SECOND).min(BURST);
        bucket.1 = now;
        if bucket.0 < 1.0 {
            return Err("Too many requests to Kiwix; wait a few seconds and try again.".into());
        }
        bucket.0 -= 1.0;
        let mut activity = self.activity.lock().map_err(|_| "Agent connector unavailable")?;
        let seconds = now_seconds();
        if activity.day != seconds / 86_400 {
            activity.day = seconds / 86_400;
            activity.calls_today = 0;
        }
        activity.calls_today += 1;
        activity.last_used = Some(seconds);
        Ok(())
    }

    fn set_error(&self, error: Option<String>) {
        if let Ok(mut activity) = self.activity.lock() {
            activity.error = error;
        }
    }

    /// A token for one call, cancelled when the connector stops.
    fn call_token(&self) -> Option<CancellationToken> {
        self.running.lock().ok()?.as_ref().map(|running| running.cancel.child_token())
    }
}

/// One line of untrusted archive text: no control or invisible characters,
/// whitespace collapsed, at most `max_chars` characters.
pub(crate) fn clean_line(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if c.is_control() || crate::exports::html::invisible(c) {
            continue;
        }
        if count >= max_chars { break; }
        if space {
            if max_chars - count < 2 { break; }
            out.push(' ');
            count += 1;
            space = false;
        }
        out.push(c);
        count += 1;
    }
    out
}

fn bounded_u64(arguments: &Value, key: &str, default: u64, max: u64, min: u64) -> Result<u64, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|value| (min..=max).contains(value))
            .ok_or_else(|| format!("{key} must be an integer from {min} to {max}")),
    }
}

fn required_text<'a>(arguments: &'a Value, key: &str, max_chars: usize) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.chars().count() <= max_chars)
        .ok_or_else(|| format!("{key} must be 1 to {max_chars} characters"))
}

/// A path as a person would type it in an MCP configuration: without the
/// Windows verbatim prefix that canonical paths carry.
fn display_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        text.into_owned()
    }
}

fn citation(archive_id: &str, location: &str) -> String {
    format!("zim://{archive_id}/{location}")
}

impl Service {
    /// Start or stop the connector to match settings. Called at startup and
    /// after settings are saved; turning it off, private mode and shutdown
    /// stop it earlier through [`Service::agent_connector_stop`].
    pub(crate) fn agent_connector_reconcile(self: &Arc<Self>) {
        let Ok(mut running) = self.agent.running.lock() else { return; };
        let want = self
            .store
            .settings()
            .map(|settings| settings.agent_connector && !settings.private_mode && !settings.read_only)
            .unwrap_or(false)
            && !self.stopping.load(std::sync::atomic::Ordering::SeqCst);
        match (want, running.is_some()) {
            (true, false) => {
                let generation = self.agent.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let cancel = CancellationToken::new();
                *running = Some(Running { generation, cancel: cancel.clone() });
                self.agent.set_error(None);
                let service = self.clone();
                tauri::async_runtime::spawn(async move {
                    // Finish retiring the old listener before the replacement claims its endpoint.
                    let _transport = service.agent.transport.lock().await;
                    if cancel.is_cancelled() { return; }
                    let result = transport::serve(service.clone(), cancel).await;
                    if let Err(error) = result {
                        eprintln!("Agent connector: {error}");
                        // Forget a failed server so turning the setting off and
                        // on again retries, unless a newer one replaced it.
                        if let Ok(mut running) = service.agent.running.lock() {
                            if running.as_ref().is_some_and(|current| current.generation == generation) {
                                service.agent.set_error(Some(error));
                                *running = None;
                            }
                        }
                    }
                });
            }
            (false, true) => {
                if let Some(current) = running.take() {
                    current.cancel.cancel();
                }
            }
            _ => {}
        }
    }

    /// Close the channel, drop connected relays and cancel in-flight work.
    pub(crate) fn agent_connector_stop(&self) {
        if let Some(current) = self.agent.running.lock().ok().and_then(|mut running| running.take()) {
            current.cancel.cancel();
        }
    }

    pub(crate) fn agent_connector_status(&self) -> Result<Value, String> {
        let activity = self.agent.activity.lock().map_err(|_| "Agent connector unavailable")?.clone();
        let running = self.agent.running.lock().map_err(|_| "Agent connector unavailable")?.is_some();
        let relay = std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name(if cfg!(windows) { "kiwix-mcp.exe" } else { "kiwix-mcp" }));
        let today = activity.day == now_seconds() / 86_400;
        let profile_arg = (!transport::is_default_profile(self.store.data_dir()))
            .then(|| display_path(self.store.data_dir()));
        Ok(json!({
            "supported": cfg!(desktop),
            "running": running,
            "relay": relay.as_ref().map(|path| path.to_string_lossy().into_owned()),
            "relayInstalled": relay.as_ref().is_some_and(|path| path.is_file()),
            "flatpak": std::env::var_os("FLATPAK_ID").is_some(),
            "flatpakId": std::env::var("FLATPAK_ID").ok(),
            "appImage": std::env::var_os("APPIMAGE").is_some(),
            "dataDir": profile_arg,
            "callsToday": if today { activity.calls_today } else { 0 },
            "lastUsed": activity.last_used,
            "error": activity.error,
        }))
    }

    /// One tool call from a relay. Every call re-checks the setting and mode.
    pub(crate) async fn agent_call(self: &Arc<Self>, tool: &str, arguments: Value) -> Result<Value, String> {
        let settings = self.store.settings()?;
        if !settings.agent_connector || settings.private_mode || settings.read_only {
            return Err(OFF.into());
        }
        let cancel = self.agent.call_token().ok_or(OFF)?;
        self.agent.admit()?;
        let deadline = tokio::time::Instant::now() + CALL_BUDGET;
        let permit = tokio::select! {
            _ = cancel.cancelled() => return Err(OFF.into()),
            permit = tokio::time::timeout(Duration::from_secs(5), self.agent.permits.clone().acquire_owned()) => permit
                .map_err(|_| "Kiwix is busy with other agent requests; try again shortly.".to_string())?
                .map_err(|_| "Agent connector unavailable".to_string())?,
        };
        // No privacy read lease: agent calls only read the library, so a
        // settings save never waits for them. Entering private mode stops the
        // connector inside the save, which cancels this call and discards its
        // result before it reaches the relay.
        let settings = self.store.settings()?;
        if !settings.agent_connector || settings.private_mode || settings.read_only || cancel.is_cancelled() {
            return Err(OFF.into());
        }
        let service = self.clone();
        let task_cancel = cancel.clone();
        let tool = tool.to_owned();
        let task = tokio::task::spawn_blocking(move || {
            // The slot is held until the native work returns, even if the
            // caller stopped waiting: abandoned libzim calls cannot pile up
            // beyond the concurrency limit.
            let _permit = permit;
            match tool.as_str() {
                "list_archives" => service.agent_list_archives(),
                "search" => service.agent_search(&arguments, &task_cancel),
                "read_article" => service.agent_read_article(&arguments, &task_cancel),
                _ => Err("Unknown tool".into()),
            }
        });
        tokio::select! {
            result = tokio::time::timeout_at(deadline, task) => match result {
                Ok(result) => result.map_err(|_| "The request stopped unexpectedly".to_string())?,
                Err(_) => {
                    cancel.cancel();
                    Err("The request took too long. Search fewer archives (archiveIds) and try again.".into())
                }
            },
            _ = cancel.cancelled() => Err(OFF.into()),
        }
    }

    fn agent_list_archives(&self) -> Result<Value, String> {
        let books = self.store.books()?;
        let archives: Vec<Value> = books
            .iter()
            .filter(|book| book.available)
            .take(1000)
            .map(|book| {
                let metadata = &book.metadata;
                json!({
                    "id": metadata.id,
                    "title": clean_line(&metadata.title, 200),
                    "description": clean_line(&metadata.description, 500),
                    "language": clean_line(&metadata.language, 32),
                    "articleCount": metadata.article_count,
                    "hasFulltext": metadata.has_fulltext_index,
                })
            })
            .collect();
        Ok(json!({"archives": archives}))
    }

    fn agent_search(&self, arguments: &Value, cancel: &CancellationToken) -> Result<Value, String> {
        let query: String = required_text(arguments, "query", MAX_QUERY_CHARS)?.trim().nfc().collect();
        let mode = crate::search::Mode::parse(arguments.get("mode").and_then(Value::as_str))?;
        let limit = bounded_u64(arguments, "limit", 10, MAX_LIMIT, 1)? as u32;
        let offset = bounded_u64(arguments, "offset", 0, MAX_OFFSET, 0)? as u32;
        let requested: Vec<String> = match arguments.get("archiveIds") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(ids)) if ids.len() <= 64 => ids
                .iter()
                .map(|id| id.as_str().map(str::to_owned).ok_or("archiveIds must be strings"))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err("archiveIds must be a list of at most 64 archive IDs".into()),
        };
        let books = self.store.books()?;
        let mut titles = std::collections::HashMap::new();
        let mut archives = Vec::new();
        for book in books
            .into_iter()
            .filter(|book| book.available && (requested.is_empty() || requested.contains(&book.metadata.id)))
        {
            crate::search::check_cancel(cancel).map_err(|_| OFF.to_string())?;
            titles.insert(book.metadata.id.clone(), clean_line(&book.metadata.title, 200));
            let archive = self.archive(&book.metadata.id);
            archives.push((book.metadata.id, archive));
        }
        if archives.is_empty() {
            return Err("No matching archive is available. Call list_archives for valid IDs.".into());
        }
        let searched = archives.len();
        // Snippets always on and no spelling dictionaries: an agent search
        // has a fixed configuration and writes nothing to the profile.
        let result = crate::search::run(archives, &query, mode, offset, limit, true, cancel)
            .map_err(|_| "The search could not be completed. Try again, or search fewer archives.".to_string())?;
        // Combined search can report one archive twice (title and full text).
        let failed = result["errors"]
            .as_array()
            .map(|errors| {
                errors.iter().filter_map(|error| error["bookId"].as_str()).collect::<std::collections::HashSet<_>>().len()
            })
            .unwrap_or(0);
        let results: Vec<Value> = result["hits"]
            .as_array()
            .map(|hits| {
                hits.iter()
                    .filter_map(|hit| {
                        let archive_id = hit["bookId"].as_str()?;
                        let path = hit["path"].as_str()?;
                        Some(json!({
                            "archiveId": archive_id,
                            "archiveTitle": titles.get(archive_id).cloned().unwrap_or_default(),
                            "path": path,
                            "title": clean_line(hit["title"].as_str().unwrap_or(""), 300),
                            "snippet": clean_line(hit["snippet"].as_str().unwrap_or(""), 600),
                            "citation": citation(archive_id, path),
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if results.is_empty() && failed >= searched {
            return Err("None of the requested archives could be searched right now.".into());
        }
        Ok(json!({
            "results": results,
            "total": result["total"].as_u64().unwrap_or(0),
            "hasMore": result["hasMore"].as_bool().unwrap_or(false),
            "unsearchedArchives": failed,
        }))
    }

    fn agent_read_article(&self, arguments: &Value, cancel: &CancellationToken) -> Result<Value, String> {
        let id = required_text(arguments, "archiveId", 64)?;
        let location = required_text(arguments, "path", 4096)?;
        let max_chars = bounded_u64(arguments, "maxChars", 20_000, MAX_ARTICLE_CHARS, 1)? as usize;
        let skip = bounded_u64(arguments, "offset", 0, MAX_ARTICLE_OFFSET, 0)? as usize;
        let book = self
            .store
            .book(id)?
            .filter(|book| book.available)
            .ok_or("No available archive has that ID. Call list_archives for valid IDs.")?;
        let archive = self.archive(&book.metadata.id).map_err(|_| CANNOT_OPEN.to_string())?;
        let path = crate::server::decode_location_path(location)
            .map_err(|_| "That path is not valid. Use a path returned by search.".to_string())?;
        let entry = archive
            .entry(&path)
            .map_err(|_| "No article has that path in this archive. Use a path returned by search.".to_string())?;
        let html = crate::archive::is_html_mime(&entry.mime);
        if !html && !entry.mime.starts_with("text/") {
            return Err("That entry is not an article (it is media or another binary file).".into());
        }
        let bytes = archive
            .read(&entry.path, 0, entry.size.min(ARTICLE_READ_BYTES))
            .map_err(|_| CANNOT_OPEN.to_string())?;
        crate::search::check_cancel(cancel).map_err(|_| OFF.to_string())?;
        let source = String::from_utf8_lossy(&bytes);
        let wanted = skip.saturating_add(max_chars);
        let (text, truncated) = if html {
            use crate::exports::html;
            let document = html::Html::parse_document_bounded(&source, &html::ARTICLE_BOUNDS, Some(cancel))
                .map_err(|_| "This article is too complex to read as text.".to_string())?;
            html::visible_text(&document.root_element(), wanted)
        } else {
            crate::exports::html::plain_text(&source, wanted)
        };
        let part: String = text.chars().skip(skip).collect();
        let more_text = truncated;
        let cut_by_size = entry.size > ARTICLE_READ_BYTES;
        let location = crate::server::encode_path(&entry.path);
        let mut result = json!({
            "title": clean_line(&entry.title, 300),
            "citation": citation(&book.metadata.id, &location),
            "text": part,
            "truncated": more_text || cut_by_size,
        });
        if more_text {
            result["nextOffset"] = json!(wanted);
        }
        Ok(result)
    }
}

/// Brings the connector back in line with the saved settings when dropped,
/// so a settings save that stopped it restarts it on every exit path.
pub(super) struct Reconcile(pub(super) Arc<Service>);
impl Drop for Reconcile {
    fn drop(&mut self) {
        self.0.agent_connector_reconcile();
    }
}

/// The pipe-name suffix of a non-default profile, shared with kiwix-mcp:
/// a short SHA-256 of its canonical, lower-cased path.
fn profile_suffix(profile: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let canonical = std::fs::canonicalize(profile).ok()?;
    let digest = Sha256::digest(canonical.to_string_lossy().to_lowercase().as_bytes());
    Some(digest[..8].iter().map(|byte| format!("{byte:02x}")).collect())
}

const MAX_FRAME: usize = 1024 * 1024;
/// Idle relays are dropped; kiwix-mcp reconnects on its next call.
const IDLE: Duration = Duration::from_secs(300);
/// A relay must accept a reply within this time, or it is dropped.
const WRITE: Duration = Duration::from_secs(30);
const MAX_CLIENTS: usize = 16;

async fn read_frame<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Value, String> {
    use tokio::io::AsyncReadExt;
    let length = reader.read_u32().await.map_err(|error| error.to_string())? as usize;
    if length > MAX_FRAME {
        return Err("Request too large".into());
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await.map_err(|error| error.to_string())?;
    serde_json::from_slice(&body).map_err(|_| "Malformed request".into())
}

/// Bounded in time and cancelled with the connector, so a relay that stops
/// reading cannot keep this task (and its channel) alive.
async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
    cancel: &CancellationToken,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let body = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let write = async {
        writer.write_u32(body.len() as u32).await?;
        writer.write_all(&body).await?;
        writer.flush().await
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("Agent connector stopped".into()),
        result = tokio::time::timeout(WRITE, write) => match result {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err("The relay stopped reading".into()),
        },
    }
}

fn same_profile(service: &Service, requested: &str) -> bool {
    same_file::is_same_file(requested, service.store.data_dir()).unwrap_or(false)
}

/// The handshake and request loop shared by the Windows pipe and Unix socket.
/// The platform transport authenticates its peer before calling this.
async fn serve_client<S>(service: Arc<Service>, mut stream: S, cancel: CancellationToken) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let hello = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        hello = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream)) => hello
            .map_err(|_| "Agent connector client sent no greeting")??,
    };
    if hello["hello"] != json!(1) {
        return write_frame(&mut stream, &json!({"error": "This kiwix-mcp does not match this Kiwix version."}), &cancel).await;
    }
    if let Some(requested) = hello["dataDir"].as_str() {
        if !same_profile(&service, requested) {
            return write_frame(&mut stream, &json!({"error": "The running Kiwix uses a different profile from kiwix-mcp --data-dir."}), &cancel).await;
        }
    }
    write_frame(&mut stream, &json!({"ok": {"version": env!("CARGO_PKG_VERSION")}}), &cancel).await?;
    loop {
        let request = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            request = tokio::time::timeout(IDLE, read_frame(&mut stream)) => match request {
                Ok(Ok(request)) => request,
                _ => return Ok(()),
            },
        };
        let id = request["id"].clone();
        let tool = request["tool"].as_str().unwrap_or_default().to_owned();
        let arguments = request.get("arguments").cloned().unwrap_or(json!({}));
        let reply = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            result = service.agent_call(&tool, arguments) => match result {
                Ok(value) => json!({"id": id, "ok": value}),
                Err(error) => json!({"id": id, "error": crate::user_error::plain(&error)}),
            },
        };
        write_frame(&mut stream, &reply, &cancel).await?;
    }
}

#[cfg(windows)]
mod transport {
    //! Windows named pipe `\\.\pipe\kiwix-mcp-<user SID>[-<profile hash>]`:
    //! owned by and accessible to the current user only, remote clients
    //! rejected, the first instance claimed exclusively, and each client's
    //! process checked to belong to this user.

    use super::super::Service;
    use serde_json::json;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use tokio_util::sync::CancellationToken;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW};
    use windows::Win32::Security::{EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

    struct User { storage: Vec<usize> }
    impl User {
        fn of(process: HANDLE) -> Result<Self, String> {
            let mut raw = HANDLE::default();
            unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw).map_err(|error| error.to_string())?; }
            let token = unsafe { OwnedHandle::from_raw_handle(raw.0) };
            let mut required = 0;
            unsafe { let _ = GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser, None, 0, &mut required); }
            if required == 0 || required > 64 * 1024 { return Err("Cannot read a process identity".into()); }
            let mut storage = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
            unsafe {
                GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser, Some(storage.as_mut_ptr().cast()), required, &mut required)
                    .map_err(|error| error.to_string())?;
            }
            Ok(Self { storage })
        }
        fn sid(&self) -> PSID { unsafe { (*(self.storage.as_ptr().cast::<TOKEN_USER>())).User.Sid } }
        fn text(&self) -> Result<String, String> {
            let mut text = PWSTR::null();
            unsafe { ConvertSidToStringSidW(self.sid(), &mut text).map_err(|error| error.to_string())?; }
            let value = unsafe { text.to_string() }.map_err(|error| error.to_string());
            unsafe { let _ = LocalFree(Some(HLOCAL(text.0.cast()))); }
            value
        }
    }

    struct Descriptor(PSECURITY_DESCRIPTOR);
    impl Descriptor {
        /// Owned by the current user, full access for the current user only,
        /// with a protected DACL. Relays check the owner.
        fn current_user_only(own: &User) -> Result<Self, String> {
            let sid = own.text()?;
            let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})").encode_utf16().chain(Some(0)).collect();
            let mut raw = PSECURITY_DESCRIPTOR::default();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), 1, &mut raw, None)
                    .map_err(|error| error.to_string())?;
            }
            Ok(Self(raw))
        }
    }
    impl Drop for Descriptor {
        fn drop(&mut self) {
            unsafe { let _ = LocalFree(Some(HLOCAL(self.0 .0))); }
        }
    }
    // The descriptor is immutable after creation and only read by CreateNamedPipe.
    unsafe impl Send for Descriptor {}
    unsafe impl Sync for Descriptor {}

    pub(super) fn is_default_profile(profile: &std::path::Path) -> bool {
        if crate::portable::PortableProfile::detect().ok().flatten().is_some() {
            return false;
        }
        let Some(default) = directories::ProjectDirs::from("org", "kiwix", "KiwixWorkbench") else { return false; };
        same_file::is_same_file(profile, default.data_dir()).unwrap_or(false)
    }

    fn pipe_name(service: &Service, own: &User) -> Result<String, String> {
        let base = format!(r"\\.\pipe\kiwix-mcp-{}", own.text()?);
        let profile = service.store.data_dir();
        if is_default_profile(profile) {
            return Ok(base);
        }
        let suffix = super::profile_suffix(profile).ok_or("Cannot identify this Kiwix profile")?;
        Ok(format!("{base}-{suffix}"))
    }

    fn create(name: &str, first: bool, descriptor: &Descriptor) -> std::io::Result<NamedPipeServer> {
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0 .0,
            bInheritHandle: false.into(),
        };
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(name, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast())
        }
    }

    fn client_is_current_user(pipe: &NamedPipeServer, own: &User) -> bool {
        let mut pid = 0u32;
        if unsafe { GetNamedPipeClientProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.is_err() || pid == 0 {
            return false;
        }
        let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false; };
        let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
        match User::of(HANDLE(process.as_raw_handle())) {
            Ok(client) => unsafe { EqualSid(client.sid(), own.sid()) }.is_ok(),
            Err(_) => false,
        }
    }

    async fn client(service: Arc<Service>, pipe: NamedPipeServer, own: Arc<User>, cancel: CancellationToken) -> Result<(), String> {
        if !client_is_current_user(&pipe, &own) {
            return Err("Refused an agent connector client from another account".into());
        }
        super::serve_client(service, pipe, cancel).await
    }


    /// Create the next instance, retrying briefly; `None` once cancelled.
    async fn next_instance(name: &str, descriptor: &Descriptor, cancel: &CancellationToken) -> Option<NamedPipeServer> {
        loop {
            match create(name, false, descriptor) {
                Ok(server) => return Some(server),
                Err(error) => eprintln!("Agent connector: cannot open another channel instance: {error}"),
            }
            tokio::select! {
                _ = cancel.cancelled() => return None,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }

    pub(super) async fn serve(service: Arc<Service>, cancel: CancellationToken) -> Result<(), String> {
        let own = Arc::new(User::of(unsafe { GetCurrentProcess() })?);
        let name = pipe_name(&service, &own)?;
        let descriptor = Descriptor::current_user_only(&own)?;
        // Instances of a connector just turned off may still be closing:
        // retry the exclusive claim briefly before reporting a conflict.
        let mut claim = Err(std::io::Error::other("not attempted"));
        for _ in 0..10 {
            claim = create(&name, true, &descriptor);
            if claim.is_ok() || cancel.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let mut server = claim.map_err(|error| {
            if matches!(error.raw_os_error(), Some(5) | Some(231)) {
                "Another program already uses the agent connector's channel. Restart Kiwix, or sign out and in again.".to_string()
            } else {
                format!("Cannot open the agent connector channel: {error}")
            }
        })?;
        let clients = Arc::new(AtomicUsize::new(0));
        loop {
            let connected = tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                connected = server.connect() => connected,
            };
            // Publish the next instance before serving this client, so the
            // name never disappears (and cannot be taken) while we are on. A
            // failed accept (a client that left at once) is not fatal.
            let Some(next) = next_instance(&name, &descriptor, &cancel).await else { return Ok(()); };
            let accepted = std::mem::replace(&mut server, next);
            if let Err(error) = connected {
                eprintln!("Agent connector: a client left before it was served: {error}");
                continue;
            }
            if clients.fetch_add(1, Ordering::SeqCst) >= super::MAX_CLIENTS {
                clients.fetch_sub(1, Ordering::SeqCst);
                // Say why, instead of looking like Kiwix is not running.
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    let mut accepted = accepted;
                    let refusal = json!({"error": "Too many assistants are connected to Kiwix. Close one and try again."});
                    let _ = super::write_frame(&mut accepted, &refusal, &cancel).await;
                });
                continue;
            }
            let (service, own, cancel, clients) = (service.clone(), own.clone(), cancel.clone(), clients.clone());
            tokio::spawn(async move {
                if let Err(error) = client(service, accepted, own, cancel).await {
                    eprintln!("Agent connector client: {error}");
                }
                clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
}

#[cfg(all(unix, desktop))]
mod transport {
    //! Unix socket at `<profile>/instance/mcp[-<profile hash>].sock`, or, in the
    //! macOS App Sandbox, `<container>/tmp/kiwix-mcp/` so the path stays under
    //! the local socket limit. The directory is 0700 and the socket 0600; each
    //! client is checked against this process's uid.

    use super::super::Service;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::{UnixListener, UnixStream};
    use tokio_util::sync::CancellationToken;

    pub(super) fn is_default_profile(profile: &Path) -> bool {
        if crate::portable::PortableProfile::detect().ok().flatten().is_some() {
            return false;
        }
        let Some(default) = directories::ProjectDirs::from("org", "kiwix", "KiwixWorkbench") else { return false; };
        same_file::is_same_file(profile, default.data_dir()).unwrap_or(false)
    }

    fn socket_dir(service: &Service) -> PathBuf {
        if crate::sandbox::is_sandboxed() {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("tmp")
                .join("kiwix-mcp")
        } else {
            service.store.data_dir().join("instance")
        }
    }

    fn socket_file(service: &Service) -> Result<String, String> {
        let profile = service.store.data_dir();
        if is_default_profile(profile) {
            return Ok("mcp.sock".into());
        }
        let suffix = super::profile_suffix(profile).ok_or("Cannot identify this Kiwix profile")?;
        Ok(format!("mcp-{suffix}.sock"))
    }

    fn peer_is_same_user(stream: &UnixStream) -> bool {
        stream.peer_cred().is_ok_and(|credential| credential.uid() == crate::fs_ext::effective_uid())
    }

    async fn client(service: Arc<Service>, stream: UnixStream, cancel: CancellationToken) -> Result<(), String> {
        if !peer_is_same_user(&stream) {
            return Err("Refused an agent connector client from another account".into());
        }
        super::serve_client(service, stream, cancel).await
    }

    pub(super) async fn serve(service: Arc<Service>, cancel: CancellationToken) -> Result<(), String> {
        let path = socket_dir(&service).join(socket_file(&service)?);
        let (listener, _entry) = crate::instance::socket::bind(&path)
            .map_err(|err| format!("Cannot open the agent connector socket: {err}"))?;
        listener.set_nonblocking(true).map_err(|err| err.to_string())?;
        let listener = UnixListener::from_std(listener).map_err(|err| err.to_string())?;
        let clients = Arc::new(AtomicUsize::new(0));
        loop {
            let accepted = tokio::select! {
                _ = cancel.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let (stream, _) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    eprintln!("Agent connector: cannot accept a client: {error}");
                    continue;
                }
            };
            if clients.fetch_add(1, Ordering::SeqCst) >= super::MAX_CLIENTS {
                clients.fetch_sub(1, Ordering::SeqCst);
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    let mut stream = stream;
                    let refusal = json!({"error": "Too many assistants are connected to Kiwix. Close one and try again."});
                    let _ = super::write_frame(&mut stream, &refusal, &cancel).await;
                });
                continue;
            }
            let (service, cancel, clients) = (service.clone(), cancel.clone(), clients.clone());
            tokio::spawn(async move {
                if let Err(error) = client(service, stream, cancel).await {
                    eprintln!("Agent connector client: {error}");
                }
                clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }
}

#[cfg(not(any(windows, desktop)))]
mod transport {
    use super::super::Service;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    pub(super) fn is_default_profile(_profile: &std::path::Path) -> bool {
        true
    }

    pub(super) async fn serve(_service: Arc<Service>, _cancel: CancellationToken) -> Result<(), String> {
        Err("The agent connector is available on desktop in this version.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_text_loses_hidden_characters() {
        assert_eq!(clean_line("  Bahrain\u{202E}\u{200B} \n island\u{E0041}\u{0007} ", 100), "Bahrain island");
        assert_eq!(clean_line("a\u{061C}b\u{3164}c\u{FE0F}", 100), "abc");
        assert_eq!(clean_line(&"x".repeat(50), 10).len(), 10);
        assert_eq!(clean_line("éé z", 2), "éé");
        assert_eq!(clean_line("éé z", 3), "éé");
    }

    #[test]
    fn rate_limit_allows_a_burst_then_refuses() {
        let connector = AgentConnector::default();
        for _ in 0..10 {
            connector.admit().unwrap();
        }
        assert!(connector.admit().is_err());
        assert_eq!(connector.activity.lock().unwrap().calls_today, 10);
    }

    #[test]
    fn configuration_paths_drop_the_verbatim_prefix() {
        assert_eq!(display_path(std::path::Path::new(r"\\?\E:\kiwix\data")), r"E:\kiwix\data");
        assert_eq!(display_path(std::path::Path::new(r"\\?\UNC\server\share\data")), r"\\server\share\data");
        assert_eq!(display_path(std::path::Path::new("/home/a/data")), "/home/a/data");
    }

    #[cfg(all(unix, desktop))]
    #[test]
    fn a_temporary_profile_is_not_the_default_profile() {
        assert!(!transport::is_default_profile(&std::env::temp_dir()));
    }

    #[test]
    fn profile_suffix_matches_the_relay_rule() {
        let dir = std::env::temp_dir();
        let suffix = profile_suffix(&dir).unwrap();
        assert_eq!(suffix.len(), 16);
        assert_eq!(profile_suffix(&dir.join(".")), Some(suffix));
    }
}
