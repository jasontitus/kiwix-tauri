//! Archive-scoped sensor consent. Only trusted shell commands may call this module.
//!
//! The permission callback has no requesting-frame origin. Its archive-scoped
//! decision requires one active archive subtree, a per-window manager, and an
//! exact nonce-origin Permissions-Policy. Descendants may share an existing
//! grant only inside that same archive origin; external frames/popups are denied.
//! No active archive or no explicit app grant always denies native requests.
//! Revocation destroys the whole subtree and remounts a fresh nonce origin:
//! WebKit can cache OS grants independently of this callback.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::webview::{PermissionKind, PermissionResponse};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SensorKind {
    Camera,
    Location,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SensorDecision {
    #[default]
    NotAsked,
    Granted,
    Denied,
}

#[derive(Clone, Copy, Default)]
struct Consent {
    camera: SensorDecision,
    location: SensorDecision,
}

struct Active {
    session_id: String,
    archive_id: String,
    origin: String,
    installation_id: String,
}

/// Dormant identity only: this never grants sensor access while suspended.
#[cfg(any(mobile, test))]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RetainedSensorSession {
    pub session_id: String,
    pub archive_id: String,
    pub installation_id: String,
    pub origin: String,
    pub generation: u64,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    #[cfg(any(mobile, test))]
    retained: Option<RetainedSensorSession>,
    decisions: HashMap<String, Consent>,
    requests: HashMap<(String, u64), u64>,
}

