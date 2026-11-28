// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Bounded, redacted local crash capture.
//!
//! A panic (or a native uncaught exception routed here) writes one small JSON
//! record under the application's private data directory. Nothing is sent
//! automatically and no network code lives in this module. The next launch can
//! show, copy, mail or delete the record.
//!
//! Privacy rules applied before anything reaches disk:
//! * External HTTP(S) URLs lose credentials, queries, and fragments. Local
//!   reader URLs and other URI schemes do not keep hosts or paths.
//! * Path-like text may be a reader route; do not keep its final component.
//!   Compiler-supplied panic locations and backtrace locations keep filenames.
//! * Values following token/secret/password/cookie/bearer keywords are replaced.
//! * Long opaque token-like runs are replaced.
//! * Every field is length-bounded; the whole record is capped at 32 KiB.
//!
//! The order of those steps is part of the rule (see `redact`). The Swift and
//! Kotlin crash intakes implement the same pipeline, and all three are pinned
//! to `tests/fixtures/crash-redaction-cases.json`.
//!
//! The record never contains archive contents, bookmark or note text, library
//! paths, user document paths, or URLs with sensitive query strings.

use serde_json::{json, Map, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

pub const SCHEMA: u32 = 1;
const RECORD_DIR: &str = "crash";
const RECORD_FILE: &str = "pending.json";
const PREVIOUS_FILE: &str = "previous.json";
const TEMP_FILE: &str = ".pending.json.tmp";
const MAX_RECORD_BYTES: usize = 32 * 1024;
const MAX_MESSAGE_CHARS: usize = 2_000;
/// Text is cut to this many characters before any redaction step runs. The
/// stored fields are far shorter, so nothing useful is lost, and no step can
/// be handed an input long enough to exhaust it and pass text through.
const MAX_REDACTION_INPUT_CHARS: usize = 32_768;
const MAX_STACK_FRAMES: usize = 24;
const MAX_FRAME_CHARS: usize = 240;
const MAX_THREAD_CHARS: usize = 64;
const MAX_VALIDATION_ENTRIES: usize = 1_024;

struct Capture {
    dir: PathBuf,
    enabled: bool,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);
static HOOK: LazyLock<()> = LazyLock::new(|| {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The hook must never introduce a second panic.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capture_panic(info);
        }));
        previous(info);
    }));
});
static VALIDATIONS: LazyLock<Mutex<BTreeMap<String, String>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Clone)]
struct ReadingContext {
    archive_id: String,
    installation_id: String,
    operation: String,
}

thread_local! {
    static READING: RefCell<Option<ReadingContext>> = const { RefCell::new(None) };
}

/// Restores the previous reading context when dropped, including during unwind.
pub struct ReadingGuard {
    previous: Option<ReadingContext>,
}

impl Drop for ReadingGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        READING.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                *slot = previous;
            }
        });
    }
}

/// Marks the current thread as serving `archive_id`. A panic on this thread then
/// records the operation phase and the last known validation result.
pub fn reading(archive_id: &str, installation_id: &str, operation: &str) -> ReadingGuard {
    let next = ReadingContext {
        archive_id: archive_id.to_owned(),
        installation_id: installation_id.to_owned(),
        operation: operation.to_owned(),
    };
    let previous = READING.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.replace(next)
    });
    ReadingGuard { previous }
}

/// Installs the process panic hook. `read_only` profiles keep the hook inert
/// because they must not mutate the profile.
pub fn install(data_dir: &Path, read_only: bool) {
    let capture = Capture {
        dir: data_dir.join(RECORD_DIR),
        enabled: !read_only,
    };
    if let Ok(mut slot) = CAPTURE.lock() {
        *slot = Some(capture);
    }
    LazyLock::force(&HOOK);
}

/// Apply the effective runtime profile policy. The capture lock also covers
/// filesystem publication, so disabling waits for any previously admitted
/// record to finish and no stale capture can write after this returns.
pub fn set_read_only(read_only: bool) {
    let mut slot = CAPTURE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(capture) = slot.as_mut() {
        capture.enabled = !read_only;
    }
}

/// Remembers a completed libzim/retained-byte validation for this installation.
/// Only the two terminal outcomes are useful in a crash report.
pub fn note_archive_validation(archive_id: &str, installation_id: &str, status: &str) {
    record_archive_validation(archive_id, installation_id, status, true);
}

/// Seeds crash-report validation state from persisted metadata without
/// overwriting a fresher live validation recorded during this session.
pub fn seed_archive_validation(archive_id: &str, installation_id: &str, status: &str) {
    record_archive_validation(archive_id, installation_id, status, false);
}

