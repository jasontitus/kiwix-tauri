// SPDX-License-Identifier: GPL-3.0-or-later
//! Manager-level acquisition fence. Desktop has no mobile metered-network policy.
#[path = "nearby_route.rs"]
mod nearby_route;
pub(crate) use nearby_route::JoinedRoute;
pub(crate) fn require_internet() -> Result<(), String> {
    #[cfg(mobile)]
    return crate::service::mobile_policy::require_internet();
    #[cfg(not(mobile))]
    Ok(())
}

pub(crate) async fn internet_denied() {
    #[cfg(mobile)]
    crate::service::mobile_policy::internet_denied().await;
    #[cfg(not(mobile))]
    std::future::pending::<()>().await;
}

pub(crate) fn admission_revision() -> Result<u64, String> {
    #[cfg(mobile)]
    return crate::service::mobile_policy::admission_revision();
    #[cfg(not(mobile))]
    Ok(0)
}

pub(crate) fn internet_revision() -> Result<u64, String> {
    #[cfg(mobile)]
    return crate::service::mobile_policy::internet_revision();
    #[cfg(not(mobile))]
    Ok(0)
}

pub(crate) fn require_internet_since(revision: u64) -> Result<(), String> {
    #[cfg(mobile)]
    return crate::service::mobile_policy::require_internet_since(revision);
    #[cfg(not(mobile))]
    { let _ = revision; Ok(()) }
}

pub(crate) async fn internet_denied_since(revision: u64) {
    #[cfg(mobile)]
    crate::service::mobile_policy::internet_denied_since(revision).await;
    #[cfg(not(mobile))]
    { let _ = revision; std::future::pending::<()>().await; }
}

pub(crate) fn local_revision() -> Result<u64, String> {
    #[cfg(mobile)]
    return crate::service::mobile_policy::local_revision();
    #[cfg(not(mobile))]
    Ok(0)
}

pub(crate) fn require_local_since(revision: u64) -> Result<(), String> {
    #[cfg(mobile)]
    return crate::service::mobile_policy::require_local_since(revision);
    #[cfg(not(mobile))]
    { let _ = revision; Ok(()) }
}

pub(crate) async fn local_denied_since(revision: u64) {
    #[cfg(mobile)]
    crate::service::mobile_policy::local_denied_since(revision).await;
    #[cfg(not(mobile))]
    { let _ = revision; std::future::pending::<()>().await; }
}


#[cfg(mobile)]
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Install once during trusted native setup, before constructing any local transport.
#[cfg(mobile)]
pub(crate) fn install(app: tauri::AppHandle) -> Result<(), String> {
    APP.set(app).map_err(|_| "Native transfer policy was already installed".to_string())?;
    Ok(())
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    safe fn kiwix_mobile_bind_local_socket(fd: std::ffi::c_int) -> i64;
    // The Swift receiver validates the route and the address tag. The ABI
    // carries only integers, with no borrowed address storage.
    safe fn kiwix_mobile_is_local_peer_address(route: u32, version: u8, high: u64, low: u64) -> i32;
}

/// Receipt for the actual bound physical route, never a later default or another LAN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalBinding {
    #[cfg(mobile)]
    route: u64,
}

