// SPDX-License-Identifier: GPL-3.0-or-later
//! Android local-only hotspot hosting: platform credentials and the Wi-Fi QR
//! payload the iPhone camera understands, plus the reservation lifecycle that
//! keeps native ownership and Rust state in agreement.
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::IpAddr;

/// Credentials exactly as the platform published them for the active
/// reservation. Never constructed from local guesses or hardcoded values.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotspotInfo {
    pub ssid: String,
    pub passphrase: String,
}

impl HotspotInfo {
    pub fn validate(self) -> Result<Self, String> {
        if self.ssid.is_empty() || self.ssid.len() > 32 {
            return Err("Android returned an invalid hotspot name".into());
        }
        if self.passphrase.len() < 8 || self.passphrase.len() > 63 {
            return Err("Android returned an invalid hotspot password".into());
        }
        if self.ssid.chars().any(char::is_control)
            || self.passphrase.chars().any(char::is_control)
        {
            return Err("Android returned hotspot credentials with control characters".into());
        }
        Ok(self)
    }

    /// `WIFI:T:WPA;S:<ssid>;P:<passphrase>;;` with the escapes the ZXing
    /// format requires. iOS Camera joins through the system prompt; the app
    /// never claims an automatic join.
    pub fn qr_payload(&self) -> String {
        format!(
            "WIFI:T:WPA;S:{};P:{};;",
            escape_wifi(&self.ssid),
            escape_wifi(&self.passphrase)
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "ssid": self.ssid,
            "passphrase": self.passphrase,
            "qr": self.qr_payload(),
        })
    }
}

fn escape_wifi(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | ';' | ',' | ':' | '"') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// The IPv4 address of the access-point link the reservation created.
///
/// A LocalOnlyHotspot owns an interface of its own for the lifetime of the
/// reservation, so the address is read from the live interface table rather
/// than from a vendor constant (`192.168.43.1` is common, not guaranteed).
/// Names are ranked because vendors differ (`ap0`, `softap0`, `swlan0`,
/// `wlan1`; the emulator reuses `wlan0` while its only Wi-Fi link is the AP).
/// An interface that carries the default route is ranked after one that does
/// not, so `wlan1` (the AP) wins over `wlan0` (the client Wi-Fi link).
/// Cellular, tunnel, Wi-Fi Direct and loopback links are never candidates, so
/// a phone with mobile data up cannot advertise its carrier address, and the
/// caller must be holding an active reservation before this means anything.
#[cfg_attr(not(target_os = "android"), expect(dead_code, reason = "Android hotspot status and binding use the live AP interface address"))]
pub fn interface_address() -> Result<IpAddr, String> {
    let default_routes = default_route_interfaces();
    let interfaces = if_addrs::get_if_addrs()
        .map_err(|error| format!("Cannot list local network interfaces: {error}"))?
        .into_iter()
        .map(|interface| { let address = interface.ip(); (interface.name, address) });
    hotspot_address(interfaces, &default_routes).ok_or_else(|| {
        "Android has not published the hotspot interface yet; wait for the hotspot to finish starting and try again".into()
    })
}

/// Pick the interface a joined peer should be told to use. An interface that
/// does not carry the default route wins first, because the AP link is not the
/// phone's Internet path; name ranks then break ties (a named AP interface,
/// then `wlan1` and above, then `wlan0`, which vendors usually reserve for the
/// client Wi-Fi link).
fn hotspot_address(
    candidates: impl IntoIterator<Item = (String, IpAddr)>,
    default_routes: &HashSet<String>,
) -> Option<IpAddr> {
    let mut ranked: Vec<(u8, u8, String, IpAddr)> = candidates
        .into_iter()
        .filter_map(|(name, address)| {
            let rank = hotspot_candidate(&name, address)?;
            let name = name.to_ascii_lowercase();
            let default_route = u8::from(default_routes.contains(&name));
            Some((default_route, rank, name, address))
        })
        .collect();
    ranked.sort_by(|left, right| {
        (left.0, left.1, &left.2, left.3).cmp(&(right.0, right.1, &right.2, right.3))
    });
    ranked.first().map(|(_, _, _, address)| *address)
}