fn record_archive_validation(
    archive_id: &str,
    installation_id: &str,
    status: &str,
    overwrite: bool,
) {
    if !matches!(status, "valid" | "invalid") {
        return;
    }
    let key = validation_key(archive_id, installation_id);
    let Ok(mut entries) = VALIDATIONS.lock() else {
        return;
    };
    if !overwrite && entries.contains_key(&key) {
        return;
    }
    if entries.len() >= MAX_VALIDATION_ENTRIES && !entries.contains_key(&key) {
        if let Some(first) = entries.keys().next().cloned() {
            entries.remove(&first);
        }
    }
    entries.insert(key, status.to_owned());
}

fn validation_key(archive_id: &str, installation_id: &str) -> String {
    format!("{archive_id}:{installation_id}")
}

fn validation_for(archive_id: &str, installation_id: &str) -> &'static str {
    if installation_id.is_empty() {
        return "unknown";
    }
    match VALIDATIONS.lock() {
        Ok(entries) => match entries
            .get(&validation_key(archive_id, installation_id))
            .map(String::as_str)
        {
            Some("valid") => "valid",
            Some("invalid") => "invalid",
            _ => "unknown",
        },
        Err(_) => "unknown",
    }
}

fn capture_panic(info: &std::panic::PanicHookInfo<'_>) {
    let slot = CAPTURE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(capture) = slot.as_ref().filter(|capture| capture.enabled) else {
        return;
    };
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("panic payload is not text");
    let location = info
        .location()
        .map(|location| format!("{}:{}", file_name(location.file()), location.line()))
        .unwrap_or_default();
    let reading = READING.with(|slot| (*slot.borrow()).clone());
    let record = build_record(
        message,
        &location,
        &std::backtrace::Backtrace::force_capture().to_string(),
        reading.as_ref(),
    );
    if let Err(error) = write_record(&capture.dir, &record, true) {
        eprintln!("Kiwix crash record unavailable: {error}");
    }
}

fn build_record(
    message: &str,
    location: &str,
    backtrace: &str,
    reading: Option<&ReadingContext>,
) -> Value {
    let mut record = Map::new();
    record.insert("schema".into(), json!(SCHEMA));
    record.insert("capturedAt".into(), json!(crate::timestamps::now()));
    record.insert("appVersion".into(), json!(env!("CARGO_PKG_VERSION")));
    record.insert("platform".into(), json!(std::env::consts::OS));
    record.insert("architecture".into(), json!(std::env::consts::ARCH));
    let thread = std::thread::current();
    record.insert(
        "thread".into(),
        json!(truncate_chars(
            &redact(thread.name().unwrap_or("unnamed")),
            MAX_THREAD_CHARS
        )),
    );
    record.insert("message".into(), json!(redact(message)));
    record.insert("location".into(), json!(redact(location)));
    record.insert("stack".into(), json!(sanitize_backtrace(backtrace)));
    if let Some(reading) = reading {
        record.insert("phase".into(), json!("reader"));
        record.insert("operation".into(), json!(redact(&reading.operation)));
        record.insert(
            "archive".into(),
            json!({
                "id": redact(&reading.archive_id),
                "validation": validation_for(&reading.archive_id, &reading.installation_id),
            }),
        );
    } else {
        record.insert("phase".into(), json!("application"));
        record.insert("operation".into(), json!("unknown"));
    }
    Value::Object(record)
}

fn sanitize_backtrace(backtrace: &str) -> Vec<String> {
    let mut frames = Vec::new();
    let mut lines = backtrace.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        // std's stable Display output puts the frame number on the symbol
        // line and its optional source location on a separate, unnumbered line.
        if !line.split_once(':').is_some_and(|(number, _)| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            continue;
        }
        let mut frame = redact_line(line);
        if let Some(location) = lines.next_if(|line| line.starts_with("at ")) {
            frame.push_str(" at ");
            let location = replace_urls(&location[3..]);
            frame.push_str(&redact_secret_values(file_name(&location)));
        }
        frames.push(truncate_chars(&frame, MAX_FRAME_CHARS));
        if frames.len() == MAX_STACK_FRAMES {
            break;
        }
    }
    frames
}

