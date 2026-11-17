// SPDX-License-Identifier: GPL-3.0-or-later
//! Retained OS association or app-owned AP interface authority for QUIC UDP.
use std::{net::{Ipv4Addr, SocketAddr, UdpSocket}, sync::Arc};
use tokio_util::sync::CancellationToken;
const LOST: &str = "The selected Nearby connection is no longer available. Connect again.";
#[derive(Clone)]
pub(crate) struct JoinedRoute(Arc<JoinOwner>);
struct JoinOwner {
    #[cfg_attr(not(mobile), expect(dead_code, reason = "Native mobile route state, socket binding and release authenticate this owner"))]
    owner: String,
    #[cfg_attr(not(mobile), expect(dead_code, reason = "Native mobile route state, socket binding and release authenticate this lease"))]
    lease: String,
    #[cfg_attr(not(mobile), expect(dead_code, reason = "Native mobile route ownership selects AP or joined-Wi-Fi lifecycle"))]
    hotspot: bool,
    cancel: CancellationToken,
}
impl JoinedRoute {
    pub(crate) fn new(owner: String, lease: String, cancel: CancellationToken) -> Result<Self, String> {
        Self::create(owner,lease,false,cancel)
    }
    #[cfg_attr(not(target_os = "android"), expect(dead_code, reason = "Android reverse Nearby connections own the temporary hotspot route"))]
    pub(crate) fn hotspot(owner: String, lease: String, cancel: CancellationToken) -> Result<Self, String> {
        Self::create(owner,lease,true,cancel)
    }
    fn create(owner: String, lease: String, hotspot: bool, cancel: CancellationToken) -> Result<Self, String> {
        for value in [&owner,&lease] {
            let id = uuid::Uuid::parse_str(value).map_err(|_|"Invalid native Nearby route receipt")?;
            if id.to_string()!=*value { return Err("Invalid native Nearby route receipt".into()); }
        }
        Ok(Self(Arc::new(JoinOwner { owner,lease,hotspot,cancel })))
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.0.cancel.is_cancelled() { return Err(LOST.into()); }
        #[cfg(mobile)]
        {
            let command = if self.0.hotspot { "hotspotQuicState" } else { "nearbyJoinState" };
            let reply = native(command,serde_json::json!({"owner":self.0.owner,"lease":self.0.lease}))?;
            if reply.get("available").and_then(serde_json::Value::as_bool)!=Some(true) { self.cancel(); return Err(LOST.into()); }
            Ok(())
        }
        #[cfg(not(mobile))]
        Err("Automatic hotspot connections require a supported mobile device".into())
    }
    #[cfg_attr(not(mobile), expect(dead_code, reason = "Native mobile state and joined-route loss callbacks revoke the route"))]
    pub(crate) fn cancel(&self) { self.0.cancel.cancel(); }
    pub(crate) fn token(&self) -> CancellationToken { self.0.cancel.clone() }
    pub(crate) async fn bind_udp(&self, endpoint: SocketAddr) -> Result<UdpSocket,String> {
        self.bind_socket(endpoint).await.map(|(socket,_)|socket)
    }
    /// Returns the fresh native-confirmed local endpoint as well as the exact
    /// interface-bound socket when a sharing listener moves onto a joined AP.
    pub(crate) async fn listener_socket(&self, endpoint: SocketAddr) -> Result<(UdpSocket,Vec<String>),String> {
        let (socket,address) = self.bind_socket(endpoint).await?;
        Ok((socket,vec![address.ok_or("The joined Wi-Fi route has no confirmed local IPv4 address")?.to_string()]))
    }
    async fn bind_socket(&self, endpoint: SocketAddr) -> Result<(UdpSocket,Option<Ipv4Addr>),String> {
        let SocketAddr::V4(endpoint) = endpoint else { return Err("Automatic Nearby joining requires an on-link IPv4 endpoint".into()); };
        if endpoint.port()==0 || !(endpoint.ip().is_private() || endpoint.ip().is_link_local()) { return Err("Nearby requires an on-link private network endpoint".into()); }
        // The native route check runs inside the blocking hop below: on mobile it
        // is a synchronous platform call and must not run on the async caller.
        #[cfg(mobile)]
        {
            let joined = self.clone();
            // This worker owns the socket until the native operation finishes:
            // cancelling its future cannot close/reuse a still-borrowed fd.
            tokio::task::spawn_blocking(move || {
                use std::os::fd::AsRawFd;
                joined.check()?;
                let (socket,reply) = if joined.0.hotspot {
                    #[cfg(target_os = "android")]
                    {
                        let reply = native("hotspotQuicRoute",serde_json::json!({"lease":joined.0.lease,"address":endpoint.ip().to_string()}))?;
                        if reply.get("lease").and_then(serde_json::Value::as_str)!=Some(joined.0.lease.as_str()) { return Err(LOST.into()); }
                        (Self::hotspot_socket(&reply,Some(*endpoint.ip()))?,reply)
                    }
                    #[cfg(not(target_os = "android"))]
                    return Err("Only Android can own a temporary hotspot route".into());
                } else {
                    #[cfg(target_os = "ios")]
                    let socket: UdpSocket = socket2::Socket::new(socket2::Domain::IPV4,socket2::Type::DGRAM,Some(socket2::Protocol::UDP))
                        .map_err(|error|error.to_string())?.into();
                    #[cfg(not(target_os = "ios"))]
                    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error|error.to_string())?;
                    let reply = native("nearbyJoinBindSocket",serde_json::json!({"owner":joined.0.owner,"lease":joined.0.lease,
                        "fd":socket.as_raw_fd(),"address":endpoint.ip().to_string()}))?;
                    if reply.get("route").and_then(serde_json::Value::as_u64).filter(|route|*route>0).is_none() {
                        return Err("Invalid native Nearby UDP route receipt".into());
                    }
                    (socket,reply)
                };
                let address = reply.get(if joined.0.hotspot {"address"} else {"localAddress"})
                    .and_then(serde_json::Value::as_str).map(str::parse::<Ipv4Addr>).transpose().map_err(|_|"Invalid native local IPv4 endpoint")?;
                if let Some(address) = address {
                    if socket.local_addr().map_err(|error|error.to_string())?.ip()!=std::net::IpAddr::V4(address) {
                        return Err("The Nearby UDP socket does not own its confirmed local address".into());
                    }
                }
                joined.check()?;
                socket.set_nonblocking(true).map_err(|error|error.to_string())?;
                Ok((socket,address))
            }).await.map_err(|error|error.to_string())?
        }
        #[cfg(not(mobile))]
        Err("Automatic hotspot connections require a supported mobile device".into())
    }
    /// All OS socket operations remain with the Rust fd owner; no hidden Java
    /// SDK methods, reflection, process binding, or weaker default-route fallback.
    #[cfg(target_os = "android")]
    pub(crate) fn hotspot_socket(reply: &serde_json::Value, peer: Option<Ipv4Addr>) -> Result<UdpSocket,String> {
        use std::{ffi::{CStr,CString},os::fd::AsRawFd};
        let field = |key|reply.get(key).and_then(serde_json::Value::as_str).ok_or("Invalid native hotspot interface receipt");
        let address: Ipv4Addr = field("address")?.parse().map_err(|_|"Invalid native hotspot IPv4 address")?;
        if !(address.is_private() || address.is_link_local()) { return Err("Invalid native hotspot IPv4 address".into()); }
        let lease = field("lease")?;
        if uuid::Uuid::parse_str(lease).is_err() { return Err("Invalid native hotspot lease".into()); }
        let name = field("interfaceName")?;
        if name.is_empty() || name.len()>=libc::IF_NAMESIZE || !name.is_ascii() { return Err("Invalid native hotspot interface name".into()); }
        let name = CString::new(name).map_err(|_|"Invalid native hotspot interface name")?;
        let index = reply.get("interfaceIndex").and_then(serde_json::Value::as_u64).and_then(|index|u32::try_from(index).ok()).filter(|index|*index>0)
            .ok_or("Invalid native hotspot interface index")?;
        let prefix = reply.get("prefixLength").and_then(serde_json::Value::as_u64).filter(|prefix|(1..=32).contains(prefix))
            .ok_or("Invalid native hotspot IPv4 prefix")? as u32;
        let mask = u32::MAX << (32-prefix);
        if let Some(peer) = peer {
            let number = u32::from(peer); let host = number & !mask;
            if peer==address || number & mask != u32::from(address) & mask || (prefix<31 && (host==0 || host==!mask)) {
                return Err("The selected peer is not on the owned hotspot interface".into());
            }
        }
        struct Interfaces(*mut libc::ifaddrs);
        impl Drop for Interfaces { fn drop(&mut self) { unsafe { libc::freeifaddrs(self.0) }; } }
        let matches = || -> Result<(),String> {
            if unsafe { libc::if_nametoindex(name.as_ptr()) }!=index { return Err(LOST.into()); }
            let mut first = std::ptr::null_mut();
            if unsafe { libc::getifaddrs(&mut first) }!=0 { return Err(std::io::Error::last_os_error().to_string()); }
            let interfaces = Interfaces(first);
            let mut current = interfaces.0;
            while !current.is_null() {
                let entry = unsafe { &*current };
                current = entry.ifa_next;
                if entry.ifa_name.is_null() || entry.ifa_addr.is_null() || entry.ifa_netmask.is_null()
                    || unsafe { CStr::from_ptr(entry.ifa_name) }!=name.as_c_str()
                    || entry.ifa_flags & (libc::IFF_UP | libc::IFF_RUNNING) as u32 != (libc::IFF_UP | libc::IFF_RUNNING) as u32
                    || entry.ifa_flags & (libc::IFF_LOOPBACK | libc::IFF_POINTOPOINT) as u32 != 0
                    || unsafe { (*entry.ifa_addr).sa_family as i32 }!=libc::AF_INET { continue; }
                let local = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
                let netmask = unsafe { &*entry.ifa_netmask.cast::<libc::sockaddr_in>() };
                if local.sin_addr.s_addr==u32::from_ne_bytes(address.octets()) && netmask.sin_addr.s_addr==mask.to_be() { return Ok(()); }
            }
            Err(LOST.into())
        };
        matches()?;
        let socket = socket2::Socket::new(socket2::Domain::IPV4,socket2::Type::DGRAM,Some(socket2::Protocol::UDP)).map_err(|error|error.to_string())?;
        let fd = socket.as_raw_fd();
        let mut kind: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
        if unsafe { libc::getsockopt(fd,libc::SOL_SOCKET,libc::SO_TYPE,(&mut kind as *mut libc::c_int).cast(),&mut length) }!=0 || kind!=libc::SOCK_DGRAM {
            return Err("The owned hotspot requires a UDP socket".into());
        }
        if unsafe { libc::setsockopt(fd,libc::SOL_SOCKET,libc::SO_BINDTODEVICE,name.as_ptr().cast(),name.as_bytes_with_nul().len() as libc::socklen_t) }!=0 {
            return Err(format!("Android cannot bind QUIC to the exact temporary-hotspot interface: {}",std::io::Error::last_os_error()));
        }
        let mut bound = [0u8;libc::IF_NAMESIZE];
        let mut length = bound.len() as libc::socklen_t;
        if unsafe { libc::getsockopt(fd,libc::SOL_SOCKET,libc::SO_BINDTODEVICE,bound.as_mut_ptr().cast(),&mut length) }!=0
            || length as usize>bound.len() || CStr::from_bytes_until_nul(&bound[..length as usize]).ok()!=Some(name.as_c_str()) {
            return Err("Android did not retain the exact hotspot interface binding".into());
        }
        socket.bind(&std::net::SocketAddrV4::new(address,0).into()).map_err(|error|error.to_string())?;
        matches()?;
        let state = native("hotspotQuicState",serde_json::json!({"lease":lease}))?;
        if state.get("available").and_then(serde_json::Value::as_bool)!=Some(true) { return Err(LOST.into()); }
        Ok(socket.into())
    }
}
impl Drop for JoinOwner {
    fn drop(&mut self) {
        self.cancel.cancel();
        // AP reservations may also serve another local owner. Their global
        // lifecycle remains with the trusted hotspot controller, not this fd.
        #[cfg(mobile)]
        {
            if self.hotspot { return; }
            if let Some(app) = super::APP.get().cloned() {
                use tauri_plugin_kiwix_mobile::MobileExt;
                let owner = self.owner.clone(); let lease = self.lease.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _ = app.kiwix_mobile().call_sync("nearbyJoinRelease",serde_json::json!({"owner":owner,"lease":lease}));
                });
            }
        }
    }
}
#[cfg(mobile)]
fn native(command: &'static str,payload:serde_json::Value)->Result<serde_json::Value,String> {
    use tauri_plugin_kiwix_mobile::MobileExt;
    super::APP.get().ok_or("Native Nearby route adapter is unavailable")?.kiwix_mobile().call_sync(command,payload)
}