#[derive(Clone, Default)]
pub struct SensorManager(Arc<Mutex<State>>);
// Native provider registries are process-global, so identifiers must not
// collide between managers belonging to separate native shell windows.
#[cfg(native_location)]
static NEXT_NATIVE_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub altitude: Option<f64>,
    pub altitude_accuracy: Option<f64>,
    pub heading: Option<f64>,
    pub speed: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Position {
    pub coords: Coordinates,
    pub timestamp: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocationError {
    pub code: u8,
    pub message: String,
    pub cancelled: bool,
}

impl LocationError {
    #[cfg(any(native_location, test))]
    pub(crate) fn denied(message: &str) -> Self {
        Self {
            code: 1,
            message: message.into(),
            cancelled: false,
        }
    }
    pub(crate) fn unavailable(message: &str) -> Self {
        Self {
            code: 2,
            message: message.into(),
            cancelled: false,
        }
    }
    #[cfg(any(target_os = "windows", target_os = "android", test))]
    pub(crate) fn timeout(message: &str) -> Self {
        Self {
            code: 3,
            message: message.into(),
            cancelled: false,
        }
    }
    #[cfg(any(native_location, mobile, test))]
    pub(crate) fn cancelled() -> Self {
        Self {
            code: 2,
            message: "Location request was cancelled".into(),
            cancelled: true,
        }
    }
}

impl SensorManager {
    /// Session IDs must be unpredictable and replaced on each iframe mount.
    /// Grants and denials are remembered only in memory, keyed by trusted installation.
    pub fn activate(
        &self,
        session_id: String,
        archive_id: String,
        installation_id: String,
        origin: String,
    ) -> Result<(), String> {
        let parsed = url::Url::parse(&origin).map_err(|e| e.to_string())?;
        let archive_uuid =
            uuid::Uuid::parse_str(&archive_id).map_err(|_| "This archive cannot use device sensors in this window. Open the article again")?;
        let archive_id = archive_uuid.hyphenated().to_string();
        let installation_id = uuid::Uuid::parse_str(&installation_id)
            .map_err(|_| "This archive cannot use device sensors in this window. Open the article again")?
            .hyphenated()
            .to_string();
        let expected_prefix = format!("{archive_id}-");
        let valid_host = parsed
            .host_str()
            .and_then(|host| host.strip_suffix(".localhost"))
            .and_then(|host| host.strip_prefix(&expected_prefix))
            .is_some_and(|nonce| nonce.len() == 16 && nonce.bytes().all(|b| b.is_ascii_hexdigit()));
        if session_id.is_empty()
            || archive_id.is_empty()
            || parsed.scheme() != "http"
            || !valid_host
            || parsed.port().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
            || parsed.origin().ascii_serialization() != origin
        {
            return Err("This archive cannot use device sensors in this window. Open the article again".into());
        }
        let mut state = self.0.lock().map_err(|_| "Sensor state unavailable")?;
        #[cfg(any(mobile, test))]
        { state.retained = None; }
        cancel_requests(&mut state);
        state.active = Some(Active {
            session_id,
            archive_id,
            origin,
            installation_id,
        });
        Ok(())
    }

    /// Exact source identity, not a consent decision. Native root services must
    /// not borrow a replaced iframe's session or another installation's grants.
    pub fn owns_session(
        &self,
        session_id: &str,
        archive_id: &str,
        installation_id: &str,
        origin: &str,
    ) -> bool {
        self.0.lock().is_ok_and(|state| state.active.as_ref().is_some_and(|active| {
            active.session_id == session_id
                && active.archive_id == archive_id
                && active.installation_id == installation_id
                && active.origin == origin
        }))
    }

    #[cfg(any(mobile, test))]
    pub(crate) fn suspend(&self, generation: u64) {
        if let Ok(mut state) = self.0.lock() {
            if let Some(active) = state.active.take() {
                state.retained = Some(RetainedSensorSession {
                    session_id: active.session_id, archive_id: active.archive_id,
                    installation_id: active.installation_id, origin: active.origin, generation,
                });
            }
            cancel_requests(&mut state);
        }
    }
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn retained_session(&self) -> Option<RetainedSensorSession> {
        self.0.lock().ok().and_then(|state| state.retained.clone())
    }
    #[cfg(any(mobile, test))]
    pub(crate) fn restore(&self, owner: &RetainedSensorSession) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Sensor state unavailable")?;
        if state.retained.as_ref() != Some(owner) ||
            state.active.as_ref().is_some_and(|active| active.session_id != owner.session_id ||
                active.archive_id != owner.archive_id || active.installation_id != owner.installation_id ||
                active.origin != owner.origin) {
            return Err("Retained sensor session is no longer owned by this reader".into());
        }
        state.active = Some(Active {
            session_id: owner.session_id.clone(), archive_id: owner.archive_id.clone(),
            installation_id: owner.installation_id.clone(), origin: owner.origin.clone(),
        });
        Ok(())
    }
    pub fn deactivate(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.active = None;
            #[cfg(any(mobile, test))]
            { state.retained = None; }
            cancel_requests(&mut state);
        }
    }
    /// Privacy-mode transitions retire all remembered decisions in this window.
    pub fn clear_decisions(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.active = None;
            #[cfg(any(mobile, test))]
            { state.retained = None; }
            cancel_requests(&mut state);
            state.decisions.clear();
        }
    }
    /// Relinking/removing an installation retires its decisions and active frame.
    pub fn forget_installation(&self, installation_id: &str) {
        if let Ok(mut state) = self.0.lock() {
            state.decisions.remove(installation_id);
            #[cfg(any(mobile, test))]
            if state.retained.as_ref().is_some_and(|owner| owner.installation_id == installation_id) {
                state.retained = None;
            }
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.installation_id == installation_id)
            {
                state.active = None;
                cancel_requests(&mut state);
            }
        }
    }
    /// Ignore late teardown from an iframe replaced by a newer session.
    pub fn deactivate_session(&self, session_id: &str) {
        if let Ok(mut state) = self.0.lock() {
            #[cfg(any(mobile, test))]
            if state.retained.as_ref().is_some_and(|owner| owner.session_id == session_id) {
                state.retained = None;
            }
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.session_id == session_id)
            {
                state.active = None;
                cancel_requests(&mut state);
            }
        }
    }

    /// Call only after the trusted shell has obtained explicit archive consent.
    /// A denial/revocation cannot become a grant until trusted reset.
    /// Revoking location immediately cancels every pending native request.
    pub fn set_consent(
        &self,
        session_id: &str,
        kind: SensorKind,
        allowed: bool,
    ) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Sensor state unavailable")?;
        let active = state
            .active
            .as_ref()
            .filter(|a| a.session_id == session_id)
            .ok_or("Archive sensor session is no longer active")?;
        let installation_id = active.installation_id.clone();
        let consent = state.decisions.entry(installation_id).or_default();
        let decision = match kind {
            SensorKind::Camera => &mut consent.camera,
            SensorKind::Location => &mut consent.location,
        };
        if allowed && *decision == SensorDecision::Denied {
            return Err("Reset this sensor's denied decision before asking again".into());
        }
        *decision = if allowed {
            SensorDecision::Granted
        } else {
            SensorDecision::Denied
        };
        if kind == SensorKind::Location && !allowed {
            cancel_requests(&mut state);
        }
        Ok(())
    }

    /// Only trusted shell controls may clear a selected sensor's decision.
    pub fn reset_consent(&self, session_id: &str, kind: SensorKind) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Sensor state unavailable")?;
        let installation_id = state
            .active
            .as_ref()
            .filter(|a| a.session_id == session_id)
            .ok_or("Archive sensor session is no longer active")?
            .installation_id
            .clone();
        let consent = state.decisions.entry(installation_id).or_default();
        match kind {
            SensorKind::Camera => consent.camera = SensorDecision::NotAsked,
            SensorKind::Location => {
                consent.location = SensorDecision::NotAsked;
                cancel_requests(&mut state);
            }
        }
        Ok(())
    }

    pub fn decision(&self, session_id: &str, kind: SensorKind) -> SensorDecision {
        let Ok(state) = self.0.lock() else {
            return SensorDecision::Denied;
        };
        let Some(active) = state.active.as_ref().filter(|a| a.session_id == session_id) else {
            return SensorDecision::Denied;
        };
        let consent = state
            .decisions
            .get(&active.installation_id)
            .copied()
            .unwrap_or_default();
        match kind {
            SensorKind::Camera => consent.camera,
            SensorKind::Location => consent.location,
        }
    }

    #[cfg(test)]
    pub fn has_consent(&self, session_id: &str, kind: SensorKind) -> bool {
        self.decision(session_id, kind) == SensorDecision::Granted
    }

    /// Whether this exact session is still the active one. Status reads use it
    /// to avoid reporting an inactive session as an explicit denial.
    pub fn owns_session_id(&self, session_id: &str) -> bool {
        self.0.lock().is_ok_and(|state| {
            state
                .active
                .as_ref()
                .is_some_and(|active| active.session_id == session_id)
        })
    }

    /// The decision remembered for an installation while no session owns it.
    /// Only trusted shell status reads may use this fallback: a session that is
    /// simply inactive must not look like a denial the user chose.
    pub fn installation_decision(
        &self,
        installation_id: &str,
        kind: SensorKind,
    ) -> SensorDecision {
        let Ok(state) = self.0.lock() else {
            return SensorDecision::Denied;
        };
        let consent = state
            .decisions
            .get(installation_id)
            .copied()
            .unwrap_or_default();
        match kind {
            SensorKind::Camera => consent.camera,
            SensorKind::Location => consent.location,
        }
    }

    /// Parent must select this per-window manager by the exact native webview
    /// label; unknown/remote webviews must never receive another window's state.
    /// WebKitGTK has no built-in permission dialog: after explicit shell consent
    /// it needs Allow. WK/WebView2 retain browser prompts via Default.
    /// Microphone (including combined AV) and unrelated permissions stay denied.
    pub fn permission_response(
        &self,
        _webview_label: &str,
        kind: PermissionKind,
    ) -> PermissionResponse {
        let Ok(state) = self.0.lock() else {
            return PermissionResponse::Deny;
        };
        let Some(active) = state.active.as_ref() else {
            return PermissionResponse::Deny;
        };
        let consent = state
            .decisions
            .get(&active.installation_id)
            .copied()
            .unwrap_or_default();
        match kind {
            PermissionKind::Camera if consent.camera == SensorDecision::Granted => {
                consented_browser_response()
            }
            // With an app provider the webview's own geolocation is never used.
            #[cfg(not(native_location))]
            PermissionKind::Geolocation if consent.location == SensorDecision::Granted => {
                consented_browser_response()
            }
            _ => PermissionResponse::Deny,
        }
    }

    pub fn cancel_location(&self, session_id: &str, request_id: u64) {
        if let Ok(mut state) = self.0.lock() {
            if let Some(token) = state.requests.remove(&(session_id.into(), request_id)) {
                cancel_native(token);
            }
        }
    }

    /// A retired request is not evidence that the user denied permission.
    /// Validate the original producer and token before delivering any native fix.
    #[cfg(any(native_location, test))]
    fn validate_location_completion(
        &self,
        request: &(String, u64),
        token: u64,
    ) -> Result<(), LocationError> {
        let state = self.0.lock()
            .map_err(|_| LocationError::unavailable("Sensor state unavailable"))?;
        let active = state.active.as_ref()
            .filter(|active| active.session_id == request.0)
            .ok_or_else(LocationError::cancelled)?;
        let decision = state.decisions.get(&active.installation_id)
            .map(|consent| consent.location).unwrap_or_default();
        if decision == SensorDecision::Denied {
            return Err(LocationError::denied("Location consent was revoked"));
        }
        if decision != SensorDecision::Granted
            || state.requests.get(request) != Some(&token)
        {
            return Err(LocationError::cancelled());
        }
        Ok(())
    }

    pub async fn location(
        &self,
        session_id: &str,
        request_id: u64,
        high_accuracy: bool,
        timeout_ms: u64,
    ) -> Result<Position, LocationError> {
        #[cfg(not(native_location))]
        {
            let _ = (session_id, request_id, high_accuracy, timeout_ms);
            Err(LocationError::unavailable(
                "Location is unavailable on this platform",
            ))
        }
        #[cfg(native_location)]
        {
            let timeout_ms = timeout_ms.min(30_000);
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let token;
            {
                let mut state = self
                    .0
                    .lock()
                    .map_err(|_| LocationError::unavailable("Sensor state unavailable"))?;
                let active = state
                    .active
                    .as_ref()
                    .filter(|a| a.session_id == session_id)
                    .ok_or_else(LocationError::cancelled)?;
                if !state
                    .decisions
                    .get(&active.installation_id)
                    .is_some_and(|c| c.location == SensorDecision::Granted)
                {
                    return Err(LocationError::denied(
                        "Location permission has not been granted for this archive",
                    ));
                }
                if state
                    .requests
                    .contains_key(&(session_id.into(), request_id))
                {
                    return Err(LocationError::unavailable("Duplicate location request"));
                }
                if state.requests.len() >= 32 {
                    return Err(LocationError::unavailable(
                        "Too many pending location requests",
                    ));
                }
                let mut current = NEXT_NATIVE_TOKEN.load(std::sync::atomic::Ordering::Relaxed);
                token = loop {
                    let next = current.checked_add(1).ok_or_else(|| {
                        LocationError::unavailable("Location request identifiers exhausted")
                    })?;
                    match NEXT_NATIVE_TOKEN.compare_exchange_weak(
                        current,
                        next,
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                    ) {
                        Ok(_) => break next,
                        Err(actual) => current = actual,
                    }
                };
                state
                    .requests
                    .insert((session_id.into(), request_id), token);
                // Enqueue start while holding the state lock, so concurrent revoke
                // cannot enqueue cancellation before the corresponding start.
                start_native(token, high_accuracy, timeout_ms, sender);
            }
            let guard = RequestGuard {
                manager: self.clone(),
                request: (session_id.into(), request_id),
                token,
            };
            // Native completion owns the acquisition deadline, which starts only
            // after OS authorization. An outer timeout would include the prompt.
            let result = receiver.await.unwrap_or_else(|_| {
                Err(LocationError::unavailable(
                    "Native location provider stopped",
                ))
            });
            // Background retirement may precede the shell's lifecycle event.
            // Preserve its cancellation identity rather than inventing a denial.
            let authority = self.validate_location_completion(&guard.request, token);
            drop(guard);
            authority?;
            result
        }
    }
}
// WebKitGTK's unhandled permission-request default is denial, not a browser
// prompt. The trusted archive-scoped shell confirmation is its permission UI.
fn consented_browser_response() -> PermissionResponse {
    if cfg!(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )) {
        PermissionResponse::Allow
    } else {
        PermissionResponse::Default
    }
}