impl LocalBinding {
    pub(crate) fn is_local_peer(self, peer: std::net::IpAddr) -> Result<bool, String> {
        #[cfg(mobile)]
        {
            use std::net::IpAddr;
            let peer = match peer {
                IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(peer),
                _ => peer,
            };
            let local_range = match peer {
                IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
                IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
            };
            // Validate the retained route even when the legacy policy classifies
            // a private address as local. Nearby library grants use only the
            // stricter on-link result below.
            let on_link = self.is_on_link_peer(peer)?;
            Ok(local_range || on_link)
        }
        #[cfg(not(mobile))]
        { self.is_on_link_peer(peer) }
    }
    pub(crate) fn is_on_link_peer(self, peer: std::net::IpAddr) -> Result<bool, String> {
        let peer = match peer {
            std::net::IpAddr::V6(ip) => ip.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(peer),
            _ => peer,
        };
        #[cfg(mobile)]
        {
            #[cfg(target_os = "ios")]
            let on_link = {
                let route = u32::try_from(self.route).map_err(|_| "Invalid native LAN interface receipt")?;
                let (version, high, low) = match peer {
                    std::net::IpAddr::V4(address) => (4, 0, u64::from(u32::from(address))),
                    std::net::IpAddr::V6(address) => {
                        let bits = u128::from(address);
                        (6, (bits >> 64) as u64, bits as u64)
                    }
                };
                match kiwix_mobile_is_local_peer_address(route, version, high, low) {
                    0 => false,
                    1 => true,
                    _ => return Err("The bound Wi-Fi or Ethernet route is no longer available".into()),
                }
            };
            #[cfg(target_os = "android")]
            let on_link = {
                use tauri_plugin_kiwix_mobile::MobileExt;
                let reply = APP.get().ok_or("Native transfer policy is not installed")?.kiwix_mobile()
                    .call_sync("isLocalPeer", serde_json::json!({"route":self.route,"address":peer.to_string()}))?;
                reply.get("local").and_then(serde_json::Value::as_bool)
                    .ok_or("Native bound-route locality reply is invalid")?
            };
            Ok(on_link)
        }
        #[cfg(not(mobile))]
        {
            let _ = self;
            let interfaces = if_addrs::get_if_addrs().map_err(|error| error.to_string())?;
            Ok(interfaces.iter().any(|interface| physical_interface(interface) && on_link_address(peer, &interface.addr)))
        }
    }
}
#[cfg(not(mobile))]
fn physical_interface(interface: &if_addrs::Interface) -> bool {
    lan_interface(&interface.name, interface.is_p2p(), interface.is_loopback())
}

/// The one answer to "is this adapter a real local network another device can
/// be on?". The peer-locality fence above and the addresses this device
/// advertises or binds for sharing (`localswarm::network::lan_addresses`) both
/// ask it, so a tunnel or container bridge cannot be a LAN for one and not
/// for the other.
///
/// Tunnel and point-to-point adapters are never such a LAN: Linux reports
/// IFF_POINTOPOINT for wg*/tun*/tap*, and Windows reports its TUN/TAP adapters
/// as point-to-point too.
pub(crate) fn lan_interface(name: &str, point_to_point: bool, loopback: bool) -> bool {
    !point_to_point && !loopback && !virtual_interface(name)
}

/// Carrier data links: Android and Linux modem interfaces and their 464XLAT
/// siblings. A carrier may hand out a private IPv4 address, so the address
/// alone cannot tell such a link from Wi-Fi. No nearby device is ever on it.
/// Apple's `pdp_ip*` is already in the virtual list. This is not part of the
/// peer-locality fence, which proves the peer's address on-link instead.
pub(crate) fn cellular_interface(name: &str) -> bool {
    const CELLULAR: [&str; 8] = ["rmnet", "ccmni", "seth", "ppp", "v4-", "clat", "wwan", "wwp"];
    let lower = name.to_ascii_lowercase();
    CELLULAR.iter().any(|prefix| lower.starts_with(prefix))
}

/// A virtual switch that can carry this machine's own LAN address. With a
/// Hyper-V external switch the physical adapter has no address at all: the
/// host's only LAN address, and its default route, are on a `vEthernet`
/// adapter. The NAT "Default Switch" and WSL adapters share the name but
/// never carry the default route, which is how the caller tells them apart.
pub(crate) fn host_switch_interface(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("vethernet")
}

