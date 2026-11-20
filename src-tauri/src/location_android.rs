//! Android location provider for archive-scoped consent (see permissions.rs).
//!
//! Archives never get the WebView's own geolocation: Kiwix asks once per
//! archive and then fetches one fresh fix through the mobile plugin
//! (`ReaderLocation.kt`), which asks Android for location permission on first
//! use and reads `LocationManager`.
//!
//! Plugin contract (Rust-only commands, never exposed to archive pages):
//! - `startLocation {token, highAccuracy, timeoutMs}` resolves
//!   `{latitude, longitude, accuracy, timestamp, altitude?, altitudeAccuracy?,
//!   heading?, speed?}` (timestamp in ms since the Unix epoch) or rejects with
//!   code `LOCATION_DENIED`, `LOCATION_UNAVAILABLE`, `LOCATION_TIMEOUT` or
//!   `LOCATION_CANCELLED`.
//! - `cancelLocation {token}` ends that request; a cancel that overtakes its
//!   start is remembered by the plugin.
//!
//! A process-wide registry maps each token to a wake-up so `cancel` also stops
//! waiting here, even if the plugin never answers (for example when another
//! permission sheet replaced the location sheet's callback).
#![forbid(unsafe_code)]

use crate::permissions::{Coordinates, LocationError, Position};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use tauri::AppHandle;
use tauri_plugin_kiwix_mobile::MobileExt;
use tokio::sync::oneshot::Sender;
use tokio::sync::Notify;

enum Slot {
    /// Handed to the plugin (or about to be); notified on cancel.
    Running(Arc<Notify>),
    /// Cancelled; a late success is reported as cancelled.
    Cancelled,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static PENDING: LazyLock<Mutex<HashMap<u64, Slot>>> = LazyLock::new(Default::default);

const DENIED: &str = "Location permission was denied for Kiwix in Android settings";

/// Installed once during application setup, beside the Service's own handle.
/// Before that every request fails as unavailable rather than panicking.
pub(crate) fn install(app: AppHandle) {
    let _ = APP.set(app);
}

pub(crate) fn start(token: u64, high_accuracy: bool, timeout_ms: u64, sender: Sender<Result<Position, LocationError>>) {
    let Some(app) = APP.get().cloned() else {
        let _ = sender.send(Err(LocationError::unavailable("Android location is not ready yet")));
        return;
    };
    let wake = Arc::new(Notify::new());
    if let Ok(mut pending) = PENDING.lock() {
        pending.insert(token, Slot::Running(wake.clone()));
    }
    tauri::async_runtime::spawn(async move {
        let result = locate(&app, token, high_accuracy, timeout_ms, &wake).await;
        let cancelled = PENDING
            .lock()
            .map_or(true, |mut pending| matches!(pending.remove(&token), Some(Slot::Cancelled)));
        let _ = sender.send(if cancelled { Err(LocationError::cancelled()) } else { result });
    });
}

pub(crate) fn cancel(token: u64) {
    let wake = {
        let Ok(mut pending) = PENDING.lock() else { return };
        match pending.get(&token) {
            Some(Slot::Running(wake)) => {
                let wake = wake.clone();
                pending.insert(token, Slot::Cancelled);
                wake
            }
            Some(Slot::Cancelled) | None => return,
        }
    };
    // A stored permit wakes the request even if it is not waiting yet.
    wake.notify_one();
    if let Some(app) = APP.get().cloned() {
        tauri::async_runtime::spawn(async move {
            let _ = app.kiwix_mobile().call("cancelLocation", json!({ "token": token })).await;
        });
    }
}

fn cancelled(token: u64) -> bool {
    PENDING.lock().map_or(true, |pending| matches!(pending.get(&token), Some(Slot::Cancelled)))
}

async fn locate(
    app: &AppHandle,
    token: u64,
    high_accuracy: bool,
    timeout_ms: u64,
    wake: &Notify,
) -> Result<Position, LocationError> {
    if cancelled(token) {
        return Err(LocationError::cancelled());
    }
    // The plugin starts the page's timeout only after Android permission is
    // settled. The ceiling below is only a backstop (page timeout + 2 min, to
    // cover the system sheet) so a lost permission callback cannot leave the
    // page waiting forever.
    let payload = json!({
        "token": token,
        "highAccuracy": high_accuracy,
        "timeoutMs": timeout_ms.max(1),
    });
    let ceiling = std::time::Duration::from_millis(timeout_ms.saturating_add(120_000));
    let reply = tokio::select! {
        reply = app.kiwix_mobile().call("startLocation", payload) => reply,
        _ = wake.notified() => return Err(LocationError::cancelled()),
        _ = tokio::time::sleep(ceiling) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = app.kiwix_mobile().call("cancelLocation", json!({ "token": token })).await;
            });
            return Err(LocationError::timeout("Android did not answer the location request"));
        }
    };
    let reply = reply.map_err(|error| failure(&error))?;
    position_from(&reply).ok_or_else(|| LocationError::unavailable("Android returned an invalid position"))
}

/// Validate and shape the plugin's fix like the other providers'. Non-finite
/// or negative optional values are dropped; a fix outside the globe, without
/// accuracy or without a post-1970 timestamp is refused.
pub(crate) fn position_from(reply: &Value) -> Option<Position> {
    let number = |key: &str| reply.get(key).and_then(Value::as_f64);
    let latitude = number("latitude")?;
    let longitude = number("longitude")?;
    let accuracy = number("accuracy")?;
    let timestamp = number("timestamp")?;
    if !latitude.is_finite()
        || !longitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !(-180.0..=180.0).contains(&longitude)
        || !accuracy.is_finite()
        || accuracy < 0.0
        || !timestamp.is_finite()
        || timestamp <= 0.0
    {
        return None;
    }
    let nonnegative = |key: &str| number(key).filter(|value| value.is_finite() && *value >= 0.0);
    Some(Position {
        timestamp: timestamp.trunc(),
        coords: Coordinates {
            latitude,
            longitude,
            accuracy,
            altitude: number("altitude").filter(|value| value.is_finite()),
            altitude_accuracy: nonnegative("altitudeAccuracy"),
            heading: nonnegative("heading"),
            speed: nonnegative("speed"),
        },
    })
}

