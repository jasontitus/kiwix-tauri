// SPDX-License-Identifier: GPL-3.0-or-later
//! Actual Quinn / Network.framework streams; no TCP or HTTP fallback.
use super::{library::LibraryPeerDescriptor, network::{self, RecvStream, SendStream}};
use crate::transfer_policy;
use std::{collections::HashMap, net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr}, sync::{Arc, LazyLock, Mutex, Weak}};
use tokio_util::sync::CancellationToken;
use futures_util::future::BoxFuture;
const MAX_REQUESTS: usize = 16;
const MAX_PEER_REQUESTS: usize = 2;
// All descriptors/grants for one destination share admission. Four acquisition
// ranges must queue locally rather than race the listener's two per-IP slots.
static CLIENT_REQUESTS: LazyLock<Mutex<HashMap<IpAddr, Weak<tokio::sync::Semaphore>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
async fn request_slot(peer: &LibraryPeerDescriptor, address: IpAddr, cancel: &CancellationToken,
    policy_revision: Option<u64>) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    let slots = {
        let mut requests = CLIENT_REQUESTS.lock().unwrap_or_else(|error| error.into_inner());
        requests.retain(|_, slots| slots.strong_count() != 0);
        let entry = requests.entry(address.to_canonical()).or_default();
        entry.upgrade().unwrap_or_else(|| {
            let slots = Arc::new(tokio::sync::Semaphore::new(MAX_PEER_REQUESTS));
            *entry = Arc::downgrade(&slots); slots
        })
    };
    let joined = peer.route.as_ref().map(|route| route.token()).unwrap_or_default();
    // Queueing is not a stalled socket: a legitimate range can take longer
    // than the I/O deadline. Cancellation and route revocation still interrupt it.
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("LocalSwarm operation cancelled".into()),
        _ = peer.cancel.cancelled() => Err("Nearby share retired".into()),
        _ = joined.cancelled() => Err("Nearby joined route retired".into()),
        _ = local_policy_lost(policy_revision) => Err("Nearby local policy changed".into()),
        _ = route_lost(peer.authority.clone()) => Err("Nearby route retired".into()),
        permit = slots.acquire_owned() => permit.map_err(|error| error.to_string()),
    }
}
struct PeerSlot { peers: Arc<Mutex<HashMap<IpAddr, usize>>>, peer: IpAddr }
impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut peers = self.peers.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(count) = peers.get_mut(&self.peer) {
            *count -= 1;
            if *count == 0 { peers.remove(&self.peer); }
        }
    }
}
pub(super) type Handler = Arc<dyn Fn(SendStream, RecvStream, CancellationToken) -> BoxFuture<'static, ()> + Send + Sync>;
pub(super) struct Transport {
    pub send: SendStream, pub recv: RecvStream,
    connection: Option<quinn::Connection>,
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    native: Option<super::apple_transport::Connection>,
    cancel: CancellationToken, _watch: tokio::task::JoinHandle<()>,
    _route: Option<crate::transfer_policy::JoinedRoute>,
}
impl Drop for Transport {
    fn drop(&mut self) {
        // Abandon the stream, not the connection: let the listener observe the
        // reset/STOP_SENDING and release admission before another range dials.
        match &mut self.send {
            SendStream::Quinn(send) => { let _ = send.reset(0u32.into()); },
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            SendStream::Apple(_) => { self._watch.abort(); },
        }
        match &mut self.recv {
            RecvStream::Quinn(recv) => { let _ = recv.stop(0u32.into()); },
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            RecvStream::Apple(_) => {},
        }
        self.cancel.cancel();
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if let Some(native) = &self.native { native.close(); }
    }
}
fn successful_remote_close(reason: &quinn::ConnectionError, cancel: &CancellationToken) -> bool {
    if cancel.is_cancelled() { return false; }
    match reason {
        quinn::ConnectionError::ApplicationClosed(close) => close.error_code == 0u32.into(),
        // Network.framework closes the owning QUIC group with a remote
        // CONNECTION_CLOSE(NO_ERROR). After validated payload and application
        // ACK this is completion too; a local TransportError(0) is not.
        quinn::ConnectionError::ConnectionClosed(close) => close.error_code == quinn::TransportErrorCode::NO_ERROR,
        _ => false,
    }
}
impl Transport {
    pub(super) async fn complete(&mut self, cancel: &CancellationToken) -> Result<(), String> {
        // The application ACK proves every response byte was consumed. A
        // transport-level write/FIN acknowledgement alone cannot prove that.
        network::guarded(cancel, self.send.write_all(&[1])).await?;
        match &mut self.send {
            // ACK may already have reached a server that closed the stream.
            // The remote connection-close check below remains authoritative.
            SendStream::Quinn(send) => { let _ = send.finish(); },
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            SendStream::Apple(_) => {},
        }
        let mut trailing = [0];
        match network::guarded(cancel, self.recv.read(&mut trailing)).await {
            Ok(Some(count)) if count != 0 => return Err("Trailing library response payload".into()),
            Err(error) => {
                if cancel.is_cancelled() || error.contains("made no progress") { return Err(error); }
                if let Some(connection) = &self.connection {
                    if !connection.close_reason().is_some_and(|reason| successful_remote_close(&reason, cancel)) {
                        return Err(error);
                    }
                }
            },
            _ => {},
        }
        if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
        if let Some(connection) = &self.connection {
            // The server closes only after consuming ACK and draining work.
            // Never close locally here: that could discard the application ACK.
            let reason = network::guarded(cancel, async { Ok(connection.closed().await) }).await?;
            if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
            return if successful_remote_close(&reason, cancel) { Ok(()) }
                else { Err(format!("Library completion failed: {reason}")) };
        }
        Ok(())
    }
}
pub(super) async fn connect(peer: &LibraryPeerDescriptor, request: &CancellationToken) -> Result<Transport, String> {
    peer.validate()?;
    let native = cfg!(any(target_os = "ios", target_os = "macos")) && !peer.native_services.is_empty();
    let mut policy_revision = if peer.route.is_none() && !native {
        let revision = transfer_policy::local_revision()?;
        transfer_policy::require_local_since(revision)?; Some(revision)
    } else { None };
    let mut local_revision = policy_revision.unwrap_or(0);
    let cancel = request.child_token();
    let lifetime = peer.cancel.clone();
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    if peer.route.is_none() {
        let mut error = None;
        for service in &peer.native_services {
            let result = tokio::select! {
                _ = lifetime.cancelled() => return Err("Nearby share retired".into()),
                result = super::apple_transport::Connection::connect_on_interface(service, &cancel, peer.direct, peer.authority.native_interface()?) => result,
            };
            match result {
                Ok((connection, _)) => {
                    connection.route().ok_or("Native QUIC supplied no exact route receipt")?;
                    peer.authority.retain_native(connection.interface_index()?, connection.local_address()?)?;
                    let authority = peer.authority.clone();
                    let watched = connection.clone(); let stopped = cancel.clone();
                    let watch = tokio::spawn(async move {
                        tokio::select! { _ = stopped.cancelled() => {}, _ = lifetime.cancelled() => {}, _ = local_policy_lost(policy_revision) => {}, _ = route_lost(authority) => {} }
                        watched.close(); stopped.cancel();
                    });
                    return Ok(Transport { send: SendStream::Apple(connection.clone()), recv: RecvStream::Apple(connection.clone()), connection: None, native: Some(connection), cancel, _watch: watch, _route: None });
                },
                Err(e) => error = Some(e),
            }
        }
        if peer.authority.native_interface()?.is_some() {
            return Err(error.unwrap_or_else(|| "The selected native QUIC interface is unavailable".into()));
        }
        if peer.addresses.is_empty() { return Err(error.unwrap_or_else(|| "No nearby QUIC endpoint".into())); }
    }
    if peer.route.is_none() && policy_revision.is_none() {
        local_revision = transfer_policy::local_revision()?;
        transfer_policy::require_local_since(local_revision)?;
        policy_revision = Some(local_revision);
    }
    let addresses: Vec<SocketAddr> = peer.addresses.iter().map(|address| address.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, peer.port)).map_err(|_| "Invalid numeric QUIC endpoint".to_string())).collect::<Result<_, _>>()?;
    let revision = transfer_policy::internet_revision()?;
    let (endpoint, connection, internet, slot) = if let Some(route) = &peer.route {
        route.check()?;
        let mut connected = None; let mut error = "No joined QUIC endpoint".to_string();
        for address in &addresses {
            let slot = request_slot(peer, address.ip(), &cancel, policy_revision).await?;
            let socket = route.bind_udp(*address).await?;
            socket.set_nonblocking(true).map_err(|e| e.to_string())?;
            let mut endpoint = quinn::Endpoint::new(quinn::EndpointConfig::default(), None, socket, Arc::new(quinn::TokioRuntime)).map_err(|e| e.to_string())?;
            endpoint.set_default_client_config(network::client_config()?);
            let attempt = endpoint.connect(*address, "localswarm.local").map_err(|e| e.to_string())?;
            let joined_cancel = route.token();
            let result = tokio::select! {
                _ = lifetime.cancelled() => Err("Nearby share retired".into()),
                _ = joined_cancel.cancelled() => Err("Nearby joined route retired".into()),
                result = network::guarded(&cancel, async { attempt.await.map_err(|e| e.to_string()) }) => result,
            };
            match result {
                Ok(connection) => { route.check()?; connected = Some((endpoint, connection, false, slot)); break; }
                Err(e) => { error = e; endpoint.close(0u32.into(), b"failed"); endpoint.wait_idle().await; }
            }
        }
        connected.ok_or(error)?
    } else {
        let config = network::client_config()?;
        let mut connected = None;
        let mut error = "No nearby QUIC endpoint".to_string();
        for address in &addresses {
            let slot = request_slot(peer, address.ip(), &cancel, policy_revision).await?;
            peer.validate()?;
            let result = tokio::select! {
                _ = lifetime.cancelled() => return Err("Nearby share retired".into()),
                result = network::connect_addresses(std::slice::from_ref(address), &cancel, config.clone(), revision, local_revision) => result,
            };
            let (endpoint, connection, binding) = match result {
                Ok(connected) => connected,
                Err(failure) => { error = failure; continue; }
            };
            if !binding.is_on_link_peer(connection.remote_address().ip())? {
                connection.close(0u32.into(), b"not a local peer");
                endpoint.close(0u32.into(), b"not local");
                endpoint.wait_idle().await;
                error = "Nearby requires a local QUIC route".into();
                continue;
            }
            let local = endpoint.local_addr().map_err(|error| error.to_string())?.ip();
            peer.authority.retain(local_revision, local, (binding, connection.remote_address().ip()))?;
            connected = Some((endpoint, connection, false, slot));
            break;
        }
        connected.ok_or(error)?
    };
    let watched = connection.clone(); let stopped = cancel.clone();
    let joined = peer.route.as_ref().map(|route| route.token()).unwrap_or_default();
    let authority = peer.authority.clone();
    let route = peer.route.clone();
    let watch = tokio::spawn(async move {
        let _route = route;
        let revoked = async {
            tokio::select! { _ = lifetime.cancelled() => {}, _ = joined.cancelled() => {}, _ = local_policy_lost(policy_revision) => {}, _ = network::peer_policy_lost(internet, revision) => {}, _ = route_lost(authority) => {} }
        };
        tokio::pin!(revoked);
        let closed = tokio::select! {
            biased;
            _ = watched.closed() => true,
            _ = &mut revoked => false,
            _ = stopped.cancelled() => {
                // Drop resets both stream directions. Keep its permit until the
                // server processes that cancellation, or the existing I/O bound
                // expires. A retained but poisoned response is bounded too.
                let cleanup = CancellationToken::new();
                tokio::select! {
                    biased;
                    result = network::guarded(&cleanup, async { watched.closed().await; Ok(()) }) => result.is_ok(),
                    _ = &mut revoked => false,
                }
            },
        };
        stopped.cancel();
        // Closing again overwrites Quinn's observed remote close reason with
        // LocallyClosed, racing complete(). A closed endpoint only needs draining.
        if !closed {
            watched.close(0u32.into(), b"request ended");
            endpoint.close(0u32.into(), b"request ended");
        }
        drop(slot);
        endpoint.wait_idle().await;
    });
    let streams = network::guarded(&cancel, async { connection.open_bi().await.map_err(|e| e.to_string()) }).await;
    let (send, recv) = match streams {
        Ok(streams) => streams,
        Err(error) => { cancel.cancel(); return Err(error); },
    };
    Ok(Transport { send: SendStream::Quinn(send), recv: RecvStream::Quinn(recv), connection: Some(connection), #[cfg(any(target_os = "ios", target_os = "macos"))] native: None, cancel, _watch: watch, _route: peer.route.clone() })
}
async fn route_lost(authority: super::library::LibraryRouteAuthority) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if authority.check().is_err() { return; }
    }
}
async fn local_policy_lost(revision: Option<u64>) {
    if let Some(revision) = revision { transfer_policy::local_denied_since(revision).await; }
    else { std::future::pending::<()>().await; }
}
pub(super) struct Listener {
    pub port: u16, pub addresses: Vec<String>, pub cancel: CancellationToken,
    pub native: bool,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Listener {
    pub fn is_finished(&self) -> bool { self.task.as_ref().is_none_or(|task| task.is_finished()) }
    pub async fn shutdown(mut self) { self.cancel.cancel(); if let Some(task) = self.task.take() { let _ = task.await; } }
}
impl Drop for Listener { fn drop(&mut self) { self.cancel.cancel(); } }
pub(super) async fn listen(handler: Handler, parent: CancellationToken, supplied: Option<(std::net::UdpSocket, Vec<String>)>) -> Result<Listener, String> {
    let cancel = parent.child_token();
    let task_cancel = cancel.clone();
    let native = cfg!(all(any(target_os = "ios", target_os = "macos"), not(test)));
    let policy_revision = if supplied.is_none() && !native {
        let revision = transfer_policy::local_revision()?;
        transfer_policy::require_local_since(revision)?; Some(revision)
    } else { None };
    #[cfg(all(any(target_os = "ios", target_os = "macos"), not(test)))]
    if supplied.is_none() {
        let mut listener = super::apple_transport::Listener::new_library(&cancel).await?;
        let port = listener.port;
        let addresses = network::reachable_addresses()?.into_iter().map(|ip| ip.to_string()).collect();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop { tokio::select! {
                _ = task_cancel.cancelled() => break,
                _ = local_policy_lost(policy_revision) => break,
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                incoming = listener.accept() => {
                    let Some((connection, ready)) = incoming else { break; };
                    if tasks.len() >= MAX_REQUESTS { connection.shutdown().await; continue; }
                    let handler = handler.clone(); let cancel = task_cancel.child_token();
                    tasks.spawn(async move {
                        if network::guarded(&cancel, async { ready.await.map_err(|_| "QUIC readiness lost".to_string())? }).await.is_ok() {
                            handler(SendStream::Apple(connection.clone()), RecvStream::Apple(connection.clone()), cancel).await;
                        }
                        connection.shutdown().await;
                    });
                }
            } }
            task_cancel.cancel(); listener.shutdown().await;
            while tasks.join_next().await.is_some() {}
        });
        return Ok(Listener { port, addresses, cancel, native: true, task: Some(task) });
    }
    {
        let (socket, binding, addresses) = if let Some((socket, addresses)) = supplied {
            let local = socket.local_addr().map_err(|e| e.to_string())?;
            if addresses != [local.ip().to_string()] || !local.ip().is_ipv4() || !network::usable_address(local.ip()) {
                return Err("The supplied library socket does not own its advertised address".into());
            }
            (socket, None, addresses)
        } else {
            let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
                .and_then(|socket| { socket.set_only_v6(false)?; socket.bind(&SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0).into())?; Ok(socket) });
            let (socket, ipv6) = match socket {
                Ok(socket) => (std::net::UdpSocket::from(socket), true),
                Err(_) => (std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?, false),
            };
            let binding = transfer_policy::bind_local_socket(&socket)?;
            let addresses = network::reachable_addresses()?.into_iter().filter(|ip| ipv6 || ip.is_ipv4()).map(|ip| ip.to_string()).collect();
            (socket, Some(binding), addresses)
        };
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let endpoint = quinn::Endpoint::new(quinn::EndpointConfig::default(), Some(network::server_config()?), socket, Arc::new(quinn::TokioRuntime)).map_err(|e| e.to_string())?;
        let port = endpoint.local_addr().map_err(|e| e.to_string())?.port();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            let peers = Arc::new(Mutex::new(HashMap::<IpAddr, usize>::new()));
            loop { tokio::select! {
                _ = task_cancel.cancelled() => break,
                _ = local_policy_lost(policy_revision) => break,
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break; };
                    let peer = incoming.remote_address().ip();
                    let local = binding.map_or_else(|| matches!(peer, IpAddr::V4(ip) if ip.is_private() || ip.is_link_local()), |binding| binding.is_on_link_peer(peer).unwrap_or(false));
                    let peer_count = peers.lock().unwrap_or_else(|error| error.into_inner()).get(&peer).copied().unwrap_or(0);
                    if tasks.len() >= MAX_REQUESTS || peer_count >= MAX_PEER_REQUESTS || !local { crate::nearby_log!("[nearby] refuse peer={peer} local={local} tracked={} peer_slots={peer_count}", tasks.len()); incoming.refuse(); continue; }
                    *peers.lock().unwrap_or_else(|error| error.into_inner()).entry(peer).or_insert(0) += 1;
                    let handler = handler.clone(); let cancel = task_cancel.child_token();
                    let slots = peers.clone();
                    tasks.spawn(async move {
                        let _slot = PeerSlot { peers: slots, peer };
                        let Ok(connection) = network::guarded(&cancel, async { incoming.await.map_err(|e| e.to_string()) }).await else { return; };
                        let work = async {
                            let (send, recv) = network::guarded(&cancel, async { connection.accept_bi().await.map_err(|e| e.to_string()) }).await?;
                            let stopped = send.stopped();
                            let serving = handler(SendStream::Quinn(send), RecvStream::Quinn(recv), cancel.clone());
                            tokio::pin!(serving);
                            tokio::select! {
                                // Cancellation interrupts rate credit/I/O, but
                                // blocking source work must finish before its
                                // admission and provider ownership are released.
                                _ = stopped => { cancel.cancel(); serving.await; },
                                _ = &mut serving => {},
                            }
                            Ok::<_, String>(())
                        };
                        tokio::pin!(work);
                        tokio::select! {
                            _ = connection.closed() => { cancel.cancel(); let _ = work.await; },
                            _ = &mut work => {},
                        }
                        // Remote closure is the client's admission handoff.
                        // Release the drained server work before announcing it.
                        drop(_slot);
                        connection.close(0u32.into(), b"request ended");
                    });
                }
            } }
            task_cancel.cancel(); endpoint.close(0u32.into(), b"stopped");
            while tasks.join_next().await.is_some() {}
            endpoint.wait_idle().await;
        });
        Ok(Listener { port, addresses, cancel, native: false, task: Some(task) })
    }
}