fn write_record(dir: &Path, record: &Value, rotate: bool) -> Result<(), String> {
    let bytes = bounded_bytes(record)?;
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let temp = dir.join(TEMP_FILE);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    let pending = dir.join(RECORD_FILE);
    if rotate && pending.exists() {
        let _ = fs::rename(&pending, dir.join(PREVIOUS_FILE));
    }
    fs::rename(&temp, &pending).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    if let Ok(directory) = File::open(dir) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn bounded_bytes(record: &Value) -> Result<Vec<u8>, String> {
    let mut value = record.clone();
    for _ in 0..MAX_STACK_FRAMES + 2 {
        let bytes = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
        if bytes.len() <= MAX_RECORD_BYTES {
            return Ok(bytes);
        }
        let dropped = value
            .get_mut("stack")
            .and_then(Value::as_array_mut)
            .is_some_and(|stack| {
                stack.pop();
                true
            });
        if !dropped {
            break;
        }
    }
    // The bounded fields already fit; this is a last-resort hard stop.
    let fallback = json!({
        "schema": SCHEMA,
        "capturedAt": crate::timestamps::now(),
        "appVersion": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "phase": value.get("phase").cloned().unwrap_or(json!("application")),
        "operation": value.get("operation").cloned().unwrap_or(json!("unknown")),
        "message": "[record exceeded size limit]",
        "stack": [],
    });
    serde_json::to_vec_pretty(&fallback).map_err(|error| error.to_string())
}

fn record_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RECORD_DIR).join(RECORD_FILE)
}

/// Reads the stored record, if any. A reported record is returned as-is so the
/// Settings panel can still show or delete it; the startup prompt only offers
/// records without `reportedAt`.
pub fn pending(data_dir: &Path) -> Result<Option<Value>, String> {
    let path = record_path(data_dir);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Kiwix could not read the saved crash report. Close this prompt and try again".to_owned()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if metadata.len() > MAX_RECORD_BYTES as u64 {
        return Err("The saved crash report is too large to open. Delete it from Troubleshooting".into());
    }
    let mut bytes = Vec::new();
    use std::io::Read;
    file.take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err("The saved crash report is too large to open. Delete it from Troubleshooting".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The saved crash report is damaged. Delete it from Troubleshooting".to_string())?;
    if value.get("schema").and_then(Value::as_u64) != Some(SCHEMA as u64) {
        return Err("The saved crash report was written by a different version of Kiwix. Delete it from Troubleshooting".into());
    }
    Ok(Some(value))
}

/// Marks the record as handed to the user's mail composer. The file stays on
/// disk until the user deletes it.
pub fn mark_reported(data_dir: &Path) -> Result<bool, String> {
    let Some(mut record) = pending(data_dir)? else {
        return Ok(false);
    };
    if let Some(object) = record.as_object_mut() {
        object.insert("reportedAt".into(), json!(crate::timestamps::now()));
    }
    write_record(&data_dir.join(RECORD_DIR), &record, false)?;
    Ok(true)
}

/// Deletes the current record and any rotated predecessor. Returns whether any
/// file was removed.
pub fn delete(data_dir: &Path) -> Result<bool, String> {
    let dir = data_dir.join(RECORD_DIR);
    let mut removed = false;
    for name in [RECORD_FILE, PREVIOUS_FILE, TEMP_FILE] {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => removed = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Kiwix could not delete the saved crash report. Try again".to_owned()),
        }
    }
    Ok(removed)
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut result: String = value.chars().take(limit.saturating_sub(1)).collect();
    result.push('…');
    result
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bounded_input(value: &str) -> String {
    value.chars().take(MAX_REDACTION_INPUT_CHARS).collect()
}

/// Full redaction for one bounded text field. The steps run in this order:
///
/// 1. Keep at most `MAX_REDACTION_INPUT_CHARS` characters.
/// 2. Secret values (`redact_secret_values`), including `bearer`. First,
///    because a value that is a path with spaces must go as a whole, and a
///    path must not swallow the key that marks the text after it as secret.
/// 3. URLs (`replace_urls`).
/// 4. Paths (`redact_paths`).
/// 5. Long opaque tokens (`redact_opaque_tokens`).
/// 6. Collapse whitespace, then cut to `MAX_MESSAGE_CHARS`.
fn redact(value: &str) -> String {
    let value = bounded_input(value);
    let value = redact_secret_values(&value);
    let value = replace_urls(&value);
    let value = redact_paths(&value);
    let value = redact_opaque_tokens(&value);
    truncate_chars(&collapse_whitespace(&value), MAX_MESSAGE_CHARS)
}

/// Line redaction for backtrace frames: steps 1 to 4 of `redact`, keeping
/// spacing so frames stay readable.
fn redact_line(value: &str) -> String {
    let value = bounded_input(value);
    let value = redact_secret_values(&value);
    let value = replace_urls(&value);
    redact_paths(&value)
}

