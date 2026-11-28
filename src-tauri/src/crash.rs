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
//! The rule itself, with the order of its steps, is written once in
//! `tests/fixtures/crash-redaction-spec.md`. The Swift and Kotlin crash intakes
//! implement the same text, and all three are pinned to
//! `tests/fixtures/crash-redaction-cases.json`.
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
/// Text is cut to this many Unicode scalar values before any redaction step
/// runs. The stored fields are far shorter, so nothing useful is lost, and no
/// step can be handed an input long enough to exhaust it and pass text through.
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
            let location: Vec<char> = location[3..].chars().collect();
            let location = text(&replace_urls(&location));
            let name: Vec<char> = file_name(&location).chars().collect();
            frame.push_str(&text(&redact_secret_values(&name)));
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

// ---------------------------------------------------------------------------
// Redaction pipeline. The rule is written down once, for all three crash
// intakes, in `tests/fixtures/crash-redaction-spec.md`; this is its Rust
// implementation, pinned to `tests/fixtures/crash-redaction-cases.json`.
// Every count, scan and cut below is over Unicode scalar values (`char`), and
// every step is linear in the length of its input.
// ---------------------------------------------------------------------------

fn text(scalars: &[char]) -> String {
    scalars.iter().collect()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut result: String = value.chars().take(limit.saturating_sub(1)).collect();
    result.push('…');
    result
}

/// Step 6: whitespace runs become one space; leading and trailing ones go.
fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Step 1: keep at most `MAX_REDACTION_INPUT_CHARS` scalars. When that cuts
/// through a run of non-whitespace, the partial run is dropped: half a URL
/// host or half a path must not be classified as if it were whole.
fn bounded_input(value: &str) -> Vec<char> {
    let mut rest = value.chars();
    let mut scalars: Vec<char> = rest.by_ref().take(MAX_REDACTION_INPUT_CHARS).collect();
    if rest.next().is_some_and(|next| !next.is_whitespace()) {
        while scalars.last().is_some_and(|last| !last.is_whitespace()) {
            scalars.pop();
        }
    }
    scalars
}

/// Full redaction for one text field: steps 1 to 7 of the specification.
fn redact(value: &str) -> String {
    let scalars = bounded_input(value);
    let scalars = redact_secret_values(&scalars);
    let scalars = replace_urls(&scalars);
    let scalars = redact_paths(&scalars);
    let scalars = redact_opaque_tokens(&scalars);
    truncate_chars(&collapse_whitespace(&text(&scalars)), MAX_MESSAGE_CHARS)
}

/// Redaction for the symbol line of a backtrace frame: steps 1 to 4, keeping
/// spacing so the frame stays readable.
fn redact_line(value: &str) -> String {
    let scalars = bounded_input(value);
    let scalars = redact_secret_values(&scalars);
    let scalars = replace_urls(&scalars);
    text(&redact_paths(&scalars))
}

fn is_line_break(scalar: char) -> bool {
    matches!(scalar, '\n' | '\r')
}

/// The index of the first scalar at or after `from` that satisfies `stop`, or
/// the length of the text.
fn find_from(scalars: &[char], from: usize, stop: impl Fn(char) -> bool) -> usize {
    (from..scalars.len())
        .find(|index| stop(scalars[*index]))
        .unwrap_or(scalars.len())
}

/// The number of backslashes directly before `index`, counted back no further
/// than `floor`.
fn backslashes_before(scalars: &[char], floor: usize, index: usize) -> usize {
    scalars[floor..index].iter().rev().take_while(|scalar| **scalar == '\\').count()
}

fn has_path_separator(scalars: &[char]) -> bool {
    scalars.iter().any(|scalar| matches!(scalar, '/' | '\\'))
}

fn matches_ascii_word(scalars: &[char], at: usize, word: &str) -> bool {
    at + word.len() <= scalars.len()
        && scalars[at..at + word.len()]
            .iter()
            .zip(word.chars())
            // ASCII-only folding: `ſ` is not `s` and the Kelvin sign is not `k`.
            .all(|(scalar, expected)| scalar.to_ascii_lowercase() == expected)
}

