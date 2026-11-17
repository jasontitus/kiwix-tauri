//! Windows location provider for archive-scoped consent (see permissions.rs).
//!
//! Archives never get WebView2's own geolocation: Kiwix asks once per archive
//! and then fetches one fix here through `Windows.Devices.Geolocation`, which
//! uses GPS, Wi-Fi or IP positioning as Windows allows. Windows itself must
//! allow it (Settings > Privacy & security > Location, including desktop apps).
//!
//! Each request runs on its own MTA thread because the WinRT calls block. A
//! process-wide registry holds each request's reply channel and, while a WinRT
//! operation is pending, a way to abort it. `cancel` answers the request at
//! once and aborts the operation; the thread then finishes quietly.
//!
//! Windows' own location privacy setting must allow Kiwix; a denied fix maps
//! to the Settings message instead of a silent POSITION_UNAVAILABLE.

use crate::permissions::{Coordinates, LocationError, Position};
use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use tokio::sync::oneshot::Sender;
use windows::Devices::Geolocation::{Geolocator, Geoposition, PositionAccuracy, PositionStatus};
use windows::Foundation::TimeSpan;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// `AsyncStatus::Canceled` (windows-future, which this crate does not name).
const ASYNC_CANCELED: i32 = 2;

#[derive(Default)]
struct Entry {
    /// Taken by whichever answers first: the worker or `cancel`.
    reply: Option<Sender<Result<Position, LocationError>>>,
    cancelled: bool,
    /// Aborts the pending WinRT operation; dropped before COM shuts down.
    abort: Option<Box<dyn Fn() + Send>>,
}

static PENDING: LazyLock<Mutex<HashMap<u64, Entry>>> = LazyLock::new(Default::default);

const TICKS_PER_MS: i64 = 10_000;
/// 100 ns ticks between 1601-01-01 (WinRT DateTime) and the Unix epoch.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

const OFF_IN_SETTINGS: &str = "Location is turned off for Kiwix in Windows. Turn on Location in Settings › Privacy & security › Location, including “Let desktop apps access your location”.";