/// Names of adapters that are virtual, tunnelled or otherwise not a physical
/// LAN. macOS prefixes are the ones this fence has always excluded; the rest
/// cover the common Linux, Windows and container runtimes. A name this list
/// does not know is admitted only after the address is proven on-link.
fn virtual_interface(name: &str) -> bool {
    const VIRTUAL: [&str; 9] = ["utun", "bridge", "awdl", "llw", "lo", "gif", "stf", "anpi", "pdp_ip"];
    const VIRTUAL_LINUX: [&str; 23] = [
        "wg", "tun", "tap", "docker", "veth", "br-", "virbr", "vmnet", "zt", "tailscale",
        "cni", "flannel", "kube", "lxc", "lxd", "dummy", "ifb", "sit", "ip6tnl", "gre",
        "vboxnet", "zerotier", "nordlynx",
    ];
    const VIRTUAL_WINDOWS: [&str; 8] = [
        "vethernet", "loopback", "teredo", "isatap", "npcap loopback",
        "bluetooth network connection", "local area connection*", "virtualbox host-only",
    ];
    let lower = name.to_ascii_lowercase();
    VIRTUAL.iter().any(|prefix| name.starts_with(prefix))
        || VIRTUAL_LINUX.iter().any(|prefix| lower.starts_with(prefix))
        || VIRTUAL_WINDOWS.iter().any(|prefix| lower.starts_with(prefix))
}
#[cfg(not(mobile))]
fn on_link_address(peer: std::net::IpAddr, local: &if_addrs::IfAddr) -> bool {
    match (peer, local) {
        (std::net::IpAddr::V4(peer), if_addrs::IfAddr::V4(local)) => {
            let mask = u32::from(local.netmask);
            mask != 0 && u32::from(peer) & mask == u32::from(local.ip) & mask
        }
        (std::net::IpAddr::V6(peer), if_addrs::IfAddr::V6(local)) => {
            let mask = u128::from(local.netmask);
            mask != 0 && u128::from(peer) & mask == u128::from(local.ip) & mask
        }
        _ => false,
    }
}
/// Bind a local transport to an OS-confirmed physical LAN route. Never fall back
/// to the default route. The socket stays owned by Rust throughout native duplication.
pub(crate) fn bind_local_socket(socket: &std::net::UdpSocket) -> Result<LocalBinding, String> {
    #[cfg(mobile)]
    {
        use std::os::fd::AsRawFd;
        bind_local_fd(socket.as_raw_fd())
    }
    #[cfg(not(mobile))]
    { let _ = socket; Ok(LocalBinding {}) }
}

pub(crate) fn bind_local_tcp_socket(socket: &tokio::net::TcpSocket) -> Result<LocalBinding, String> {
    #[cfg(mobile)]
    {
        use std::os::fd::AsRawFd;
        bind_local_fd(socket.as_raw_fd())
    }
    #[cfg(not(mobile))]
    { let _ = socket; Ok(LocalBinding {}) }
}

#[cfg(mobile)]
fn bind_local_fd(fd: std::os::fd::RawFd) -> Result<LocalBinding, String> {
    #[cfg(target_os = "ios")]
    {
        // SAFETY: `fd` is a live descriptor borrowed for the call; native binds
        // the route and retains no descriptor or pointer. The return is an
        // integer route ID, not a handle.
        let route = kiwix_mobile_bind_local_socket(fd);
        if route <= 0 {
            return Err("A confirmed Wi-Fi or Ethernet route is required for local sharing".into());
        }
        Ok(LocalBinding { route: route as u64 })
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_kiwix_mobile::MobileExt;
        let reply = APP.get().ok_or("Native transfer policy is not installed")?.kiwix_mobile()
            .call_sync("bindLocalSocket", serde_json::json!({"fd":fd}))?;
        let route = reply.get("route").and_then(serde_json::Value::as_u64)
            .filter(|route| *route > 0).ok_or("Native LAN binding has no route receipt")?;
        Ok(LocalBinding { route })
    }
}

/// Captured native Internet admission for one automatic reconnect resume.
/// The mobile reconciler maps its policy snapshot into this value; desktop has
/// no network policy and never constructs a reconnect attempt.
#[cfg(any(mobile, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InternetAdmission {
    Admitted,
    /// The platform has not reported a usable snapshot yet.
    Unknown,
    Offline,
    /// Connected, but metered/cellular and the user has not allowed those.
    MeteredDenied,
    /// The previous admission's retirement has not completed yet.
    Retiring,
}

/// Why one connectivity change did not start an automatic resume. The
/// reconciler records the reason instead of retrying, so a denied or retired
/// app cannot spin on the same network event.
#[cfg(any(mobile, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeSkip {
    ReadOnly,
    PrivateMode,
    AutoResumeDisabled,
    AppRetired,
    NetworkUnknown,
    Offline,
    MeteredDenied,
    NetworkRetiring,
    AlreadyAttempted,
}

#[cfg(mobile)]
impl ResumeSkip {
    // Mobile reconnect diagnostics retain the admission decision's precise cause.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only mode",
            Self::PrivateMode => "private mode",
            Self::AutoResumeDisabled => "automatic resume is disabled",
            Self::AppRetired => "the application is not active",
            Self::NetworkUnknown => "network status is unknown",
            Self::Offline => "no Internet connection",
            Self::MeteredDenied => "cellular or metered downloads are disabled",
            Self::NetworkRetiring => "the previous connection is still closing",
            Self::AlreadyAttempted => "this connectivity change was already attempted",
        }
    }
}

