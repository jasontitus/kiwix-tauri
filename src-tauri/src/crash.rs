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

/// Full redaction for one bounded text field.
fn redact(value: &str) -> String {
    let value = replace_urls(value);
    let value = redact_paths(&value);
    let value = redact_secret_values(&value);
    let value = redact_opaque_tokens(&value);
    truncate_chars(&collapse_whitespace(&value), MAX_MESSAGE_CHARS)
}

/// Line redaction for backtrace frames: keeps spacing so frames stay readable.
fn redact_line(value: &str) -> String {
    let value = replace_urls(value);
    let value = redact_paths(&value);
    redact_secret_values(&value)
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
        let end = tail
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '<' | '>' | '`')
            })
            .map_or(value.len(), |end| colon + 1 + end);
        output.push_str(&value[cursor..start]);
        output.push_str(&safe_url(&value[start..end], scheme));
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

fn path_at(chars: &[char], index: usize) -> Option<usize> {
    let boundary = index == 0
        || chars[index - 1].is_whitespace()
        || matches!(
            chars[index - 1],
            '=' | '(' | '[' | '{' | '"' | '\'' | ',' | ';' | ':' | '`'
        );
    if !boundary {
        return None;
    }
    let current = chars[index];
    let windows_drive = current.is_ascii_alphabetic()
        && chars.get(index + 1) == Some(&':')
        && matches!(chars.get(index + 2), Some('\\' | '/'));
    let unix = current == '/'
        && chars.get(index + 1).is_some_and(|next| *next != '/' && !next.is_whitespace());
    let tilde = current == '~' && chars.get(index + 1) == Some(&'/');
    let unc = current == '\\' && chars.get(index + 1) == Some(&'\\');
    if !(windows_drive || unix || tilde || unc) {
        return None;
    }
    let mut end = index;
    while end < chars.len()
        && !chars[end].is_whitespace()
        && !matches!(chars[end], '"' | '<' | '>' | '`')
    {
        end += 1;
    }
    Some(end)
}

fn redact_secret_values(value: &str) -> String {
    const KEYS: [&str; 12] = [
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
    let bytes = value.as_bytes();
    let lower = value.to_ascii_lowercase();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    'outer: while cursor < bytes.len() {
        for key in KEYS {
            if lower[cursor..].starts_with(key)
                && (cursor == 0 || !is_word_byte(bytes[cursor - 1]))
            {
                let after_key = cursor + key.len();
                if after_key < bytes.len() && is_word_byte(bytes[after_key]) {
                    continue;
                }
                let mut separator = after_key;
                while separator < bytes.len()
                    && matches!(bytes[separator], b' ' | b'\t' | b'\r' | b'\n')
                {
                    separator += 1;
                }
                let explicit = separator < bytes.len()
                    && matches!(bytes[separator], b'=' | b':');
                let bearer = matches!(key, "authorization" | "bearer");
                if !explicit && !(bearer && separator > after_key) {
                    continue;
                }
                if explicit {
                    separator += 1;
                    while separator < bytes.len()
                        && matches!(bytes[separator], b' ' | b'\t')
                    {
                        separator += 1;
                    }
                }
                let mut quote = None;
                if separator < bytes.len() && matches!(bytes[separator], b'"' | b'\'') {
                    quote = Some(bytes[separator]);
                    separator += 1;
                }
                let mut end = separator;
                while end < bytes.len()
                    && !matches!(
                        bytes[end],
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b';' | b'&' | b')' | b']' | b'}'
                    )
                    && quote != Some(bytes[end])
                {
                    end += 1;
                }
                if key == "authorization"
                    && value[separator..end].eq_ignore_ascii_case("bearer")
                {
                    let mut next = end;
                    while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                        next += 1;
                    }
                    while next < bytes.len()
                        && !matches!(
                            bytes[next],
                            b' ' | b'\t' | b'\r' | b'\n' | b',' | b';' | b'&' | b')' | b']' | b'}'
                        )
                    {
                        next += 1;
                    }
                    end = next;
                }
                if end > separator {
                    output.push_str(&value[cursor..separator]);
                    output.push_str("[redacted]");
                    cursor = end;
                    if (quote == Some(b'"') || quote == Some(b'\'')) && cursor < bytes.len() {
                        output.push(bytes[cursor] as char);
                        cursor += 1;
                    }
                    continue 'outer;
                }
            }
        }
        let character = value[cursor..].chars().next().unwrap_or('\u{fffd}');
        output.push(character);
        cursor += character.len_utf8();
    }
    output
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
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
    fn local_reader_and_non_http_urls_keep_no_path_or_host_identity() {
        let text = redact(
            "reader HTTP://12b34567-1234-5678-90ab-123456789abc-deadbeef.localhost.:9/A/Pri)vate_page?secret=1 \
             loopback http://127.0.0.2:9/book-uuid/A/Private \
             ipv6 http://[::1]:9/book-uuid/A/Private \
             bridge https://example.org/book-uuid/%2Ekiwix/token/A/Private \
             path=/book-uuid/A/Private relative=/.kiwix/token/A/Private \
             ZIM://book-uuid/A/Private MAILTO:private@example.org \
             content://provider/Private file:///Documents/Private%20Notes.txt \
             custom://host/Private public https://user:pass@example.org/a/b?token=secret#x \
             adjacent https://example.org/a,http://book-uuid.localhost/A/Private",
        );
        assert_eq!(
            text,
            "reader [reader-url] loopback [reader-url] ipv6 [reader-url] bridge [reader-url] \
             path=[path] relative=[path] [url] [url] [url] [url] [url] public https://example.org/a/b \
             adjacent [reader-url]"
        );
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