// --- Step 4 helpers, shared with steps 2 and 3 -----------------------------

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
fn path_end(scalars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < scalars.len() {
        let stop = match scalars[end] {
            '\n' | '\r' | '<' | '>' | '`' => true,
            // Debug-formatted paths escape an embedded quote as `\"`.
            '"' => backslashes_before(scalars, start, end).is_multiple_of(2),
            ':' => scalars.get(end + 1).is_some_and(|next| next.is_whitespace()),
            _ => false,
        };
        if stop {
            break;
        }
        end += 1;
    }
    // The separator before the delimiter is not part of the path.
    while end > start && scalars[end - 1].is_whitespace() {
        end -= 1;
    }
    end
}

/// Whether a path may begin right after `previous` (`None` is the start of
/// the text): anything but an ASCII letter, an ASCII digit, `_`, `.` or `-`.
/// A list of the punctuation that may precede a path always missed some
/// (`„`, `「`, `@`, a zero-width space).
fn path_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|previous| !(previous.is_ascii_alphanumeric() || matches!(previous, '_' | '.' | '-')))
}

/// Whether the text at `index` has the shape of a path start: a drive
/// (`C:\`, `C:/`), an absolute path (`/x`), a home path (`~/`, `~\`), a share
/// (`\\`, `//host`), or a rooted Windows path (`\Users\…`, `\Device\…`).
fn path_start(scalars: &[char], index: usize) -> bool {
    let Some(&current) = scalars.get(index) else {
        return false;
    };
    let next = scalars.get(index + 1).copied();
    let name = |scalar: Option<char>| scalar.is_some_and(|scalar| scalar != '/' && !scalar.is_whitespace());
    let drive = current.is_ascii_alphabetic()
        && next == Some(':')
        && matches!(scalars.get(index + 2), Some('\\' | '/'));
    let absolute = current == '/' && name(next);
    let home = current == '~' && matches!(next, Some('/' | '\\'));
    let share = (current == '\\' && next == Some('\\'))
        || (current == '/' && next == Some('/') && name(scalars.get(index + 2).copied()));
    // One backslash and a name is a path only if another backslash follows
    // before the next whitespace; `\n` in prose is not.
    let rooted = current == '\\'
        && next.is_some_and(|next| next.is_ascii_alphanumeric())
        && scalars[index + 1..]
            .iter()
            .take_while(|scalar| !scalar.is_whitespace())
            .any(|scalar| *scalar == '\\');
    drive || absolute || home || share || rooted
}

fn path_at(scalars: &[char], index: usize) -> Option<usize> {
    let previous = index.checked_sub(1).map(|before| scalars[before]);
    if !path_boundary(previous) || !path_start(scalars, index) {
        return None;
    }
    // Neither slash of `://` starts a path: what follows is the authority of
    // a URL that step 3 kept (`https://example.org`).
    if scalars[index] == '/' {
        let first = previous == Some(':') && scalars.get(index + 1) == Some(&'/');
        let second = previous == Some('/') && index >= 2 && scalars[index - 2] == ':';
        if first || second {
            return None;
        }
    }
    Some(path_end(scalars, index))
}

/// Step 4: every path becomes `[path]`.
fn redact_paths(scalars: &[char]) -> Vec<char> {
    let mut output = Vec::with_capacity(scalars.len());
    let mut index = 0;
    while index < scalars.len() {
        if let Some(end) = path_at(scalars, index) {
            output.extend("[path]".chars());
            index = end;
            continue;
        }
        output.push(scalars[index]);
        index += 1;
    }
    output
}

// --- Step 2: secrets ---------------------------------------------------------

const SECRET_KEYS: [&str; 27] = [
    "authorization",
    "access_token",
    "refresh_token",
    "api_key",
    "api-key",
    "apikey",
    "password",
    "passwd",
    "passphrase",
    "pass",
    "pwd",
    "private_key",
    "private-key",
    "privatekey",
    "client_secret",
    "secret",
    "session_id",
    "sessionid",
    "jsessionid",
    "credentials",
    "credential",
    "signature",
    "cookie",
    "bearer",
    "token",
    "auth",
    "pin",
];

