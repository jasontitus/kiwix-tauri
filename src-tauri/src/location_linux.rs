//! Linux location provider for archive-scoped consent (see permissions.rs).
//!
//! Archives never get WebKitGTK's own geolocation: Kiwix asks once per archive
//! and then fetches one fix here from GeoClue over the system D-Bus. GeoClue
//! picks Wi-Fi, GPS, 3G or IP positioning; the desktop can still refuse
//! (GNOME: Settings › Privacy › Location Services).
//!
//! Each request runs on its own thread with its own bus connection, driving
//! zbus's async API to completion there. The async API is used because a
//! blocking signal iterator can neither time out nor be interrupted, so a
//! helper thread waiting on it would outlive a timed-out or cancelled request.
//! A process-wide registry maps the request token to a cancel channel created
//! before the thread starts, so a cancel is honoured at any point, including
//! before the connection exists.
#![forbid(unsafe_code)]

use crate::permissions::{Coordinates, LocationError, Position};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::oneshot;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, DBusError, Proxy};

type Reply = oneshot::Sender<Result<Position, LocationError>>;

/// Pending requests: sending on (or dropping) the channel cancels the request.
static PENDING: LazyLock<Mutex<HashMap<u64, oneshot::Sender<()>>>> = LazyLock::new(Default::default);

const SERVICE: &str = "org.freedesktop.GeoClue2";
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const MANAGER_INTERFACE: &str = "org.freedesktop.GeoClue2.Manager";
const CLIENT_INTERFACE: &str = "org.freedesktop.GeoClue2.Client";
const LOCATION_INTERFACE: &str = "org.freedesktop.GeoClue2.Location";
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
/// GeoClue matches this against the installed .desktop file and the
/// desktop's per-app location permission; it is the Tauri bundle identifier.
const DESKTOP_ID: &str = "org.kiwix.workbench";
/// GClueAccuracyLevel values.
const ACCURACY_STREET: u32 = 6;
const ACCURACY_EXACT: u32 = 8;
/// Upper bound for Stop/DeleteClient after the result has been delivered.
const CLEANUP_LIMIT: Duration = Duration::from_secs(2);

const NEEDS_GEOCLUE: &str = "Location needs the GeoClue service (geoclue-2.0), which is not installed or not running on this system. Install GeoClue with your distribution's package manager to use location in archives.";
const OFF_IN_SETTINGS: &str = "Location is turned off for Kiwix on this desktop. Turn on Location Services in your desktop's Privacy settings (for example GNOME Settings › Privacy › Location Services) and allow Kiwix.";
const TIMED_OUT: &str = "GeoClue could not get a location in time";
const NO_AGENT: &str = "This desktop has no GeoClue location agent, so it cannot share your location. Desktops such as GNOME and KDE provide one.";

pub(crate) fn start(token: u64, high_accuracy: bool, timeout_ms: u64, sender: Reply) {
    let (cancel, cancelled) = oneshot::channel();
    pending().insert(token, cancel);
    let spawned = std::thread::Builder::new()
        .name("kiwix-location".into())
        .spawn(move || {
            // Not a runtime worker, so blocking on the shared runtime is allowed;
            // its timers drive the page's timeout.
            tauri::async_runtime::block_on(run(token, high_accuracy, timeout_ms, cancelled, sender));
        });
    if spawned.is_err() {
        pending().remove(&token);
        // Dropping the sender (moved into the failed closure) reports that the
        // provider stopped.
    }
}

pub(crate) fn cancel(token: u64) {
    if let Some(cancel) = pending().remove(&token) {
        let _ = cancel.send(());
    }
}

fn pending() -> MutexGuard<'static, HashMap<u64, oneshot::Sender<()>>> {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn deliver(token: u64, sender: Reply, result: Result<Position, LocationError>) {
    pending().remove(&token);
    let _ = sender.send(result);
}