#[cfg(any(mobile, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeAdmission {
    Attempt,
    Skip(ResumeSkip),
}

/// Decide one automatic resume for a connectivity change. `generation` is the
/// policy revision captured by the reconciler and `last_attempt` is the revision
/// of the last attempted resume; equal values mean this change was already tried.
/// The order of the checks is the order the skip reasons are reported in.
#[cfg(any(mobile, test))]
pub(crate) fn reconnect_resume(
    settings: &crate::store::Settings,
    network: InternetAdmission,
    app_active: bool,
    generation: u64,
    last_attempt: u64,
) -> ResumeAdmission {
    use ResumeSkip as Skip;
    if settings.read_only { return ResumeAdmission::Skip(Skip::ReadOnly); }
    if settings.private_mode { return ResumeAdmission::Skip(Skip::PrivateMode); }
    if !settings.auto_resume_downloads { return ResumeAdmission::Skip(Skip::AutoResumeDisabled); }
    if !app_active { return ResumeAdmission::Skip(Skip::AppRetired); }
    match network {
        InternetAdmission::Admitted => {}
        InternetAdmission::Unknown => return ResumeAdmission::Skip(Skip::NetworkUnknown),
        InternetAdmission::Offline => return ResumeAdmission::Skip(Skip::Offline),
        InternetAdmission::MeteredDenied => return ResumeAdmission::Skip(Skip::MeteredDenied),
        InternetAdmission::Retiring => return ResumeAdmission::Skip(Skip::NetworkRetiring),
    }
    if generation == last_attempt { return ResumeAdmission::Skip(Skip::AlreadyAttempted); }
    ResumeAdmission::Attempt
}

#[cfg(test)]
mod tests {
    use super::{reconnect_resume, InternetAdmission, ResumeAdmission, ResumeSkip};

