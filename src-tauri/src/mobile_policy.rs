// SPDX-License-Identifier: GPL-3.0-or-later
//! Include as a Service child module. Policy is an additional fence, never a substitute
//! for iOS checkpoint ownership, Android foreground-service admission or privacy leases.
#![forbid(unsafe_code)]
#[cfg(mobile)]
use super::Service;
use serde::Deserialize;
#[cfg(mobile)]
use serde_json::json;
use serde_json::Value;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeNetworkState {
    pub known: bool,
    pub connected: bool,
    pub metered: bool,
    pub cellular: bool,
    pub local_available: bool,
    pub generation: u64,
}
impl NativeNetworkState {
    pub(crate) fn internet_allowed(self, allow_metered: bool) -> bool {
        self.known && self.connected && (allow_metered || (!self.metered && !self.cellular))
    }
    fn local_allowed(self) -> bool { self.known && self.local_available }
}

#[derive(Clone, Copy)]
struct PolicySnapshot {
    native: NativeNetworkState,
    allow_metered: bool,
    denial_revision: u64,
    acknowledged_revision: u64,
    local_denial_revision: u64,
    local_acknowledged_revision: u64,
    /// Monotonic admission frontier for automatic reconnect resume. Bumped by
    /// every native connectivity change and by a metered-allowance edit, so a
    /// reconnect and a settings re-admission are distinct one-shot attempts.
    resume_revision: u64,
}
impl PolicySnapshot {
    fn network_allowed(self) -> bool { self.native.internet_allowed(self.allow_metered) }
    fn retirement_pending(self) -> bool { self.denial_revision != self.acknowledged_revision }
    fn admitted(self) -> bool { self.network_allowed() && !self.retirement_pending() }
    fn admits_revision(self, revision: u64) -> bool { self.admitted() && self.denial_revision == revision }
    fn local_retirement_pending(self) -> bool { self.local_denial_revision != self.local_acknowledged_revision }
    fn local_admitted(self) -> bool { self.native.local_allowed() && !self.local_retirement_pending() }
    fn admits_local_revision(self, revision: u64) -> bool {
        self.local_admitted() && self.local_denial_revision == revision
    }
    /// Apple Bonjour/Network.framework can establish AWDL before a default-path
    /// monitor sees any LAN. This admits an attempt, not a route: the native
    /// transport prohibits cellular and validates its actual connected path.
    /// Quinn sockets retain the separate physical-LAN revision below.
    #[cfg(mobile)]
    fn sharing_admitted(self) -> bool {
        #[cfg(target_os = "ios")]
        { self.native.known }
        #[cfg(not(target_os = "ios"))]
        { self.local_admitted() }
    }
    fn deny(&mut self) {
        self.denial_revision += 1;
        NETWORK_CHANGED.notify_waiters();
    }
    fn deny_local(&mut self) {
        self.local_denial_revision += 1;
        NETWORK_CHANGED.notify_waiters();
    }
}

pub(crate) struct NetworkPolicyState {
    state: Mutex<PolicySnapshot>,
}
impl NetworkPolicyState {
    const fn new() -> Self {
        Self { state: Mutex::new(PolicySnapshot {
            native: NativeNetworkState {
                known: false, connected: false, metered: true, cellular: false,
                local_available: false, generation: 0,
            },
            allow_metered: false, denial_revision: 0, acknowledged_revision: 0,
            local_denial_revision: 0, local_acknowledged_revision: 0, resume_revision: 0,
        }) }
    }

    fn publish(&self, payload: Value) -> Result<NativeNetworkState, String> {
        let next: NativeNetworkState = serde_json::from_value(payload)
            .map_err(|e| format!("Invalid native network policy: {e}"))?;
        let mut state = self.state.lock().map_err(|_| "Native network policy unavailable")?;
        if next.generation >= state.native.generation {
            let was_allowed = state.network_allowed();
            let was_local = state.native.local_allowed();
            let changed = next.generation > state.native.generation;
            state.native = next;
            // Duplicate publications of the same native generation are not a
            // new connectivity change and must not re-arm automatic resume.
            if changed { state.resume_revision += 1; }
            if was_allowed && !state.network_allowed() { state.deny(); }
            if was_local && !state.native.local_allowed() { state.deny_local(); }
        }
        Ok(state.native)
    }

    fn policy_snapshot(&self) -> Result<PolicySnapshot, String> {
        self.state.lock().map(|state| *state).map_err(|_| "Native network policy unavailable".into())
    }
    fn snapshot(&self) -> Result<NativeNetworkState, String> {
        self.policy_snapshot().map(|state| state.native)
    }

    fn set_allow_metered(&self, allowed: bool) {
        if let Ok(mut state) = self.state.lock() {
            let changed = state.allow_metered != allowed;
            state.allow_metered = allowed;
            // A settings re-admission (for example enabling metered downloads)
            // is a distinct attempt even though the native generation is stable.
            if changed { state.resume_revision += 1; }
            // OS-owned transfers retain their start-time cellular allowance.
            // A narrower setting must retire those leases even on current
            // Wi-Fi, before they can outlive Rust and move onto cellular.
            if changed && !allowed { state.deny(); }
        }
        // A poisoned lock is already fail-closed in every admission/snapshot path.
    }

    fn invalidate(&self) {
        if let Ok(mut state) = self.state.lock() {
            let was_local = state.native.local_allowed();
            state.native.known = false;
            state.native.local_available = false;
            state.resume_revision += 1;
            state.deny();
            if was_local { state.deny_local(); }
        }
    }