async fn run(
    token: u64,
    high_accuracy: bool,
    timeout_ms: u64,
    mut cancel: oneshot::Receiver<()>,
    sender: Reply,
) {
    if timeout_ms == 0 {
        return deliver(token, sender, Err(timed_out()));
    }
    let (connection, manager, client_path) = match until_cancelled(&mut cancel, open()).await {
        Ok(session) => session,
        Err(error) => return deliver(token, sender, Err(error)),
    };
    let client = match until_cancelled(&mut cancel, async {
        Proxy::new(&connection, SERVICE, client_path.as_str(), CLIENT_INTERFACE)
            .await
            .map_err(|error| failure(&error))
    })
    .await
    {
        Ok(client) => client,
        Err(error) => return deliver(token, sender, Err(error)),
    };
    let result = acquire(&connection, &client, high_accuracy, timeout_ms, &mut cancel).await;
    // Answer first, so a cancel or timeout completes promptly, then release
    // GeoClue. Closing the connection also makes GeoClue drop the client.
    deliver(token, sender, result);
    let _ = tokio::time::timeout(CLEANUP_LIMIT, async {
        let _: zbus::Result<()> = client.call("Stop", &()).await;
        let _: zbus::Result<()> = manager.call("DeleteClient", &(client_path.clone(),)).await;
    })
    .await;
}

/// Connect to the system bus and obtain a GeoClue client for this connection.
async fn open() -> Result<(Connection, Proxy<'static>, OwnedObjectPath), LocationError> {
    let connection = Connection::system()
        .await
        .map_err(|_| LocationError::unavailable(NEEDS_GEOCLUE))?;
    let manager = Proxy::new(&connection, SERVICE, MANAGER_PATH, MANAGER_INTERFACE)
        .await
        .map_err(|error| failure(&error))?;
    // Activates GeoClue if it is installed but not running. The connection is
    // private to this request, so GetClient always returns a fresh client.
    let client_path: OwnedObjectPath = manager.call("GetClient", &()).await.map_err(|error| failure(&error))?;
    Ok((connection, manager, client_path))
}

async fn acquire(
    connection: &Connection,
    client: &Proxy<'_>,
    high_accuracy: bool,
    timeout_ms: u64,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<Position, LocationError> {
    let level = if high_accuracy { ACCURACY_EXACT } else { ACCURACY_STREET };
    let mut updates = until_cancelled(cancel, async {
        client
            .set_property("DesktopId", DESKTOP_ID)
            .await
            .map_err(|error| fdo_failure(&error))?;
        client
            .set_property("RequestedAccuracyLevel", level)
            .await
            .map_err(|error| fdo_failure(&error))?;
        // Subscribe before Start so the first fix cannot be missed.
        let updates = client
            .receive_signal("LocationUpdated")
            .await
            .map_err(|error| failure(&error))?;
        // Start returns once the desktop's location agent has authorized the
        // app (or refused it); the page's timeout starts after authorization,
        // as on the other platforms.
        let () = client.call("Start", &()).await.map_err(|error| failure(&error))?;
        Ok::<_, LocationError>(updates)
    })
    .await?;
    let first_fix = async {
        let message = updates
            .next()
            .await
            .ok_or_else(|| LocationError::unavailable("GeoClue stopped before reporting a location"))?;
        let body = message.body();
        let (_old, new): (OwnedObjectPath, OwnedObjectPath) = body.deserialize().map_err(|_| invalid())?;
        let location = Proxy::new(connection, SERVICE, new.as_str(), PROPERTIES_INTERFACE)
            .await
            .map_err(|error| failure(&error))?;
        let values: HashMap<String, OwnedValue> = location
            .call("GetAll", &(LOCATION_INTERFACE,))
            .await
            .map_err(|error| failure(&error))?;
        convert(&values)
    };
    tokio::select! {
        biased;
        _ = &mut *cancel => Err(LocationError::cancelled()),
        _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => Err(timed_out()),
        result = first_fix => result,
    }
}

/// Run `work` unless the request is cancelled first. A cancel channel that
/// has fired is never polled again: every caller returns on cancellation.
async fn until_cancelled<T>(
    cancel: &mut oneshot::Receiver<()>,
    work: impl Future<Output = Result<T, LocationError>>,
) -> Result<T, LocationError> {
    tokio::select! {
        biased;
        _ = &mut *cancel => Err(LocationError::cancelled()),
        result = work => result,
    }
}

/// TIMEOUT (code 3), built directly so this module needs no permissions.rs change.
fn timed_out() -> LocationError {
    LocationError { code: 3, message: TIMED_OUT.into(), cancelled: false }
}

fn invalid() -> LocationError {
    LocationError::unavailable("GeoClue returned an invalid position")
}

/// Shape a GeoClue Location object's properties (`GetAll` result).
fn convert(values: &HashMap<String, OwnedValue>) -> Result<Position, LocationError> {
    let number = |name: &str| match values.get(name).map(|value| &**value) {
        Some(Value::F64(number)) => Some(*number),
        _ => None,
    };
    let timestamp = match values.get("Timestamp").map(|value| &**value) {
        Some(Value::Structure(structure)) => match structure.fields() {
            [Value::U64(seconds), Value::U64(micros)] => Some((*seconds, *micros)),
            _ => None,
        },
        _ => None,
    }
    .or_else(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
        Some((now.as_secs(), u64::from(now.subsec_micros())))
    });
    let (Some(latitude), Some(longitude), Some(accuracy), Some((seconds, micros))) = (
        number("Latitude"),
        number("Longitude"),
        number("Accuracy"),
        timestamp,
    ) else {
        return Err(invalid());
    };
    position_from(
        latitude,
        longitude,
        accuracy,
        number("Altitude"),
        number("Heading"),
        number("Speed"),
        seconds,
        micros,
    )
    .ok_or_else(invalid)
}

