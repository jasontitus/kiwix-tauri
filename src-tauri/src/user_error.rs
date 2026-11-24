// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! The single boundary between internal error text and what a person reads.
//!
//! Native plugins answer with machine codes (`SPEECH_SESSION_RETIRED`, iOS
//! `{"code": ...}` envelopes) and background tasks can fail with a panic
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

    #[test]
    fn a_stopped_operation_keeps_only_the_instruction() {
        assert_eq!(
            plain("Application operation stopped: task 3 panicked"),
            "That action stopped unexpectedly. Try it again."
        );
    }
}