    /// Acknowledge only the observed frontier under the publication mutex. A newer
    /// denial remains pending even if it arrives as an older retirement completes.
    fn acknowledge(&self, revision: u64) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "Native network policy unavailable")?;
        state.acknowledged_revision = state.acknowledged_revision.max(revision.min(state.denial_revision));
        Ok(())
    }

    fn acknowledge_local(&self, revision: u64) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "Native network policy unavailable")?;
        state.local_acknowledged_revision = state.local_acknowledged_revision
            .max(revision.min(state.local_denial_revision));
        Ok(())
    }
}
// Shared policy module is compiled on desktop for its pure-state regression tests;
// these retained runtime fences are invoked only by native mobile admission.
#[cfg_attr(not(mobile), allow(dead_code))]
static NETWORK: NetworkPolicyState = NetworkPolicyState::new();
static NETWORK_CHANGED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Admitted policy revision published by reconciliation for one pending
/// automatic resume. Zero means no attempt is pending.
#[cfg(mobile)]
static READMISSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Revision of the last automatic resume attempt, so one connectivity change
/// cannot start transfers twice.
#[cfg(mobile)]
static RESUME_ATTEMPTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Map a policy snapshot onto the manager-level reconnect decision. Kept pure
/// so the metered/cellular distinction is testable without native callbacks.
fn internet_admission_of(policy: PolicySnapshot) -> crate::transfer_policy::InternetAdmission {
    use crate::transfer_policy::InternetAdmission;
    if !policy.native.known { InternetAdmission::Unknown }
    else if !policy.native.connected { InternetAdmission::Offline }
    else if !policy.network_allowed() { InternetAdmission::MeteredDenied }
    else if policy.retirement_pending() { InternetAdmission::Retiring }
    else { InternetAdmission::Admitted }
}

/// Current native admission for the automatic reconnect resume.
#[cfg(mobile)]
pub(crate) fn internet_admission() -> crate::transfer_policy::InternetAdmission {
    NETWORK.policy_snapshot().map(internet_admission_of)
        .unwrap_or(crate::transfer_policy::InternetAdmission::Unknown)
}