/// Map a plugin rejection to the Geolocation API's errors. Tauri renders a
/// rejection as `[CODE] - message`; anything else (a bridge failure) is
/// "position unavailable".
pub(crate) fn failure(error: &str) -> LocationError {
    let (code, message) = match error.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        Some((code, rest)) => (code, rest.strip_prefix(" - ").unwrap_or(rest).trim()),
        None => ("", error.trim()),
    };
    let or = |fallback: &str| if message.is_empty() { fallback.to_string() } else { message.to_string() };
    match code {
        "LOCATION_DENIED" => LocationError::denied(DENIED),
        "LOCATION_TIMEOUT" => LocationError::timeout(&or("Android could not get a location in time")),
        "LOCATION_CANCELLED" => LocationError::cancelled(),
        "LOCATION_UNAVAILABLE" => LocationError::unavailable(&or("Android could not get a location")),
        _ => LocationError::unavailable(&format!("Android could not get a location: {}", error.trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_2026_MS: i64 = 1_767_268_800_000; // 2026-01-01T12:00:00Z

    #[test]
    fn android_fixes_are_shaped_like_the_web_api() {
        let fix = position_from(&json!({
            "latitude": 32.3, "longitude": -64.8, "accuracy": 25.0, "timestamp": NOON_2026_MS,
            "altitude": 12.5, "altitudeAccuracy": -1.0, "heading": 90.0, "speed": 1.5,
        }))
        .unwrap();
        assert_eq!(fix.timestamp, 1_767_268_800_000.0);
        assert_eq!((fix.coords.latitude, fix.coords.longitude, fix.coords.accuracy), (32.3, -64.8, 25.0));
        assert_eq!(fix.coords.altitude, Some(12.5));
        assert_eq!(fix.coords.altitude_accuracy, None, "negative accuracy is unknown, not a value");
        assert_eq!(fix.coords.heading, Some(90.0));
        assert_eq!(fix.coords.speed, Some(1.5));
        let bare = position_from(&json!({"latitude": 0, "longitude": 0, "accuracy": 3, "timestamp": NOON_2026_MS})).unwrap();
        assert_eq!((bare.coords.altitude, bare.coords.altitude_accuracy, bare.coords.heading, bare.coords.speed), (None, None, None, None));
    }

    #[test]
    fn impossible_android_fixes_are_refused() {
        let fix = |latitude: f64, longitude: f64, accuracy: Value, timestamp: i64| {
            position_from(&json!({"latitude": latitude, "longitude": longitude, "accuracy": accuracy, "timestamp": timestamp}))
        };
        assert!(fix(91.0, 0.0, json!(5.0), NOON_2026_MS).is_none());
        assert!(fix(0.0, 181.0, json!(5.0), NOON_2026_MS).is_none());
        assert!(fix(0.0, 0.0, json!(-1.0), NOON_2026_MS).is_none());
        assert!(fix(0.0, 0.0, Value::Null, NOON_2026_MS).is_none(), "accuracy is required");
        assert!(fix(0.0, 0.0, json!(5.0), 0).is_none(), "missing or pre-1970 timestamp");
        assert!(position_from(&json!({"longitude": 0.0, "accuracy": 5.0, "timestamp": NOON_2026_MS})).is_none());
        assert!(position_from(&json!({"latitude": "1", "longitude": 0.0, "accuracy": 5.0, "timestamp": NOON_2026_MS})).is_none());
        assert!(position_from(&Value::Null).is_none());
    }

    #[test]
    fn android_rejections_map_to_geolocation_codes() {
        let denied = failure("[LOCATION_DENIED] - Location permission was denied for Kiwix in Android settings");
        assert_eq!((denied.code, denied.cancelled), (1, false));
        assert_eq!(denied.message, DENIED);
        let off = failure("[LOCATION_UNAVAILABLE] - Location is turned off on this device.");
        assert_eq!((off.code, off.message.as_str()), (2, "Location is turned off on this device."));
        let timeout = failure("[LOCATION_TIMEOUT] - Android could not get a location in time");
        assert_eq!(timeout.code, 3);
        assert!(failure("[LOCATION_CANCELLED] - Location request was cancelled").cancelled);
        assert_eq!(failure("[LOCATION_TIMEOUT]").message, "Android could not get a location in time");
        let bridge = failure("jni error: JavaException");
        assert_eq!((bridge.code, bridge.cancelled), (2, false));
        assert!(bridge.message.contains("jni error"));
        let other = failure("[ACTIVITY_DESTROYED] - ACTIVITY_DESTROYED");
        assert_eq!((other.code, other.cancelled), (2, false));
    }

    #[test]
    fn a_cancel_marks_the_token_and_wakes_the_request() {
        let token = u64::MAX - 11;
        let wake = Arc::new(Notify::new());
        PENDING.lock().unwrap().insert(token, Slot::Running(wake.clone()));
        cancel(token);
        assert!(cancelled(token));
        // The stored permit is consumed without blocking.
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        runtime.block_on(async { tokio::time::timeout(std::time::Duration::from_secs(1), wake.notified()).await }).unwrap();
        PENDING.lock().unwrap().remove(&token);
        cancel(token); // unknown tokens are ignored
        assert!(!cancelled(token));
    }
}
