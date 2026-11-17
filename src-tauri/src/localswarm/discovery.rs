//! Opt-in DNS-SD. Construction and snapshots never open network sockets.
use super::protocol::{self, Manifest};
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use super::network;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use mdns_sd::{DaemonStatus, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use std::net::IpAddr;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use std::sync::{Arc, Mutex};
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use std::time::{Duration, Instant};
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use tokio::task::JoinHandle;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use tokio_util::sync::CancellationToken;

const SERVICE: &str = "_localswarm._tcp.local.";
const MAX_PEERS: usize = 256;
const MAX_ADVERTISEMENTS: usize = 32;
const MAX_TXT: usize = 2048;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub peer_id: String,
    pub swarm_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher_name: Option<String>,
    pub total_bytes: u64,
    pub chunk_count: usize,
    pub host: String,
    pub port: u16,
    pub locked: bool,
    pub transport: String,
    pub addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_count: Option<usize>,
}

pub(super) fn valid_library_session_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A DNS-SD instance label that changes with the library session while staying
/// inside the 63-byte label bound. Receivers therefore see a restarted share as
/// a new record instead of holding the old session TXT until TTL.
pub(super) fn library_instance(peer_id: &str, session_id: &str) -> String {
    format!("Kiwix-Library-{peer_id}-{}", &session_id[..8])
}

#[cfg(test)]
mod library_instance_tests {
    use super::{library_addresses, library_instance};
    #[test]
    fn changes_with_session_and_fits_a_dns_label() {
        let peer = "074d1402-0cb2-485e-bdae-12d6d5f1383c";
        let first = library_instance(peer, &"a".repeat(32));
        let second = library_instance(peer, &"b".repeat(32));
        assert_ne!(first, second);
        assert_eq!(first, format!("Kiwix-Library-{peer}-aaaaaaaa"));
        assert!(first.len() <= 63);
    }

    #[test]
    fn ipv4_advertisement_projects_and_deduplicates_a_mixed_listener() {
        let addresses = vec!["192.168.1.59".into(), "fd12:3456:789a:419c::10".into(),
            "192.168.1.59".into(), "10.0.0.2".into()];
        assert_eq!(library_addresses(&addresses).unwrap(), vec![
            std::net::Ipv4Addr::new(192, 168, 1, 59), std::net::Ipv4Addr::new(10, 0, 0, 2),
        ]);
    }

    #[test]
    fn ipv4_advertisement_rejects_unreachable_or_missing_ipv4_without_inventing_an_address() {
        assert!(library_addresses(&[]).is_err());
        assert!(library_addresses(&vec!["192.168.1.59".into(); 33]).is_err());
        assert!(library_addresses(&["fd12:3456:789a:419c::10".into()]).is_err());
        for invalid in ["not-an-ip", "0.0.0.0", "127.0.0.1", "169.254.1.1", "255.255.255.255",
            "224.0.0.1", "::", "::1", "fe80::1", "ff02::1", "::ffff:192.168.1.59"] {
            assert!(library_addresses(&["192.168.1.59".into(), invalid.into()]).is_err(), "{invalid}");
        }
    }
}

/// Project validated listener addresses onto this adapter's explicit DNS A
/// records. Keep IPv6 in the source LAN descriptor; never invent an IPv4 route.
#[cfg(any(target_os = "ios", target_os = "macos", test))]
pub(super) fn library_addresses(addresses: &[String]) -> Result<Vec<std::net::Ipv4Addr>, String> {
    if addresses.is_empty() || addresses.len() > 32 { return Err("No bounded reachable IPv4 library listener addresses".into()); }
    let mut ips = Vec::with_capacity(addresses.len());
    for address in addresses {
        let ip: std::net::IpAddr = address.parse().map_err(|_| "Library sharing requires actual listener addresses")?;
        if !super::network::usable_address(ip) { return Err("Cannot advertise an unreachable library listener address".into()); }
        if let std::net::IpAddr::V4(ip) = ip {
            if !ips.contains(&ip) { ips.push(ip); }
        }
    }
    if ips.is_empty() { return Err("No reachable IPv4 library listener address for this advertisement".into()); }
    Ok(ips)
}