/// Validate and shape a GeoClue fix like the other providers'. GeoClue marks
/// an unknown altitude with -DBL_MAX and unknown speed/heading with -1; those
/// and non-finite values are dropped. `seconds`/`micros` are GeoClue's
/// Timestamp (since the Unix epoch). A fix outside the globe, without a usable
/// accuracy, or without a timestamp is refused.
#[allow(clippy::too_many_arguments)]
pub(crate) fn position_from(
    latitude: f64,
    longitude: f64,
    accuracy: f64,
    altitude: Option<f64>,
    heading: Option<f64>,
    speed: Option<f64>,
    seconds: u64,
    micros: u64,
) -> Option<Position> {
    if !latitude.is_finite()
        || !longitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !(-180.0..=180.0).contains(&longitude)
        || !accuracy.is_finite()
        || accuracy < 0.0
        || seconds == 0
        || micros >= 1_000_000
    {
        return None;
    }
    let millis = seconds.checked_mul(1000)?.checked_add(micros / 1000)?;
    let nonnegative = |value: Option<f64>| value.filter(|value| value.is_finite() && *value >= 0.0);
    Some(Position {
        timestamp: millis as f64,
        coords: Coordinates {
            latitude,
            longitude,
            accuracy,
            // Altitude may legitimately be negative; only GeoClue's sentinel is unknown.
            altitude: altitude.filter(|value| value.is_finite() && *value > f64::MIN),
            altitude_accuracy: None,
            heading: nonnegative(heading),
            speed: nonnegative(speed),
        },
    })
}

/// What a D-Bus error name means for a location request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The bus policy, GeoClue or the desktop's location agent refused.
    Denied,
    /// GeoClue is not installed or could not be started.
    NoService,
    Other,
}

pub(crate) fn classify(name: &str) -> Failure {
    match name {
        "org.freedesktop.DBus.Error.AccessDenied" => Failure::Denied,
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" => Failure::NoService,
        name if name.starts_with("org.freedesktop.DBus.Error.Spawn.") => Failure::NoService,
        _ => Failure::Other,
    }
}

fn error_name(error: &zbus::Error) -> Option<String> {
    match error {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str().to_owned()),
        zbus::Error::FDO(error) => fdo_error_name(error),
        _ => None,
    }
}

fn fdo_error_name(error: &zbus::fdo::Error) -> Option<String> {
    match error {
        zbus::fdo::Error::ZBus(error) => error_name(error),
        error => Some(DBusError::name(error).as_str().to_owned()),
    }
}

fn mapped(name: Option<String>, detail: String) -> LocationError {
    match name.as_deref().map(classify) {
        Some(Failure::Denied) if detail.contains("no agent") => LocationError::unavailable(NO_AGENT),
        Some(Failure::Denied) => LocationError::denied(OFF_IN_SETTINGS),
        Some(Failure::NoService) => LocationError::unavailable(NEEDS_GEOCLUE),
        _ => LocationError::unavailable(&format!("GeoClue could not get a location: {detail}")),
    }
}