    #[cfg(not(mobile))]
    #[test]
    fn private_address_class_is_not_on_link_authority() {
        let v4 = if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
            ip: "192.168.5.241".parse().unwrap(), netmask: "255.255.252.0".parse().unwrap(),
            broadcast: None, prefixlen: 22,
        });
        let v6 = if_addrs::IfAddr::V6(if_addrs::Ifv6Addr {
            ip: "fd12:3456:789a:419c::2".parse().unwrap(), netmask: "ffff:ffff:ffff:ffff::".parse().unwrap(),
            broadcast: None, prefixlen: 64,
        });
        for (local, admitted, rejected) in [
            (&v4, "192.168.4.1", "192.168.3.255"),
            (&v4, "192.168.7.254", "10.0.0.1"),
            (&v6, "fd12:3456:789a:419c::ffff", "fd12:3456:789a:419d::1"),
            (&v6, "fd12:3456:789a:419c::1", "fd00::1"),
        ] {
            assert!(super::on_link_address(admitted.parse().unwrap(), local));
            assert!(!super::on_link_address(rejected.parse().unwrap(), local));
        }
        assert!(!super::on_link_address("192.168.1.2".parse().unwrap(), &v6));
        assert!(!super::on_link_address("fd12:3456:789a:419c::1".parse().unwrap(), &v4));
    }

    #[cfg(not(mobile))]
    #[test]
    fn tunnels_and_container_adapters_are_not_physical_lans() {
        for name in [
            // macOS, unchanged.
            "utun3", "bridge0", "awdl0", "llw0", "lo0", "gif0", "stf0", "anpi1", "pdp_ip0",
            // Linux tunnels, bridges and container links.
            "wg0", "tun0", "tap0", "docker0", "veth1a2b", "br-9f8e", "virbr0", "vmnet8",
            "zt0", "tailscale0", "cni0", "flannel.1", "kube-ipvs0", "lxcbr0", "lxdbr0",
            "dummy0", "ifb0", "sit0", "ip6tnl0", "gre0", "vboxnet0", "nordlynx",
            // Windows virtual adapters.
            "vEthernet (Default Switch)", "Loopback Pseudo-Interface 1", "Teredo Tunneling Pseudo-Interface",
            "isatap.example.org", "Npcap Loopback Adapter", "Bluetooth Network Connection",
            "Local Area Connection* 1", "VirtualBox Host-Only Network", "VirtualBox Host-Only Ethernet Adapter #2",
            "ZeroTier One [8056c2e21c000001]", "NordLynx",
        ] {
            assert!(super::virtual_interface(name), "{name}");
        }
        for name in ["en0", "en7", "eth0", "enp3s0", "wlan0", "wlp2s0", "Ethernet", "Ethernet 2", "Wi-Fi"] {
            assert!(!super::virtual_interface(name), "{name}");
        }
    }

    /// The peer-locality fence and the addresses offered for sharing ask one
    /// question. Carrier links are a separate one, asked only of addresses
    /// this device offers.
    #[test]
    fn one_classifier_decides_what_a_lan_interface_is() {
        for name in ["en0", "eth0", "enp3s0", "wlan0", "wlp2s0", "Ethernet", "Wi-Fi", "ap0", "swlan0", "wlan1", "p2p1"] {
            assert!(super::lan_interface(name, false, false), "{name}");
            assert!(!super::lan_interface(name, true, false), "{name} as a tunnel");
            assert!(!super::lan_interface(name, false, true), "{name} as loopback");
            assert!(!super::cellular_interface(name), "{name}");
        }
        for name in ["docker0", "vEthernet (WSL)", "tun0", "wg0", "utun3", "bridge100", "pdp_ip0", "lo",
            "vboxnet0", "VirtualBox Host-Only Network", "ZeroTier One [8056c2e21c000001]", "NordLynx"] {
            assert!(!super::lan_interface(name, false, false), "{name}");
        }
        // Only a Hyper-V switch can be the adapter that holds the host's LAN.
        for name in ["vEthernet (External Switch)", "vEthernet (Default Switch)", "vEthernet (WSL)"] {
            assert!(super::host_switch_interface(name), "{name}");
        }
        for name in ["docker0", "NordLynx", "wg0", "tailscale0", "bridge100", "Ethernet", "Wi-Fi"] {
            assert!(!super::host_switch_interface(name), "{name}");
        }
        for name in ["rmnet_data0", "rmnet0", "ccmni1", "seth_lte0", "ppp0", "v4-rmnet_data0", "clat4", "wwan0", "wwp0s20f0u6"] {
            assert!(super::cellular_interface(name), "{name}");
            // Not part of the locality fence: that proves the peer on-link.
            assert!(super::lan_interface(name, false, false), "{name}");
        }
    }

    fn opted_in() -> crate::store::Settings {
        crate::store::Settings { auto_resume_downloads: true, ..Default::default() }
    }

    #[test]
    fn reconnect_resume_requires_every_admission_gate() {
        let settings = opted_in();
        assert_eq!(
            reconnect_resume(&settings, InternetAdmission::Admitted, true, 3, 2),
            ResumeAdmission::Attempt
        );
        let mut read_only = settings.clone();
        read_only.read_only = true;
        assert_eq!(
            reconnect_resume(&read_only, InternetAdmission::Admitted, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::ReadOnly)
        );
        let mut private = settings.clone();
        private.private_mode = true;
        assert_eq!(
            reconnect_resume(&private, InternetAdmission::Admitted, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::PrivateMode)
        );
        let mut manual = settings.clone();
        manual.auto_resume_downloads = false;
        assert_eq!(
            reconnect_resume(&manual, InternetAdmission::Admitted, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::AutoResumeDisabled)
        );
        assert_eq!(
            reconnect_resume(&settings, InternetAdmission::Admitted, false, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::AppRetired)
        );
        assert_eq!(
            reconnect_resume(&settings, InternetAdmission::Unknown, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::NetworkUnknown)
        );
        assert_eq!(
            reconnect_resume(&settings, InternetAdmission::Offline, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::Offline)
        );
        assert_eq!(
            reconnect_resume(&settings, InternetAdmission::MeteredDenied, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::MeteredDenied)
        );
        assert_eq!(
            reconnect_resume(&settings, InternetAdmission::Retiring, true, 3, 2),
            ResumeAdmission::Skip(ResumeSkip::NetworkRetiring)
        );
    }

    #[test]
    fn one_connectivity_change_yields_one_attempt() {
        let settings = opted_in();
        let admitted = InternetAdmission::Admitted;
        assert_eq!(
            reconnect_resume(&settings, admitted, true, 4, 3),
            ResumeAdmission::Attempt
        );
        assert_eq!(
            reconnect_resume(&settings, admitted, true, 4, 4),
            ResumeAdmission::Skip(ResumeSkip::AlreadyAttempted)
        );
        assert_eq!(
            reconnect_resume(&settings, admitted, true, 5, 4),
            ResumeAdmission::Attempt
        );
    }
}