pub(super) fn library_peer(peer: Peer) -> Result<Peer, String> {
    let native = peer.host.starts_with("bonjour:") || peer.host.starts_with("bonjour-lan:");
    if peer.transport != "quic-library" || peer.port == 0 || peer.locked
        || !peer.library_session_id.as_deref().is_some_and(valid_library_session_id)
        || !peer.library_count.is_some_and(|count| count > 0 && count <= 10_000)
        || !peer.swarm_id.is_empty() || peer.total_bytes != 0 || peer.chunk_count != 0
        || peer.name != "Shared library"
        || (!native && !peer.addresses.iter().any(|address| address == &peer.host))
        || peer.addresses.len() > 32 || peer.addresses.iter().any(|address| {
            !address.parse::<std::net::IpAddr>().is_ok_and(super::network::usable_address)
        })
    { return Err("The shared library is no longer available; find nearby and browse again".into()); }
    Ok(peer)
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[path = "ios_discovery.rs"]
mod ios_discovery;
#[cfg(any(target_os = "ios", target_os = "macos"))]
pub use ios_discovery::Discovery;

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
pub struct Discovery {
    peer_id: String,
    daemon: Option<ServiceDaemon>,
    browse: Option<(CancellationToken, JoinHandle<()>)>,
    peers: Arc<Mutex<BTreeMap<String, Peer>>>,
    error: Arc<Mutex<Option<String>>>,
    advertisements: HashSet<String>,
}
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
impl Discovery {
    pub fn new(peer_id: String) -> Result<Self, String> {
        if !protocol::valid_peer_id(&peer_id) { return Err("Invalid LocalSwarm peer ID".into()); }
        Ok(Self { peer_id, daemon: None, browse: None, peers: Arc::new(Mutex::new(BTreeMap::new())),
            error: Arc::new(Mutex::new(None)), advertisements: HashSet::new() })
    }
    fn daemon(&mut self) -> Result<&ServiceDaemon, String> {
        if self.daemon.is_none() {
            self.daemon = Some(ServiceDaemon::new().map_err(|e| format!("Local network discovery unavailable: {e}"))?);
        }
        self.daemon.as_ref().ok_or_else(|| "LocalSwarm discovery is stopped".into())
    }
    pub fn is_browsing(&self) -> bool {
        self.browse.as_ref().is_some_and(|(_, task)| !task.is_finished())
    }
    pub fn error(&self) -> Option<String> { self.error.lock().unwrap_or_else(|e| e.into_inner()).clone() }
    pub fn peers(&self) -> Vec<Peer> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }
    pub fn library_peer(&self, id: &str) -> Result<Peer, String> {
        if !self.is_browsing() { return Err("Find nearby before browsing a shared library".into()); }
        let peer = self.peers.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
            .ok_or("The shared library is no longer nearby; find nearby and browse again")?;
        library_peer(peer)
    }
    pub fn selected_library_peer(&self, id: &str) -> Result<Option<Peer>, String> {
        self.peers.lock().unwrap_or_else(|error| error.into_inner()).get(id).cloned()
            .map(library_peer).transpose()
    }
    /// Direct (peer-to-peer) mode is Apple-only; this platform has no AWDL and
    /// keeps the infrastructure route.
    pub async fn set_direct(&mut self, _direct: bool) -> Result<(), String> { Ok(()) }
    pub async fn start_browse(&mut self) -> Result<(), String> {
        if self.is_browsing() { return Ok(()); }
        if self.browse.is_some() { return Err("Stop the failed discovery session before restarting".into()); }
        let receiver = self.daemon()?.browse(SERVICE).map_err(|e| format!("Cannot browse local network: {e}"))?;
        let task_daemon = self.daemon()?.clone();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let peers = self.peers.clone(); let error = self.error.clone(); let self_id = self.peer_id.clone();
        *error.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let task = tokio::spawn(async move {
            let mut window = Instant::now();
            let mut events = 0usize;
            loop {
                let event = tokio::select! {
                    _ = task_cancel.cancelled() => break,
                    event = receiver.recv_async() => match event {
                        Ok(event) => event,
                        Err(_) => { *error.lock().unwrap_or_else(|e| e.into_inner()) = Some("Local network discovery stopped unexpectedly".into()); break; }
                    },
                };
                if window.elapsed() >= Duration::from_secs(60) { window = Instant::now(); events = 0; }
                events += 1;
                if events > 8192 {
                    *error.lock().unwrap_or_else(|e| e.into_inner()) = Some("Local network discovery traffic limit exceeded; stop and restart discovery".into());
                    break;
                }
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        if let Some(peer) = parse_peer(&info, &self_id) {
                            let mut peers = peers.lock().unwrap_or_else(|e| e.into_inner());
                            if !peers.contains_key(&peer.id) && peers.len() >= MAX_PEERS {
                                if let Some(oldest) = peers.keys().next().cloned() { peers.remove(&oldest); }
                            }
                            peers.insert(peer.id.clone(), peer);
                        } else { peers.lock().unwrap_or_else(|e| e.into_inner()).remove(&info.fullname); }
                    }
                    ServiceEvent::ServiceRemoved(_, id) => { peers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id); }
                    ServiceEvent::SearchStopped(_) => break,
                    _ => {},
                }
            }
            // Dropping the subscriber alone does not stop the browse query.
            let _ = task_daemon.stop_browse(SERVICE);
            peers.lock().unwrap_or_else(|e| e.into_inner()).clear();
        });
        self.browse = Some((cancel, task));
        Ok(())
    }
    pub async fn stop_browse(&mut self) -> Result<(), String> {
        let mut result = Ok(());
        if let Some((cancel, task)) = self.browse.take() {
            if let Some(daemon) = &self.daemon { result = daemon.stop_browse(SERVICE).map_err(|e| e.to_string()); }
            cancel.cancel();
            task.await.map_err(|_| "LocalSwarm discovery receiver stopped unexpectedly")?;
        }
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).clear();
        if self.advertisements.is_empty() { self.stop_daemon().await?; }
        result
    }
    pub async fn advertise_addresses(&mut self, manifest: &Manifest, port: u16, locked: bool, addresses: &[String], publisher_name: &str) -> Result<String, String> {
        manifest.validate()?;
        if publisher_name.is_empty() || !crate::publisher::valid_name(publisher_name) { return Err("Invalid publisher name".into()); }
        if port == 0 || self.advertisements.len() >= MAX_ADVERTISEMENTS { return Err("LocalSwarm advertisement limit or invalid port".into()); }
        let mut ips = Vec::new();
        for address in addresses.iter().take(32) {
            let ip: IpAddr = address.parse().map_err(|_| "Invalid listener address")?;
            if !network::usable_address(ip) { return Err("Cannot advertise an unreachable LocalSwarm listener address".into()); }
            if !ips.contains(&ip) { ips.push(ip); }
        }
        if ips.is_empty() { return Err("No reachable local network address; use a direct connection instead".into()); }
        let instance = format!("Kiwix-{}", uuid::Uuid::new_v4().simple());
        let hostname = format!("kiwix-{}.local.", uuid::Uuid::new_v4().simple());
        let mut txt = HashMap::new();
        txt.insert("sid".to_string(), manifest.swarm_id.clone());
        let name = if locked { "Protected share".to_string() } else { truncate_utf8(&manifest.name, 240) };
        txt.insert("name".into(), name);
        txt.insert("device".into(), publisher_name.into());
        txt.insert("bytes".into(), manifest.total_bytes.to_string());
        txt.insert("chunks".into(), manifest.chunk_hashes.len().to_string());
        txt.insert("pid".into(), self.peer_id.clone());
        txt.insert("tp".into(), "quic".into()); txt.insert("pv".into(), "1".into());
        if locked { txt.insert("lk".into(), "1".into()); }
        let info = ServiceInfo::new(SERVICE, &instance, &hostname, ips.as_slice(), port, txt)
            .map_err(|e| format!("Cannot prepare LocalSwarm advertisement: {e}"))?;
        let id = format!("{}#{}", info.get_fullname(), uuid::Uuid::new_v4().simple());
        self.daemon()?.register(info).map_err(|e| format!("Cannot advertise LocalSwarm share: {e}"))?;
        self.advertisements.insert(id.clone());
        Ok(id)
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn advertise_library(&mut self, session_id: &str, port: u16, count: usize, addresses: &[String], publisher_name: &str, generation: u64, _native: bool) -> Result<String, String> {
        if !valid_library_session_id(session_id) || port == 0 || count == 0 || count > 10_000 || generation == 0 || self.advertisements.len() >= MAX_ADVERTISEMENTS {
            return Err("Invalid library session, listener port, or advertisement limit".into());
        }
        if publisher_name.is_empty() || !crate::publisher::valid_name(publisher_name) { return Err("Invalid publisher name".into()); }
        if addresses.is_empty() || addresses.len() > 32 { return Err("No bounded reachable library listener addresses".into()); }
        let mut ips = Vec::with_capacity(addresses.len());
        for address in addresses {
            let ip: IpAddr = address.parse().map_err(|_| "Invalid library listener address")?;
            if !network::usable_address(ip) { return Err("Cannot advertise an unreachable library listener address".into()); }
            if !ips.contains(&ip) { ips.push(ip); }
        }
        // A fresh session must be a fresh DNS-SD record; a restarted sender
        // otherwise leaves receivers holding the old session TXT until TTL.
        let instance = library_instance(&self.peer_id, session_id);
        let hostname = format!("kiwix-{}.local.", uuid::Uuid::new_v4().simple());
        let txt = HashMap::from([
            ("sid".to_string(), session_id.to_string()), ("pid".into(), self.peer_id.clone()),
            ("name".into(), "Shared library".into()),
            ("bytes".into(), "0".into()), ("chunks".into(), "0".into()),
            ("tp".into(), "quic-library".into()), ("pv".into(), "1".into()),
            ("count".into(), count.to_string()),
            ("gen".into(), generation.to_string()),
            ("device".into(), publisher_name.into()),
        ]);
        let info = ServiceInfo::new(SERVICE, &instance, &hostname, ips.as_slice(), port, txt)
            .map_err(|e| format!("Cannot prepare library advertisement: {e}"))?;
        let fullname = info.get_fullname();
        if self.advertisements.iter().any(|id| id.strip_prefix(fullname).is_some_and(|suffix| suffix.starts_with('#'))) {
            return Err("The previous library announcement is still stopping; stop sharing before starting it again".into());
        }
        // Withdrawal owns this publication, not every future use of its name.
        let id = format!("{fullname}#{}", uuid::Uuid::new_v4().simple());
        self.daemon()?.register(info).map_err(|e| format!("Cannot advertise shared library: {e}"))?;
        self.advertisements.insert(id.clone());
        Ok(id)
    }
    pub async fn unadvertise(&mut self, id: &str) -> Result<(), String> {
        if !self.advertisements.contains(id) { return Ok(()); }
        if let Some(daemon) = &self.daemon {
            let fullname = id.split_once('#').map(|(name, _)| name)
                .ok_or("Invalid owned LocalSwarm announcement")?;
            let receiver = daemon.unregister(fullname).map_err(|e| e.to_string())?;
            tokio::time::timeout(Duration::from_secs(5), receiver.recv_async()).await
                .map_err(|_| "LocalSwarm unadvertise timed out")?.map_err(|_| "LocalSwarm discovery daemon stopped")?;
        }
        self.advertisements.remove(id);
        if self.advertisements.is_empty() && self.browse.is_none() { self.stop_daemon().await?; }
        Ok(())
    }
    async fn stop_daemon(&mut self) -> Result<(), String> {
        if let Some(daemon) = self.daemon.as_ref() {
            let receiver = match daemon.shutdown() {
                Ok(receiver) => receiver,
                Err(_) => daemon.status().map_err(|e| format!("Cannot stop LocalSwarm discovery: {e}"))?,
            };
            let status = tokio::time::timeout(Duration::from_secs(5), receiver.recv_async()).await
                .map_err(|_| "LocalSwarm discovery shutdown timed out")?.map_err(|_| "LocalSwarm discovery shutdown failed")?;
            if status != DaemonStatus::Shutdown { return Err("LocalSwarm discovery did not stop".into()); }
            self.daemon = None;
        }
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some((cancel, task)) = self.browse.take() {
            cancel.cancel(); let _ = task.await;
        }
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.stop_daemon().await?;
        self.advertisements.clear();
        Ok(())
    }
}
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
impl Drop for Discovery {
    fn drop(&mut self) {
        if let Some((cancel, _)) = &self.browse { cancel.cancel(); }
        if let Some(daemon) = &self.daemon { let _ = daemon.shutdown(); }
    }
}

