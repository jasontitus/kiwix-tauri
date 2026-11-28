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

/// Why a Quit attempt stopped before the process could exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuitFailure {
    /// A shell could not save or acknowledge its session. `prepare` cancelled
    /// the attempt and every shell resumed.
    Flush,
    /// The sessions were flushed but `Service::shutdown` reported an error. The
    /// attempt stays prepared and its shells stay frozen.
    Retirement,
}

/// The button labels of the failed-Quit dialog, as shown to the person.
pub(crate) struct FailedQuitLabels {
    pub retry: String,
    pub quit_without_saving: String,
    pub cancel: String,
}

/// What the lib.rs exit handler does after the failed-Quit dialog closes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FailedQuitAction {
    /// Request Quit again. A pending relaunch stays armed.
    Retry,
    /// The person accepted the loss: retire the service once more and exit
    /// whatever that reports.
    QuitWithoutSaving,
    /// Keep Kiwix running. After a failed flush the attempt is over, so a
    /// pending relaunch is dropped; a prepared attempt keeps it for its retry.
    Stay { clear_relaunch: bool },
}

/// Only the button labelled "Quit Without Saving" may give up unsaved state.
/// Closing the dialog or an unexpected result leaves Kiwix running: the dialog
/// plugin reports a closed dialog as the Cancel button, so that slot must
/// never mean "quit".
///
/// A translation may give two buttons one label. The destructive answer is
/// accepted only while its label differs from both others, and Retry only
/// while it differs from Cancel; a label shared with Cancel means Cancel.
pub(crate) fn failed_quit_action(
    failure: QuitFailure,
    result: &tauri_plugin_dialog::MessageDialogResult,
    labels: &FailedQuitLabels,
) -> FailedQuitAction {
    use tauri_plugin_dialog::MessageDialogResult::Custom;
    let discard_is_unambiguous =
        labels.quit_without_saving != labels.retry && labels.quit_without_saving != labels.cancel;
    let retry_is_unambiguous = labels.retry != labels.cancel;
    match result {
        Custom(label) if discard_is_unambiguous && *label == labels.quit_without_saving => {
            FailedQuitAction::QuitWithoutSaving
        }
        Custom(label) if retry_is_unambiguous && *label == labels.retry => FailedQuitAction::Retry,
        _ => FailedQuitAction::Stay {
            clear_relaunch: failure == QuitFailure::Flush,
        },
    }
}

/// What the answer becomes once the dialog has closed.
///
/// Staying needs a window to stay in. With no application window left, a
/// running Kiwix would be invisible, would still hold the profile, and could
/// only be ended from a system monitor. A Quit asked for while the dialog was
/// open (the menu, or closing the last window) was held back by the exit
/// handler and is still wanted. In both cases Cancel becomes Retry: Quit is
/// attempted again and, if it still cannot finish, asked about again. Retry
/// never gives up state.
pub(crate) fn settle_failed_quit(
    action: FailedQuitAction,
    shell_window_remains: bool,
    quit_requested_while_asking: bool,
) -> FailedQuitAction {
    match action {
        FailedQuitAction::Stay { .. } if !shell_window_remains || quit_requested_while_asking => {
            FailedQuitAction::Retry
        }
        action => action,
    }
}

/// No one reads a question and answers it sooner than this.
const DIALOG_ANSWER_FLOOR: Duration = Duration::from_millis(50);

/// Whether the failed-Quit dialog was really put in front of the person.
///
/// The dialog plugin has no error path: a dialog that the platform could not
/// display comes back at once as "Cancel", exactly like a closed one. Retry
/// and Quit Without Saving can only come from their buttons, so only a Cancel
/// that arrived faster than a person can answer counts as "not shown".
pub(crate) fn failed_quit_dialog_was_shown(answer: &FailedQuitAction, elapsed: Duration) -> bool {
    !matches!(answer, FailedQuitAction::Stay { .. }) || elapsed >= DIALOG_ANSWER_FLOOR
}