fn replace_urls(value: &str) -> String {
    const OPAQUE_SCHEMES: &[&str] = &["blob", "content", "data", "file", "mailto", "tauri", "tel", "zim"];
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    for (colon, _) in value.match_indices(':') {
        if colon < cursor {
            continue;
        }
        let scheme_len = value.as_bytes()[cursor..colon]
            .iter()
            .rev()
            .take_while(|&&byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
            .count();
        let start = colon - scheme_len;
        let scheme = &value[start..colon];
        if !scheme.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let tail = &value[colon + 1..];
        if !tail.starts_with("//")
            && (tail.starts_with(':')
                || !OPAQUE_SCHEMES.iter().any(|name| scheme.eq_ignore_ascii_case(name)))
        {
            continue;
        }
        let token_end = tail
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '<' | '>' | '`')
            })
            .map_or(value.len(), |end| colon + 1 + end);
        // A URL that names a local file or an archive page is a path and may
        // be written with unencoded spaces: it ends where a path would.
        let path_like_end = || {
            let chars: Vec<char> = tail.chars().collect();
            let length = path_end(&chars, 0);
            colon + 1 + chars[..length].iter().map(|character| character.len_utf8()).sum::<usize>()
        };
        let (end, replacement) = if scheme.eq_ignore_ascii_case("file")
            && tail.chars().next().is_some_and(|first| !first.is_whitespace())
        {
            // Any form (`file:///x`, `file:C:/x`). `file: ` followed by a
            // space is prose; a path after it is found by `redact_paths`.
            (path_like_end(), "[url]".to_owned())
        } else {
            let replacement = safe_url(&value[start..token_end], scheme);
            // Every `scheme://` URL that is not kept (reader pages, content
            // and archive links, shares) goes whole, like a path.
            if tail.starts_with("//") && matches!(replacement.as_str(), "[url]" | "[reader-url]") {
                (token_end.max(path_like_end()), replacement)
            } else {
                (token_end, replacement)
            }
        };
        output.push_str(&value[cursor..start]);
        output.push_str(&replacement);
        cursor = end;
    }
    output.push_str(&value[cursor..]);
    output
}

fn safe_url(raw: &str, scheme: &str) -> String {
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return "[url]".into();
    }
    let Ok(url) = url::Url::parse(raw) else {
        return "[url]".into();
    };
    let Some(host) = url.host() else {
        return "[url]".into();
    };
    let local = match host {
        url::Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.');
            domain == "localhost" || domain.ends_with(".localhost")
        }
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback())
        }
    };
    let path = url.path();
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    let nested = decoded.contains("://")
        || ["mailto:", "data:", "file:", "content:", "zim:", "blob:", "tauri:", "tel:"]
            .iter()
            .any(|prefix| decoded.contains(prefix));
    if local || decoded.contains("/.kiwix/") || nested {
        return "[reader-url]".into();
    }
    let port = url.port().map(|port| format!(":{port}")).unwrap_or_default();
    format!("{}://{host}{port}{path}", url.scheme())
}

fn redact_paths(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        if let Some(end) = path_at(&chars, index) {
            output.push_str("[path]");
            index = end;
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

/// Whether a path may begin right after `previous` (`None` is the start of
/// the text).
fn path_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|previous| {
        previous.is_whitespace()
            || matches!(
                previous,
                '=' | '(' | '[' | '{' | '"' | '\'' | ',' | ';' | ':' | '`' | '<' | '>' | '|' | '“' | '‘'
            )
    })
}

/// Whether the text at `index` has the shape of a path start: a drive
/// (`C:\`, `C:/`), an absolute path (`/x`), a home path (`~/`, `~\`), or a
/// share (`\\`, `//host`).
fn path_start(chars: &[char], index: usize) -> bool {
    let Some(&current) = chars.get(index) else {
        return false;
    };
    let next = chars.get(index + 1).copied();
    let name = |character: Option<char>| character.is_some_and(|c| c != '/' && !c.is_whitespace());
    let windows_drive =
        current.is_ascii_alphabetic() && next == Some(':') && matches!(chars.get(index + 2), Some('\\' | '/'));
    let unix = current == '/' && name(next);
    let home = current == '~' && matches!(next, Some('/' | '\\'));
    let share = (current == '\\' && next == Some('\\'))
        || (current == '/' && next == Some('/') && name(chars.get(index + 2).copied()));
    windows_drive || unix || home || share
}

fn path_at(chars: &[char], index: usize) -> Option<usize> {
    let previous = index.checked_sub(1).map(|before| chars[before]);
    if !path_boundary(previous) || !path_start(chars, index) {
        return None;
    }
    // `//host` right after a colon is the authority of a URL that the URL step
    // kept (`https://example.org/a`), not a share.
    if previous == Some(':') && chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
        return None;
    }
    Some(path_end(chars, index))
}