fn truncate_utf8(value: &str, cap: usize) -> String {
    let mut end = value.len().min(cap);
    while !value.is_char_boundary(end) { end -= 1; }
    value[..end].to_string()
}
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn parse_peer(info: &ResolvedService, self_id: &str) -> Option<Peer> {
    if info.ty_domain != SERVICE || info.fullname.len() > 255 || info.port == 0 { return None; }
    let properties = info.get_properties();
    let mut txt_bytes = 0usize;
    let mut count = 0;
    for property in properties.iter() {
        count += 1;
        txt_bytes = txt_bytes.checked_add(property.key().len() + property.val().map_or(0, |v| v.len()) + 1)?;
        if count > 16 || txt_bytes > MAX_TXT { return None; }
    }
    let text = |key: &str| properties.get_property_val_str(key);
    if text("pv") != Some("1") { return None; }
    let publisher_name = crate::publisher::advertised_name(text("device")).ok()?;
    if text("tp") == Some("quic-library") {
        let session_id = text("sid")?;
        let peer_id = text("pid")?;
        let library_count = text("count")?.parse::<usize>().ok()?;
        let generation = text("gen").and_then(|value| value.parse::<u64>().ok());
        if !valid_library_session_id(session_id) || !protocol::valid_peer_id(peer_id) || peer_id == self_id
            || text("bytes") != Some("0") || text("chunks") != Some("0")
            || library_count == 0 || library_count > 10_000 || text("awdl").is_some_and(|value| value != "1")
            || text("name") != Some("Shared library")
            || properties.iter().any(|property| !["pv", "tp", "sid", "pid", "name", "bytes", "chunks", "count", "gen", "device", "awdl"].contains(&property.key()))
        { return None; }
        let mut ips: Vec<IpAddr> = info.get_addresses().iter().map(|ip| ip.to_ip_addr())
            .filter(|ip| network::usable_address(*ip)).take(32).collect();
        ips.sort_by_key(|ip| (ip.is_ipv6(), *ip)); ips.dedup();
        return Some(Peer { id: info.fullname.clone(), peer_id: peer_id.into(), swarm_id: String::new(),
            library_session_id: Some(session_id.into()), library_generation: generation, name: text("name")?.into(), total_bytes: 0, chunk_count: 0,
            publisher_name, library_count: Some(library_count),
            host: ips.first()?.to_string(), port: info.port, locked: false, transport: "quic-library".into(),
            addresses: ips.iter().map(ToString::to_string).collect() });
    }
    if text("tp") != Some("quic") { return None; }
    let swarm_id = text("sid")?;
    let peer_id = text("pid")?;
    let name = text("name")?;
    if !protocol::valid_hash(swarm_id) || !protocol::valid_peer_id(peer_id) || peer_id == self_id
        || name.is_empty() || name.len() > 240 || name.chars().any(char::is_control)
        || text("lk").is_some_and(|value| value != "0" && value != "1")
    { return None; }
    let total_bytes = text("bytes")?.parse::<u64>().ok()?;
    let chunk_count = text("chunks")?.parse::<usize>().ok()?;
    if chunk_count == 0 || chunk_count > protocol::MAX_CHUNKS || total_bytes == 0
        || total_bytes > protocol::MAX_CHUNK_SIZE as u64 * chunk_count as u64
    { return None; }
    let mut ips: Vec<IpAddr> = info.get_addresses().iter().map(|ip| ip.to_ip_addr()).filter(|ip| network::usable_address(*ip)).take(32).collect();
    ips.sort_by_key(|ip| (ip.is_ipv6(), *ip)); ips.dedup();
    let host = ips.first()?.to_string();
    let locked = text("lk") == Some("1");
    Some(Peer { id: info.fullname.clone(), peer_id: peer_id.into(), swarm_id: swarm_id.into(),
        name: if locked { "Protected share".into() } else { name.into() }, total_bytes, chunk_count, host,
        publisher_name,
        port: info.port, locked, transport: "quic".into(), addresses: ips.iter().map(ToString::to_string).collect(),
        library_session_id: None, library_generation: None, library_count: None })
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "macos"))))]
mod tests {
    use super::*;
    #[test]
    fn bounded_unicode_txt_and_no_unusable_addresses() {
        assert_eq!(truncate_utf8("ééé", 3), "é");
        for ip in ["0.0.0.0", "127.0.0.1", "169.254.1.1", "fe80::1", "::", "::1", "ff02::1"] {
            assert!(!network::usable_address(ip.parse().unwrap()));
        }
        assert!(network::usable_address("192.168.1.2".parse().unwrap()));
        let discovery = Discovery::new("peer".into()).unwrap();
        assert!(!discovery.is_browsing());
        assert!(discovery.daemon.is_none());
        assert!(discovery.peers().is_empty());
    }
    fn library_info(properties: HashMap<String, String>, addresses: &[IpAddr]) -> ResolvedService {
        ServiceInfo::new(SERVICE, "library-test", "library-test.local.", addresses, 8090, properties)
            .unwrap().as_resolved_service()
    }
    fn library_txt() -> HashMap<String, String> {
        HashMap::from([
            ("pv".into(), "1".into()), ("tp".into(), "quic-library".into()),
            ("sid".into(), "a".repeat(32)), ("pid".into(), "remote-peer".into()),
            ("name".into(), "Shared library".into()), ("count".into(), "12".into()),
            ("gen".into(), "1".into()),
            ("bytes".into(), "0".into()), ("chunks".into(), "0".into()),
        ])
    }
    #[test]
    fn publisher_hint_is_bounded_and_independent_of_peer_identity() {
        let ips = ["192.168.1.20".parse().unwrap()];
        let mut txt = library_txt();
        txt.insert("device".into(), "図書館の Android".into());
        let peer = parse_peer(&library_info(txt, &ips), "self-peer").unwrap();
        assert_eq!(peer.publisher_name.as_deref(), Some("図書館の Android"));
        assert_eq!(peer.peer_id, "remote-peer");
        assert_eq!(peer.name, "Shared library");
        for name in ["", " padded", "spoof\u{202e}", &"é".repeat(81)] {
            let mut txt = library_txt();
            txt.insert("device".into(), name.into());
            assert!(parse_peer(&library_info(txt, &ips), "self-peer").is_none());
        }
    }
    #[test]
    fn library_shape_is_distinct_public_and_accepts_reachable_numeric_families() {
        let ips = ["192.168.1.20".parse().unwrap(), "2001:db8::1".parse().unwrap()];
        let peer = parse_peer(&library_info(library_txt(), &ips), "self-peer").unwrap();
        assert_eq!(peer.transport, "quic-library");
        assert_eq!(peer.library_session_id.as_deref(), Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(peer.swarm_id.is_empty());
        assert_eq!(peer.addresses, ["192.168.1.20", "2001:db8::1"]);
        assert!(library_peer(peer).is_ok());
        for (key, value) in [("sid", "A".repeat(32)), ("sid", "a".repeat(64)),
            ("name", "Secret archive title".into()), ("bytes", "128".into()),
            ("chunks", "1".into()), ("awdl", "0".into()), ("token", "secret".into()),
            ("pin", "1234".into()), ("tp", "quic".into())] {
            let mut txt = library_txt(); txt.insert(key.into(), value);
            assert!(parse_peer(&library_info(txt, &ips), "self-peer").is_none(), "{key}");
        }
        assert!(parse_peer(&library_info(library_txt(), &ips[1..]), "self-peer").is_some());
        assert!(parse_peer(&library_info(library_txt(), &ips), "remote-peer").is_none());
    }
    #[test]
    fn apple_awdl_capability_keeps_its_numeric_lan_library_discoverable() {
        let mut txt = library_txt();
        txt.insert("awdl".into(), "1".into());
        txt.insert("device".into(), "Apple sender".into());
        let ips = ["192.168.1.20".parse().unwrap()];
        let peer = parse_peer(&library_info(txt.clone(), &ips), "self-peer").unwrap();
        assert_eq!(peer.transport, "quic-library");
        assert_eq!(peer.host, "192.168.1.20");
        assert!(library_peer(peer).is_ok());
        for value in ["0", "true", "2", ""] {
            txt.insert("awdl".into(), value.into());
            assert!(parse_peer(&library_info(txt.clone(), &ips), "self-peer").is_none());
        }
        txt.insert("awdl".into(), "1".into());
        assert!(parse_peer(&library_info(txt.clone(), &["127.0.0.1".parse().unwrap()]), "self-peer").is_none());
        txt.insert("unrecognized".into(), "1".into());
        assert!(parse_peer(&library_info(txt, &ips), "self-peer").is_none());
    }
    #[tokio::test]
    async fn library_advertisement_survives_browse_off_but_not_withdrawal_or_shutdown() {
        let mut discovery = Discovery::new("self-peer".into()).unwrap();
        discovery.advertisements.insert("owned-library".into());
        discovery.stop_browse().await.unwrap();
        assert!(discovery.advertisements.contains("owned-library"));
        assert!(discovery.daemon.is_none());
        discovery.unadvertise("owned-library").await.unwrap();
        assert!(discovery.advertisements.is_empty());
        discovery.advertisements.insert("shutdown-library".into());
        discovery.shutdown().await.unwrap();
        assert!(discovery.advertisements.is_empty());
        assert!(discovery.peers().is_empty());
    }
    #[tokio::test]
    async fn invalid_library_listener_never_opens_discovery_and_stale_snapshot_is_rejected() {
        let mut discovery = Discovery::new("self-peer".into()).unwrap();
        for address in ["::1", "fe80::1", "127.0.0.1", "0.0.0.0", "169.254.1.1", "not-an-ip"] {
            assert!(discovery.advertise_library(&"a".repeat(32), 8090, 1, &[address.into()], "Test sender", 1, false).await.is_err());
        }
        assert!(discovery.daemon.is_none());
        assert!(discovery.advertisements.is_empty());
        let peer = parse_peer(&library_info(library_txt(), &["192.168.1.20".parse().unwrap()]), "self-peer").unwrap();
        let id = peer.id.clone();
        discovery.peers.lock().unwrap().insert(id.clone(), peer);
        assert!(discovery.library_peer(&id).is_err());
        discovery.shutdown().await.unwrap();
        assert!(discovery.peers().is_empty());
    }
}