/// How long to wait before Quit is attempted again when `unshown` dialogs in a
/// row could not be shown: no wait after a dialog that was shown, then 1 s,
/// 2 s, 4 s and so on, at most 60 s. With no window and no dialog, Cancel
/// becomes Retry (`settle_failed_quit`), and without this wait that would spin.
pub(crate) fn unshown_dialog_retry_delay(unshown: u32) -> Duration {
    match unshown {
        0 => Duration::ZERO,
        failures => Duration::from_secs(1_u64.checked_shl(failures - 1).unwrap_or(u64::MAX).min(60)),
    }
}

/// How long to wait before the Quit attempt that follows the failed-Quit
/// dialog.
///
/// Nobody pressed Retry when the answer is `Stay` and Quit is retried only
/// because no window is left to stay in. Such automatic retries are at least
/// one second apart, however the dialog was classified: a dialog that could
/// not be shown but took longer than the 50 ms floor to say so (a busy main
/// thread) would otherwise retry many times a second.
pub(crate) fn failed_quit_retry_delay(answer: &FailedQuitAction, shell_window_remains: bool, unshown: u32) -> Duration {
    let backoff = unshown_dialog_retry_delay(unshown);
    if matches!(answer, FailedQuitAction::Stay { .. }) && !shell_window_remains {
        backoff.max(Duration::from_secs(1))
    } else {
        backoff
    }
}

/// The failed-Quit dialog text: what could not be done, the cause, how to fix
/// it, and exactly what quitting without saving gives up.
pub(crate) fn failed_quit_message(
    strings: &crate::localization::Localizer,
    failure: QuitFailure,
    error: &str,
) -> String {
    let summary = match failure {
        QuitFailure::Flush => strings.text(
            "Kiwix could not save your open tabs and reading positions, so it did not quit.",
            &[],
        ),
        QuitFailure::Retirement => strings.text(
            "Kiwix could not finish saving its work, so it did not quit.",
            &[],
        ),
    };
    let choices = strings.text(
        "Retry after fixing the problem, for example by freeing disk space or reconnecting the drive that holds your Kiwix data. If you quit without saving, changes to open tabs and reading positions since the last successful save are lost, and unfinished transfers may have to repeat their most recent progress.",
        &[],
    );
    format!("{summary}\n\n{error}\n\n{choices}")
}