/// Where the redaction of a path that starts at `start` stops.
///
/// Folder and file names contain spaces (`C:\Users\Jane Doe\…`,
/// `/Volumes/Family Backup/…`), so whitespace cannot end a path: stopping there
/// kept every later component. Only text that cannot continue the path ends
/// it: a closing double quote, an angle bracket, a backtick, a line break, or
/// the `: ` that introduces an error's cause. Prose between the path and that
/// delimiter is dropped with it. Losing those words is the accepted price of
/// never keeping part of a path. An apostrophe does not end a path, because
/// names such as `Jane's Backup` contain one.
fn path_end(chars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < chars.len() {
        let stop = match chars[end] {
            '\n' | '\r' | '<' | '>' | '`' => true,
            // Debug-formatted paths escape an embedded quote as `\"`.
            '"' => !backslash_escaped(chars, start, end),
            ':' => chars.get(end + 1).is_some_and(|next| next.is_whitespace()),
            _ => false,
        };
        if stop {
            break;
        }
        end += 1;
    }
    // The separator before the delimiter is not part of the path.
    while end > start && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    end
}

/// Replace the value that follows a secret keyword with `[redacted]`.
///
/// A keyword is one of `SECRET_KEYS`, in any letter case, with no ASCII word
/// character (`A-Z a-z 0-9 _`) directly before or after it. After the keyword
/// come, in order: an optional closing quote of a quoted key (`"password":`),
/// optional spaces, tabs or line breaks, and `=` or `:` followed by optional
/// spaces or tabs. `authorization` and `bearer` also accept plain whitespace
/// instead of `=` or `:`. What follows is the value (`secret_value`).
fn redact_secret_values(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < chars.len() {
        if let Some((start, end)) = secret_at(&chars, cursor) {
            output.extend(&chars[cursor..start]);
            output.push_str("[redacted]");
            cursor = end;
            continue;
        }
        output.push(chars[cursor]);
        cursor += 1;
    }
    output
}

const SECRET_KEYS: [&str; 12] = [
    "authorization",
    "access_token",
    "refresh_token",
    "api_key",
    "apikey",
    "password",
    "passwd",
    "cookie",
    "bearer",
    "token",
    "secret",
    "pin",
];

fn is_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// The value range of a secret whose keyword starts at `at`, if any.
fn secret_at(chars: &[char], at: usize) -> Option<(usize, usize)> {
    if at > 0 && is_word_char(chars[at - 1]) {
        return None;
    }
    for key in SECRET_KEYS {
        let after_key = at + key.len();
        if after_key > chars.len()
            || !chars[at..after_key]
                .iter()
                .zip(key.chars())
                .all(|(character, expected)| character.to_ascii_lowercase() == expected)
            || chars.get(after_key).is_some_and(|next| is_word_char(*next))
        {
            continue;
        }
        let mut next = after_key;
        let quoted_key = matches!(chars.get(next), Some('"' | '\''));
        if quoted_key {
            next += 1;
        }
        let gap = next;
        while matches!(chars.get(next), Some(' ' | '\t' | '\r' | '\n')) {
            next += 1;
        }
        if matches!(chars.get(next), Some('=' | ':')) {
            next += 1;
            while matches!(chars.get(next), Some(' ' | '\t')) {
                next += 1;
            }
        } else if quoted_key || next == gap || !matches!(key, "authorization" | "bearer") {
            continue;
        }
        let (start, end) = secret_value(chars, next, key);
        if end > start {
            return Some((start, end));
        }
    }
    None
}

/// The range of the secret value that begins at `start`.
///
/// * `"…"`: everything up to the closing quote (one not escaped by a
///   backslash), a line break or the end. The quotes stay.
/// * `'…'`: everything up to the last `'` on the line, or to the end of the
///   line when there is none. A value may itself contain an apostrophe.
/// * A path start: everything `path_end` takes, so a path with spaces goes
///   whole.
/// * After `cookie`: the rest of the line, because one header carries several
///   `name=value` pairs.
/// * Otherwise one token, which ends at a space, tab, line break or one of
///   `, ; & ) ] }`. After `authorization` a second token separated by spaces
///   or tabs goes too (`Basic dXNlcjpwYXNz`, `Bearer …`).
fn secret_value(chars: &[char], start: usize, key: &str) -> (usize, usize) {
    let line_end = |from: usize| {
        (from..chars.len())
            .find(|index| matches!(chars[*index], '\n' | '\r'))
            .unwrap_or(chars.len())
    };
    let token_end = |from: usize| {
        (from..chars.len())
            .find(|index| matches!(chars[*index], ' ' | '\t' | '\r' | '\n' | ',' | ';' | '&' | ')' | ']' | '}'))
            .unwrap_or(chars.len())
    };
    match chars.get(start) {
        Some('"') => {
            let line = line_end(start + 1);
            let close = (start + 1..line)
                .find(|index| chars[*index] == '"' && !backslash_escaped(chars, start + 1, *index));
            (start + 1, close.unwrap_or(line))
        }
        Some('\'') => {
            let line = line_end(start + 1);
            let close = (start + 1..line).rev().find(|index| chars[*index] == '\'');
            (start + 1, close.unwrap_or(line))
        }
        _ if path_start(chars, start) => (start, path_end(chars, start)),
        _ if key == "cookie" => (start, line_end(start)),
        _ => {
            let mut end = token_end(start);
            if key == "authorization" && end > start {
                let mut second = end;
                while matches!(chars.get(second), Some(' ' | '\t')) {
                    second += 1;
                }
                if second > end && token_end(second) > second {
                    end = token_end(second);
                }
            }
            (start, end)
        }
    }
}