/// Publish the admitted frontier after retirement completes. A newer revision
/// supersedes an unconsumed older one; a duplicate reconcile is a no-op.
#[cfg(mobile)]
fn publish_readmission(revision: u64) {
    let attempted = RESUME_ATTEMPTED.load(std::sync::atomic::Ordering::SeqCst);
    if revision > attempted {
        READMISSION.store(revision, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Take the pending automatic-resume frontier exactly once. Callers that skip
/// the attempt leave the revision unrecorded so the next reconcile can re-arm it.
#[cfg(mobile)]
pub(crate) fn take_readmission() -> Option<u64> {
    let revision = READMISSION.swap(0, std::sync::atomic::Ordering::SeqCst);
    (revision != 0).then_some(revision)
}

#[cfg(mobile)]
pub(crate) fn last_resume_attempt() -> u64 {
    RESUME_ATTEMPTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Record the one attempt allowed for `revision` before any work starts, so a
/// burst of native events cannot resume the same connectivity change twice.
#[cfg(mobile)]
pub(crate) fn record_resume_attempt(revision: u64) {
    RESUME_ATTEMPTED.fetch_max(revision, std::sync::atomic::Ordering::SeqCst);
}

/// A retained map checkpoint is not a foreground-service lease. Admit only
/// phases with an owned worker; paused/terminal and unknown phases stay idle.
#[cfg(any(target_os = "android", test))]
pub(super) fn map_transfer_running(status: &str) -> bool {
    matches!(status, "queued" | "downloading" | "composing" | "validating")
}

/// Navigation-only Android entries never carry archive, query or server-start
/// authority. Native and shell intake must agree on these empty payloads.
#[cfg(any(target_os = "android", test))]
pub(super) fn validate_native_entry(kind: &str, value: &str) -> Result<(), String> {
    if !matches!(kind, "library" | "search" | "new-tab" | "get-content" | "download-action" | "article" | "library-notice" | "sharing" | "voice-search" | "bookmarks") {
        return Err("Unsupported native app entry kind".into());
    }
    if value.chars().count() > 4096 { return Err("Native app entry value is too large".into()); }
    if matches!(kind, "sharing" | "voice-search" | "bookmarks") && !value.is_empty() {
        return Err("This native app entry does not accept a value".into());
    }
    Ok(())
}

/// Reader flush can legitimately outlive a UI frame: a hydrated session may
/// contain several archive frames, and each storage drain has its own bounded
/// acknowledgement. Keep the write window inside the native checkpoint grace
/// while giving that work enough time to finish. A foreground return still
/// supersedes the request immediately, and the native checkpoint remains the
/// hard deadline on iOS.
pub(crate) const BACKGROUND_CHECKPOINT_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);

/// Only the trusted root's finite durability drain may use a background
/// checkpoint. The ID is immutable: an expired drain never becomes an
/// ordinary foreground write after a later Activity resumes.
pub(super) fn checkpoint_write_command(command: &str) -> bool {
    matches!(command, "save_session" | "save_settings" | "set_speech_preference"
        | "set_book_zoom" | "set_book_pinned" | "reader_storage_write" | "record_visit" | "sensor_revoke" | "sensor_reset")
}

/// Commands whose first effect is a local-network socket: mDNS discovery,
/// QUIC browse/inspect/seed/download, the LAN HTTP share, or private reading
/// sync's 0.0.0.0 listener and LAN peer connections. Android 17 gates each of
/// them behind ACCESS_LOCAL_NETWORK. Bluetooth-only `nearby_*` commands and the
/// hotspot start are excluded: their own NEARBY_DEVICES request already grants
/// the shared group.
#[cfg(any(target_os = "android", test))]
pub(crate) fn local_network_command(command: &str, args: &Value) -> bool {
    match command {
        "localswarm_discovery" => args.get("enabled").and_then(Value::as_bool) == Some(true),
        "resume_download" => args.get("id").and_then(Value::as_str).is_some_and(|id| id.starts_with("localswarm:")),
        // A loopback-only share never leaves the device and needs no LAN grant.
        "sharing_start" => match args.get("binding") {
            None => true,
            Some(binding) => match binding.get("mode").and_then(Value::as_str) {
                Some("loopback") => false,
                Some("interface") => binding.get("address").and_then(Value::as_str)
                    .and_then(|address| address.parse::<std::net::IpAddr>().ok())
                    .is_none_or(|address| !address.is_loopback()),
                _ => true,
            },
        },
        // Disabling private sync only retires its listener.
        "private_sync_configure" => args.get("enabled").and_then(Value::as_bool) == Some(true),
        "private_sync_pair_start" | "private_sync_pair_join" | "private_sync_now"
        | "private_sync_peer_update" | "private_sync_status" | "private_sync_pair_confirm" => true,
        "localswarm_inspect" | "localswarm_download" | "localswarm_seed" | "localswarm_share_all"
        | "shared_library_browse" | "shared_library_download" => true,
        _ => false,
    }
}

pub(super) fn checkpoint_open(
    request_generation: u64, generation: u64, acknowledged: bool,
    deadline: tokio::time::Instant, now: tokio::time::Instant,
) -> bool {
    request_generation == generation && !acknowledged && now < deadline
}

pub(super) fn checkpoint_write_admitted(
    command: &str, requested: Option<&str>, current: Option<(&str, bool)>, desired_active: bool,
) -> bool {
    !desired_active && checkpoint_write_command(command)
        && current.is_some_and(|(id, open)| open && requested.is_none_or(|requested| id == requested))
}

#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) const NETWORK_BLOCKED: &str = "MOBILE_NETWORK_BLOCKED";

/// Shared fail-closed fence used by every manager, including recovered/background work.
#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) fn require_internet() -> Result<(), String> {
    admission_revision().map(|_| ())
}

/// Capture before registering/spawning owned work; never rebase it to a newer denial.
#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) fn admission_revision() -> Result<u64, String> {
    let policy = NETWORK.policy_snapshot()?;
    if policy.admitted() { Ok(policy.denial_revision) } else { Err(NETWORK_BLOCKED.into()) }
}

/// Observe a baseline even while denied, for sessions that perform their own admitted
/// start check before transport creation and retain this baseline across all phases.
#[cfg(mobile)]
pub(crate) fn internet_revision() -> Result<u64, String> {
    NETWORK.policy_snapshot().map(|policy| policy.denial_revision)
}

#[cfg(mobile)]
pub(crate) fn require_internet_since(revision: u64) -> Result<(), String> {
    let policy = NETWORK.policy_snapshot()?;
    if policy.admits_revision(revision) { Ok(()) }
    else { Err(NETWORK_BLOCKED.into()) }
}

/// Independent physical-LAN baseline: an Internet denial never revokes this grant.
#[cfg(mobile)]
pub(crate) fn local_revision() -> Result<u64, String> {
    NETWORK.policy_snapshot().map(|policy| policy.local_denial_revision)
}

#[cfg(mobile)]
pub(crate) fn require_local_since(revision: u64) -> Result<(), String> {
    if NETWORK.policy_snapshot()?.admits_local_revision(revision) { Ok(()) }
    else { Err("MOBILE_LOCAL_NETWORK_BLOCKED".into()) }
}

#[cfg(mobile)]
pub(crate) async fn local_denied_since(revision: u64) {
    loop {
        let changed = NETWORK_CHANGED.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if require_local_since(revision).is_err() { return; }
        changed.await;
    }
}

/// Called synchronously with settings commit, before releasing its transfer/privacy gate.
#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) fn set_allow_metered(allowed: bool) { NETWORK.set_allow_metered(allowed); }

/// Drops in-flight request/stream/engine futures on policy loss, even if the network
/// becomes allowed again before that task next runs.
#[cfg(mobile)]
pub(crate) async fn internet_denied() {
    let Ok(revision) = admission_revision() else { return; };
    internet_denied_since(revision).await;
}

#[cfg(mobile)]
pub(crate) async fn internet_denied_since(revision: u64) {
    loop {
        let changed = NETWORK_CHANGED.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if require_internet_since(revision).is_err() { return; }
        changed.await;
    }
}

#[cfg(mobile)]
impl Service {
    /// Fresh native admission is REQUIRED before every Internet start/resume, including
    /// recovered jobs, torrent metadata/trackers/seeding and StreetZim recipe/range fetches.
    /// Caller holds transfer_gate across this check, final generation check and launch.
    /// LocalSwarm's transport must enforce locality itself. On Apple, Bonjour
    /// discovery can create a peer-to-peer route even without a Wi-Fi access point.
    pub(crate) async fn mobile_network_admit(&self, internet: bool) -> Result<u64, String> {
        let payload = match self.mobile_call("networkState", json!({})).await {
            Ok(payload) => payload,
            Err(error) => { NETWORK.invalidate(); return Err(error); }
        };
        let state = match NETWORK.publish(payload) {
            Ok(state) => state,
            Err(error) => { NETWORK.invalidate(); return Err(error); }
        };
        set_allow_metered(self.store.settings()?.allow_metered_downloads);
        self.mobile_network_check(internet)?;
        Ok(state.generation)
    }

    /// Synchronous worker/resumption fence: native event handlers publish immediately,
    /// before scheduling durable retirement. Never admits unknown/offline state.
    pub(crate) fn mobile_network_check(&self, internet: bool) -> Result<(), String> {
        let policy = NETWORK.policy_snapshot()?;
        let allowed = if internet { policy.admitted() } else { policy.sharing_admitted() };
        if !allowed {
            return Err(if !internet && !policy.native.local_available {
                "Turn on Wi-Fi and connect both devices to the same network, or create a hotspot. Internet access is not required."
            } else if !policy.native.known {
                "Waiting for network status. Try again when the connection is ready."
            } else if internet && !policy.native.connected {
                "No Internet connection is available. Connect to a network, then try the download again."
            } else if internet && !policy.network_allowed() {
                "Cellular and metered downloads are disabled. Enable them in Settings, or connect to an unmetered Wi-Fi network, then try the download again."
            } else if !internet && !policy.sharing_admitted() {
                "No local connection is available for nearby sharing."
            } else {
                "The previous network connection is still being closed. Try again after it finishes."
            }.into());
        }
        Ok(())
    }

    /// Reject stale launch plans even when a network changed away and back while awaiting
    /// FGS/checkpoint/metadata admission. Re-run native admission rather than reuse a lease.
    pub(crate) fn mobile_network_validate(&self, generation: u64, internet: bool) -> Result<(), String> {
        if NETWORK.snapshot()?.generation != generation {
            return Err("Network changed during transfer admission; start or resume again".into());
        }
        self.mobile_network_check(internet)
    }

    /// Publish events synchronously at ingress; then spawn mobile_network_reconcile.
    pub(crate) fn mobile_network_event(&self, payload: Value) -> Result<(), String> {
        match NETWORK.publish(payload) {
            Ok(_) => Ok(()),
            Err(error) => { NETWORK.invalidate(); Err(error) }
        }
    }

    /// Durable network-change/settings-transition retirement. HTTP/torrents preserve the
    /// existing interrupted bit (not user-pause); maps cancel using their owned-partial
    /// journal and require explicit restart. When retirement completes and the policy
    /// admits Internet again, this arms exactly one automatic resume for that
    /// connectivity change; the resume executor still applies settings and lifecycle gates.
    pub(crate) async fn mobile_network_reconcile(&self) -> Result<(), String> {
        let _serial = self.mobile.serial.lock().await;
        let _transfer = self.mobile.transfer_gate.lock().await;
        let _mode = self.privacy_gate.read().await;
        self.mobile_network_reconcile_locked().await
    }

    /// Settings commits already own transfer_gate and the privacy writer. Reuse the
    /// retirement body without acquiring serial or reentering either admission gate.
    /// Other callers must use mobile_network_reconcile's serial -> transfer -> privacy order.
    pub(crate) async fn mobile_network_reconcile_locked(&self) -> Result<(), String> {
        let settings = self.store.settings()?;
        set_allow_metered(settings.allow_metered_downloads);
        let policy = NETWORK.policy_snapshot()?;
        let state = policy.native;
        let retirement_revision = policy.denial_revision;
        let local_retirement_revision = policy.local_denial_revision;
        let mut internet_errors = Vec::new();
        let mut local_errors = Vec::new();
        if !policy.network_allowed() || policy.retirement_pending() {
            if let Some(manager) = self.downloads.get() {
                if let Err(error) = manager.interrupt_internet().await { internet_errors.push(error); }
            }
            let torrents = self.torrents.lock().map_err(|_| "Torrent manager unavailable")?.clone();
            if let Some(manager) = torrents {
                if let Err(error) = manager.suspend().await { internet_errors.push(error); }
            }
            let maps = self.maps.lock().map_err(|_| "Map manager unavailable")?.clone();
            if let Some(manager) = maps {
                if let Err(error) = manager.cancel().await { internet_errors.push(error); }
            }
            if let Err(error) = crate::maps::retire_catalogs().await { internet_errors.push(error); }
        }
        // QUIC and BLE owners enforce their retained LAN, native or joined/AP
        // route. Default-network changes must not cancel unrelated route owners.
        // Only Web listeners bound to the retired client LAN are closed here.
        if !state.local_allowed() || policy.local_retirement_pending() {
            let mut sharing = self.sharing.lock().await;
            if sharing.as_ref().is_some_and(|server| !server.loopback_only() && !server.uses_hotspot()) {
                if let Some(server) = sharing.as_mut() {
                    if let Err(error) = server.shutdown().await { local_errors.push(error); }
                }
                *sharing = None;
            }
        }
        // Each domain owns its own revision: one domain's failure must never
        // keep the other domain's completed retirement pending.
        let mut errors = internet_errors;
        if errors.is_empty() {
            if let Err(error) = NETWORK.acknowledge(retirement_revision) { errors.push(error); }
            // Retirement is complete and the current policy admits Internet:
            // arm exactly one automatic resume for this connectivity change.
            // The resume executor (mobile_resume_after_network) applies the
            // remaining settings/lifecycle gates before starting any transfer.
            match NETWORK.policy_snapshot() {
                Ok(admitted) => if admitted.admitted() { publish_readmission(admitted.resume_revision); },
                Err(error) => errors.push(error),
            }
        }
        if local_errors.is_empty() {
            if let Err(error) = NETWORK.acknowledge_local(local_retirement_revision) { errors.push(error); }
        }
        errors.extend(local_errors);
        #[cfg(target_os = "android")]
        if let Err(error) = self.mobile_transfer_update_locked().await { errors.push(error); }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }

    /// Run blocking filesystem work under a normal-writable/privacy read lease;
    /// publication may proceed concurrently. Native conditional exclusion
    /// serializes its admission recheck and mutation with publisher marker writes,
    /// so a stale negative scan cannot undo a newer publication. Admission uses
    /// retained Store/publication evidence across the profile, never name heuristics.
    pub(crate) fn mobile_apply_archive_backup(&self) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        crate::library_files::reconcile_archive_backup(&self.store)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{internet_admission_of, NativeNetworkState, NetworkPolicyState};
    use crate::transfer_policy::InternetAdmission;
    use serde_json::json;

    #[test]
    fn local_network_commands_are_admitted_before_lan_access() {
        assert!(super::local_network_command("localswarm_discovery", &json!({"enabled": true})));
        assert!(!super::local_network_command("localswarm_discovery", &json!({"enabled": false})));
        for command in [
            "localswarm_inspect", "localswarm_download", "localswarm_seed", "localswarm_share_all",
            "shared_library_browse", "shared_library_download",
        ] {
            assert!(super::local_network_command(command, &json!({})), "{command}");
        }
        assert!(super::local_network_command("resume_download", &json!({"id": "localswarm:job"})));
        assert!(!super::local_network_command("resume_download", &json!({"id": "torrent:job"})));
        assert!(super::local_network_command("sharing_start", &json!({})));
        assert!(super::local_network_command("sharing_start", &json!({"binding": {"mode": "ipv4All"}})));
        assert!(!super::local_network_command("sharing_start", &json!({"binding": {"mode": "loopback"}})));
        for address in ["127.0.0.1", "127.0.0.2", "::1"] {
            assert!(!super::local_network_command("sharing_start",
                &json!({"binding": {"mode": "interface", "address": address}})), "{address}");
        }
        for address in ["192.168.1.5", "10.0.0.2", "2001:db8::1"] {
            assert!(super::local_network_command("sharing_start",
                &json!({"binding": {"mode": "interface", "address": address}})), "{address}");
        }
        assert!(super::local_network_command("sharing_start",
            &json!({"binding": {"mode": "interface", "address": "localhost"}})));
        // Private sync binds 0.0.0.0 and connects to LAN peers.
        for command in [
            "private_sync_pair_start", "private_sync_pair_join", "private_sync_now", "private_sync_peer_update",
            "private_sync_status", "private_sync_pair_confirm",
        ] {
            assert!(super::local_network_command(command, &json!({})), "{command}");
        }
        assert!(super::local_network_command("private_sync_configure", &json!({"enabled": true})));
        assert!(!super::local_network_command("private_sync_configure", &json!({"enabled": false})));
        // Bluetooth commands request their own NEARBY_DEVICES grant.
        for command in [
            "localswarm_status", "localswarm_stop", "localswarm_stop_library", "localswarm_stop_seed",
            "sharing_stop", "nearby_discover", "nearby_connect", "start_download",
            "private_sync_pair_cancel", "private_sync_unpair", "private_sync_resolve", "private_sync_resume",
            "private_sync_rebootstrap", "private_sync_clear_positions",
        ] {
            assert!(!super::local_network_command(command, &json!({"enabled": true})), "{command}");
        }
    }

    #[test]
    fn checkpoint_durability_authority_cannot_outlive_or_cross_its_request() {
        let current = Some(("checkpoint-one", true));
        for command in ["save_session", "save_settings", "set_speech_preference", "set_book_zoom", "set_book_pinned", "reader_storage_write", "record_visit", "sensor_revoke", "sensor_reset"] {
            assert!(super::checkpoint_write_admitted(command, Some("checkpoint-one"), current, false));
            // A command dispatched before the synchronous background handoff
            // can drain under the live issued checkpoint without retagging.
            assert!(super::checkpoint_write_admitted(command, None, current, false));
            assert!(!super::checkpoint_write_admitted(command, Some("checkpoint-two"), current, false));
            assert!(!super::checkpoint_write_admitted(command, Some("checkpoint-one"), None, false));
            assert!(!super::checkpoint_write_admitted(command, None, None, false));
            assert!(!super::checkpoint_write_admitted(command, Some("checkpoint-one"), Some(("checkpoint-one", false)), false));
            assert!(!super::checkpoint_write_admitted(command, Some("checkpoint-one"), current, true));
        }
        for command in ["start_download", "open_article", "mobile_pin_shortcut", "import_archives"] {
            assert!(!super::checkpoint_write_admitted(command, Some("checkpoint-one"), current, false));
        }
    }

    #[test]
    fn background_write_window_covers_a_reader_storage_drain() {
        // Reader documents acknowledge each storage drain within five seconds.
        // A background transition that closes writes before that boundary can
        // surface a durability error even when the drain was making progress.
        let now = tokio::time::Instant::now();
        assert!(super::checkpoint_open(
            7,
            7,
            false,
            now + super::BACKGROUND_CHECKPOINT_WINDOW,
            now + std::time::Duration::from_secs(5),
        ));
    }

    #[test]
    fn checkpoint_deadline_closes_admission_without_extension_or_scene_reuse() {
        let deadline = tokio::time::Instant::now();
        let before = deadline - std::time::Duration::from_nanos(1);
        assert!(super::checkpoint_open(7, 7, false, deadline, before));
        assert!(!super::checkpoint_open(7, 7, false, deadline, deadline));
        assert!(!super::checkpoint_open(7, 7, false, deadline, deadline + std::time::Duration::from_nanos(1)));
        assert!(!super::checkpoint_open(7, 8, false, deadline, before));
        assert!(!super::checkpoint_open(7, 7, true, deadline, before));
    }

    #[test]
    fn navigation_only_native_entries_carry_no_payload_authority() {
        for kind in ["sharing", "voice-search", "bookmarks"] {
            super::validate_native_entry(kind, "").unwrap();
            for payload in ["book-id\nA/Article", "https://example.org/", "query", "\0"] {
                assert!(super::validate_native_entry(kind, payload).is_err(), "{kind}");
            }
        }
        super::validate_native_entry("search", "ordinary query").unwrap();
        super::validate_native_entry("article", "book-id\nA/Article").unwrap();
        assert!(super::validate_native_entry("unknown", "").is_err());
        assert!(super::validate_native_entry("search", &"q".repeat(4097)).is_err());
    }

    #[test]
    fn restricting_metered_policy_retires_existing_leases_even_on_wifi() {
        let policy = NetworkPolicyState::new();
        policy.set_allow_metered(true);
        publish(&policy, 1, false);
        let admitted = policy.policy_snapshot().unwrap();
        assert!(admitted.admitted());
        policy.set_allow_metered(false);
        let restricted = policy.policy_snapshot().unwrap();
        assert!(restricted.network_allowed());
        assert!(restricted.retirement_pending());
        assert!(!restricted.admitted());
        assert!(!restricted.admits_revision(admitted.denial_revision));
        // Re-reading the same settings during admission must not revoke the
        // replacement lease repeatedly.
        policy.set_allow_metered(false);
        assert_eq!(policy.policy_snapshot().unwrap().denial_revision, restricted.denial_revision);
        policy.acknowledge(restricted.denial_revision).unwrap();
        let retired = policy.policy_snapshot().unwrap();
        assert!(retired.admitted());
        assert!(retired.admits_revision(restricted.denial_revision));
        assert!(!retired.admits_revision(admitted.denial_revision));
    }

    #[test]
    fn paused_map_checkpoint_does_not_retain_foreground_execution() {
        for running in ["queued", "downloading", "composing", "validating"] {
            assert!(super::map_transfer_running(running), "{running}");
        }
        for idle in ["paused", "complete", "cancelled", "failed", "", "unknown"] {
            assert!(!super::map_transfer_running(idle), "{idle}");
        }
    }

    #[cfg(target_os = "ios")]
    #[tokio::test]
    async fn reconnect_waiting_on_transfer_gate_rechecks_policy_before_native_admission() {
        use crate::service::Service;
        struct Profile(std::path::PathBuf);
        impl Drop for Profile {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        for cutover in ["private", "read-only", "manual-resume", "background"] {
            let profile = Profile(std::env::temp_dir()
                .join(format!("kiwix-reconnect-{}", uuid::Uuid::new_v4())));
            let service = Service::new(profile.0.clone(), false, false).unwrap();
            service.store.update(|state| {
                state.settings.auto_resume_downloads = true;
                Ok(())
            }).unwrap();
            {
                let mut state = service.mobile.state.lock().unwrap();
                state.active = true;
                state.desired_active = true;
                state.inactive = false;
            }
            let generation = super::NETWORK.snapshot().unwrap().generation + 1;
            service.mobile_network_event(json!({
                "known": true, "connected": true, "metered": false, "cellular": false,
                "localAvailable": true, "generation": generation,
            })).unwrap();
            let policy = super::NETWORK.policy_snapshot().unwrap();
            super::NETWORK.acknowledge(policy.denial_revision).unwrap();
            super::publish_readmission(policy.resume_revision);
            let last_attempt = super::last_resume_attempt();
            let transfer = service.mobile.transfer_gate.lock().await;
            let privacy = service.privacy_gate.write().await;
            let mut resume = std::pin::pin!(service.mobile_resume_after_network());
            // Poll the actual executor through its admission decision and into
            // the held gate, without timing or a forwarding/native test double.
            tokio::select! {
                biased;
                result = &mut resume => panic!("reconnect did not wait for the gate: {result:?}"),
                _ = async {} => {}
            }
            if cutover == "background" {
                service.mobile.state.lock().unwrap().desired_active = false;
            } else {
                service.store.update(|state| {
                    match cutover {
                        "private" => state.settings.private_mode = true,
                        "read-only" => state.settings.read_only = true,
                        "manual-resume" => state.settings.auto_resume_downloads = false,
                        _ => unreachable!(),
                    }
                    Ok(())
                }).unwrap();
            }
            drop(privacy);
            drop(transfer);
            // There is intentionally no native adapter: reaching networkState
            // would fail, so success proves denial before native/manager work.
            resume.await.unwrap();
            assert_eq!(super::last_resume_attempt(), last_attempt, "{cutover}");
        }
    }

    #[test]
    fn reconnect_admission_distinguishes_offline_metered_and_retiring() {
        let policy = NetworkPolicyState::new();
        assert_eq!(
            internet_admission_of(policy.policy_snapshot().unwrap()),
            InternetAdmission::Unknown
        );
        policy.publish(json!({
            "known": true, "connected": false, "metered": false, "cellular": false,
            "localAvailable": false, "generation": 1,
        })).unwrap();
        assert_eq!(
            internet_admission_of(policy.policy_snapshot().unwrap()),
            InternetAdmission::Offline
        );
        policy.publish(json!({
            "known": true, "connected": true, "metered": false, "cellular": false,
            "localAvailable": false, "generation": 2,
        })).unwrap();
        assert_eq!(
            internet_admission_of(policy.policy_snapshot().unwrap()),
            InternetAdmission::Admitted
        );
        policy.publish(json!({
            "known": true, "connected": true, "metered": true, "cellular": true,
            "localAvailable": false, "generation": 3,
        })).unwrap();
        // Connected but metered without the user's allowance is a policy denial,
        // not an offline state; the resume decision must name that reason.
        assert_eq!(
            internet_admission_of(policy.policy_snapshot().unwrap()),
            InternetAdmission::MeteredDenied
        );
        policy.set_allow_metered(true);
        assert_eq!(
            internet_admission_of(policy.policy_snapshot().unwrap()),
            InternetAdmission::Retiring
        );
        let pending = policy.policy_snapshot().unwrap();
        policy.acknowledge(pending.denial_revision).unwrap();
        assert_eq!(
            internet_admission_of(policy.policy_snapshot().unwrap()),
            InternetAdmission::Admitted
        );
    }

    #[test]
    fn resume_revision_changes_once_per_connectivity_or_settings_transition() {
        let policy = NetworkPolicyState::new();
        policy.publish(json!({
            "known": true, "connected": true, "metered": false, "cellular": false,
            "localAvailable": false, "generation": 1,
        })).unwrap();
        let first = policy.policy_snapshot().unwrap().resume_revision;
        assert!(first > 0);
        // A duplicate publication of the same native generation is not a new
        // connectivity change and must not re-arm automatic resume.
        policy.publish(json!({
            "known": true, "connected": true, "metered": false, "cellular": false,
            "localAvailable": false, "generation": 1,
        })).unwrap();
        assert_eq!(policy.policy_snapshot().unwrap().resume_revision, first);
        policy.publish(json!({
            "known": true, "connected": true, "metered": true, "cellular": true,
            "localAvailable": false, "generation": 2,
        })).unwrap();
        let denied = policy.policy_snapshot().unwrap().resume_revision;
        assert!(denied > first);
        // Enabling metered downloads is a settings re-admission with a stable
        // native generation; it must still produce a fresh attempt revision.
        policy.set_allow_metered(true);
        assert!(policy.policy_snapshot().unwrap().resume_revision > denied);
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn peer_to_peer_attempt_does_not_authorize_cellular_or_unbound_quinn() {
        let policy = NetworkPolicyState::new();
        policy.publish(json!({
            "known": true, "connected": true, "metered": true, "cellular": true,
            "localAvailable": false, "generation": 1,
        })).unwrap();
        let cellular = policy.policy_snapshot().unwrap();
        assert!(cellular.sharing_admitted());
        assert!(!cellular.admitted());
        assert!(!cellular.local_admitted());
        policy.invalidate();
        assert!(!policy.policy_snapshot().unwrap().sharing_admitted());
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn infrastructure_loss_does_not_revoke_native_peer_to_peer_attempts() {
        let policy = NetworkPolicyState::new();
        for (generation, local) in [(1, true), (2, false)] {
            policy.publish(json!({
                "known": true, "connected": true, "metered": true, "cellular": true,
                "localAvailable": local, "generation": generation,
            })).unwrap();
        }
        let lost = policy.policy_snapshot().unwrap();
        assert!(lost.local_retirement_pending());
        assert!(lost.sharing_admitted());
        assert!(!lost.local_admitted());
        assert!(!lost.admitted());
    }

    #[test]
    fn metered_lan_loss_stays_pending_after_return_without_internet_transition() {
        let policy = NetworkPolicyState::new();
        let update = |generation, local| {
            policy.publish(json!({
                "known": true, "connected": true, "metered": true, "cellular": false,
                "localAvailable": local, "generation": generation,
            })).unwrap();
        };
        update(1, true);
        assert!(policy.policy_snapshot().unwrap().local_admitted());
        update(2, false);
        update(3, true);
        let returned = policy.policy_snapshot().unwrap();
        assert!(!returned.network_allowed());
        assert_eq!(returned.denial_revision, 0);
        assert_eq!(returned.local_denial_revision, 1);
        assert!(returned.local_retirement_pending());
        assert!(!returned.local_admitted());
        policy.acknowledge(returned.denial_revision).unwrap();
        assert!(!policy.policy_snapshot().unwrap().local_admitted());
        policy.acknowledge_local(returned.local_denial_revision).unwrap();
        // Joined local retirement restores LAN admission without authorizing Internet.
        let retired = policy.policy_snapshot().unwrap();
        assert!(retired.local_admitted());
        assert!(!retired.admits_local_revision(0));
        assert!(retired.admits_local_revision(returned.local_denial_revision));
        assert!(!retired.admitted());
    }

    #[test]
    fn old_local_retirement_cannot_acknowledge_a_newer_lan_loss() {
        let policy = NetworkPolicyState::new();
        let update = |generation, local| {
            policy.publish(json!({
                "known": true, "connected": true, "metered": true, "cellular": false,
                "localAvailable": local, "generation": generation,
            })).unwrap();
        };
        update(1, true);
        update(2, false);
        let first = policy.policy_snapshot().unwrap().local_denial_revision;
        update(3, true);
        update(4, false);
        update(5, true);
        policy.acknowledge_local(first).unwrap();
        let latest = policy.policy_snapshot().unwrap();
        assert!(latest.local_retirement_pending());
        assert!(!latest.local_admitted());
        policy.acknowledge_local(latest.local_denial_revision).unwrap();
        assert!(policy.policy_snapshot().unwrap().local_admitted());
    }

    #[test]
    fn work_admitted_before_spawn_cannot_rebase_after_retired_denial() {
        let policy = NetworkPolicyState::new();
        publish(&policy, 1, false);
        let captured = policy.policy_snapshot().unwrap().denial_revision;
        publish(&policy, 2, true);
        publish(&policy, 3, false);
        let retired = policy.policy_snapshot().unwrap().denial_revision;
        policy.acknowledge(retired).unwrap();
        let latest = policy.policy_snapshot().unwrap();
        assert!(latest.admitted());
        assert!(!latest.admits_revision(captured));
        assert!(latest.admits_revision(retired));
    }

    fn publish(policy: &NetworkPolicyState, generation: u64, metered: bool) {
        policy.publish(json!({
            "known": true, "connected": true, "metered": metered, "cellular": false,
            "localAvailable": true, "generation": generation,
        })).unwrap();
    }

    #[test]
    fn late_retirement_acknowledges_only_its_observed_denial_frontier() {
        let policy = NetworkPolicyState::new();
        publish(&policy, 1, false);
        publish(&policy, 2, true);
        let first = policy.policy_snapshot().unwrap();
        publish(&policy, 3, false);
        publish(&policy, 4, true);
        publish(&policy, 5, false);
        // Simulate an older joined retirement completing after a newer denial
        // and return-to-allowed were already delivered.
        policy.acknowledge(first.denial_revision).unwrap();
        let newer = policy.policy_snapshot().unwrap();
        assert!(newer.network_allowed());
        assert!(newer.retirement_pending());
        assert!(!newer.admitted());
        policy.acknowledge(newer.denial_revision).unwrap();
        assert!(policy.policy_snapshot().unwrap().admitted());
    }

    #[test]
    fn setting_revocation_then_allowed_network_cannot_erase_the_denial() {
        let policy = NetworkPolicyState::new();
        policy.set_allow_metered(true);
        publish(&policy, 1, true);
        assert!(policy.policy_snapshot().unwrap().admitted());
        policy.set_allow_metered(false);
        let revoked = policy.policy_snapshot().unwrap();
        assert_eq!(revoked.denial_revision, 1);
        publish(&policy, 2, false);
        let restored = policy.policy_snapshot().unwrap();
        assert!(restored.network_allowed());
        assert!(!restored.admitted());
        assert_eq!(restored.denial_revision, revoked.denial_revision);
        policy.acknowledge(revoked.denial_revision).unwrap();
        assert!(policy.policy_snapshot().unwrap().admitted());
    }

    #[test]
    fn callbacks_preserve_denial_even_when_reconciler_only_sees_latest_allowed_path() {
        let policy = NetworkPolicyState::new();
        publish(&policy, 1, false);
        publish(&policy, 2, true);
        publish(&policy, 3, false);
        let latest = policy.policy_snapshot().unwrap();
        assert!(latest.network_allowed());
        assert!(latest.native.local_available);
        assert_eq!(latest.denial_revision, 1);
        assert_eq!(latest.acknowledged_revision, 0);
        assert!(!latest.admitted());
    }

    #[test]
    fn local_admission_requires_native_knowledge_not_default_internet_connectivity() {
        let policy = NetworkPolicyState::new();
        policy.publish(json!({
            "known": true, "connected": false, "metered": true, "cellular": false,
            "localAvailable": true, "generation": 1,
        })).unwrap();
        assert!(policy.snapshot().unwrap().local_allowed());
        assert!(!policy.policy_snapshot().unwrap().admitted());
        policy.invalidate();
        assert!(!policy.snapshot().unwrap().local_allowed());
        // An explicitly unknown native snapshot cannot reuse an old local grant.
        policy.publish(json!({
            "known": false, "connected": false, "metered": true, "cellular": false,
            "localAvailable": true, "generation": 2,
        })).unwrap();
        assert!(!policy.snapshot().unwrap().local_allowed());
    }

    #[test]
    fn native_network_admission_is_conservative_and_cellular_is_explicit() {
        let mut state = NativeNetworkState::default();
        assert!(!state.internet_allowed(false));
        assert!(!state.internet_allowed(true));
        state.known = true;
        assert!(!state.internet_allowed(true));
        state.connected = true;
        assert!(state.internet_allowed(false));
        state.metered = true;
        assert!(!state.internet_allowed(false));
        assert!(state.internet_allowed(true));
        state.metered = false;
        state.cellular = true;
        assert!(!state.internet_allowed(false));
        assert!(state.internet_allowed(true));
        state.connected = false;
        assert!(!state.internet_allowed(true));
    }
}
