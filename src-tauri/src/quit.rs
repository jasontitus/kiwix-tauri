// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Quit waits for each trusted shell's latest session, never archive messages.
#![forbid(unsafe_code)]
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::{
    sync::Notify,
    time::{sleep, sleep_until, Instant},
};
use uuid::Uuid;

const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);
const WINDOW_POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct QuitManager {
    attempt: Mutex<Option<Attempt>>,
    changed: Notify,
}

struct Attempt {
    id: String,
    requested: HashSet<String>,
    pending: HashSet<String>,
    error: Option<String>,
    prepared: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FlushRequest {
    request_id: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Cancelled {
    request_id: String,
    error: String,
}

pub(crate) fn shell_label(label: &str) -> bool {
    label == "main" || label.starts_with("window-")
}

fn trusted(window: &WebviewWindow) -> Result<(), String> {
    if !shell_label(window.label()) {
        return Err("Quit flush is restricted to application windows".into());
    }
    let url = window
        .url()
        .map_err(|error| format!("Cannot verify application window: {error}"))?;
    let packaged = url.port().is_none()
        && ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost")));
    let development = cfg!(debug_assertions)
        && window
            .app_handle()
            .config()
            .build
            .dev_url
            .as_ref()
            .is_some_and(|allowed| url.origin() == allowed.origin());
    if !url.username().is_empty() || url.password().is_some() || !(packaged || development) {
        return Err("Quit flush is restricted to the trusted application origin".into());
    }
    Ok(())
}

/// Service window creation must reject new windows while this is true. A successful
/// prepare remains active through geometry capture and final service shutdown.
pub fn is_preparing(app: &AppHandle) -> bool {
    app.try_state::<QuitManager>().is_some_and(|manager| {
        manager
            .attempt
            .lock()
            .map(|attempt| attempt.is_some())
            .unwrap_or(true)
    })
}

/// Hold the quit gate through actual main-thread creation, not just scheduling it.
/// prepare takes the same lock while activating and snapshotting the open windows.
pub fn with_window_creation<T>(
    app: &AppHandle,
    create: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let manager = app
        .try_state::<QuitManager>()
        .ok_or("Quit manager unavailable")?;
    let guard = manager
        .attempt
        .lock()
        .map_err(|_| "Quit state unavailable")?;
    if guard.is_some() {
        return Err("Cannot open a window while application Quit is pending".into());
    }
    let result = create();
    drop(guard);
    result
}

/// Prove a received event belongs to a live native request before the shell flushes.
/// There is deliberately no frontend command for initiating application Quit.
#[tauri::command]
pub fn validate_quit_flush(
    window: WebviewWindow,
    manager: tauri::State<'_, QuitManager>,
    request_id: String,
) -> Result<(), String> {
    trusted(&window)?;
    let guard = manager
        .attempt
        .lock()
        .map_err(|_| "Quit state unavailable")?;
    let attempt = guard.as_ref().ok_or("No application quit is pending")?;
    if attempt.id != request_id
        || !attempt.pending.contains(window.label())
        || attempt.error.is_some()
    {
        return Err("Unknown, stale, or already acknowledged quit request".into());
    }
    Ok(())
}

#[tauri::command]
pub fn ack_quit_flush(
    window: WebviewWindow,
    manager: tauri::State<'_, QuitManager>,
    request_id: String,
    error: Option<String>,
) -> Result<(), String> {
    trusted(&window)?;
    let mut guard = manager
        .attempt
        .lock()
        .map_err(|_| "Quit state unavailable")?;
    let attempt = guard.as_mut().ok_or("No application quit is pending")?;
    if attempt.id != request_id || !attempt.pending.remove(window.label()) {
        return Err("Unknown, stale, or already acknowledged quit request".into());
    }
    if let Some(error) = error {
        // A failed acknowledgement always cancels, including an empty error string.
        attempt.error.get_or_insert_with(|| {
            format!(
                "Session flush failed for {}: {}",
                window.label(),
                error.chars().take(2048).collect::<String>()
            )
        });
    }
    drop(guard);
    manager.changed.notify_one();
    Ok(())
}

/// Closing a window is already guarded by its own save/discard-confirmation flow.
/// Removing it cannot leave native Quit waiting for an acknowledgement forever.
pub fn window_destroyed(app: &AppHandle, label: &str) {
    let Some(manager) = app.try_state::<QuitManager>() else {
        return;
    };
    if let Ok(mut guard) = manager.attempt.lock() {
        if let Some(attempt) = guard.as_mut() {
            attempt.pending.remove(label);
        }
    }
    manager.changed.notify_one();
}

fn reconcile(
    manager: &QuitManager,
    request_id: &str,
    windows: HashMap<String, WebviewWindow>,
) -> Result<Vec<WebviewWindow>, String> {
    let mut shells = Vec::new();
    for window in windows
        .values()
        .filter(|window| shell_label(window.label()))
    {
        // A shell navigated away from its trusted origin is not silently discarded.
        trusted(window)?;
        shells.push(window.clone());
    }
    let mut guard = manager
        .attempt
        .lock()
        .map_err(|_| "Quit state unavailable")?;
    let attempt = guard
        .as_mut()
        .filter(|attempt| attempt.id == request_id)
        .ok_or("Quit request was cancelled")?;
    attempt.pending.retain(|label| windows.contains_key(label));
    let mut newly_opened = Vec::new();
    for window in shells {
        if attempt.requested.insert(window.label().to_string()) {
            attempt.pending.insert(window.label().to_string());
            newly_opened.push(window);
        }
    }
    Ok(newly_opened)
}

/// A prepared shell stays frozen until service retirement succeeds. A retry may
/// reuse its acknowledged snapshot because new shell creation remains gated.
fn needs_flush(slot: &mut Option<Attempt>, request_id: &str) -> Result<bool, String> {
    if let Some(active) = slot {
        return if active.prepared {
            Ok(false)
        } else {
            Err("Application quit is already pending".into())
        };
    }
    *slot = Some(Attempt {
        id: request_id.into(),
        requested: HashSet::new(),
        pending: HashSet::new(),
        error: None,
        prepared: false,
    });
    Ok(true)
}

fn cancel(app: &AppHandle, manager: &QuitManager, request_id: &str, error: &str) {
    if let Ok(mut guard) = manager.attempt.lock() {
        if guard
            .as_ref()
            .is_some_and(|attempt| attempt.id == request_id)
        {
            *guard = None;
        }
    }
    let payload = Cancelled {
        request_id: request_id.into(),
        error: error.into(),
    };
    for window in app
        .webview_windows()
        .values()
        .filter(|window| shell_label(window.label()))
    {
        if trusted(window).is_ok() {
            if let Err(emit_error) = app.emit_to(
                tauri::EventTarget::webview_window(window.label()),
                "kiwix-quit-cancelled",
                payload.clone(),
            ) {
                eprintln!("Reporting cancelled Quit: {emit_error}");
            }
        }
    }
}

/// Call before Service::shutdown; on Err leave the application running and reset
/// the caller's exiting flag. Repeated ExitRequested events must stay prevented.
/// The ten-second deadline fails closed for an unresponsive or unhydrated shell.
pub async fn prepare(app: &AppHandle) -> Result<(), String> {
    let manager = app
        .try_state::<QuitManager>()
        .ok_or("Quit manager unavailable")?;
    let request_id = Uuid::new_v4().to_string();
    let mut initial_windows = Some({
        let mut guard = manager
            .attempt
            .lock()
            .map_err(|_| "Quit state unavailable")?;
        if !needs_flush(&mut guard, &request_id)? { return Ok(()); }
        app.webview_windows()
    });
    let deadline = Instant::now() + FLUSH_TIMEOUT;
    let result = async {
        loop {
            for window in reconcile(&manager, &request_id, initial_windows.take().unwrap_or_else(|| app.webview_windows()))? {
                if let Err(error) = app.emit_to(tauri::EventTarget::webview_window(window.label()), "kiwix-quit-flush", FlushRequest { request_id: request_id.clone() }) {
                    // A simultaneously closed window no longer needs a flush.
                    if app.get_webview_window(window.label()).is_some() {
                        return Err(format!("Cannot request session flush for {}: {error}", window.label()));
                    }
                }
            }
            let pending = {
                let guard = manager.attempt.lock().map_err(|_| "Quit state unavailable")?;
                let attempt = guard.as_ref().filter(|attempt| attempt.id == request_id).ok_or("Quit request was cancelled")?;
                if let Some(error) = &attempt.error { return Err(error.clone()); }
                attempt.pending.iter().cloned().collect::<Vec<_>>()
            };
            if pending.is_empty() { break; }
            if Instant::now() >= deadline {
                let mut pending = pending;
                pending.sort_unstable();
                return Err(format!("Quit cancelled: session flush timed out for {}. Retry Quit or close the affected window explicitly.", pending.join(", ")));
            }
            tokio::select! {
                _ = manager.changed.notified() => {},
                _ = sleep(WINDOW_POLL) => {},
                _ = sleep_until(deadline) => {},
            }
        }
        // Geometry is persisted only after all session acknowledgements succeeded.
        let service = app.try_state::<Arc<crate::service::Service>>().ok_or("Library service unavailable during Quit")?;
        crate::windows::capture(app, &service.store)?;
        // Snapshot only the windows that were actually open. Private and
        // read-only profiles skip this inside the store; a failure to write the
        // convenience snapshot must not cancel a user's Quit.
        let open: HashSet<String> = app
            .webview_windows()
            .into_keys()
            .filter(|label| label == "main" || label.starts_with("window-"))
            .collect();
        if let Err(error) = service.store.save_session_snapshot(&open) {
            eprintln!("Saving the restorable session: {error}");
        }
        let mut guard = manager.attempt.lock().map_err(|_| "Quit state unavailable")?;
        let attempt = guard.as_mut().filter(|attempt| attempt.id == request_id)
            .ok_or("Quit request was cancelled")?;
        attempt.prepared = true;
        Ok(())
    }.await;
    if let Err(error) = &result {
        cancel(app, &manager, &request_id, error);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_prepared_attempt_can_retry_without_rewriting_sessions() {
        let mut slot = None;
        assert!(needs_flush(&mut slot, "first").unwrap());
        assert!(needs_flush(&mut slot, "second").is_err());
        slot.as_mut().unwrap().prepared = true;
        assert!(!needs_flush(&mut slot, "retry").unwrap());
        assert_eq!(slot.as_ref().unwrap().id, "first");
        // Ordinary shell-flush cancellation clears the attempt and must flush again.
        slot = None;
        assert!(needs_flush(&mut slot, "after cancellation").unwrap());
    }
}