/// Line ends and the last apostrophe of each line, computed once so that no
/// secret value rescans its line.
struct Lines {
    /// For each index (and the text length): the first line break at or after it.
    end: Vec<usize>,
    /// For each index: the last `'` on its line, if the line has one.
    last_apostrophe: Vec<Option<usize>>,
}

impl Lines {
    fn new(scalars: &[char]) -> Self {
        let mut end = vec![scalars.len(); scalars.len() + 1];
        let mut last_apostrophe = vec![None; scalars.len() + 1];
        let mut line_end = scalars.len();
        let mut apostrophe = None;
        for index in (0..scalars.len()).rev() {
            if is_line_break(scalars[index]) {
                line_end = index;
                apostrophe = None;
            } else if scalars[index] == '\'' && apostrophe.is_none() {
                apostrophe = Some(index);
            }
            end[index] = line_end;
            last_apostrophe[index] = apostrophe;
        }
        Self { end, last_apostrophe }
    }
}

/// Step 2: the value that follows a secret keyword becomes `[redacted]`.
fn redact_secret_values(scalars: &[char]) -> Vec<char> {
    let lines = Lines::new(scalars);
    let mut output = Vec::with_capacity(scalars.len());
    let mut cursor = 0;
    while cursor < scalars.len() {
        if let Some((start, end)) = secret_at(scalars, &lines, cursor) {
            output.extend_from_slice(&scalars[cursor..start]);
            output.extend("[redacted]".chars());
            cursor = end;
            continue;
        }
        output.push(scalars[cursor]);
        cursor += 1;
    }
    output
}

/// The value range of a secret whose keyword starts at `at`, if any.
fn secret_at(scalars: &[char], lines: &Lines, at: usize) -> Option<(usize, usize)> {
    let first = scalars[at];
    if !first.is_ascii_alphabetic() {
        return None;
    }
    // Left boundary: not inside a longer word, except at a camelCase hump
    // (`accessToken`, `oauth2Token`).
    let previous = at.checked_sub(1).map(|before| scalars[before]);
    let apart = previous.is_none_or(|previous| !previous.is_ascii_alphanumeric());
    let hump = first.is_ascii_uppercase()
        && previous.is_some_and(|previous| previous.is_ascii_lowercase() || previous.is_ascii_digit());
    if !(apart || hump) {
        return None;
    }
    for key in SECRET_KEYS {
        let after_key = at + key.len();
        if !matches_ascii_word(scalars, at, key)
            || scalars
                .get(after_key)
                .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_')
        {
            continue;
        }
        // A quoted key ends with its quote, which an outer string may have
        // escaped: `"password":`, `\"password\":`, `("authorization", …)`.
        let mut next = after_key;
        let quote = after_key + usize::from(scalars.get(after_key) == Some(&'\\'));
        let quoted_key = matches!(scalars.get(quote), Some('"' | '\''));
        if quoted_key {
            next = quote + 1;
        }
        let gap = next;
        next = find_from(scalars, next, |scalar| !scalar.is_whitespace());
        let separator = match scalars.get(next) {
            Some('=') if scalars.get(next + 1) == Some(&'>') => 2,
            Some('=') => 1,
            // `::` separates Rust path segments, not a key from its value.
            Some(':') if scalars.get(next + 1) != Some(&':') => 1,
            // Tuples and pairs: `("token", "…")`, `(authorization, Basic …)`.
            Some(',') if quoted_key || key == "authorization" => 1,
            _ => 0,
        };
        if separator > 0 {
            next = find_from(scalars, next + separator, |scalar| !scalar.is_whitespace());
        } else if quoted_key || next == gap || !matches!(key, "authorization" | "bearer") {
            continue;
        }
        let (start, end) = secret_value(scalars, lines, next, key);
        if end > start {
            return Some((start, end));
        }
    }
    None
}