fn cancel_requests(state: &mut State) {
    for (_, token) in state.requests.drain() {
        cancel_native(token);
    }
}

/// Hand one request to the platform provider. It must complete `sender`
/// exactly once (dropping it reports "provider stopped"), and `cancel_native`
/// with the same token must end it promptly.
#[cfg(native_location)]
fn start_native(
    token: u64,
    high_accuracy: bool,
    timeout_ms: u64,
    sender: tokio::sync::oneshot::Sender<Result<Position, LocationError>>,
) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    unsafe {
        kiwix_location_start(
            token,
            high_accuracy,
            timeout_ms,
            Box::into_raw(Box::new(sender)).cast(),
            location_callback,
        );
    }
    #[cfg(target_os = "windows")]
    crate::location_windows::start(token, high_accuracy, timeout_ms, sender);
    #[cfg(all(target_os = "linux", native_location))]
    crate::location_linux::start(token, high_accuracy, timeout_ms, sender);
    #[cfg(all(target_os = "android", native_location))]
    crate::location_android::start(token, high_accuracy, timeout_ms, sender);
}

fn cancel_native(token: u64) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    unsafe {
        kiwix_location_cancel(token);
    }
    #[cfg(target_os = "windows")]
    crate::location_windows::cancel(token);
    #[cfg(all(target_os = "linux", native_location))]
    crate::location_linux::cancel(token);
    #[cfg(all(target_os = "android", native_location))]
    crate::location_android::cancel(token);
    #[cfg(not(native_location))]
    let _ = token;
}