#[cfg(test)]
mod completion_tests {
    use super::{successful_remote_close, CancellationToken};
    #[test]
    fn completion_requires_an_exact_uncancelled_remote_no_error_close() {
        let application = |code: u32| quinn::ConnectionError::ApplicationClosed(quinn::ApplicationClose {
            error_code: code.into(), reason: bytes::Bytes::new(),
        });
        let transport = |code| quinn::ConnectionError::ConnectionClosed(quinn::ConnectionClose {
            error_code: code, frame_type: None, reason: bytes::Bytes::new(),
        });
        let cancel = CancellationToken::new();
        assert!(successful_remote_close(&application(0), &cancel));
        assert!(successful_remote_close(&transport(quinn::TransportErrorCode::NO_ERROR), &cancel));
        let cancelled = CancellationToken::new(); cancelled.cancel();
        assert!(!successful_remote_close(&application(0), &cancelled));
        assert!(!successful_remote_close(&transport(quinn::TransportErrorCode::NO_ERROR), &cancelled));
        for reason in [
            application(1),
            transport(quinn::TransportErrorCode::INTERNAL_ERROR),
            // A locally detected transport error with the same numeric code
            // cannot stand in for the peer's CONNECTION_CLOSE frame.
            quinn::ConnectionError::TransportError(quinn::TransportErrorCode::NO_ERROR.into()),
            quinn::ConnectionError::LocallyClosed,
            quinn::ConnectionError::Reset,
            quinn::ConnectionError::TimedOut,
            quinn::ConnectionError::VersionMismatch,
            quinn::ConnectionError::CidsExhausted,
        ] {
            assert!(!successful_remote_close(&reason, &cancel), "Accepted non-completion closure: {reason:?}");
        }
    }
}