/// The range of the secret value that begins at `start`. The forms are tried
/// in this order.
///
/// 1. Quoted, possibly inside wrappers such as `Some(` or `Optional(`:
///    * `"…"`: up to the closing quote (one not escaped by a backslash), a
///      line break or the end.
///    * `\"…\"` (the quote itself escaped by one backslash, as inside an
///      outer string): up to the next `"` with exactly one backslash before
///      it, a line break or the end.
///    * `'…'`: up to the last `'` on the line, or to the end of the line when
///      there is none, because a value may itself contain an apostrophe. If it
///      contains `/` or `\` it takes at least what `path_end` takes.
///
///    The quotes stay.
/// 2. After `cookie`: the rest of the line, because one header carries several
///    `name=value` pairs.
/// 3. One token, which ends at whitespace or one of `, ; & ) ] }`. After
///    `authorization` a second token separated by whitespace goes too
///    (`Basic dXNlcjpwYXNz`, `Bearer …`). A value that contains `/` or `\` is
///    a path or a URL and may contain spaces: it takes at least what
///    `path_end` takes from where it starts.
fn secret_value(scalars: &[char], lines: &Lines, start: usize, key: &str) -> (usize, usize) {
    if let Some(quoted) = quoted_value(scalars, lines, skip_wrappers(scalars, start)) {
        return quoted;
    }
    if key == "cookie" {
        return (start, lines.end[start]);
    }
    let token_end = |from: usize| {
        find_from(scalars, from, |scalar| {
            scalar.is_whitespace() || matches!(scalar, ',' | ';' | '&' | ')' | ']' | '}')
        })
    };
    let mut end = token_end(start);
    if key == "authorization" && end > start {
        let second = find_from(scalars, end, |scalar| !scalar.is_whitespace());
        if second > end && token_end(second) > second {
            end = token_end(second);
        }
    }
    if has_path_separator(&scalars[start..end]) {
        end = end.max(path_end(scalars, start));
    }
    (start, end)
}

/// The index after any `Name(` wrappers that begin at `start`.
fn skip_wrappers(scalars: &[char], mut start: usize) -> usize {
    loop {
        if !scalars.get(start).is_some_and(|first| first.is_ascii_alphabetic()) {
            return start;
        }
        let name_end = find_from(scalars, start, |scalar| !(scalar.is_ascii_alphanumeric() || scalar == '_'));
        if scalars.get(name_end) != Some(&'(') {
            return start;
        }
        start = name_end + 1;
    }
}

/// The content range of a quoted value whose opening quote is at `open`.
fn quoted_value(scalars: &[char], lines: &Lines, open: usize) -> Option<(usize, usize)> {
    let escaped = scalars.get(open) == Some(&'\\');
    let quote = open + usize::from(escaped);
    match scalars.get(quote) {
        Some('"') => {
            let start = quote + 1;
            let line = lines.end[start];
            let close = (start..line).find(|index| {
                scalars[*index] == '"' && {
                    let before = backslashes_before(scalars, start, *index);
                    if escaped { before == 1 } else { before.is_multiple_of(2) }
                }
            });
            // The backslash that escapes the closing quote stays with it.
            Some((start, close.map_or(line, |close| close - usize::from(escaped))))
        }
        Some('\'') if !escaped => {
            let start = quote + 1;
            let mut end = match lines.last_apostrophe[start] {
                Some(close) if close >= start => close,
                _ => lines.end[start],
            };
            // The closing quote may be missing, and then the last `'` is an
            // apostrophe inside a path.
            if has_path_separator(&scalars[start..end]) {
                end = end.max(path_end(scalars, start));
            }
            Some((start, end))
        }
        _ => None,
    }
}

// --- Step 3: URLs ------------------------------------------------------------

const OPAQUE_SCHEMES: [&str; 8] = ["blob", "content", "data", "file", "mailto", "tauri", "tel", "zim"];
/// Final labels of host names that only mean something on the person's own
/// network, where the name itself is theirs (`janes-macbook.local`).
const PRIVATE_SUFFIXES: [&str; 6] = ["local", "localdomain", "lan", "home", "internal", "arpa"];

