// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in bounded local diagnostics; never network telemetry or article history.
#![forbid(unsafe_code)]
use crate::store::Store;
use serde_json::{json, Value};
use std::{io::{Read, Write}, sync::Mutex};
static LOG_LOCK: Mutex<()> = Mutex::new(());
const LIMIT: u64 = 2 * 1024 * 1024;

fn safe_operation(operation: &str) -> &str {
    match operation {
        "check_integrity" | "diagnostics_integrity" | "cancel_integrity"
        | "search" | "suggest" | "import_archives" | "scan_folder" | "relink_book"
        | "start_download" | "resume_download" | "pause_download" | "cancel_download"
        | "save_settings" | "snapshot" | "diagnostics" | "library_maintenance" => operation,
        "frontend_ready" => "frontend_ready",
        _ => "application_operation",
    }
}

fn export_events(raw: &str) -> String {
    let mut output = String::new();
    for line in raw.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
        let Some(time) = event["time"].as_str().and_then(crate::timestamps::normalized) else { continue };
        let level = if event["level"] == "error" { "error" } else { "info" };
        let category = match event["category"].as_str() {
            Some(category @ ("permission" | "capacity" | "network" | "archive" | "state" | "operation")) => Some(category),
            _ => None,
        };
        let operation = safe_operation(event["operation"].as_str().unwrap_or(""));
        // Old or externally modified logs never export arbitrary fields/values.
        let safe = json!({"time":time,"level":level,"category":category,"operation":operation});
        output.push_str(&safe.to_string());
        output.push('\n');
    }
    output
}

pub fn record(store: &Store, operation: &str, error: Option<&str>) {
    let Ok(settings) = store.settings() else { return };
    if !settings.diagnostics_enabled || settings.private_mode || settings.read_only {
        return;
    }
    if error.is_none() && !matches!(settings.log_level.as_str(), "info" | "debug") {
        return;
    }
    // Never log arbitrary command arguments, URLs, tokens, article names or paths.
    let category = error.map(|error| {
        if error.contains("permission") || error.contains("Permission") {
            "permission"
        } else if error.contains("space") || error.contains("capacity") {
            "capacity"
        } else if error.contains("network") || error.contains("HTTP") {
            "network"
        } else if error.contains("archive") || error.contains("Archive") {
            "archive"
        } else if error.contains("state") || error.contains("profile") {
            "state"
        } else {
            "operation"
        }
    });
    let operation = safe_operation(operation);
    let event = json!({"time":crate::timestamps::now(),"level":if error.is_some(){"error"}else{"info"},"operation":operation,"category":category,"os":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION")});
    let Ok(_guard) = LOG_LOCK.lock() else { return };
    let path = store.data_dir().join("diagnostics.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LIMIT) {
        let _ = std::fs::rename(&path, store.data_dir().join("diagnostics.previous.jsonl"));
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut file) => {
            let _ = writeln!(file, "{event}");
        }
        Err(error) => eprintln!("Cannot write opt-in diagnostics: {error}"),
    }
}

pub fn report(store: &Store) -> Result<Value, String> {
    let state = store.read()?;
    let log = store.data_dir().join("diagnostics.jsonl");
    let events = if !state.settings.private_mode
        && std::fs::metadata(&log).is_ok_and(|m| m.len() <= LIMIT * 2)
    {
        let mut raw = String::new();
        std::fs::File::open(log).map_err(|e| e.to_string())?
            .take(LIMIT * 2).read_to_string(&mut raw).map_err(|e| e.to_string())?;
        export_events(&raw)
    } else {
        String::new()
    };
    let integrity = crate::integrity::summary(store)?;
    Ok(
        json!({"application":"Kiwix","version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"books":state.books.len(),"missing":state.books.iter().filter(|b|!b.available).count(),"privateMode":state.settings.private_mode,"readOnly":state.settings.read_only,"telemetry":false,"localEvents":events,"integrity":integrity}),
    )
}

/// Per-archive ordinals correspond to the retained run order shown locally.
/// Never export identifiers, titles, paths or untrusted error strings.
pub fn integrity_run(job: &crate::integrity::Job) -> Value {
    let archives: Vec<_> = job.targets.iter().enumerate().map(|(index, target)| {
        let result = job.results.get(index).filter(|result| result.book_id == target.book_id
            && result.installation_id == target.installation_id);
        json!({"archive":index+1,
            "state":if result.is_some() { "completed" } else if job.running && job.book_id.as_deref() == Some(target.book_id.as_str()) { "current" } else if job.running { "queued" } else { "not-checked" },
            "status":result.map(|result| match result.status.as_str() {
                status @ ("valid" | "invalid" | "unavailable") => status, _ => "unknown"
            }).unwrap_or("unknown"),
            "checkedAt":result.and_then(|result| result.checked_at.as_deref())
                .and_then(crate::timestamps::normalized),
            "persisted":result.is_some_and(|result|result.persisted)})
    }).collect();
    json!({"running":job.running,"phase":job.phase,"completed":job.completed,
        "total":job.total,"bytesChecked":job.bytes_checked,"totalBytes":job.total_bytes,
        "archiveNumbering":"run-order","archives":archives,
        "methods":["retained-byte-md5","zimru-structure"],"publisherAuthenticity":false,
        "outcomes":{
            "valid":job.results.iter().filter(|result|result.status=="valid").count(),
            "invalid":job.results.iter().filter(|result|result.status=="invalid").count(),
            "unavailable":job.results.iter().filter(|result|result.status=="unavailable").count(),
            "unknown":job.results.iter().filter(|result|result.status=="unknown").count()
        }})
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_export_cannot_leak_arbitrary_log_values() {
        let exported = super::export_events(r#"{"time":"2026-10-29T00:00:00Z","level":"/private/path","operation":"/private/path","category":"/private/path","article":"/private/path"}"#);
        assert!(!exported.contains("/private/path"));
        assert!(exported.contains("application_operation"));
        assert!(super::export_events("not json").is_empty());
    }
}