fn failure(error: &zbus::Error) -> LocationError {
    mapped(error_name(error), error.to_string())
}

fn fdo_failure(error: &zbus::fdo::Error) -> LocationError {
    mapped(fdo_error_name(error), error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_2026: u64 = 1_767_268_800; // 2026-01-01T12:00:00Z

    #[test]
    fn geoclue_fixes_are_shaped_like_the_web_api() {
        let fix = position_from(32.3, -64.8, 25.0, Some(-12.5), Some(-1.0), Some(1.5), NOON_2026, 250_999).unwrap();
        assert_eq!(fix.timestamp, 1_767_268_800_250.0);
        assert_eq!((fix.coords.latitude, fix.coords.longitude, fix.coords.accuracy), (32.3, -64.8, 25.0));
        assert_eq!(fix.coords.altitude, Some(-12.5), "below sea level is a real altitude");
        assert_eq!(fix.coords.altitude_accuracy, None);
        assert_eq!(fix.coords.heading, None, "-1 heading is unknown");
        assert_eq!(fix.coords.speed, Some(1.5));
    }

    #[test]
    fn geoclue_unknown_markers_are_dropped() {
        let fix = position_from(0.0, 0.0, 5.0, Some(f64::MIN), Some(f64::NAN), Some(-1.0), NOON_2026, 0).unwrap();
        assert_eq!(fix.coords.altitude, None, "-DBL_MAX altitude is unknown");
        assert_eq!(fix.coords.heading, None);
        assert_eq!(fix.coords.speed, None);
    }

    #[test]
    fn impossible_geoclue_fixes_are_refused() {
        assert!(position_from(91.0, 0.0, 5.0, None, None, None, NOON_2026, 0).is_none());
        assert!(position_from(0.0, -181.0, 5.0, None, None, None, NOON_2026, 0).is_none());
        assert!(position_from(f64::NAN, 0.0, 5.0, None, None, None, NOON_2026, 0).is_none());
        assert!(position_from(0.0, 0.0, -1.0, None, None, None, NOON_2026, 0).is_none());
        assert!(position_from(0.0, 0.0, f64::INFINITY, None, None, None, NOON_2026, 0).is_none());
        assert!(position_from(0.0, 0.0, 5.0, None, None, None, 0, 0).is_none(), "no timestamp");
        assert!(position_from(0.0, 0.0, 5.0, None, None, None, NOON_2026, 1_000_000).is_none());
        assert!(position_from(0.0, 0.0, 5.0, None, None, None, u64::MAX, 0).is_none(), "overflow");
    }

    #[test]
    fn dbus_errors_map_to_geolocation_codes() {
        assert_eq!(classify("org.freedesktop.DBus.Error.AccessDenied"), Failure::Denied);
        assert_eq!(classify("org.freedesktop.DBus.Error.ServiceUnknown"), Failure::NoService);
        assert_eq!(classify("org.freedesktop.DBus.Error.NameHasNoOwner"), Failure::NoService);
        assert_eq!(classify("org.freedesktop.DBus.Error.Spawn.ChildExited"), Failure::NoService);
        assert_eq!(classify("org.freedesktop.DBus.Error.Failed"), Failure::Other);
        assert_eq!(mapped(Some("org.freedesktop.DBus.Error.AccessDenied".into()), String::new()).code, 1);
        assert_eq!(mapped(Some("org.freedesktop.DBus.Error.ServiceUnknown".into()), String::new()).code, 2);
        assert_eq!(mapped(None, "broken".into()).code, 2);
    }

    #[test]
    fn a_cancel_before_the_thread_runs_is_honoured() {
        let token = u64::MAX - 7;
        let (sender, mut receiver) = oneshot::channel();
        pending().insert(token, sender);
        cancel(token);
        assert!(!pending().contains_key(&token));
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let result = runtime.block_on(until_cancelled(&mut receiver, std::future::pending::<Result<(), LocationError>>()));
        assert!(result.unwrap_err().cancelled);
    }
}
