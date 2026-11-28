// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! The single boundary between internal error text and what a person reads.
//!
//! Native plugins answer with machine codes (`SPEECH_SESSION_RETIRED`, iOS
//! `{"code": ...}` envelopes, Android `[CODE] - {...}` rejections and Java
//! exception text) and background tasks can fail with a panic
//! description. Those are useful in a local log, never in the red band above
//! the library. `plain` turns them into one sentence that says what happened
//! and what to do next; anything already written for a person passes through.
//! Renderer control messages for suspension and a disk-space decision retain
//! their envelopes; the renderer handles them rather than displaying them raw.
//!
//! Internal jargon must not survive this function. If a future error text
//! slips through with one of the words below, the caller still sees a usable
//! instruction instead of implementation vocabulary, and the original text is
//! kept in the local diagnostic log by the caller.

use crate::disk_warning::{SpaceWarning, WARNING_PREFIX};

/// A lifecycle gate, not a failure. The command was refused only because the
/// app was not interactive; the frontend waits for the foreground, retries it
/// once and never paints it. Callers must not match the sentence, so the code
/// travels with the message through the one dispatch boundary.
pub const SUSPENDED_CODE: &str = "KIWIX_SUSPENDED";
#[cfg(any(mobile, test))]
pub const SUSPENDED_MESSAGE: &str = "The mobile application is suspended; return to the foreground";

/// The serialized form of the gate. `plain` passes it through untouched so
/// the frontend still receives the machine code, not a reworded sentence.
#[cfg(any(mobile, test))]
pub fn suspended() -> String {
    serde_json::json!({"code": SUSPENDED_CODE, "message": SUSPENDED_MESSAGE}).to_string()
}

/// Words that describe how the app is built, not what the person needs to do.
const INTERNAL_TERMS: [&str; 15] = [
    "database",
    "record",
    "ipc",
    "snapshot",
    "generation",
    "authority",
    "retired",
    "installation id",
    "uuid",
    "handoff",
    "quarantine",
    "managed storage",
    "token",
    "shared archive",
    "schema",
];

const TRY_AGAIN: &str = "That did not work. Try again, and restart Kiwix if it keeps happening.";

fn has_internal_terms(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    INTERNAL_TERMS.iter().any(|term| lower.contains(term))
}

fn speech_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "SPEECH_FOREGROUND_REQUIRED" => "Bring Kiwix back to the screen to keep reading aloud.",
        "SPEECH_BUSY" => "Another article is already being read aloud. Stop it first.",
        "SPEECH_SESSION_REGISTERED" | "SPEECH_SESSION_CONSUMED" | "SPEECH_SESSION_SUPERSEDED"
        | "SPEECH_SESSION_RETIRED" | "SPEECH_SESSION_UNREGISTERED" | "SPEECH_SOURCE_RETIRING"
        | "SPEECH_ARCHIVE_INSTALLATION_RETIRED" | "SPEECH_READER_SESSION_RETIRED"
        | "SPEECH_READER_LISTENER_RETIRED" => {
            "Read aloud stopped. Press Read aloud to start again."
        }
        "SPEECH_LANGUAGE_UNAVAILABLE" | "SPEECH_VOICE_UNAVAILABLE" => {
            "No voice is available for this language. Choose another language or voice in Settings, then try again."
        }
        "SPEECH_TEXT_LIMIT" => "This article is too large to read aloud.",
        "SPEECH_AUDIO_SESSION_FAILED" => "The device did not start audio. Check the volume, then try again.",
        "SPEECH_SEQUENCE_EXHAUSTED" | "SPEECH_INVALID_REQUEST" => TRY_AGAIN,
        _ => return None,
    })
}

fn native_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "provider_error" => "The file could not be opened. Choose it again.",
        "not_text_document" => "Choose a text or XML file to import.",
        "invalid_export_arguments" => "This item cannot be exported. Choose a different item.",
        "export_cleanup_failed" => {
            "The file was saved, but temporary copies were left behind. Restart Kiwix to clean them up."
        }
        _ => return None,
    })
}