/// Interface names that currently carry an IPv4 default route. The local-only
/// hotspot provides no Internet, so the AP link never appears here while the
/// client Wi-Fi link (the one the hotspot replaced) does.
#[cfg(any(target_os = "android", target_os = "linux"))]
#[cfg_attr(not(target_os = "android"), expect(dead_code, reason = "The AP address selector is called by Android hotspot hosting"))]
fn default_route_interfaces() -> HashSet<String> {
    let Ok(table) = std::fs::read_to_string("/proc/net/route") else {
        return HashSet::new();
    };
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let destination = fields.next()?;
            let _gateway = fields.next()?;
            let flags = u32::from_str_radix(fields.next()?, 16).ok()?;
            // Destination 0.0.0.0 with RTF_UP is the default route.
            (destination == "00000000" && flags & 0x1 != 0)
                .then(|| name.to_ascii_lowercase())
        })
        .collect()
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
#[expect(dead_code, reason = "The non-Android fallback keeps AP address selection portable without exposing a default route")]
fn default_route_interfaces() -> HashSet<String> {
    HashSet::new()
}

/// Lower ranks are more likely to be the access point; `None` excludes the
/// link. Private IPv4 is required because a peer types this address into a
/// browser and the AP hands out addresses from the same private subnet.
fn hotspot_candidate(name: &str, address: IpAddr) -> Option<u8> {
    let IpAddr::V4(address) = address else {
        return None;
    };
    if !address.is_private() {
        return None;
    }
    let name = name.to_ascii_lowercase();
    if [
        "lo", "rmnet", "ccmni", "seth", "p2p", "tun", "ppp", "dummy", "v4-", "ip6tnl", "sit",
        "clat",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
    {
        return None;
    }
    Some(if ["ap", "softap", "swlan"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        0
    } else if name.starts_with("wlan") && name != "wlan0" {
        // Android creates the local-only AP as a new wlan link (usually
        // wlan1) while wlan0 stays the client Wi-Fi link.
        1
    } else if name.starts_with("wlan") {
        2
    } else if name.starts_with("eth") {
        3
    } else {
        4
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotspotPhase {
    #[default]
    Idle,
    Starting,
    Active,
    Stopping,
}

/// Serializes the platform reservation. A generation guards completion of an
/// in-flight start: a Stop that arrives first supersedes it, so a late native
/// success is closed instead of being adopted.
#[derive(Debug, Default)]
pub struct HotspotState {
    phase: HotspotPhase,
    info: Option<HotspotInfo>,
    generation: u64,
}

impl HotspotState {
    pub fn phase(&self) -> HotspotPhase {
        self.phase
    }

    pub fn info(&self) -> Option<&HotspotInfo> {
        self.info.as_ref()
    }

    /// Reserve the right to start. Returns the generation the native result
    /// must present to be adopted.
    pub fn begin_start(&mut self) -> Result<u64, String> {
        match self.phase {
            HotspotPhase::Idle => {
                self.generation = self.generation.wrapping_add(1);
                self.phase = HotspotPhase::Starting;
                Ok(self.generation)
            }
            HotspotPhase::Starting => Err("The Wi-Fi hotspot is already starting".into()),
            HotspotPhase::Active => Err("The Wi-Fi hotspot is already running".into()),
            HotspotPhase::Stopping => Err("The Wi-Fi hotspot is still stopping".into()),
        }
    }

    /// Adopt platform credentials only for the current start. `false` means a
    /// Stop superseded this start; the caller must close the native reservation.
    pub fn accept(&mut self, generation: u64, info: HotspotInfo) -> bool {
        if self.phase != HotspotPhase::Starting || self.generation != generation {
            return false;
        }
        self.info = Some(info);
        self.phase = HotspotPhase::Active;
        true
    }

    /// A start whose credentials cannot be used still owns whatever the
    /// platform created. Returns whether this start needs a native release; a
    /// concurrent Stop that already superseded it owns that release instead.
    /// The phase deliberately stays `Starting` so the ordinary stop path
    /// performs the native stop rather than the caller abandoning it.
    pub fn failed_start(&self, generation: u64) -> bool {
        self.phase == HotspotPhase::Starting && self.generation == generation
    }

    /// Returns the generation and whether the native reservation must be
    /// closed. Idempotent: repeated stops do not invent native work. A pending
    /// start is cancelled; a transient system dialog now reports `inactive`
    /// instead of reaching background retirement.
    pub fn begin_stop(&mut self) -> (u64, bool) {
        match self.phase {
            HotspotPhase::Idle | HotspotPhase::Stopping => (self.generation, false),
            HotspotPhase::Starting | HotspotPhase::Active => {
                self.generation = self.generation.wrapping_add(1);
                self.phase = HotspotPhase::Stopping;
                self.info = None;
                (self.generation, true)
            }
        }
    }

    pub fn stopped(&mut self, generation: u64) {
        if self.phase == HotspotPhase::Stopping && self.generation == generation {
            self.phase = HotspotPhase::Idle;
            self.info = None;
        }
    }

    /// A native Stop failed, so the reservation may still be live. Restore the
    /// credentials the platform published rather than advertising a stopped
    /// hotspot whose network is still up.
    pub fn retry(&mut self, generation: u64, info: HotspotInfo) {
        if self.phase == HotspotPhase::Stopping && self.generation == generation {
            self.info = Some(info);
            self.phase = HotspotPhase::Active;
        }
    }

    /// The platform closed the reservation without an application Stop (Wi-Fi
    /// turned off, tethering policy change, Activity retirement).
    pub fn lost(&mut self) {
        if self.phase == HotspotPhase::Idle && self.info.is_none() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.phase = HotspotPhase::Idle;
        self.info = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> HotspotInfo {
        HotspotInfo {
            ssid: "KiwixShare_1234".into(),
            passphrase: "correct horse battery".into(),
        }
    }

    #[test]
    fn qr_payload_matches_the_camera_format() {
        assert_eq!(
            info().qr_payload(),
            "WIFI:T:WPA;S:KiwixShare_1234;P:correct horse battery;;"
        );
    }

    #[test]
    fn qr_payload_escapes_wifi_metacharacters() {
        let info = HotspotInfo {
            ssid: r#"Ki;wix,"net":"#.into(),
            passphrase: r"pa\ss;word,1".into(),
        };
        assert_eq!(
            info.qr_payload(),
            r#"WIFI:T:WPA;S:Ki\;wix\,\"net\"\:;P:pa\\ss\;word\,1;;"#
        );
    }

    #[test]
    fn rejects_credentials_outside_the_platform_contract() {
        assert!(HotspotInfo { ssid: String::new(), passphrase: "abcdefgh".into() }
            .validate().is_err());
        assert!(HotspotInfo { ssid: "ok".into(), passphrase: "short".into() }
            .validate().is_err());
        assert!(HotspotInfo { ssid: "ok\nname".into(), passphrase: "abcdefgh".into() }
            .validate().is_err());
        assert!(info().validate().is_ok());
    }

    #[test]
    fn start_then_stop_releases_and_clears_credentials() {
        let mut state = HotspotState::default();
        let generation = state.begin_start().unwrap();
        assert_eq!(state.phase(), HotspotPhase::Starting);
        assert!(state.accept(generation, info()));
        assert_eq!(state.phase(), HotspotPhase::Active);
        assert_eq!(state.info(), Some(&info()));
        let (stop_generation, native) = state.begin_stop();
        assert!(native);
        assert_eq!(state.phase(), HotspotPhase::Stopping);
        assert!(state.info().is_none());
        state.stopped(stop_generation);
        assert_eq!(state.phase(), HotspotPhase::Idle);
        // A repeated Stop performs no native work.
        assert_eq!(state.begin_stop(), (stop_generation, false));
    }

    #[test]
    fn stop_during_start_supersedes_the_late_native_success() {
        let mut state = HotspotState::default();
        let start = state.begin_start().unwrap();
        let (stop, native) = state.begin_stop();
        assert!(native);
        assert_ne!(start, stop);
        assert!(!state.accept(start, info()));
        assert_eq!(state.phase(), HotspotPhase::Stopping);
        assert!(state.info().is_none());
        state.stopped(stop);
        assert_eq!(state.phase(), HotspotPhase::Idle);
    }

    #[test]
    fn failed_start_keeps_native_ownership_for_the_stop_path() {
        let mut state = HotspotState::default();
        let generation = state.begin_start().unwrap();
        // Unusable credentials were never adopted, but the platform may have
        // created the reservation. It must not look idle before the release.
        assert!(state.failed_start(generation));
        assert_eq!(state.phase(), HotspotPhase::Starting);
        assert!(state.info().is_none());
        let (stop, native) = state.begin_stop();
        assert!(native);
        state.stopped(stop);
        assert_eq!(state.phase(), HotspotPhase::Idle);
        assert!(state.begin_start().is_ok());
        // A concurrent Stop already owns the cleanup of a superseded start.
        let mut superseded = HotspotState::default();
        let generation = superseded.begin_start().unwrap();
        superseded.begin_stop();
        assert!(!superseded.failed_start(generation));
    }

    #[test]
    fn failed_release_restores_the_still_live_reservation() {
        let mut state = HotspotState::default();
        let generation = state.begin_start().unwrap();
        assert!(state.accept(generation, info()));
        let (stop, native) = state.begin_stop();
        assert!(native);
        assert!(state.info().is_none());
        state.retry(stop, info());
        assert_eq!(state.phase(), HotspotPhase::Active);
        assert_eq!(state.info(), Some(&info()));
        let (again, native) = state.begin_stop();
        assert!(native);
        state.stopped(again);
        assert_eq!(state.phase(), HotspotPhase::Idle);
    }

    #[test]
    fn platform_loss_clears_an_active_reservation() {
        let mut state = HotspotState::default();
        let generation = state.begin_start().unwrap();
        assert!(state.accept(generation, info()));
        state.lost();
        assert_eq!(state.phase(), HotspotPhase::Idle);
        assert!(state.info().is_none());
        // Losing an idle hotspot is a no-op, not a generation churn.
        let idle = HotspotState::default();
        let mut idle = idle;
        idle.lost();
        assert_eq!(idle.phase(), HotspotPhase::Idle);
    }

    #[test]
    fn duplicate_starts_are_refused_while_native_ownership_is_held() {
        let mut state = HotspotState::default();
        let generation = state.begin_start().unwrap();
        assert!(state.begin_start().is_err());
        assert!(state.accept(generation, info()));
        assert!(state.begin_start().is_err());
    }

    #[test]
    fn status_json_carries_exactly_what_the_phone_must_show() {
        let value = info().to_json();
        assert_eq!(value["ssid"], "KiwixShare_1234");
        assert_eq!(value["passphrase"], "correct horse battery");
        assert_eq!(
            value["qr"],
            "WIFI:T:WPA;S:KiwixShare_1234;P:correct horse battery;;"
        );
    }

    #[test]
    fn hotspot_address_prefers_the_access_point_over_the_client_default_route() {
        let v4 = |value: &str| value.parse::<IpAddr>().unwrap();
        let client = HashSet::from(["wlan0".to_string()]);
        let both = vec![
            ("wlan0".to_string(), v4("192.168.168.209")),
            ("wlan1".to_string(), v4("192.168.43.1")),
        ];
        // Both Wi-Fi links are up: the AP does not carry the default route.
        assert_eq!(
            hotspot_address(both.clone(), &client),
            Some(v4("192.168.43.1"))
        );
        // Without route evidence, wlan1 is still the likelier AP.
        assert_eq!(
            hotspot_address(both.clone(), &HashSet::new()),
            Some(v4("192.168.43.1"))
        );
        // A lone AP interface is used even when it carries the default route
        // (the emulator's only Wi-Fi link).
        assert_eq!(
            hotspot_address(vec![("wlan0".to_string(), v4("192.168.232.2"))], &client),
            Some(v4("192.168.232.2"))
        );
        // The route evidence overrides the name when the AP really is wlan0
        // and wlan1 is the client link.
        assert_eq!(
            hotspot_address(
                vec![
                    ("wlan0".to_string(), v4("192.168.43.1")),
                    ("wlan1".to_string(), v4("192.168.168.209")),
                ],
                &HashSet::from(["wlan1".to_string()]),
            ),
            Some(v4("192.168.43.1"))
        );
        // A named AP interface outranks a generic wlan link regardless.
        assert_eq!(
            hotspot_address(
                vec![
                    ("wlan1".to_string(), v4("192.168.43.1")),
                    ("ap0".to_string(), v4("192.168.49.1")),
                ],
                &HashSet::new(),
            ),
            Some(v4("192.168.49.1"))
        );
    }

    #[test]
    fn hotspot_address_ranks_the_access_point_over_client_links() {
        let v4 = |value: &str| value.parse::<IpAddr>().unwrap();
        assert_eq!(hotspot_candidate("ap0", v4("192.168.43.1")), Some(0));
        assert_eq!(hotspot_candidate("softap0", v4("192.168.43.1")), Some(0));
        assert_eq!(hotspot_candidate("swlan0", v4("192.168.49.1")), Some(0));
        assert_eq!(hotspot_candidate("wlan1", v4("192.168.43.1")), Some(1));
        assert_eq!(hotspot_candidate("wlan0", v4("192.168.168.209")), Some(2));
        assert_eq!(hotspot_candidate("eth0", v4("10.0.2.15")), Some(3));
        // Cellular, Wi-Fi Direct, tunnel, loopback, IPv6 and public addresses
        // never become the address a joined peer is told to type.
        for (name, address) in [
            ("rmnet_data0", "10.0.0.2"),
            ("ccmni0", "10.0.0.2"),
            ("seth0", "10.0.0.2"),
            ("p2p0", "192.168.49.1"),
            ("tun0", "10.8.0.2"),
            ("lo", "127.0.0.1"),
            ("wlan0", "203.0.113.4"),
            ("wlan0", "fe80::1"),
        ] {
            assert_eq!(
                hotspot_candidate(name, v4(address)),
                None,
                "{name}/{address}"
            );
        }
        assert!(
            hotspot_candidate("ap0", v4("192.168.43.1")).unwrap()
                < hotspot_candidate("wlan0", v4("192.168.168.209")).unwrap()
        );
    }
}