/// Call before Service::shutdown. On Err the attempt is cancelled and every
/// shell resumes; the caller keeps its exiting flag set until the person has
/// answered the failed-Quit dialog. Repeated ExitRequested events must stay
/// prevented. The ten-second deadline fails closed for an unresponsive or
/// unhydrated shell.
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
        // Window positions are a convenience, like the session snapshot below.
        // A profile that can no longer be written must not make Quit impossible
        // for them: with no shell left to flush, this was the write that kept
        // the process alive with no window.
        if let Err(error) = crate::windows::capture(app, &service.store) {
            eprintln!("Saving window positions: {error}");
        }
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

    fn labels() -> FailedQuitLabels {
        FailedQuitLabels {
            retry: "Retry".into(),
            quit_without_saving: "Quit Without Saving".into(),
            cancel: "Cancel".into(),
        }
    }

    #[test]
    fn a_failed_quit_can_be_retried_left_open_or_overridden_only_by_the_explicit_button() {
        use tauri_plugin_dialog::MessageDialogResult::{Cancel, Custom, No, Ok, Yes};
        for failure in [QuitFailure::Flush, QuitFailure::Retirement] {
            // The person can always leave: a profile that cannot be written no
            // longer makes Quit impossible.
            assert_eq!(
                failed_quit_action(failure, &Custom("Quit Without Saving".into()), &labels()),
                FailedQuitAction::QuitWithoutSaving
            );
            assert_eq!(
                failed_quit_action(failure, &Custom("Retry".into()), &labels()),
                FailedQuitAction::Retry
            );
            // Closing the dialog arrives as the Cancel button or as a bare
            // Cancel. Neither that nor any other result may discard state.
            let stay = FailedQuitAction::Stay { clear_relaunch: failure == QuitFailure::Flush };
            for dismissed in [Custom("Cancel".into()), Cancel, Ok, Yes, No, Custom(String::new()), Custom("quit without saving".into())] {
                assert_eq!(failed_quit_action(failure, &dismissed, &labels()), stay, "{dismissed:?}");
            }
        }
        let stay = FailedQuitAction::Stay { clear_relaunch: false };
        let answer = |label: &str, labels: &FailedQuitLabels| {
            failed_quit_action(QuitFailure::Retirement, &Custom(label.into()), labels)
        };
        // A translation gave the destructive button the Cancel label: a closed
        // dialog looks the same, so nothing is discarded. Retry still works.
        let ambiguous = FailedQuitLabels { cancel: "Quit Without Saving".into(), ..labels() };
        assert_eq!(answer("Quit Without Saving", &ambiguous), stay);
        assert_eq!(answer("Retry", &ambiguous), FailedQuitAction::Retry);
        // The destructive button shares the Retry label: it only ever retries.
        let ambiguous = FailedQuitLabels { retry: "Quit Without Saving".into(), ..labels() };
        assert_eq!(answer("Quit Without Saving", &ambiguous), FailedQuitAction::Retry);
        assert_eq!(answer("Cancel", &ambiguous), stay);
        // Retry and Cancel share a label: that label means Cancel, and the
        // destructive button, still unmistakable, keeps the way out open.
        let ambiguous = FailedQuitLabels { retry: "Cancel".into(), ..labels() };
        assert_eq!(answer("Cancel", &ambiguous), stay);
        assert_eq!(answer("Quit Without Saving", &ambiguous), FailedQuitAction::QuitWithoutSaving);
        // All three alike: nothing but staying.
        let ambiguous = FailedQuitLabels { retry: "OK".into(), quit_without_saving: "OK".into(), cancel: "OK".into() };
        assert_eq!(answer("OK", &ambiguous), stay);
    }

    #[test]
    fn cancel_cannot_leave_kiwix_running_without_a_window_or_drop_a_second_quit() {
        for clear_relaunch in [true, false] {
            let stay = FailedQuitAction::Stay { clear_relaunch };
            // A window is open and nobody asked again: Cancel means stay.
            assert_eq!(
                settle_failed_quit(FailedQuitAction::Stay { clear_relaunch }, true, false),
                stay
            );
            // The last window was closed while the dialog was open (or there
            // never was one): staying would leave an invisible process that
            // holds the profile, so Quit is attempted and asked about again.
            assert_eq!(
                settle_failed_quit(FailedQuitAction::Stay { clear_relaunch }, false, false),
                FailedQuitAction::Retry
            );
            // Quit was chosen again while the dialog was open: it is not lost.
            assert_eq!(
                settle_failed_quit(FailedQuitAction::Stay { clear_relaunch }, true, true),
                FailedQuitAction::Retry
            );
            assert_eq!(
                settle_failed_quit(FailedQuitAction::Stay { clear_relaunch }, false, true),
                FailedQuitAction::Retry
            );
        }
        // An explicit answer is never changed, and nothing here can turn into
        // the destructive one.
        for (window, again) in [(true, true), (true, false), (false, true), (false, false)] {
            assert_eq!(
                settle_failed_quit(FailedQuitAction::Retry, window, again),
                FailedQuitAction::Retry
            );
            assert_eq!(
                settle_failed_quit(FailedQuitAction::QuitWithoutSaving, window, again),
                FailedQuitAction::QuitWithoutSaving
            );
            assert_ne!(
                settle_failed_quit(FailedQuitAction::Stay { clear_relaunch: true }, window, again),
                FailedQuitAction::QuitWithoutSaving
            );
        }
    }

    #[test]
    fn a_dialog_that_cannot_be_shown_is_retried_with_backoff_and_never_quits() {
        let stay = FailedQuitAction::Stay { clear_relaunch: false };
        // A Cancel faster than a person can answer: the dialog was not shown.
        for elapsed in [Duration::ZERO, Duration::from_millis(3), Duration::from_millis(49)] {
            assert!(!failed_quit_dialog_was_shown(&stay, elapsed), "{elapsed:?}");
            assert!(!failed_quit_dialog_was_shown(&FailedQuitAction::Stay { clear_relaunch: true }, elapsed));
        }
        for elapsed in [Duration::from_millis(50), Duration::from_secs(2), Duration::from_secs(3600)] {
            assert!(failed_quit_dialog_was_shown(&stay, elapsed), "{elapsed:?}");
        }
        // A button was pressed, however quickly: that is an answer.
        for answer in [FailedQuitAction::Retry, FailedQuitAction::QuitWithoutSaving] {
            assert!(failed_quit_dialog_was_shown(&answer, Duration::ZERO));
        }
        // No wait after a real answer; then 1, 2, 4 … seconds, at most 60.
        let delays: Vec<u64> = (0..=9).map(|unshown| unshown_dialog_retry_delay(unshown).as_secs()).collect();
        assert_eq!(delays, [0, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
        for unshown in [64, 65, 1_000, u32::MAX] {
            assert_eq!(unshown_dialog_retry_delay(unshown), Duration::from_secs(60));
        }
        // An unshown dialog with no window left is retried, never turned into
        // quitting without saving.
        assert_eq!(settle_failed_quit(stay, false, false), FailedQuitAction::Retry);
    }

    #[test]
    fn automatic_retries_without_a_window_are_at_least_a_second_apart() {
        let cancel = FailedQuitAction::Stay { clear_relaunch: false };
        // No window left, so Cancel retries by itself: never sooner than 1 s,
        // also when the dialog was (rightly or wrongly) counted as shown.
        let delays: Vec<u64> = (0..=8)
            .map(|unshown| failed_quit_retry_delay(&cancel, false, unshown).as_secs())
            .collect();
        assert_eq!(delays, [1, 1, 2, 4, 8, 16, 32, 60, 60]);
        // A person pressed Retry: at once, window or not.
        for window in [true, false] {
            assert_eq!(failed_quit_retry_delay(&FailedQuitAction::Retry, window, 0), Duration::ZERO);
        }
        // A window remains and Quit was chosen again while the dialog was
        // open: that is a person's request too.
        assert_eq!(failed_quit_retry_delay(&cancel, true, 0), Duration::ZERO);
        // The backoff for unshown dialogs still applies with a window.
        assert_eq!(failed_quit_retry_delay(&cancel, true, 3), Duration::from_secs(4));
    }

    #[test]
    fn the_failed_quit_dialog_states_the_cause_and_what_quitting_without_saving_loses() {
        let strings = crate::localization::Localizer::new("en", "en");
        let cause = "Session flush failed for main: No space left on device (os error 28)";
        let flush = failed_quit_message(&strings, QuitFailure::Flush, cause);
        let retirement = failed_quit_message(&strings, QuitFailure::Retirement, cause);
        assert!(flush.starts_with("Kiwix could not save your open tabs and reading positions"), "{flush}");
        assert!(retirement.starts_with("Kiwix could not finish saving its work"), "{retirement}");
        for message in [flush, retirement] {
            assert!(message.contains(cause), "{message}");
            assert!(message.contains("Retry after fixing the problem"), "{message}");
            assert!(
                message.contains("changes to open tabs and reading positions since the last successful save are lost"),
                "{message}"
            );
            assert!(message.contains("unfinished transfers may have to repeat"), "{message}");
        }
    }
}