/// A failure reported by the Android plugin, recognised by its transport shape.
struct AndroidFailure {
    /// The plugin's status code, when one could be read.
    code: Option<String>,
    /// The platform's own description, used only to recognise a full disk.
    detail: String,
    /// The plugin command rejected directly (`[CODE] - ...`) rather than
    /// through the Java service endpoint.
    bracketed: bool,
}

/// Android status codes are upper-case words joined by single underscores,
/// the same grammar the shell recognises as a native code. One upper-case word
/// in a sentence ("ZIM", "HTTP") is not a code.
fn is_android_code(text: &str) -> bool {
    text.len() <= 96
        && text.starts_with(|c: char| c.is_ascii_uppercase())
        && text.contains('_')
        && text.split('_').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
}

/// `java.lang.IllegalStateException`, `org.kiwix.mobile.NativeDocumentError`.
fn is_java_throwable(name: &str) -> bool {
    let Some((package, class)) = name.rsplit_once('.') else {
        return false;
    };
    let identifier = |segment: &str| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    };
    package
        .split('.')
        .all(|segment| identifier(segment) && segment.starts_with(|c: char| c.is_ascii_lowercase()))
        && identifier(class)
        && class.starts_with(|c: char| c.is_ascii_uppercase())
        && (class.ends_with("Exception") || class.ends_with("Error"))
}