/// Whether the character at `index` follows an odd run of backslashes that
/// begins no earlier than `floor`.
fn backslash_escaped(chars: &[char], floor: usize, index: usize) -> bool {
    chars[floor..index].iter().rev().take_while(|character| **character == '\\').count() % 2 == 1
}

fn redact_opaque_tokens(value: &str) -> String {
    const MIN_TOKEN: usize = 40;
    let mut output = String::with_capacity(value.len());
    let mut run = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric()
            || matches!(character, '_' | '-' | '+' | '/' | '=' | '.')
        {
            run.push(character);
            continue;
        }
        push_opaque(&mut output, &run, MIN_TOKEN);
        run.clear();
        output.push(character);
    }
    push_opaque(&mut output, &run, MIN_TOKEN);
    output
}

fn push_opaque(output: &mut String, run: &str, minimum: usize) {
    if run.len() >= minimum
        && run.chars().any(|character| character.is_ascii_digit())
        && run.chars().any(|character| character.is_ascii_alphabetic())
    {
        output.push_str("[redacted]");
    } else {
        output.push_str(run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kiwix-crash-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn redaction_drops_paths_url_authority_and_secrets() {
        let text = redact(
            "open /Users/alice/Private/notes.txt?token=abc failed: \
             https://example.org/read?token=secret&q=book#frag \
             reader http://127.0.0.1:9/.kiwix/8f4c2a9e1b/A/Secret_Page \
             Authorization: Bearer abc.def.ghi token=deadbeef \
             blob=QWxhZGRpbjpvcGVuIHNlc2FtZTEyMzQ1Njc4OTBhYmNkZWY=",
        );
        assert!(text.contains("[path]"), "{text}");
        assert!(!text.contains("notes.txt"), "{text}");
        assert!(!text.contains("alice"), "{text}");
        assert!(!text.contains("secret&q"), "{text}");
        assert!(!text.contains("?token"), "{text}");
        assert!(!text.contains("8f4c2a9e1b"), "{text}");
        assert!(!text.contains("abc.def.ghi"), "{text}");
        assert!(!text.contains("deadbeef"), "{text}");
        assert!(!text.contains("QWxhZGRpbjpvcGVu"), "{text}");
        assert!(text.contains("[redacted]"), "{text}");
    }

    #[test]
    fn redaction_keeps_no_part_of_a_path_that_contains_spaces() {
        // Stopping at the first space kept ` Doe\AppData\…` and
        // ` Backup/Health Records/…`: the rest of the profile or library path.
        let windows = redact(
            "Failed to setup app: Cannot parse C:\\Users\\Jane Doe\\AppData\\Roaming\\kiwix\\My Library\\state.json: expected value at line 1 column 1",
        );
        assert_eq!(windows, "Failed to setup app: Cannot parse [path]: expected value at line 1 column 1");
        let volume = redact("open /Volumes/Family Backup/Health Records/medical_wikipedia.zim failed");
        assert_eq!(volume, "open [path]");
        for leaked in ["Doe", "AppData", "Library", "Backup", "Health", "medical_wikipedia"] {
            assert!(!windows.contains(leaked) && !volume.contains(leaked), "{windows} / {volume}");
        }
        // Quoted paths end at their closing quote, so the cause survives.
        assert_eq!(
            redact("cannot read \"/home/jane doe/My Books/a.zim\" (os error 13)"),
            "cannot read \"[path]\" (os error 13)"
        );
        assert_eq!(
            redact("cannot read \"C:\\\\Users\\\\Jane Doe\\\\\" (os error 5)"),
            "cannot read \"[path]\" (os error 5)"
        );
        // An apostrophe is part of a name, not the end of the path.
        let apostrophe = redact("missing '/Volumes/Jane's Backup/Private Notes/a.zim' on disk");
        assert_eq!(apostrophe, "missing '[path]");
        // A UNC share, a home-relative path and a file URL with raw spaces.
        assert_eq!(redact("at \\\\nas\\Family Share\\a.zim now"), "at [path]");
        assert_eq!(redact("at ~/My Documents/Private Notes.txt now"), "at [path]");
        assert_eq!(redact("open file:///Users/Jane Doe/Private Notes.txt now"), "open [url]");
        // The frame line of a backtrace is redacted the same way.
        assert_eq!(redact_line("3: read /Users/Jane Doe/Secret Notes/a.zim again"), "3: read [path]");
    }

    #[test]
    fn local_reader_and_non_http_urls_keep_no_path_or_host_identity() {
        // One input per line: a URL that is not kept is redacted to the end of
        // its clause, like a path, so it would hide the inputs after it.
        for (input, expected) in [
            (
                "reader HTTP://12b34567-1234-5678-90ab-123456789abc-deadbeef.localhost.:9/A/Pri)vate_page?secret=1",
                "reader [reader-url]",
            ),
            ("loopback http://127.0.0.2:9/book-uuid/A/Private", "loopback [reader-url]"),
            ("ipv6 http://[::1]:9/book-uuid/A/Private", "ipv6 [reader-url]"),
            (
                "bridge https://example.org/book-uuid/%2Ekiwix/token/A/Private",
                "bridge [reader-url]",
            ),
            ("path=/book-uuid/A/Private relative=/.kiwix/token/A/Private", "path=[path]"),
            ("relative=/.kiwix/token/A/Private", "relative=[path]"),
            ("ZIM://book-uuid/A/Private", "[url]"),
            ("MAILTO:private@example.org", "[url]"),
            ("content://provider/Private", "[url]"),
            ("file:///Documents/Private%20Notes.txt", "[url]"),
            ("custom://host/Private", "[url]"),
            (
                "public https://user:pass@example.org/a/b?token=secret#x",
                "public https://example.org/a/b",
            ),
            (
                "adjacent https://example.org/a,http://book-uuid.localhost/A/Private",
                "adjacent [reader-url]",
            ),
        ] {
            assert_eq!(redact(input), expected, "{input}");
        }
    }

    /// The vectors the Swift and Kotlin crash intakes are ported against. Each
    /// `expected` is the exact output of `redact` for `input`.
    #[test]
    fn redaction_matches_the_canonical_cases_shared_with_the_mobile_intakes() {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Case {
            input: String,
            expected: String,
        }
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/fixtures/crash-redaction-cases.json")).unwrap();
        assert!(cases.len() >= 80, "the shared fixture lost cases: {}", cases.len());
        let differing: Vec<String> = cases
            .iter()
            .filter_map(|case| {
                let actual = redact(&case.input);
                (actual != case.expected).then(|| {
                    format!("input:    {:?}\nexpected: {:?}\nactual:   {:?}", case.input, case.expected, actual)
                })
            })
            .collect();
        assert!(
            differing.is_empty(),
            "{} of {} cases differ:\n{}",
            differing.len(),
            cases.len(),
            differing.join("\n\n")
        );
    }

    #[test]
    fn secrets_go_first_so_neither_a_spaced_path_nor_a_swallowed_key_leaks() {
        // Secrets after paths: the path took the `password` key and the value
        // was left behind.
        assert_eq!(redact("open /var/x password: \"hunter2\""), "open [path]: \"[redacted]\"");
        // Secrets before paths, with a value that ends at the first space:
        // ` Books/a.zim` was left behind.
        for input in [
            "password=/storage/emulated/0/My Books/a.zim",
            "token: /sdcard/My Books/a.zim",
            "bearer /storage/My Books/a.zim",
        ] {
            let output = redact(input);
            assert!(output.ends_with("[redacted]"), "{output}");
            assert!(!output.contains("Books") && !output.contains("a.zim"), "{output}");
        }
        // The frame line of a backtrace runs the same steps in the same order.
        assert_eq!(
            redact_line("3: read /Users/Jane Doe/Secret Notes/a.zim password: \"hunter2\""),
            "3: read [path]: \"[redacted]\""
        );
    }

    #[test]
    fn redaction_input_is_bounded_before_any_step_runs() {
        let long = format!("/{}", "a".repeat(MAX_REDACTION_INPUT_CHARS * 4));
        assert_eq!(bounded_input(&long).chars().count(), MAX_REDACTION_INPUT_CHARS);
        assert_eq!(redact(&long), "[path]");
        assert_eq!(redact_line(&long), "[path]");
        // Characters, not bytes: a multi-byte character is never split.
        let wide = "é".repeat(MAX_REDACTION_INPUT_CHARS + 10);
        assert_eq!(bounded_input(&wide).chars().count(), MAX_REDACTION_INPUT_CHARS);
        // A short field is untouched.
        assert_eq!(bounded_input("lib.rs:12"), "lib.rs:12");
        // A panic location is never redacted.
        assert_eq!(redact("lib.rs:1234"), "lib.rs:1234");
        assert_eq!(redact("store.rs:88"), "store.rs:88");
    }

    #[test]
    fn backtrace_budget_counts_frames_not_source_lines() {
        let mut backtrace = "stack backtrace:\n".to_owned();
        for number in 0..MAX_STACK_FRAMES + 2 {
            let name = if number == 20 { "tauri::reader::read_article" } else { "std::runtime" };
            backtrace.push_str(&format!(
                " {number:2}: {name}\n         at /Users/private/source file.rs:12:23\n"
            ));
        }
        let stack = sanitize_backtrace(&backtrace);
        assert_eq!(stack.len(), MAX_STACK_FRAMES);
        assert_eq!(stack[20], "20: tauri::reader::read_article at source file.rs:12:23");
        assert!(stack.last().unwrap().starts_with("23: "), "{stack:?}");
        assert!(stack.iter().all(|frame| !frame.contains("private")));

        let long_frame = format!("0: {}", "💥".repeat(MAX_FRAME_CHARS * 2));
        assert_eq!(sanitize_backtrace(&long_frame)[0].chars().count(), MAX_FRAME_CHARS);
    }

    #[test]
    fn actual_panic_round_trip_is_bounded_redacted_and_attached_to_validation() {
        // Service fixtures also change this process-global hook's capture policy.
        // Keep the real panic/policy transition in a process with no sibling tests.
        const CHILD: &str = "KIWIX_CRASH_CAPTURE_ISOLATED_TEST";
        if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "crash::tests::actual_panic_round_trip_is_bounded_redacted_and_attached_to_validation", "--test-threads=1"])
                .env(CHILD, "1")
                .output()
                .expect("launch isolated real panic capture");
            assert!(output.status.success(), "isolated panic capture failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        let profile = Profile::new();
        install(&profile.0, true);
        assert!(std::panic::catch_unwind(|| panic!("read-only startup")).is_err());
        assert!(!profile.0.join(RECORD_DIR).exists());
        set_read_only(false);
        note_archive_validation("book-uuid", "installation-uuid", "invalid");
        let result = std::panic::catch_unwind(|| {
            let _reading = reading(
                "book-uuid",
                "installation-uuid",
                "read_article /Users/alice/secret.txt?token=abc",
            );
            #[inline(never)]
            fn panic_from_reader() {
                panic!(
                    "synthetic crash at /Users/alice/Private/notes.txt \
                     reader=http://book-uuid-reader.localhost:9/A/Private_Article \
                     url=https://example.org/a?token=leak secret=leak"
                );
            }
            panic_from_reader();
        });
        assert!(result.is_err());
        let record = pending(&profile.0).unwrap().expect("panic record");
        let text = serde_json::to_string(&record).unwrap();
        assert_eq!(record["schema"], json!(SCHEMA));
        assert_eq!(record["phase"], "reader");
        assert_eq!(record["archive"]["id"], "book-uuid");
        assert_eq!(record["archive"]["validation"], "invalid");
        assert!(!text.contains("alice"), "{text}");
        assert!(!text.contains("leak"), "{text}");
        assert!(!text.contains("token="), "{text}");
        assert!(!text.contains("book-uuid-reader"), "{text}");
        assert!(!text.contains("Private_Article"), "{text}");
        let stack = record["stack"].as_array().expect("stack frames");
        assert!(!stack.is_empty() && stack.len() <= MAX_STACK_FRAMES, "{stack:?}");
        #[cfg(debug_assertions)]
        assert!(stack.iter().any(|frame| {
            frame.as_str().unwrap().contains("panic_from_reader")
        }), "{stack:?}");
        let saved_bytes = fs::metadata(record_path(&profile.0)).unwrap().len();
        assert!(saved_bytes <= MAX_RECORD_BYTES as u64);
        assert!(mark_reported(&profile.0).unwrap());
        let reported = pending(&profile.0).unwrap().unwrap();
        assert!(reported.get("reportedAt").is_some());
        assert!(delete(&profile.0).unwrap());
        assert!(pending(&profile.0).unwrap().is_none());
        set_read_only(true);
        assert!(std::panic::catch_unwind(|| panic!("read-only capture must stay inert")).is_err());
        assert!(pending(&profile.0).unwrap().is_none());
        assert!(!profile.0.join(RECORD_DIR).join(PREVIOUS_FILE).exists());
        set_read_only(false);
        assert!(std::panic::catch_unwind(|| panic!("writable capture resumes")).is_err());
        let resumed = pending(&profile.0).unwrap().expect("capture after leaving read-only mode");
        assert_eq!(resumed["message"], "writable capture resumes");
        let unchanged = fs::read(record_path(&profile.0)).unwrap();
        set_read_only(true);
        assert!(std::panic::catch_unwind(|| panic!("must not rotate or replace")).is_err());
        assert_eq!(fs::read(record_path(&profile.0)).unwrap(), unchanged);
        assert!(!profile.0.join(RECORD_DIR).join(PREVIOUS_FILE).exists());
        // Keep later panics in this process from writing to the deleted profile.
        if let Ok(mut slot) = CAPTURE.lock() {
            *slot = None;
        }
    }
}
