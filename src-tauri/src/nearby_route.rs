// SPDX-License-Identifier: GPL-3.0-or-later
//! Retained OS association or app-owned AP interface authority for QUIC UDP.
#![forbid(unsafe_code)]

#[cfg(any(target_os = "android", all(test, unix)))]
#[path = "nearby_hotspot.rs"]
mod hotspot;
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
        let receipt = hotspot::Receipt::parse(reply, peer)?;
        let socket = receipt.bind_socket()?;
        let lease = receipt.lease;
        let state = native("hotspotQuicState",serde_json::json!({"lease":lease}))?;
        if state.get("available").and_then(serde_json::Value::as_bool)!=Some(true) { return Err(LOST.into()); }
        Ok(socket)
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