pub(crate) fn start(token: u64, high_accuracy: bool, timeout_ms: u64, sender: Sender<Result<Position, LocationError>>) {
    if let Ok(mut pending) = PENDING.lock() {
        pending.insert(token, Entry { reply: Some(sender), ..Entry::default() });
    } else {
        let _ = sender.send(Err(LocationError::unavailable("Location state unavailable")));
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("kiwix-location".into())
        .spawn(move || {
            // WinRT activation needs an apartment; this thread owns its own MTA.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let result = locate(token, high_accuracy, timeout_ms);
            // Removing the entry drops its WinRT reference while COM is alive.
            let entry = PENDING.lock().ok().and_then(|mut pending| pending.remove(&token));
            if com.is_ok() {
                unsafe { CoUninitialize() };
            }
            if let Some(reply) = entry.and_then(|entry| entry.reply) {
                let _ = reply.send(result);
            }
        });
    if spawned.is_err() {
        if let Some(reply) = PENDING.lock().ok().and_then(|mut pending| pending.remove(&token)).and_then(|entry| entry.reply) {
            let _ = reply.send(Err(LocationError::unavailable("Location provider could not start")));
        }
    }
}

/// Answer the request as cancelled now and abort any pending operation. The
/// worker thread then ends quietly (its own result is discarded).
pub(crate) fn cancel(token: u64) {
    let Ok(mut pending) = PENDING.lock() else { return };
    let Some(entry) = pending.get_mut(&token) else { return };
    entry.cancelled = true;
    if let Some(abort) = &entry.abort {
        abort();
    }
    if let Some(reply) = entry.reply.take() {
        let _ = reply.send(Err(LocationError::cancelled()));
    }
}

fn cancelled(token: u64) -> bool {
    PENDING.lock().map_or(true, |pending| pending.get(&token).is_none_or(|entry| entry.cancelled))
}

/// Register `abort` for the pending operation, unless the request was
/// cancelled meanwhile (then abort at once and report cancellation).
fn running(token: u64, abort: Box<dyn Fn() + Send>) -> Result<(), LocationError> {
    let Ok(mut pending) = PENDING.lock() else {
        abort();
        return Err(LocationError::cancelled());
    };
    match pending.get_mut(&token) {
        Some(entry) if !entry.cancelled => {
            entry.abort = Some(abort);
            Ok(())
        }
        _ => {
            abort();
            Err(LocationError::cancelled())
        }
    }
}

fn idle(token: u64) {
    if let Ok(mut pending) = PENDING.lock() {
        if let Some(entry) = pending.get_mut(&token) {
            entry.abort = None;
        }
    }
}

fn locate(token: u64, high_accuracy: bool, timeout_ms: u64) -> Result<Position, LocationError> {
    if cancelled(token) {
        return Err(LocationError::cancelled());
    }
    if timeout_ms == 0 {
        // The Geolocation API's answer for timeout 0 with no cached position.
        return Err(LocationError::timeout("Windows could not get a location in time"));
    }
    let locator = Geolocator::new().map_err(|error| failure(&error))?;
    let _ = locator.SetDesiredAccuracy(if high_accuracy { PositionAccuracy::High } else { PositionAccuracy::Default });
    if locator.LocationStatus().is_ok_and(|status| status == PositionStatus::Disabled) {
        return Err(LocationError::denied(OFF_IN_SETTINGS));
    }
    // A fresh fix (no cached position), within the page's own timeout.
    let timeout = TimeSpan { Duration: (timeout_ms as i64).saturating_mul(TICKS_PER_MS) };
    let operation = locator
        .GetGeopositionAsyncWithAgeAndTimeout(TimeSpan { Duration: 0 }, timeout)
        .map_err(|error| failure(&error))?;
    let abort = operation.clone();
    running(token, Box::new(move || { let _ = abort.Cancel(); }))?;
    let position = operation.join();
    let aborted = operation.Status().is_ok_and(|status| status.0 == ASYNC_CANCELED);
    drop(operation);
    idle(token);
    if aborted || cancelled(token) {
        return Err(LocationError::cancelled());
    }
    let position = position.map_err(|error| failure(&error))?;
    convert(&position)
}

fn convert(position: &Geoposition) -> Result<Position, LocationError> {
    let invalid = || LocationError::unavailable("Windows returned an invalid position");
    let coordinate = position.Coordinate().map_err(|_| invalid())?;
    let point = coordinate.Point().and_then(|point| point.Position()).map_err(|_| invalid())?;
    let accuracy = coordinate.Accuracy().map_err(|_| invalid())?;
    let ticks = coordinate.Timestamp().map_err(|_| invalid())?.UniversalTime;
    let optional = |value: windows::core::Result<windows::Foundation::IReference<f64>>| {
        value.and_then(|reference| reference.Value()).ok()
    };
    position_from(
        point.Latitude,
        point.Longitude,
        accuracy,
        Some(point.Altitude),
        optional(coordinate.AltitudeAccuracy()),
        optional(coordinate.Heading()),
        optional(coordinate.Speed()),
        ticks,
    )
    .ok_or_else(invalid)
}

/// Validate and shape a Windows fix like the Apple provider's. `ticks` is a
/// WinRT DateTime (100 ns since 1601); non-finite or negative optional values
/// are dropped, and a fix outside the globe or without accuracy is refused.
#[allow(clippy::too_many_arguments)]
pub(crate) fn position_from(
    latitude: f64,
    longitude: f64,
    accuracy: f64,
    altitude: Option<f64>,
    altitude_accuracy: Option<f64>,
    heading: Option<f64>,
    speed: Option<f64>,
    ticks: i64,
) -> Option<Position> {
    if !latitude.is_finite()
        || !longitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !(-180.0..=180.0).contains(&longitude)
        || !accuracy.is_finite()
        || accuracy < 0.0
        || ticks <= UNIX_EPOCH_TICKS
    {
        return None;
    }
    let nonnegative = |value: Option<f64>| value.filter(|value| value.is_finite() && *value >= 0.0);
    Some(Position {
        timestamp: ((ticks - UNIX_EPOCH_TICKS) / TICKS_PER_MS) as f64,
        coords: Coordinates {
            latitude,
            longitude,
            accuracy,
            altitude: altitude.filter(|value| value.is_finite()),
            altitude_accuracy: nonnegative(altitude_accuracy),
            heading: nonnegative(heading),
            speed: nonnegative(speed),
        },
    })
}

/// Map a WinRT failure to the Geolocation API's codes.
pub(crate) fn failure_code(hresult: i32) -> u8 {
    match hresult as u32 {
        0x8007_0005 => 1,                // E_ACCESSDENIED: Windows refused location
        0x8007_05B4 | 0x8007_0102 => 3,  // ERROR_TIMEOUT, WAIT_TIMEOUT
        _ => 2,                          // position unavailable
    }
}

fn failure(error: &windows::core::Error) -> LocationError {
    let code = error.code().0;
    if matches!(code as u32, 0x8000_4004 | 0x8007_04C7) {
        // E_ABORT, ERROR_CANCELLED
        return LocationError::cancelled();
    }
    match failure_code(code) {
        1 => LocationError::denied(OFF_IN_SETTINGS),
        3 => LocationError::timeout("Windows could not get a location in time"),
        _ => LocationError::unavailable(&format!("Windows could not get a location: {}", error.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_2026: i64 = UNIX_EPOCH_TICKS + 1_767_268_800_000 * TICKS_PER_MS; // 2026-01-01T12:00:00Z

    #[test]
    fn windows_fixes_are_shaped_like_the_web_api() {
        let fix = position_from(32.3, -64.8, 25.0, Some(12.5), Some(-1.0), Some(f64::NAN), Some(1.5), NOON_2026).unwrap();
        assert_eq!(fix.timestamp, 1_767_268_800_000.0);
        assert_eq!((fix.coords.latitude, fix.coords.longitude, fix.coords.accuracy), (32.3, -64.8, 25.0));
        assert_eq!(fix.coords.altitude, Some(12.5));
        assert_eq!(fix.coords.altitude_accuracy, None, "negative accuracy is unknown, not a value");
        assert_eq!(fix.coords.heading, None, "NaN heading is unknown");
        assert_eq!(fix.coords.speed, Some(1.5));
    }

    #[test]
    fn impossible_windows_fixes_are_refused() {
        assert!(position_from(91.0, 0.0, 5.0, None, None, None, None, NOON_2026).is_none());
        assert!(position_from(0.0, 181.0, 5.0, None, None, None, None, NOON_2026).is_none());
        assert!(position_from(f64::NAN, 0.0, 5.0, None, None, None, None, NOON_2026).is_none());
        assert!(position_from(0.0, 0.0, -1.0, None, None, None, None, NOON_2026).is_none());
        assert!(position_from(0.0, 0.0, f64::INFINITY, None, None, None, None, NOON_2026).is_none());
        assert!(position_from(0.0, 0.0, 5.0, None, None, None, None, 0).is_none(), "pre-1970 timestamp");
    }

    #[test]
    fn windows_failures_map_to_geolocation_codes() {
        assert_eq!(failure_code(0x8007_0005_u32 as i32), 1);
        assert_eq!(failure_code(0x8007_05B4_u32 as i32), 3);
        assert_eq!(failure_code(0x8007_0102_u32 as i32), 3);
        assert_eq!(failure_code(0x8000_4005_u32 as i32), 2);
        assert!(failure(&windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4004_u32 as i32))).cancelled);
    }

    #[test]
    fn cancel_answers_at_once_and_stops_the_request() {
        let token = u64::MAX - 7;
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        PENDING.lock().unwrap().insert(token, Entry { reply: Some(sender), ..Entry::default() });
        cancel(token);
        // The caller is answered immediately, even if a WinRT operation is pending.
        assert!(receiver.try_recv().unwrap().unwrap_err().cancelled);
        assert!(cancelled(token));
        assert!(locate(token, false, 1000).unwrap_err().cancelled);
        // An operation registered after the cancel is aborted at once.
        let aborted = std::sync::Arc::new(AtomicBool::new(false));
        let flag = aborted.clone();
        assert!(running(token, Box::new(move || flag.store(true, Ordering::SeqCst))).is_err());
        assert!(aborted.load(Ordering::SeqCst));
        PENDING.lock().unwrap().remove(&token);
    }

    #[test]
    fn a_zero_timeout_is_an_immediate_timeout() {
        let token = u64::MAX - 8;
        PENDING.lock().unwrap().insert(token, Entry::default());
        assert_eq!(locate(token, false, 0).unwrap_err().code, 3);
        PENDING.lock().unwrap().remove(&token);
    }
}