/// Step 3: every URL becomes `[url]`, `[reader-url]`, or its scheme and host.
fn replace_urls(scalars: &[char]) -> Vec<char> {
    let mut output = Vec::with_capacity(scalars.len());
    let mut cursor = 0;
    let mut colon = 0;
    while colon < scalars.len() {
        if scalars[colon] != ':' {
            colon += 1;
            continue;
        }
        // The scheme is the run of scheme characters before the colon, from
        // its first letter (`+https:` is still `https:`). It cannot reach back
        // into text that an earlier URL already replaced.
        let mut start = colon;
        while start > cursor
            && (scalars[start - 1].is_ascii_alphanumeric() || matches!(scalars[start - 1], '+' | '-' | '.'))
        {
            start -= 1;
        }
        while start < colon && !scalars[start].is_ascii_alphabetic() {
            start += 1;
        }
        let tail = colon + 1;
        let slashes = scalars.get(tail) == Some(&'/') && scalars.get(tail + 1) == Some(&'/');
        let scheme: String = scalars[start..colon].iter().map(char::to_ascii_lowercase).collect();
        let opaque = OPAQUE_SCHEMES.contains(&scheme.as_str());
        if start == colon || (!slashes && (scalars.get(tail) == Some(&':') || !opaque)) {
            colon += 1;
            continue;
        }
        let token_end = find_from(scalars, tail, |scalar| {
            scalar.is_whitespace() || matches!(scalar, '"' | '<' | '>' | '`')
        });
        let has_body = scalars.get(tail).is_some_and(|first| !first.is_whitespace());
        let (end, replacement) = if scheme == "file" || scheme == "data" {
            // A file URL is a path, in any form (`file:///x`, `file:C:/x`), and
            // may be written with unencoded spaces. A data URL carries content
            // itself, spaces and markup included: the rest of the line goes.
            let end = match (has_body, scheme.as_str()) {
                (false, _) => tail,
                (true, "file") => path_end(scalars, tail),
                (true, _) => find_from(scalars, tail, is_line_break),
            };
            (end, "[url]".to_owned())
        } else if slashes {
            let authority_end = find_from(scalars, tail + 2, |scalar| matches!(scalar, '/' | '\\' | '?' | '#')).min(token_end);
            let replacement = safe_url(&scheme, &scalars[tail + 2..authority_end]);
            // A URL that is not kept, or whose path, query or fragment is
            // dropped, goes whole, like a path: what follows the first space
            // of an unencoded article title must not stay behind.
            if replacement.starts_with('[') || authority_end < token_end {
                (token_end.max(path_end(scalars, tail)), replacement)
            } else {
                (token_end, replacement)
            }
        } else {
            (token_end, "[url]".to_owned())
        };
        output.extend_from_slice(&scalars[cursor..start]);
        output.extend(replacement.chars());
        cursor = end;
        colon = end;
    }
    output.extend_from_slice(&scalars[cursor..]);
    output
}