/// The code and platform detail inside one Android rejection payload: a bare
/// code, `Throwable: payload`, the plugin's `{"code","data":{"detail"}}`
/// message, or the `{"message","code","data"}` object the Java service
/// endpoint relays. Nesting is bounded; anything else has no code.
fn android_payload(text: &str, depth: u8) -> (Option<String>, String) {
    let text = text.trim();
    if is_android_code(text) {
        return (Some(text.to_owned()), String::new());
    }
    if depth == 0 {
        return (None, String::new());
    }
    if let Some((class, rest)) = text.split_once(": ") {
        if is_java_throwable(class) {
            return android_payload(rest, depth - 1);
        }
    }
    if !text.starts_with('{') {
        return (None, String::new());
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return (None, String::new());
    };
    let detail = value
        .get("data")
        .and_then(|data| data.get("detail"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let code = value
        .get("code")
        .and_then(serde_json::Value::as_str)
        .filter(|code| is_android_code(code))
        .map(str::to_owned);
    let (inner_code, inner_detail) = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(|message| android_payload(message, depth - 1))
        .unwrap_or_default();
    (
        code.or(inner_code),
        if detail.is_empty() {
            inner_detail
        } else {
            detail
        },
    )
}

/// Tauri prints an Android plugin rejection as `[CODE] - message`, where the
/// message is the plugin's JSON envelope. Calls routed through the Java
/// service endpoint arrive instead as `java.lang.IllegalStateException: {...}`.
/// Only a whole message in one of those two shapes is an Android failure:
/// lower-case iOS codes and Rust's own JSON envelopes are not touched here.
fn android_failure(message: &str) -> Option<AndroidFailure> {
    if let Some(rest) = message.strip_prefix('[') {
        let (code, rest) = rest.split_once(']')?;
        if !is_android_code(code) {
            return None;
        }
        let payload = if rest.is_empty() {
            ""
        } else {
            rest.strip_prefix(" - ")?
        };
        let (_, detail) = android_payload(payload, 3);
        return Some(AndroidFailure {
            code: Some(code.to_owned()),
            detail,
            bracketed: true,
        });
    }
    let (class, rest) = message.split_once(": ")?;
    if !is_java_throwable(class) {
        return None;
    }
    let (code, detail) = android_payload(rest, 3);
    Some(AndroidFailure {
        code,
        detail,
        bracketed: false,
    })
}

/// Nearby, Bluetooth and temporary Wi-Fi codes are localized by the shell,
/// which matches them as bare codes.
fn shell_owned_code(code: &str) -> bool {
    ["NEARBY_", "BLE_", "JOIN_", "HOTSPOT_", "LOCAL_NETWORK_"]
        .iter()
        .any(|family| code.starts_with(family))
}

fn android_message(code: &str, detail: &str) -> Option<&'static str> {
    // A full disk surfaces under several codes; the cause decides the advice.
    if ["ENOSPC", "EDQUOT", "No space left"]
        .iter()
        .any(|cause| detail.contains(cause))
    {
        return Some(
            "There is not enough free space on this device. Free some space, then try again.",
        );
    }
    Some(match code {
        "NOT_AN_ARCHIVE_DOCUMENT" => {
            "That file is not a ZIM archive. Choose a file whose name ends in .zim."
        }
        "DUPLICATE_DOCUMENT_NAME" => {
            "Two of the chosen files have the same name. Import them one at a time."
        }
        "NATIVE_OPERATION_BUSY" => {
            "Another file action is still open. Finish or cancel it, then try again."
        }
        "DOCUMENT_PERMISSION_DENIED" | "PROVIDER_PERMISSION_DENIED" => {
            "Android did not let Kiwix read that file. Choose it again."
        }
        "TEXT_DOCUMENT_TOO_LARGE" => "That file is too large to import. Choose a smaller file.",
        "EXPORT_DESTINATION_NOT_EMPTY" => {
            "Kiwix does not overwrite an existing file. Save it under a new name."
        }
        "EXPORT_CANCELLED_PARTIAL" => {
            "Saving stopped before the file was complete. Delete the unfinished file, then save again."
        }
        "STAGING_COPY_BUDGET_EXCEEDED" => {
            "That is too much to import at once. Import fewer files at a time."
        }
        "STAGING_DURABILITY_ERROR" | "EXPORT_DURABILITY_ERROR" => {
            "The file could not be saved completely. Check the free space on this device, then try again."
        }
        "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT" => {
            "That file is not available right now. Reconnect its storage and try again, or import a copy."
        }
        "PROVIDER_UNAVAILABLE_USE_COPY_IMPORT"
        | "PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT"
        | "SCANNED_ARCHIVE_UNAVAILABLE" => {
            "That file cannot be read where it is. Import a copy into Kiwix instead."
        }
        "PROVIDER_CANNOT_REMEMBER_USE_COPY_IMPORT" => {
            "Android will not let Kiwix keep access to that location. Import a copy into Kiwix instead."
        }
        "PROVIDER_INCOMING_CHOOSE_ORIGINAL_OR_COPY" => {
            "This file cannot be opened from its current location. Add a copy to your library, or choose the original file in Files."
        }
        "DOCUMENT_PICKER_FAILED"
        | "DOCUMENT_PICKER_EMPTY_RESULT"
        | "DOCUMENT_URI_REQUIRED"
        | "DOCUMENT_METADATA_UNAVAILABLE"
        | "DOCUMENT_STREAM_UNAVAILABLE"
        | "DOCUMENT_TREE_UNAVAILABLE" => "The file could not be opened. Choose it again.",
        "DOCUMENT_PROVIDER_STILL_LOADING" => {
            "That folder is still loading. Wait a moment, then try again."
        }
        "SCAN_RESULT_EXPIRED" => "That scan result is out of date. Scan again, then choose the file.",
        "SCAN_ALREADY_RUNNING" => "A scan is already running. Wait for it to finish.",
        "VISIBLE_ACTIVITY_REQUIRED" => "Bring Kiwix back to the screen, then try again.",
        "ACTIVITY_RECREATED" => "Kiwix restarted its window before this finished. Start it again.",
        "ACTIVITY_DESTROYED" | "STALE_ACTIVITY_GENERATION" => {
            "Kiwix was closed before this finished. Open Kiwix and try again."
        }
        "TRANSFER_FOREGROUND_START_FAILED" | "TRANSFER_FOREGROUND_START_TIMEOUT" => {
            "Android did not let the download keep running in the background. Keep Kiwix open, then try again."
        }
        "TRANSFER_START_PENDING" | "TRANSFER_START_CANCELLED" | "TRANSFER_CONTROL_PENDING" => {
            "Downloads were just paused or are still starting. Wait a moment, then try again."
        }
        _ => return None,
    })
}

/// What the shell is given for an Android failure: an instruction where this
/// boundary has one, otherwise the bare status code. The shell localizes bare
/// codes itself, with its own instruction for the Nearby families and a
/// generic one that keeps any other code for a support request. JSON, brackets
/// around a payload and Java class names never survive.
fn android_plain(failure: &AndroidFailure, original: &str) -> String {
    let Some(code) = failure.code.as_deref() else {
        // Java exception text with no status code is never copy for a person.
        eprintln!("KIWIX_NATIVE_ERROR uncoded");
        return TRY_AGAIN.to_owned();
    };
    if let Some(text) = speech_message(code) {
        return text.to_owned();
    }
    if let Some(text) = android_message(code, &failure.detail) {
        return text.to_owned();
    }
    if shell_owned_code(code) {
        // The shell reads every bracketed form itself, including a sentence the
        // plugin wrote beside the code; only the Java wrapping is removed here.
        return if failure.bracketed {
            original.to_owned()
        } else {
            code.to_owned()
        };
    }
    // The payload may name a person's file: only the code is logged or shown.
    eprintln!("KIWIX_NATIVE_ERROR {code}");
    code.to_owned()
}

/// JSON envelopes from the iOS plugin carry a code and recovery details.
fn envelope_code(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    value.get("code")?.as_str().map(str::to_owned)
}

fn panic_description(message: &str) -> Option<&str> {
    message
        .split_once("operation stopped: ")
        .map(|(_, detail)| detail)
        .filter(|detail| !detail.trim().is_empty())
}

fn valid_disk_warning(payload: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Envelope {
        #[serde(flatten)]
        warning: SpaceWarning,
        // Map jobs may append one user-facing recovery sentence.
        #[serde(rename = "recoveryWarning", default)]
        _recovery_warning: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, serde_json::Value>,
    }

    let Ok(Envelope { warning, extra, .. }) = serde_json::from_str(payload) else {
        return false;
    };
    if !extra.is_empty() {
        return false;
    }
    !warning.acknowledgement.is_empty()
        && !warning.directory.is_empty()
        && warning.projected_free_bytes < warning.threshold_bytes
        && warning
            .available_bytes
            .checked_sub(warning.remaining_bytes)
            .is_some_and(|after_download| warning.projected_free_bytes <= after_download)
}

/// Return a renderer control message or a user-facing sentence.
pub fn plain(message: &str) -> String {
    let trimmed = message.trim();
    if let Some(payload) = trimmed.strip_prefix(WARNING_PREFIX) {
        return if valid_disk_warning(payload) {
            trimmed.to_owned()
        } else {
            TRY_AGAIN.to_owned()
        };
    }
    if let Some(code) = envelope_code(trimmed) {
        // The frontend needs the code, not a paraphrase, to retry on resume.
        if code == SUSPENDED_CODE {
            return trimmed.to_owned();
        }
        if let Some(text) = native_message(&code) {
            return text.to_owned();
        }
    }
    if let Some(failure) = android_failure(trimmed) {
        return android_plain(&failure, trimmed);
    }
    // A bare Nearby, Bluetooth or temporary Wi-Fi code is the shell's own
    // vocabulary on every platform. It must reach the shell unchanged, before
    // the vocabulary check below reads "RETIRED" in a code as internal jargon.
    if is_android_code(trimmed) && shell_owned_code(trimmed) {
        return trimmed.to_owned();
    }
    if let Some(text) = native_message(trimmed) {
        return text.to_owned();
    }
    if let Some(text) = speech_message(trimmed) {
        return text.to_owned();
    }
    if trimmed.contains("Malformed archive") || trimmed.contains("ZIM file format") {
        return "This is not a valid ZIM archive, or the file is truncated.".to_owned();
    }
    if trimmed.contains("Native archive error") {
        return "The archive could not be opened. Try again, and restart Kiwix if it keeps happening."
            .to_owned();
    }
    if let Some(detail) = panic_description(trimmed) {
        // Keep the panic text in the local log; the band gets the action.
        eprintln!("KIWIX_OPERATION_STOPPED {detail}");
        return "That action stopped unexpectedly. Try it again.".to_owned();
    }
    if has_internal_terms(trimmed) {
        eprintln!("KIWIX_INTERNAL_ERROR {trimmed}");
        return TRY_AGAIN.to_owned();
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_codes_become_instructions() {
        assert_eq!(
            plain("SPEECH_SESSION_RETIRED"),
            "Read aloud stopped. Press Read aloud to start again."
        );
        assert_eq!(
            plain(r#"{"code":"provider_error","data":{}}"#),
            "The file could not be opened. Choose it again."
        );
    }

    #[test]
    fn raw_native_archive_errors_become_plain_sentences() {
        assert_eq!(
            plain("Malformed archive: Zim file(s) is of bad size or corrupted"),
            "This is not a valid ZIM archive, or the file is truncated."
        );
        assert_eq!(
            plain("Native archive error: Cannot open file /private/var/x.zim"),
            "The archive could not be opened. Try again, and restart Kiwix if it keeps happening."
        );
    }

    #[test]
    fn a_suspended_command_keeps_its_machine_code_and_never_becomes_copy() {
        let signal = suspended();
        let value: serde_json::Value = serde_json::from_str(&signal).unwrap();
        assert_eq!(value["code"], SUSPENDED_CODE);
        assert_eq!(value["message"], SUSPENDED_MESSAGE);
        assert_eq!(plain(&signal), signal);
    }

    #[test]
    fn download_admission_warning_survives_the_ui_error_boundary() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("database record snapshot token");
        let error = crate::disk_warning::check_with_available(
            &destination,
            Some(1_200_000_000),
            "test download",
            None,
            Some(2_000_000_000),
        )
        .unwrap_err();
        assert_eq!(plain(&error), error);

        // Map composition sends a composite opaque acknowledgement, and job
        // recovery may add one user-facing field without changing the warning.
        let mut map: serde_json::Value =
            serde_json::from_str(error.strip_prefix(WARNING_PREFIX).unwrap()).unwrap();
        let token = map["acknowledgement"].clone();
        map["acknowledgement"] = serde_json::json!({
            "attempt": null, "output": [token], "cache": [],
        })
        .to_string()
        .into();
        map["recoveryWarning"] = "Some earlier map downloads could not be cleaned up.".into();
        let error = format!("{WARNING_PREFIX}{map}");
        assert_eq!(plain(&error), error);

        map["debug"] = "internal database record".into();
        assert_eq!(plain(&format!("{WARNING_PREFIX}{map}")), TRY_AGAIN);
        let duplicate = error.replacen(
            "\"directory\":",
            "\"directory\":\"internal database record\",\"directory\":",
            1,
        );
        assert_eq!(plain(&duplicate), TRY_AGAIN);
        assert_eq!(plain("LOW_DISK_SPACE:{\"record\":\"internal\"}"), TRY_AGAIN);
        assert_eq!(plain("LOW_DISK_SPACE:not json"), TRY_AGAIN);
    }

    #[test]
    fn archive_metadata_limit_remains_actionable_without_exposing_internal_records() {
        let error = crate::archive::archive_error(zimru::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "ZIM metadata record exceeds configured byte limit",
        )));
        assert_eq!(
            plain(&error),
            "An archive metadata entry exceeds the reader’s size limit. Choose a different edition."
        );
        assert_eq!(plain("Invalid saved record"), TRY_AGAIN);
    }

    #[test]
    fn ordinary_copy_passes_through() {
        assert_eq!(
            plain("The application is shutting down"),
            "The application is shutting down"
        );
        assert_eq!(
            plain("Browsing session expired after a privacy transition"),
            "Browsing session expired after a privacy transition"
        );
    }

    #[test]
    fn jargon_never_reaches_the_band() {
        for text in [
            "Archive UUID collision: this UUID already belongs to a library installation",
            "Invalid shared archive record",
            "Mobile archives must be imported into application-managed storage before reading",
        ] {
            let shown = plain(text);
            assert!(
                !has_internal_terms(&shown),
                "internal vocabulary survived: {shown}"
            );
            assert!(shown.len() > 20, "no actionable sentence: {shown}");
        }
    }

    /// Every code `android_message` maps; each must read as an instruction.
    const ANDROID_CODES: &[&str] = &[
        "NOT_AN_ARCHIVE_DOCUMENT",
        "DUPLICATE_DOCUMENT_NAME",
        "NATIVE_OPERATION_BUSY",
        "DOCUMENT_PERMISSION_DENIED",
        "PROVIDER_PERMISSION_DENIED",
        "TEXT_DOCUMENT_TOO_LARGE",
        "EXPORT_DESTINATION_NOT_EMPTY",
        "EXPORT_CANCELLED_PARTIAL",
        "STAGING_COPY_BUDGET_EXCEEDED",
        "STAGING_DURABILITY_ERROR",
        "EXPORT_DURABILITY_ERROR",
        "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT",
        "PROVIDER_UNAVAILABLE_USE_COPY_IMPORT",
        "PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT",
        "SCANNED_ARCHIVE_UNAVAILABLE",
        "PROVIDER_CANNOT_REMEMBER_USE_COPY_IMPORT",
        "PROVIDER_INCOMING_CHOOSE_ORIGINAL_OR_COPY",
        "DOCUMENT_PICKER_FAILED",
        "DOCUMENT_PICKER_EMPTY_RESULT",
        "DOCUMENT_URI_REQUIRED",
        "DOCUMENT_METADATA_UNAVAILABLE",
        "DOCUMENT_STREAM_UNAVAILABLE",
        "DOCUMENT_TREE_UNAVAILABLE",
        "DOCUMENT_PROVIDER_STILL_LOADING",
        "SCAN_RESULT_EXPIRED",
        "SCAN_ALREADY_RUNNING",
        "VISIBLE_ACTIVITY_REQUIRED",
        "ACTIVITY_RECREATED",
        "ACTIVITY_DESTROYED",
        "STALE_ACTIVITY_GENERATION",
        "TRANSFER_FOREGROUND_START_FAILED",
        "TRANSFER_FOREGROUND_START_TIMEOUT",
        "TRANSFER_START_PENDING",
        "TRANSFER_START_CANCELLED",
        "TRANSFER_CONTROL_PENDING",
    ];

    /// The plugin's own message: `{"code":C,"data":{"paths":[],"detail":D}}`.
    fn android_envelope(code: &str, detail: &str) -> String {
        serde_json::json!({"code": code, "data": {"paths": [], "detail": detail}}).to_string()
    }

    /// How Tauri prints a rejected Android plugin command.
    fn android_rejection(code: &str, detail: &str) -> String {
        format!("[{code}] - {}", android_envelope(code, detail))
    }

    /// How the Java service endpoint relays the same rejection.
    fn android_relayed(code: &str, detail: &str) -> String {
        format!(
            "java.lang.IllegalStateException: {}",
            serde_json::json!({
                "message": android_envelope(code, detail),
                "code": code,
                "data": {"paths": [], "detail": detail},
            })
        )
    }

    fn assert_copy_for_a_person(shown: &str) {
        assert!(
            !shown.contains(['{', '}', '[', ']', '"']),
            "transport syntax survived: {shown}"
        );
        assert!(!shown.contains("java."), "Java class survived: {shown}");
        assert!(
            !shown
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(is_android_code),
            "status code survived: {shown}"
        );
        assert!(
            !has_internal_terms(shown),
            "internal vocabulary survived: {shown}"
        );
        assert!(
            shown.len() > 20 && shown.ends_with('.'),
            "no actionable sentence: {shown}"
        );
    }

    #[test]
    fn android_plugin_rejections_become_instructions() {
        assert_eq!(
            plain(
                r#"[NOT_AN_ARCHIVE_DOCUMENT] - {"code":"NOT_AN_ARCHIVE_DOCUMENT","data":{"paths":[],"detail":"NOT_AN_ARCHIVE_DOCUMENT"}}"#
            ),
            "That file is not a ZIM archive. Choose a file whose name ends in .zim."
        );
        for code in ANDROID_CODES {
            for raw in [android_rejection(code, code), android_relayed(code, code)] {
                let shown = plain(&raw);
                assert_copy_for_a_person(&shown);
            }
        }
    }

    #[test]
    fn android_service_endpoint_failures_become_instructions() {
        // The endpoint wraps the rejection in a Java exception.
        assert_eq!(
            plain(&android_relayed(
                "VISIBLE_ACTIVITY_REQUIRED",
                "VISIBLE_ACTIVITY_REQUIRED"
            )),
            "Bring Kiwix back to the screen, then try again."
        );
        // A rejection that carries only the code.
        assert_eq!(
            plain(
                r#"java.lang.IllegalStateException: {"message":"TRANSFER_FOREGROUND_START_TIMEOUT","code":"TRANSFER_FOREGROUND_START_TIMEOUT"}"#
            ),
            "Android did not let the download keep running in the background. Keep Kiwix open, then try again."
        );
        // A rejection that carries only the exception's own text.
        assert_eq!(
            plain(
                r#"java.lang.IllegalStateException: {"message":"org.kiwix.mobile.NativeDocumentError: NATIVE_OPERATION_BUSY"}"#
            ),
            "Another file action is still open. Finish or cancel it, then try again."
        );
        // Read-aloud codes keep their existing sentences in every shape.
        for raw in [
            r#"java.lang.IllegalStateException: {"message":"SPEECH_FOREGROUND_REQUIRED"}"#,
            "[SPEECH_FOREGROUND_REQUIRED] - SPEECH_FOREGROUND_REQUIRED",
        ] {
            assert_eq!(
                plain(raw),
                "Bring Kiwix back to the screen to keep reading aloud."
            );
        }
    }

    #[test]
    fn a_full_disk_is_named_whatever_code_carried_it() {
        let space =
            "There is not enough free space on this device. Free some space, then try again.";
        let detail = "write failed: ENOSPC (No space left on device)";
        for code in ["NATIVE_DOCUMENT_ERROR", "STAGING_DURABILITY_ERROR"] {
            assert_eq!(plain(&android_rejection(code, detail)), space);
            assert_eq!(plain(&android_relayed(code, detail)), space);
        }
        // Without that cause the generic code is left to the shell's fallback.
        assert_eq!(
            plain(&android_rejection(
                "NATIVE_DOCUMENT_ERROR",
                "NATIVE_DOCUMENT_ERROR"
            )),
            "NATIVE_DOCUMENT_ERROR"
        );
    }

    #[test]
    fn unmapped_android_codes_reach_the_shell_bare_for_its_localized_fallback() {
        for raw in [
            android_rejection(
                "SOME_FUTURE_FAILURE",
                "/storage/emulated/0/Download/private.zim: open failed: EACCES",
            ),
            android_relayed("SOME_FUTURE_FAILURE", "SOME_FUTURE_FAILURE"),
            "[SOME_FUTURE_FAILURE] - SOME_FUTURE_FAILURE".to_owned(),
            "[SOME_FUTURE_FAILURE] - A sentence the plugin wrote".to_owned(),
            "[SOME_FUTURE_FAILURE]".to_owned(),
            "org.kiwix.mobile.NativeDocumentError: SOME_FUTURE_FAILURE".to_owned(),
        ] {
            assert_eq!(plain(&raw), "SOME_FUTURE_FAILURE", "{raw}");
        }
    }

    #[test]
    fn android_failures_without_a_code_never_show_java_text_or_paths() {
        for raw in [
            r#"java.lang.IllegalStateException: {"message":"org.json.JSONException: No value for uris"}"#,
            "java.lang.SecurityException: Permission Denial: reading content://media/external/file/12",
            "java.io.FileNotFoundException: /storage/emulated/0/Download/private.zim: open failed: ENOENT",
        ] {
            assert_eq!(plain(raw), TRY_AGAIN, "{raw}");
        }
    }

    #[test]
    fn nearby_codes_reach_the_shell_in_a_form_it_localizes() {
        // The shell reads bracketed Nearby rejections itself, with any sentence
        // the plugin wrote beside the code.
        for raw in [
            "[NEARBY_PERMISSION_DENIED] - NEARBY_PERMISSION_DENIED",
            "[HOTSPOT_PERMISSION_DENIED] - Allow nearby Wi-Fi devices to create a hotspot",
        ] {
            assert_eq!(plain(raw), raw);
        }
        // Java wrapping is not a form the shell reads: only the code remains.
        assert_eq!(
            plain("org.kiwix.mobile.NativeDocumentError: LOCAL_NETWORK_REQUIRED"),
            "LOCAL_NETWORK_REQUIRED"
        );
        assert_eq!(
            plain(
                r#"java.lang.IllegalStateException: {"message":"NEARBY_JOIN_BUSY","code":"NEARBY_JOIN_BUSY"}"#
            ),
            "NEARBY_JOIN_BUSY"
        );
    }

    #[test]
    fn bare_nearby_codes_reach_the_shell_even_when_they_contain_internal_words() {
        // iOS rejects with the bare code. "RETIRED" inside a code is not
        // jargon shown to a person: the shell maps the code to its sentence.
        for code in [
            "BLE_OWNER_RETIRED",
            "BLE_CONNECTION_RETIRED",
            "NEARBY_BLUETOOTH_RETIRED",
            "NEARBY_ACTIVITY_RETIRED",
            "JOIN_ROUTE_LOST",
            "HOTSPOT_START_CANCELLED",
            "LOCAL_NETWORK_PERMISSION_DENIED",
        ] {
            assert_eq!(plain(code), code);
            assert_eq!(plain(&format!("  {code}\n")), code);
        }
        // Only a whole-message code of those families is exempt.
        assert_eq!(plain("The owner was retired"), TRY_AGAIN);
        assert_eq!(
            plain("BLE_OWNER_RETIRED after the owner was retired"),
            TRY_AGAIN
        );
    }

    #[test]
    fn messages_that_are_not_android_failures_keep_their_text() {
        // The provider recovery envelope is parsed by the shell to offer a
        // retry; its code is also an Android status code and must survive.
        let provider = serde_json::json!({
            "code": "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT",
            "message": "This archive is temporarily unavailable. Reconnect its storage, then reopen or reload the article.",
            "retry": true,
            "sourceReference": "0b9f3c5e-6d0a-4a55-9d53-0f0f3a7b9a11",
        })
        .to_string();
        assert_eq!(plain(&provider), provider);
        for text in [
            // iOS codes are lower-case and matched by the shell in this form.
            r#"[provider_incoming_choose_original_or_copy] - {"code":"provider_incoming_choose_original_or_copy","data":{}}"#,
            // A Rust sentence in front is not the Android transport shape.
            "Creating the Wi-Fi hotspot failed: [HOTSPOT_PERMISSION_DENIED] - Allow nearby Wi-Fi devices to create a hotspot",
            "Import failed: the file is missing",
            "[File] could not be read",
            "Error: the catalog did not answer",
            // Bare codes are the shell's own vocabulary.
            "MOBILE_NETWORK_BLOCKED",
            "VISIBLE_ACTIVITY_REQUIRED",
            "LOCAL_NETWORK_PERMISSION_DENIED",
        ] {
            assert_eq!(plain(text), text);
        }
    }

    #[test]
    fn android_status_codes_are_distinguished_from_words() {
        for code in [
            "NOT_AN_ARCHIVE_DOCUMENT",
            "ACTIVITY_RECREATED",
            "A_B",
            "HTTP_2_REQUIRED",
        ] {
            assert!(is_android_code(code), "{code}");
        }
        for word in [
            "ZIM",
            "HTTP",
            "Not_A_Code",
            "_LEADING",
            "TRAILING_",
            "DOUBLE__UNDERSCORE",
            "FILE NAME",
            "",
            "A_",
        ] {
            assert!(!is_android_code(word), "{word}");
        }
        for class in [
            "java.lang.IllegalStateException",
            "org.kiwix.mobile.NativeDocumentError",
            "java.io.IOException",
        ] {
            assert!(is_java_throwable(class), "{class}");
        }
        for text in [
            "Error",
            "IllegalStateException",
            "Import failed",
            "java.lang.String",
            "Java.lang.Error",
            "std::io::Error",
            "a..Error",
        ] {
            assert!(!is_java_throwable(text), "{text}");
        }
    }

    #[test]
    fn a_stopped_operation_keeps_only_the_instruction() {
        assert_eq!(
            plain("Application operation stopped: task 3 panicked"),
            "That action stopped unexpectedly. Try it again."
        );
    }
}