#[cfg(native_location)]
struct RequestGuard {
    manager: SensorManager,
    request: (String, u64),
    token: u64,
}

#[cfg(native_location)]
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.manager.0.lock() {
            if state.requests.get(&self.request) == Some(&self.token) {
                state.requests.remove(&self.request);
                cancel_native(self.token);
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[repr(C)]
struct NativePosition {
    latitude: f64,
    longitude: f64,
    accuracy: f64,
    altitude: f64,
    altitude_accuracy: f64,
    heading: f64,
    speed: f64,
    timestamp: f64,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
unsafe extern "C" {
    fn kiwix_location_start(
        token: u64,
        high_accuracy: bool,
        timeout_ms: u64,
        context: *mut std::ffi::c_void,
        callback: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            i32,
            *const NativePosition,
            *const std::ffi::c_char,
        ),
    );
    fn kiwix_location_cancel(token: u64);
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
unsafe extern "C" fn location_callback(
    context: *mut std::ffi::c_void,
    code: i32,
    position: *const NativePosition,
    message: *const std::ffi::c_char,
) {
    let sender: Box<tokio::sync::oneshot::Sender<Result<Position, LocationError>>> =
        unsafe { Box::from_raw(context.cast()) };
    let result = if code == 0 && !position.is_null() {
        let p = unsafe { &*position };
        if !p.latitude.is_finite()
            || !p.longitude.is_finite()
            || !p.accuracy.is_finite()
            || !p.timestamp.is_finite()
            || !(-90.0..=90.0).contains(&p.latitude)
            || !(-180.0..=180.0).contains(&p.longitude)
            || p.accuracy < 0.0
        {
            Err(LocationError::unavailable(
                "Native location provider returned an invalid position",
            ))
        } else {
            let nonnegative = |v: f64| {
                if v.is_finite() && v >= 0.0 {
                    Some(v)
                } else {
                    None
                }
            };
            Ok(Position {
                timestamp: p.timestamp,
                coords: Coordinates {
                    latitude: p.latitude,
                    longitude: p.longitude,
                    accuracy: p.accuracy,
                    altitude: p.altitude.is_finite().then_some(p.altitude),
                    altitude_accuracy: nonnegative(p.altitude_accuracy),
                    heading: nonnegative(p.heading),
                    speed: nonnegative(p.speed),
                },
            })
        }
    } else {
        let message = if message.is_null() {
            "Location unavailable".into()
        } else {
            unsafe { std::ffi::CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        };
        Err(LocationError {
            code: match code {
                1 => 1,
                3 => 3,
                _ => 2,
            },
            message,
            cancelled: false,
        })
    };
    let _ = sender.send(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCHIVE_A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const ARCHIVE_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn activate(manager: &SensorManager, session: &str, archive: &str) {
        manager
            .activate(
                session.into(),
                archive.into(),
                archive.into(),
                format!("http://{archive}-0123456789abcdef.localhost:8123"),
            )
            .unwrap();
    }

    #[test]
    fn inactive_session_keeps_the_installation_decision_for_status_reads() {
        let manager = SensorManager::default();
        activate(&manager, "first", ARCHIVE_A);
        manager
            .set_consent("first", SensorKind::Camera, true)
            .unwrap();
        assert!(manager.owns_session_id("first"));
        manager.deactivate_session("first");
        assert!(!manager.owns_session_id("first"));
        // A status read keyed by the dead session fails closed...
        assert_eq!(
            manager.decision("first", SensorKind::Camera),
            SensorDecision::Denied
        );
        // ...but the trusted shell can still name the installation to see the
        // decision the store actually holds instead of a false block.
        assert_eq!(
            manager.installation_decision(ARCHIVE_A, SensorKind::Camera),
            SensorDecision::Granted
        );
        assert_eq!(
            manager.installation_decision(ARCHIVE_A, SensorKind::Location),
            SensorDecision::NotAsked
        );
    }

    #[test]
    fn unasked_denied_and_granted_are_distinct_and_denial_requires_reset() {
        let manager = SensorManager::default();
        activate(&manager, "first", ARCHIVE_A);
        assert_eq!(
            manager.decision("first", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        assert!(matches!(
            manager.permission_response("main", PermissionKind::Camera),
            PermissionResponse::Deny
        ));
        manager
            .set_consent("first", SensorKind::Camera, false)
            .unwrap();
        assert_eq!(
            manager.decision("first", SensorKind::Camera),
            SensorDecision::Denied
        );
        assert!(!manager.has_consent("first", SensorKind::Camera));
        assert!(manager
            .set_consent("first", SensorKind::Camera, true)
            .is_err());
        manager.reset_consent("first", SensorKind::Camera).unwrap();
        assert_eq!(
            manager.decision("first", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        manager
            .set_consent("first", SensorKind::Camera, true)
            .unwrap();
        assert!(manager.has_consent("first", SensorKind::Camera));
        assert!(!matches!(
            manager.permission_response("main", PermissionKind::Camera),
            PermissionResponse::Deny
        ));
        manager
            .set_consent("first", SensorKind::Camera, false)
            .unwrap();
        assert!(matches!(
            manager.permission_response("main", PermissionKind::Camera),
            PermissionResponse::Deny
        ));
        assert_eq!(
            serde_json::to_value(SensorDecision::NotAsked).unwrap(),
            "notAsked"
        );
        assert_eq!(
            serde_json::to_value(SensorDecision::Denied).unwrap(),
            "denied"
        );
        assert_eq!(
            serde_json::to_value(SensorDecision::Granted).unwrap(),
            "granted"
        );
    }

    #[test]
    fn decisions_survive_remount_but_are_archive_and_window_scoped() {
        let manager = SensorManager::default();
        activate(&manager, "a-first", ARCHIVE_A);
        manager
            .set_consent("a-first", SensorKind::Camera, false)
            .unwrap();
        manager
            .set_consent("a-first", SensorKind::Location, true)
            .unwrap();
        manager.deactivate_session("a-first");
        activate(&manager, "b-first", ARCHIVE_B);
        assert_eq!(
            manager.decision("b-first", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        assert_eq!(
            manager.decision("b-first", SensorKind::Location),
            SensorDecision::NotAsked
        );
        manager
            .set_consent("b-first", SensorKind::Camera, true)
            .unwrap();
        activate(&manager, "a-second", ARCHIVE_A);
        assert_eq!(
            manager.decision("a-second", SensorKind::Camera),
            SensorDecision::Denied
        );
        assert_eq!(
            manager.decision("a-second", SensorKind::Location),
            SensorDecision::Granted
        );
        manager.deactivate_session("a-first");
        assert!(manager.has_consent("a-second", SensorKind::Location));
        assert!(!manager.has_consent("a-first", SensorKind::Location));

        let other_window = SensorManager::default();
        activate(&other_window, "other-window", ARCHIVE_A);
        assert_eq!(
            other_window.decision("other-window", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        assert_eq!(
            other_window.decision("other-window", SensorKind::Location),
            SensorDecision::NotAsked
        );
        activate(&manager, "b-second", ARCHIVE_B);
        assert!(manager.has_consent("b-second", SensorKind::Camera));
    }

    #[test]
    fn reset_requires_active_session_clears_only_selected_sensor_and_cancels_location() {
        let manager = SensorManager::default();
        assert!(manager
            .reset_consent("missing", SensorKind::Location)
            .is_err());
        activate(&manager, "active", ARCHIVE_A);
        manager
            .set_consent("active", SensorKind::Camera, false)
            .unwrap();
        manager
            .set_consent("active", SensorKind::Location, true)
            .unwrap();
        // Native tokens start at one: zero represents no real hardware request.
        manager
            .0
            .lock()
            .unwrap()
            .requests
            .insert(("active".into(), 7), 0);
        assert!(manager
            .reset_consent("stale", SensorKind::Location)
            .is_err());
        assert_eq!(manager.0.lock().unwrap().requests.len(), 1);
        assert!(manager.has_consent("active", SensorKind::Location));
        manager.reset_consent("active", SensorKind::Camera).unwrap();
        assert_eq!(
            manager.decision("active", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        assert_eq!(manager.0.lock().unwrap().requests.len(), 1);
        manager
            .set_consent("active", SensorKind::Camera, true)
            .unwrap();
        manager
            .reset_consent("active", SensorKind::Location)
            .unwrap();
        assert!(manager.0.lock().unwrap().requests.is_empty());
        assert_eq!(
            manager.decision("active", SensorKind::Location),
            SensorDecision::NotAsked
        );
        assert!(manager.has_consent("active", SensorKind::Camera));
        manager.deactivate();
        assert!(manager.reset_consent("active", SensorKind::Camera).is_err());
    }

    #[test]
    fn privacy_transition_retires_active_requests_and_all_remembered_decisions() {
        let manager = SensorManager::default();
        activate(&manager, "a", ARCHIVE_A);
        manager.set_consent("a", SensorKind::Camera, false).unwrap();
        activate(&manager, "b", ARCHIVE_B);
        manager
            .set_consent("b", SensorKind::Location, true)
            .unwrap();
        manager
            .0
            .lock()
            .unwrap()
            .requests
            .insert(("b".into(), 8), 0);
        manager.clear_decisions();
        assert!(manager.0.lock().unwrap().requests.is_empty());
        assert!(manager.reset_consent("b", SensorKind::Location).is_err());
        activate(&manager, "a-new", ARCHIVE_A);
        assert_eq!(
            manager.decision("a-new", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        activate(&manager, "b-new", ARCHIVE_B);
        assert_eq!(
            manager.decision("b-new", SensorKind::Location),
            SensorDecision::NotAsked
        );
    }
    #[test]
    fn same_public_uuid_does_not_share_installation_decisions_and_retirement_cancels_requests() {
        let manager = SensorManager::default();
        activate(&manager, "old", ARCHIVE_A);
        manager
            .set_consent("old", SensorKind::Camera, true)
            .unwrap();
        manager
            .set_consent("old", SensorKind::Location, false)
            .unwrap();
        manager
            .activate(
                "new".into(),
                ARCHIVE_A.into(),
                ARCHIVE_B.into(),
                format!("http://{ARCHIVE_A}-0123456789abcdef.localhost:8123"),
            )
            .unwrap();
        assert_eq!(
            manager.decision("new", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        assert_eq!(
            manager.decision("new", SensorKind::Location),
            SensorDecision::NotAsked
        );
        activate(&manager, "old-again", ARCHIVE_A);
        manager
            .0
            .lock()
            .unwrap()
            .requests
            .insert(("old-again".into(), 9), 0);
        manager.forget_installation(ARCHIVE_A);
        assert!(manager.0.lock().unwrap().requests.is_empty());
        assert!(manager.0.lock().unwrap().active.is_none());
        activate(&manager, "old-retired", ARCHIVE_A);
        assert_eq!(
            manager.decision("old-retired", SensorKind::Camera),
            SensorDecision::NotAsked
        );
        assert_eq!(
            manager.decision("old-retired", SensorKind::Location),
            SensorDecision::NotAsked
        );
    }

    #[test]
    fn native_completion_distinguishes_retirement_from_denial_and_requires_original_token() {
        let manager = SensorManager::default();
        activate(&manager, "active", ARCHIVE_A);
        manager.set_consent("active", SensorKind::Location, true).unwrap();
        manager.0.lock().unwrap().requests.insert(("active".into(), 7), 0);
        assert!(manager.validate_location_completion(&("active".into(), 7), 0).is_ok());
        assert!(manager.validate_location_completion(&("active".into(), 7), 1).unwrap_err().cancelled);
        manager.cancel_location("active", 7);
        assert!(manager.validate_location_completion(&("active".into(), 7), 0).unwrap_err().cancelled);
        manager.0.lock().unwrap().requests.insert(("active".into(), 8), 0);
        manager.deactivate();
        let retired = manager.validate_location_completion(&("active".into(), 8), 0).unwrap_err();
        assert_eq!(retired.code, 2);
        assert_eq!(serde_json::to_value(retired).unwrap()["cancelled"], true);
        activate(&manager, "resumed", ARCHIVE_A);
        assert!(manager.has_consent("resumed", SensorKind::Location));
        assert!(manager.validate_location_completion(&("active".into(), 8), 0).unwrap_err().cancelled);
        manager.0.lock().unwrap().requests.insert(("resumed".into(), 9), 0);
        manager.set_consent("resumed", SensorKind::Location, false).unwrap();
        let revoked = manager.validate_location_completion(&("resumed".into(), 9), 0).unwrap_err();
        assert_eq!(revoked.code, 1);
        assert!(!revoked.cancelled);
        manager.reset_consent("resumed", SensorKind::Location).unwrap();
        assert!(manager.validate_location_completion(&("resumed".into(), 9), 0).unwrap_err().cancelled);
    }

    #[test]
    fn source_ownership_requires_exact_session_archive_installation_and_origin() {
        let manager = SensorManager::default();
        let origin = format!("http://{ARCHIVE_A}-0123456789abcdef.localhost:8123");
        activate(&manager, "first", ARCHIVE_A);
        assert!(manager.owns_session("first", ARCHIVE_A, ARCHIVE_A, &origin));
        assert!(!manager.owns_session("stale", ARCHIVE_A, ARCHIVE_A, &origin));
        assert!(!manager.owns_session("first", ARCHIVE_B, ARCHIVE_A, &origin));
        assert!(!manager.owns_session("first", ARCHIVE_A, ARCHIVE_B, &origin));
        assert!(!manager.owns_session("first", ARCHIVE_A, ARCHIVE_A, &origin.replace("8123", "8124")));
        activate(&manager, "replacement", ARCHIVE_A);
        manager.deactivate_session("first");
        assert!(!manager.owns_session("first", ARCHIVE_A, ARCHIVE_A, &origin));
        assert!(manager.owns_session("replacement", ARCHIVE_A, ARCHIVE_A, &origin));
        manager.forget_installation(ARCHIVE_A);
        assert!(!manager.owns_session("replacement", ARCHIVE_A, ARCHIVE_A, &origin));
    }
    #[test]
    fn background_retains_only_dormant_identity_and_every_terminal_boundary_invalidates_it() {
        let manager = SensorManager::default();
        activate(&manager, "retained", ARCHIVE_A);
        manager.set_consent("retained", SensorKind::Camera, true).unwrap();
        manager.0.lock().unwrap().requests.insert(("retained".into(), 8), 0);
        manager.suspend(7);
        let retained = manager.retained_session().unwrap();
        assert!(!manager.owns_session_id("retained"));
        assert!(!manager.has_consent("retained", SensorKind::Camera));
        assert!(manager.0.lock().unwrap().requests.is_empty());
        for field in ["session", "archive", "installation", "origin", "generation"] {
            let mut changed = retained.clone();
            match field {
                "session" => changed.session_id = "replacement".into(),
                "archive" => changed.archive_id = ARCHIVE_B.into(),
                "installation" => changed.installation_id = ARCHIVE_B.into(),
                "origin" => changed.origin.push_str(".replacement"),
                _ => changed.generation += 1,
            }
            assert!(manager.restore(&changed).is_err(), "{field}");
            assert!(!manager.owns_session_id("retained"));
        }
        manager.restore(&retained).unwrap();
        assert!(manager.owns_session_id("retained"));
        assert!(manager.has_consent("retained", SensorKind::Camera));
        manager.suspend(7);
        activate(&manager, "replacement", ARCHIVE_A);
        assert!(manager.restore(&retained).is_err());
        assert!(manager.owns_session_id("replacement"));
        for boundary in ["deactivate", "privacy", "installation", "session"] {
            let manager = SensorManager::default();
            activate(&manager, "retained", ARCHIVE_A);
            manager.suspend(7);
            let retained = manager.retained_session().unwrap();
            match boundary {
                "deactivate" => manager.deactivate(),
                "privacy" => manager.clear_decisions(),
                "installation" => manager.forget_installation(ARCHIVE_A),
                _ => manager.deactivate_session("retained"),
            }
            assert!(manager.retained_session().is_none(), "{boundary}");
            assert!(manager.restore(&retained).is_err(), "{boundary}");
            assert!(!manager.owns_session_id("retained"));
        }
    }
}