/// What may be kept of a `scheme://authority…` URL, decided from its text
/// alone so that every platform decides alike.
///
/// * `http` and `https` only; any other scheme is `[url]`.
/// * The host is the authority without its userinfo (everything up to the
///   last `@`), without its port, lowercased, and without one trailing dot.
/// * `[reader-url]`: `localhost`, a name ending in `.localhost`, `127.`
///   followed only by digits and dots, or `[::1]`. These are archive pages.
/// * Kept as `scheme://host[:port]`: a DNS name of at least two labels of
///   `a-z 0-9 -` whose last label starts with a letter and is not a
///   private-network suffix. The default port is dropped. The path, query and
///   fragment are never kept: a path can name the article being read.
/// * Everything else is `[url]`: IP literals in any spelling, single-label
///   hosts, private-network names, a malformed port.
fn safe_url(scheme: &str, authority: &[char]) -> String {
    let default_port = match scheme {
        "http" => "80",
        "https" => "443",
        _ => return "[url]".into(),
    };
    let host_and_port = match authority.iter().rposition(|scalar| *scalar == '@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    let (host, port) = if host_and_port.first() == Some(&'[') {
        match host_and_port.iter().position(|scalar| *scalar == ']') {
            Some(close) => (&host_and_port[..=close], &host_and_port[close + 1..]),
            None => return "[url]".into(),
        }
    } else {
        match host_and_port.iter().rposition(|scalar| *scalar == ':') {
            Some(colon) => (&host_and_port[..colon], &host_and_port[colon..]),
            None => (host_and_port, &host_and_port[host_and_port.len()..]),
        }
    };
    let mut host: String = host.iter().map(char::to_ascii_lowercase).collect();
    if host.ends_with('.') {
        host.pop();
    }
    let loopback = host
        .strip_prefix("127.")
        .is_some_and(|rest| rest.chars().all(|scalar| scalar.is_ascii_digit() || scalar == '.'));
    if host == "localhost" || host.ends_with(".localhost") || loopback || host == "[::1]" {
        return "[reader-url]".into();
    }
    let port: String = match port {
        [] => String::new(),
        [':', digits @ ..]
            if (1..=5).contains(&digits.len()) && digits.iter().all(char::is_ascii_digit) =>
        {
            digits.iter().collect()
        }
        _ => return "[url]".into(),
    };
    let labels: Vec<&str> = host.split('.').collect();
    let last = labels.last().copied().unwrap_or_default();
    let dns_name = labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.chars().all(|scalar| scalar.is_ascii_lowercase() || scalar.is_ascii_digit() || scalar == '-')
        })
        && last.starts_with(|first: char| first.is_ascii_lowercase())
        && !PRIVATE_SUFFIXES.contains(&last);
    if !dns_name {
        return "[url]".into();
    }
    if port.is_empty() || port.trim_start_matches('0') == default_port {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

// --- Step 5: opaque tokens -----------------------------------------------------

/// Step 5: a run of 40 or more token characters that has both a digit and a
/// letter becomes `[redacted]`.
fn redact_opaque_tokens(scalars: &[char]) -> Vec<char> {
    const MIN_TOKEN: usize = 40;
    let token_scalar =
        |scalar: char| scalar.is_ascii_alphanumeric() || matches!(scalar, '_' | '-' | '+' | '/' | '=' | '.');
    let mut output = Vec::with_capacity(scalars.len());
    let mut index = 0;
    while index < scalars.len() {
        let end = find_from(scalars, index, |scalar| !token_scalar(scalar));
        if end == index {
            output.push(scalars[index]);
            index += 1;
            continue;
        }
        let run = &scalars[index..end];
        if run.len() >= MIN_TOKEN
            && run.iter().any(char::is_ascii_digit)
            && run.iter().any(char::is_ascii_alphabetic)
        {
            output.extend("[redacted]".chars());
        } else {
            output.extend_from_slice(run);
        }
        index = end;
    }
    output
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
            // A URL on another host keeps its scheme and host and nothing
            // else: the path can name the book and the article.
            (
                "bridge https://example.org/book-uuid/%2Ekiwix/token/A/Private",
                "bridge https://example.org",
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
                "public https://example.org",
            ),
            (
                "adjacent https://example.org/a,http://book-uuid.localhost/A/Private",
                "adjacent https://example.org",
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
        assert!(cases.len() >= 200, "the shared fixture lost cases: {}", cases.len());
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
        let bound = MAX_REDACTION_INPUT_CHARS;
        // Scalar values, not bytes and not UTF-16 units: nothing is split.
        let wide = format!("{} tail", "é ".repeat(bound));
        assert_eq!(bounded_input(&wide).len(), bound);
        let astral = "😀 ".repeat(bound);
        assert_eq!(bounded_input(&astral).len(), bound);
        // A short field, and one exactly at the bound, are untouched.
        assert_eq!(text(&bounded_input("lib.rs:12")), "lib.rs:12");
        let exact = "a".repeat(bound);
        assert_eq!(bounded_input(&exact).len(), bound);
        // The cut falls between words, on either side of the space: every
        // whole word before it stays.
        let space_last = format!("{} next", "a".repeat(bound - 1));
        assert_eq!(bounded_input(&space_last).len(), bound);
        let space_next = format!("{} next", "a".repeat(bound));
        assert_eq!(bounded_input(&space_next).len(), bound);

        // The cut falls inside a token: the partial token goes. Kept, the
        // first half of this reader URL's host would be a "DNS name" and stay.
        let prefix = " ".repeat(bound - 19);
        let straddling = format!("{prefix}http://12b34567.localhost:9/A/Private_Page");
        assert_eq!(text(&bounded_input(&straddling)), prefix);
        assert_eq!(redact(&straddling), "");
        // Whole, the same URL is recognised.
        assert_eq!(redact("http://12b34567.localhost:9/A/Private_Page"), "[reader-url]");
        // A path cut in half is dropped too, and with it nothing of the path stays.
        let path = format!("{}/home/jane doe/Private Notes/a.zim", "x ".repeat(bound / 2 - 8));
        let output = redact(&path);
        assert!(!output.contains("jane") && !output.contains("Private"), "{}", &output[output.len() - 40..]);
        // One unbroken run longer than the bound leaves nothing.
        let long = format!("/{}", "a".repeat(bound * 4));
        assert!(bounded_input(&long).is_empty());
        assert_eq!(redact(&long), "");
        assert_eq!(redact_line(&long), "");
        // A panic location is never redacted.
        assert_eq!(redact("lib.rs:1234"), "lib.rs:1234");
        assert_eq!(redact("store.rs:88"), "store.rs:88");
    }

    #[test]
    fn whitespace_is_exactly_the_unicode_white_space_property() {
        // The list the specification gives the Swift and Kotlin ports.
        let listed = |scalar: char| {
            matches!(
                scalar,
                '\u{9}'..='\u{d}'
                    | '\u{20}'
                    | '\u{85}'
                    | '\u{a0}'
                    | '\u{1680}'
                    | '\u{2000}'..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
            )
        };
        for scalar in (0..=0x10ffff_u32).filter_map(char::from_u32) {
            assert_eq!(scalar.is_whitespace(), listed(scalar), "U+{:04X}", scalar as u32);
        }
    }

    #[test]
    fn every_redaction_step_is_linear_in_its_input() {
        // The worst inputs found in review: each unit starts a URL, path or
        // secret candidate. Rescanning the rest of the text per candidate made
        // these quadratic (about one second for 32,768 scalars in this build).
        let repeated = |unit: &str, length: usize| -> Vec<char> {
            unit.chars().cycle().take(length - length % unit.chars().count()).collect()
        };
        let steps = |scalars: &[char]| {
            let scalars = redact_secret_values(scalars);
            let scalars = replace_urls(&scalars);
            let scalars = redact_paths(&scalars);
            redact_opaque_tokens(&scalars).len()
        };
        let fastest = |scalars: &[char]| {
            (0..3)
                .map(|_| {
                    let started = std::time::Instant::now();
                    std::hint::black_box(steps(scalars));
                    started.elapsed()
                })
                .min()
                .unwrap()
        };
        for unit in [
            "a://b<", "file:x<", "zz://q`", "password=\"\" ", "data:x\n", "/a: ", " \\a", "a:", "'password'", "token='x ", "C:\\a\" ",
        ] {
            let small = fastest(&repeated(unit, MAX_REDACTION_INPUT_CHARS));
            let large = fastest(&repeated(unit, MAX_REDACTION_INPUT_CHARS * 4));
            // Four times the input takes about four times as long when linear
            // and sixteen times when quadratic. The floor absorbs timer noise.
            let allowed = small.max(std::time::Duration::from_millis(5)) * 10;
            assert!(large <= allowed, "{unit:?}: {small:?} for the bound, {large:?} for four times as much");
            // Generous for a shared, unoptimised test build; the quadratic
            // version needed 400 ms to 1 s here.
            assert!(small < std::time::Duration::from_millis(300), "{unit:?}: {small:?}");
        }
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
