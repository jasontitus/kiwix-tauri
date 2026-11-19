// SPDX-License-Identifier: GPL-3.0-or-later
//! Narrow byte-stream bridge. Native code owns QUIC/AWDL; Rust owns the protocol.
use std::{ffi::{c_char, c_void, CString}, ptr::NonNull, sync::{Arc, LazyLock}, time::Duration};
use block2::{Block, RcBlock};
use dispatch2::DispatchData;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

mod read_request;

type Completion = Block<dyn Fn(i32, usize)>;
unsafe extern "C" {
    fn kiwix_swarm_identity(p12: *const u8, length: usize, der: *const u8, der_length: usize) -> bool;
    fn kiwix_swarm_connect(instance: *const c_char, peer_to_peer: bool) -> *mut c_void;
    fn kiwix_swarm_connect_on_interface(instance: *const c_char, peer_to_peer: bool, required_index: u32) -> *mut c_void;
    fn kiwix_swarm_interface_index(raw: *mut c_void) -> u32;
    fn kiwix_swarm_local_address(raw: *mut c_void, buffer: *mut u8, capacity: usize) -> usize;
    fn kiwix_swarm_interface_has_address(index: u32, bytes: *const u8, length: usize) -> bool;
    fn kiwix_swarm_start(raw: *mut c_void, callback: &Completion);
    fn kiwix_swarm_read(raw: *mut c_void, maximum: usize, request: u64, callback: extern "C" fn(u64, i32, Option<&dispatch2::DispatchData>));
    fn kiwix_swarm_write(raw: *mut c_void, data: &DispatchData, callback: &Completion);
    fn kiwix_swarm_cancel(raw: *mut c_void);
    fn kiwix_swarm_join(raw: *mut c_void);
    fn kiwix_swarm_release(raw: *mut c_void);
    fn kiwix_swarm_route(raw: *mut c_void, buffer: *mut c_char, capacity: usize) -> isize;
    fn kiwix_swarm_listen(accept: &Block<dyn Fn(*mut c_void)>, ready: &Completion, one_request: bool) -> *mut c_void;
    fn kiwix_swarm_listener_cancel(raw: *mut c_void);
    fn kiwix_swarm_listener_join(raw: *mut c_void);
    fn kiwix_swarm_listener_release(raw: *mut c_void);
}

fn identity() -> Result<(), String> {
    static IDENTITY: LazyLock<Result<(), String>> = LazyLock::new(|| {
        let (certificates, _) = super::network::identity()?;
        let p12 = include_bytes!("../../native/localswarm.p12");
        let certificate = certificates[0].as_ref();
        if unsafe { kiwix_swarm_identity(p12.as_ptr(), p12.len(), certificate.as_ptr(), certificate.len()) } { Ok(()) }
        else { Err("Cannot import the pinned LocalSwarm Apple identity".into()) }
    });
    IDENTITY.clone()
}

// Persist an opaque local service token, never a hostname sent to Internet DNS.
// Hex preserves the UTF-8 instance without DNS escaping.
pub fn service_host(instance: &str, peer_to_peer: bool) -> String {
    format!("{}{}", if peer_to_peer { "bonjour:" } else { "bonjour-lan:" }, hex::encode(instance.as_bytes()))
}
pub fn service_instance(host: &str) -> Result<CString, String> {
    let encoded = host.strip_prefix("bonjour:").or_else(|| host.strip_prefix("bonjour-lan:"))
        .ok_or("Not a Bonjour endpoint")?;
    let bytes = hex::decode(encoded).map_err(|_| "Invalid Bonjour endpoint")?;
    if bytes.is_empty() || bytes.len() > 63 || std::str::from_utf8(&bytes).is_err() {
        return Err("Invalid Bonjour service instance".into());
    }
    CString::new(bytes).map_err(|_| "Invalid Bonjour service instance".into())
}
pub(super) fn interface_has_address(index: u32, address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            let bytes = address.octets();
            unsafe { kiwix_swarm_interface_has_address(index, bytes.as_ptr(), bytes.len()) }
        },
        std::net::IpAddr::V6(address) => {
            let bytes = address.octets();
            unsafe { kiwix_swarm_interface_has_address(index, bytes.as_ptr(), bytes.len()) }
        },
    }
}

/// Largest single native read. The bridge re-enters its serial dispatch queue
/// per read, so a 16 KiB cap bounded the whole transfer at single-digit MB/s.
const MAX_READ: usize = 256 * 1024;

// block2 erases these bounds from its block type. Check them before handing a
// closure to the native dispatch queue, which can invoke and dispose it there.
fn queue_callback<F: Send + Sync + 'static>(callback: F) -> F { callback }

enum Operation {
    Ready,
    Write { bytes: usize, stats: Option<Arc<super::network::SeedStats>> },
}

fn completion(operation: Operation) ->
    (RcBlock<dyn Fn(i32, usize)>, oneshot::Receiver<Result<usize, String>>)
{
    let (reply, received) = oneshot::channel();
    let reply = std::sync::Mutex::new(Some(reply));
    let block = RcBlock::new(queue_callback(move |status: i32, length: usize| {
        let sender = reply.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(sender) = sender else { return; };
        let result = if status != 0 {
            Err("LocalSwarm Apple QUIC connection failed or left the local link".into())
        } else if let Operation::Write { bytes, stats } = &operation {
            if *bytes == length {
                // Accepted pieces count even after the serving loop drops its receipt.
                if let Some(stats) = stats { stats.record_upload(length as u64); }
                Ok(length)
            } else {
                Err("Apple QUIC send completion length mismatch".into())
            }
        } else {
            Ok(length)
        };
        let _ = sender.send(result);
    }));
    (block, received)
}

struct RawConnection(NonNull<c_void>);
// SAFETY: `RawConnection` owns the only reference to its native connection and
// no Rust code dereferences or mutates the pointee. The native bridge accepts
// these calls from any thread and serializes them on its own Network.framework
// queue, so neither `Send` nor `Sync` can produce concurrent native access.
// Enforced by the field being private to this module (only the `Connection`
// methods below call the bridge) and by `Drop` releasing the handle exactly
// once, after the last `Arc` clone is gone.
unsafe impl Send for RawConnection {}
unsafe impl Sync for RawConnection {}
impl Drop for RawConnection {
    fn drop(&mut self) { unsafe { kiwix_swarm_cancel(self.0.as_ptr()); kiwix_swarm_release(self.0.as_ptr()); } }
}
#[derive(Clone)]
pub struct Connection(Arc<RawConnection>);
impl Connection {
    fn start(raw: NonNull<c_void>) -> (Self, oneshot::Receiver<Result<usize, String>>) {
        let connection = Self(Arc::new(RawConnection(raw)));
        let (callback, ready) = completion(Operation::Ready);
        unsafe { kiwix_swarm_start(raw.as_ptr(), &callback); }
        (connection, ready)
    }
    /// Longest infrastructure attempt. An idle local network answers in
    /// milliseconds; this bound only covers a missing or wedged route.
    const ROUTE_TIMEOUT: Duration = Duration::from_secs(8);
    /// Direct mode bounds the AWDL attempt so a device that is not currently
    /// reachable peer-to-peer cannot hide a working infrastructure route.
    const DIRECT_ROUTE_TIMEOUT: Duration = Duration::from_secs(4);
    pub async fn connect(host: &str, cancel: &CancellationToken, direct: bool) -> Result<(Self, &'static str), String> {
        Self::connect_impl(host, cancel, direct, None, false).await
    }
    pub(super) async fn connect_on_interface(host: &str, cancel: &CancellationToken, direct: bool, required_index: Option<u32>) -> Result<(Self, &'static str), String> {
        Self::connect_impl(host, cancel, direct, required_index, true).await
    }
    async fn connect_impl(host: &str, cancel: &CancellationToken, direct: bool, required_index: Option<u32>, owns_transport: bool) -> Result<(Self, &'static str), String> {
        identity()?;
        let instance = service_instance(host)?;
        // Peer-to-peer is AWDL/llw and infrastructure is Wi-Fi/Ethernet. The
        // default order is unchanged: infrastructure first (the fast path when
        // both peers share a network), then AWDL for routerless use. Direct
        // mode inverts that with a short AWDL bound, then falls back.
        let routes: &[(bool, Duration)] = if direct {
            &[(true, Self::DIRECT_ROUTE_TIMEOUT), (false, Self::ROUTE_TIMEOUT)]
        } else if host.starts_with("bonjour:") {
            &[(false, Self::ROUTE_TIMEOUT), (true, Self::ROUTE_TIMEOUT)]
        } else {
            &[(false, Self::ROUTE_TIMEOUT)]
        };
        let mut last_error = "No local peer route is available".to_string();
        for &(peer_to_peer, bound) in routes {
            if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
            let raw = NonNull::new(unsafe {
                if owns_transport {
                    kiwix_swarm_connect_on_interface(instance.as_ptr(), peer_to_peer, required_index.unwrap_or(0))
                } else {
                    kiwix_swarm_connect(instance.as_ptr(), peer_to_peer)
                }
            })
                .ok_or("Cannot create Apple QUIC connection")?;
            let (connection, ready) = Self::start(raw);
            let result = super::network::guarded(cancel, async {
                tokio::time::timeout(bound, ready).await
                    .map_err(|_| if peer_to_peer {
                        "No nearby peer-to-peer connection; trying the local network".to_string()
                    } else {
                        "No nearby peer connection; trying the local network".to_string()
                    })?
                    .map_err(|_| "Apple QUIC readiness callback lost".to_string())?
            }).await;
            match result {
                // The successful native attempt is itself route evidence: the
                // peer-to-peer adapter only accepts AWDL/llw interfaces and
                // requires that interface, while infrastructure excludes them.
                Ok(_) => {
                    if required_index.is_some_and(|expected| connection.interface_index().ok() != Some(expected)) {
                        connection.shutdown().await;
                        return Err("The selected native QUIC interface changed".into());
                    }
                    return Ok((connection, if peer_to_peer { "peer-to-peer" } else { "infrastructure" }));
                }
                Err(error) => {
                    connection.shutdown().await;
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }
    pub(super) fn interface_index(&self) -> Result<u32, String> {
        let index = unsafe { kiwix_swarm_interface_index(self.0.0.as_ptr()) };
        if index == 0 { return Err("Native QUIC has no live required interface".into()); }
        Ok(index)
    }
    pub(super) fn local_address(&self) -> Result<std::net::IpAddr, String> {
        let mut bytes = [0; 16];
        match unsafe { kiwix_swarm_local_address(self.0.0.as_ptr(), bytes.as_mut_ptr(), bytes.len()) } {
            4 => Ok(std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).into()),
            16 => {
                let address = std::net::Ipv6Addr::from(bytes);
                Ok(address.to_ipv4_mapped().map_or(std::net::IpAddr::V6(address), std::net::IpAddr::V4))
            },
            _ => Err("Native QUIC has no live effective local address".into()),
        }
    }
    /// Bounded description of the live Network.framework path, for the job
    /// receipt. The native side reports the interface name plus its local
    /// address and the resolved peer address, e.g. `awdl0 192.168.1.5 -> ...`.
    pub fn route(&self) -> Option<String> {
        let mut buffer = [0u8; 256];
        let length = unsafe { kiwix_swarm_route(self.0.0.as_ptr(), buffer.as_mut_ptr().cast(), buffer.len()) };
        let bytes = buffer.get(..usize::try_from(length).ok()?)?;
        let text = std::str::from_utf8(bytes).ok()?.trim();
        (!text.is_empty() && text.len() <= 240 && !text.chars().any(char::is_control)).then(|| text.to_string())
    }
    pub fn close(&self) { unsafe { kiwix_swarm_cancel(self.0.0.as_ptr()); } }
    pub async fn shutdown(&self) {
        self.close();
        let connection = self.clone();
        // The join owns its handle even if this future is dropped.
        let _ = tokio::task::spawn_blocking(move || unsafe { kiwix_swarm_join(connection.0.0.as_ptr()); }).await;
    }
    pub async fn read(&self, output: &mut [u8]) -> Result<Option<usize>, String> {
        if output.is_empty() { return Ok(Some(0)); }
        let maximum = output.len().min(MAX_READ);
        let bytes = loop {
            let (ticket, received) = read_request::register(maximum)?;
            // The native operation retains the connection and only echoes the ID.
            unsafe { kiwix_swarm_read(self.0.0.as_ptr(), maximum, ticket.id(), read_request::complete); }
            match received.await.map_err(|_| "Apple QUIC receive callback lost")?? {
                read_request::Received::Bytes(bytes) => break bytes,
                read_request::Received::NextContext => continue,
            }
        };
        if bytes.is_empty() { return Ok(None); }
        output.get_mut(..bytes.len()).ok_or("Apple QUIC receive exceeded its request")?.copy_from_slice(&bytes);
        Ok(Some(bytes.len()))
    }
    /// Hand every piece of `input` to Network.framework without waiting for the
    /// sends to be accepted, returning one completion per piece in wire order.
    /// The dispatch data owns a synchronous copy, so `input` may be released on
    /// return. Calls made from one task stay ordered on the QUIC stream, which
    /// lets a seeder keep several chunk responses in flight instead of paying a
    /// send-completion round trip per piece.
    pub fn dispatch_write(&self, mut input: &[u8], stats: Option<Arc<super::network::SeedStats>>)
        -> Result<Vec<oneshot::Receiver<Result<usize, String>>>, String>
    {
        let mut completions = Vec::with_capacity(input.len().div_ceil(super::network::UNLIMITED_PIECE));
        while !input.is_empty() {
            let length = input.len().min(super::network::UNLIMITED_PIECE);
            let data = DispatchData::from_bytes(&input[..length]);
            let (callback, sent) = completion(Operation::Write { bytes: length, stats: stats.clone() });
            unsafe { kiwix_swarm_write(self.0.0.as_ptr(), &data, &callback); }
            completions.push(sent);
            input = &input[length..];
        }
        Ok(completions)
    }
    pub async fn write_all(&self, input: &[u8]) -> Result<(), String> {
        for sent in self.dispatch_write(input, None)? {
            sent.await.map_err(|_| "Apple QUIC send callback lost")??;
        }
        Ok(())
    }
}

type Incoming = (Connection, oneshot::Receiver<Result<usize, String>>);
struct RawListener(NonNull<c_void>);
// SAFETY: as for `RawConnection`, the listener handle is owned here and Rust
// never dereferences it. The native bridge serializes accept, cancel and
// release on its own Network.framework queue, so sharing or moving the handle
// cannot create concurrent native access. Enforced by the private field (only
// this module calls the listener bridge) and by `Drop` releasing it exactly
// once, after the last `Arc` clone held by `Listener` is gone.
unsafe impl Send for RawListener {}
unsafe impl Sync for RawListener {}
impl Drop for RawListener {
    fn drop(&mut self) { unsafe { kiwix_swarm_listener_cancel(self.0.as_ptr()); kiwix_swarm_listener_release(self.0.as_ptr()); } }
}
pub struct Listener { raw: Arc<RawListener>, incoming: mpsc::Receiver<Incoming>, pub port: u16 }
impl Listener {
    pub async fn new(cancel: &CancellationToken) -> Result<Self, String> {
        Self::new_inner(cancel, false).await
    }
    /// Library completion retires exactly one request's QUIC transport, not
    /// merely its stream. Legacy swarm listeners retain multiplexed semantics.
    pub(super) async fn new_library(cancel: &CancellationToken) -> Result<Self, String> {
        Self::new_inner(cancel, true).await
    }
    async fn new_inner(cancel: &CancellationToken, one_request: bool) -> Result<Self, String> {
        identity()?;
        let (sender, incoming) = mpsc::channel(16);
        let (raw, ready) = {
            let accept = RcBlock::new(queue_callback(move |raw: *mut c_void| {
                if let Some(raw) = NonNull::new(raw) {
                    // A full or closed channel drops the incoming connection and cancels it.
                    let _ = sender.try_send(Connection::start(raw));
                }
            }));
            let (callback, ready) = completion(Operation::Ready);
            (unsafe { kiwix_swarm_listen(&accept, &callback, one_request) }, ready)
        };
        let raw = NonNull::new(raw).ok_or("Cannot create Apple QUIC listener")?;
        let mut listener = Self { raw: Arc::new(RawListener(raw)), incoming, port: 0 };
        let result = super::network::guarded(cancel, async { ready.await.map_err(|_| "Apple QUIC listener callback lost".to_string())? }).await;
        match result.and_then(|port| u16::try_from(port).ok().filter(|port| *port != 0).ok_or("Invalid Apple QUIC listener port".into())) {
            Ok(port) => { listener.port = port; Ok(listener) }
            Err(error) => { listener.shutdown().await; Err(error) }
        }
    }
    pub async fn accept(&mut self) -> Option<Incoming> { self.incoming.recv().await }
    pub fn close(&self) { unsafe { kiwix_swarm_listener_cancel(self.raw.0.as_ptr()); } }
    pub async fn shutdown(&mut self) {
        self.close();
        let raw = self.raw.clone();
        let _ = tokio::task::spawn_blocking(move || unsafe { kiwix_swarm_listener_join(raw.0.as_ptr()); }).await;
        self.incoming.close();
        while let Some((connection, _)) = self.incoming.recv().await { connection.shutdown().await; }
    }
}
impl Drop for Listener { fn drop(&mut self) { self.close(); } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn write_completion_is_counted_once_even_if_the_receiver_leaves() {
        let stats = Arc::new(super::super::network::SeedStats::default());
        let (callback, receiver) = completion(Operation::Write { bytes: 17, stats: Some(stats.clone()) });
        drop(receiver);
        callback.call((0, 17));
        callback.call((0, 17));
        assert_eq!(stats.uploaded(), 17);
        for (status, length) in [(-1, 17), (0, 18)] {
            let (callback, receiver) = completion(Operation::Write { bytes: 17, stats: Some(stats.clone()) });
            callback.call((status, length));
            assert!(receiver.blocking_recv().unwrap().is_err());
        }
        assert_eq!(stats.uploaded(), 17);
    }

    #[test]
    fn readiness_follows_the_retained_block_lifetime() {
        let (callback, ready) = completion(Operation::Ready);
        let retained = callback.clone();
        drop(callback);
        retained.call((0, 4242));
        retained.call((-1, 0));
        assert_eq!(ready.blocking_recv().unwrap(), Ok(4242));

        let (callback, ready) = completion(Operation::Ready);
        drop(callback);
        assert!(ready.blocking_recv().is_err());
    }

    #[test]
    fn service_tokens_are_local_instances_not_dns_targets() {
        let name = "Archive. café";
        for direct in [true, false] {
            assert_eq!(service_instance(&service_host(name, direct)).unwrap().to_str().unwrap(), name);
        }
        for invalid in ["example.com", "bonjour:", "bonjour:00", "bonjour:ff", "bonjour:zz"] { assert!(service_instance(invalid).is_err()); }
        assert!(service_instance(&service_host(&"a".repeat(64), true)).is_err());
    }
    /// The production identity bridge (no network): the pinned certificate
    /// pairs with its in-memory key and that key signs what the certificate
    /// verifies. Runs in the macOS CI test step (PRs and main); iOS uses
    /// PKCS#12 instead. It does not perform a TLS handshake.
    #[cfg(target_os = "macos")]
    #[test]
    fn pinned_identity_is_created_in_memory_and_signs() {
        identity().expect("Pinned LocalSwarm identity must import and sign");
    }
}

#[cfg(test)]
#[path = "apple_transport_interop_tests.rs"]
mod interop_tests;

#[cfg(test)]
#[tokio::test]
#[ignore = "Exercises real native QUIC and Bonjour on an active local interface"]
async fn native_route_detail_owns_the_enumerated_interface_name() {
    native_library_regression(false).await;
}
#[cfg(test)]
#[tokio::test]
#[ignore = "Exercises concurrent real native QUIC library requests on an active local interface"]
async fn native_library_request_completion_preserves_other_requests() {
    native_library_regression(true).await;
}

#[cfg(test)]
#[tokio::test]
#[ignore = "Exercises native QUIC handshake-only admission on an active local interface"]
async fn native_listener_retires_a_peer_that_never_opens_a_request_stream() {
    let cancel = CancellationToken::new();
    let mut listener = Listener::new_library(&cancel).await.unwrap();
    let peer = uuid::Uuid::new_v4().to_string();
    let session = uuid::Uuid::new_v4().simple().to_string();
    let mut discovery = super::discovery::Discovery::new(peer.clone()).unwrap();
    let advertisement = discovery.advertise_library(
        &session, listener.port, 1, &[], "Native admission test", 1, true,
    ).await.unwrap();
    let mut client = None;
    let result = async {
        let endpoint = service_host(&super::discovery::library_instance(&peer, &session), false);
        client = Some(Connection::connect_on_interface(&endpoint, &cancel, false, None).await?.0);
        let connection = client.as_ref().unwrap();
        // Complete TLS but send no stream bytes. The native producer must
        // retire this admitted transport without a Rust request consumer.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        if let Some((unexpected, _)) = incoming {
                            unexpected.shutdown().await;
                            return Err("Handshake-only peer produced an application request".to_string());
                        }
                        return Err("The admission deadline stopped the entire listener".to_string());
                    },
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        if connection.route().is_none() { return Ok(()); }
                    },
                }
            }
        }).await.map_err(|_| "Handshake-only native transport outlived its admission deadline".to_string())?
    }.await;
    if let Some(client) = client { client.shutdown().await; }
    listener.shutdown().await;
    let withdrawal = discovery.unadvertise(&advertisement).await;
    let shutdown = discovery.shutdown().await;
    result.unwrap();
    withdrawal.unwrap(); shutdown.unwrap();
}

#[cfg(test)]
async fn native_library_regression(concurrent: bool) {
    use super::{library_wire as wire, network::{SendStream, RecvStream}};
    struct TestDirectory(std::path::PathBuf);
    impl Drop for TestDirectory { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let root = TestDirectory(std::env::temp_dir().canonicalize().unwrap()
        .join(format!("kiwix-native-route-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&root.0).unwrap();
    let path = root.0.join("library.zim");
    std::fs::copy(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &path).unwrap();
    let expected = if concurrent { std::fs::read(&path).unwrap() } else { Vec::new() };
    let book = crate::store::Book {
        metadata: crate::archive::Archive::open(&path).unwrap().metadata().unwrap(),
        installation_id: crate::store::new_installation_id(), path: path.to_string_lossy().into_owned(),
        source_reference: None, added_at: "2026-01-01T00:00:00Z".into(), available: true,
        managed: false, integrity: None,
    };
    let book_id = book.metadata.id.clone();
    let store = Arc::new(crate::store::Store::new(root.0.join("profile")).unwrap());
    store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
    let cancel = CancellationToken::new();
    let mut listener = Listener::new_library(&cancel).await.unwrap();
    let port = listener.port;
    let peer = uuid::Uuid::new_v4().to_string();
    let session = uuid::Uuid::new_v4().simple().to_string();
    let mut discovery = super::discovery::Discovery::new(peer.clone()).unwrap();
    let advertisement = discovery.advertise_library(&session, listener.port, 1, &[], "Native route test", 1, true).await.unwrap();
    let server_cancel = cancel.clone();
    let server_peer = peer.clone(); let server_session = session.clone();
    let mut server = tokio::spawn(async move {
        // Network.framework delivers multiplexed streams, not just one
        // accepted object per QUIC transport. Match the production accept loop.
        let peers_cancel = server_cancel.child_token();
        let mut peers = tokio::task::JoinSet::<Result<(), String>>::new();
        let mut completed_requests = 0;
        let expected_requests = if concurrent { 3 } else { 1 };
        let result = loop {
            tokio::select! {
                _ = server_cancel.cancelled() => break Err("Native inventory test retired".into()),
                completed = peers.join_next(), if !peers.is_empty() => {
                    match completed {
                        Some(Ok(Ok(()))) => {
                            completed_requests += 1;
                            if completed_requests == expected_requests { break Ok(()); }
                        },
                        Some(Err(error)) => break Err(error.to_string()),
                        _ => {},
                    }
                },
                incoming = listener.accept() => {
                    let Some((connection, ready)) = incoming else { break Err("Native listener stopped".into()); };
                    if peers.len() >= 16 { connection.shutdown().await; continue; }
                    let store = store.clone(); let peer = server_peer.clone(); let session = server_session.clone();
                    let cancel = peers_cancel.child_token();
                    peers.spawn(async move {
                        let result = async {
                            super::network::guarded(&cancel, async {
                                ready.await.map_err(|_| "Native readiness callback lost".to_string())?
                            }).await?;
                            super::library::test_serve_native_request(store, peer, session,
                                "Native route test".into(), SendStream::Apple(connection.clone()),
                                RecvStream::Apple(connection.clone()), cancel).await
                        }.await;
                        connection.shutdown().await;
                        result
                    });
                },
            }
        };
        peers_cancel.cancel();
        listener.shutdown().await;
        while peers.join_next().await.is_some() {}
        result
    });
    let mut client = None;
    let exchange = tokio::time::timeout(Duration::from_secs(20), async {
        let endpoint = service_host(&super::discovery::library_instance(&peer, &session), false);
        client = Some(Connection::connect_on_interface(&endpoint, &cancel, false, None).await?.0);
        let connection = client.as_ref().ok_or("Native client missing")?;
        for _ in 0..64 {
            let route = connection.route().ok_or("A live native route returned no valid owned UTF-8 evidence")?;
            if !(route.contains(" (infrastructure)") || route.contains(" (peer-to-peer)")) || route.chars().any(char::is_control) {
                return Err("Native route text is invalid".to_string());
            }
            tokio::task::yield_now().await;
        }
        let mut send = SendStream::Apple(connection.clone());
        let mut recv = RecvStream::Apple(connection.clone());
        let request = wire::Request { version: wire::VERSION, session_id: session.clone(), peer_id: peer.clone(),
            book_id: None, range: None, if_range: None };
        wire::send(&mut send, &request, wire::CONTROL_LIMIT, &cancel).await?;
        let reply: wire::Reply = wire::receive(&mut recv, wire::INVENTORY_LIMIT, &cancel).await?;
        if reply.status != 200 || reply.session_id != session || reply.peer_id != peer {
            return Err("Native inventory response identity changed".to_string());
        }
        let inventory = reply.inventory.ok_or("Native inventory is missing")?;
        if inventory.books.len() != 1 || inventory.books[0].id != book_id {
            return Err("Native inventory did not contain its installed archive".to_string());
        }
        let descriptor = if concurrent {
            let authority = super::library::LibraryRouteAuthority::default();
            authority.retain_native(connection.interface_index()?, connection.local_address()?)?;
            Some(super::library::LibraryPeerDescriptor { session_id: session.clone(), peer_id: peer.clone(),
                name: "Native route test".into(), port, addresses: Vec::new(),
                native_services: vec![service_host(&super::discovery::library_instance(&peer, &session), true)],
                direct: false, route: None, cancel: cancel.clone(), authority })
        } else { None };
        // Route getters require the live transport: capture before this ACK
        // permits the production server to retire the inventory stream.
        super::network::guarded(&cancel, send.write_all(&[1])).await?;
        if let Some(descriptor) = descriptor {
            // Retire browsing exactly as production does before accepted jobs dial.
            let mut end = [0];
            let _ = super::network::guarded(&cancel, recv.read(&mut end)).await;
            drop(send); drop(recv);
            if let Some(connection) = client.take() { connection.shutdown().await; }
            async fn read_book(mut response: super::library::LibraryResponse) -> Result<Vec<u8>, String> {
                if response.status() != reqwest::StatusCode::OK { return Err("Native book request was not a full response".into()); }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? { bytes.extend_from_slice(&chunk); }
                drop(response);
                Ok(bytes)
            }
            let (first_response, second_response) = tokio::try_join!(
                super::library::open(&descriptor, &book_id, reqwest::header::HeaderMap::new(), cancel.child_token()),
                super::library::open(&descriptor, &book_id, reqwest::header::HeaderMap::new(), cancel.child_token()),
            )?;
            // Both requests have live response streams. Finish and drop the
            // first transport before consuming the second stream's payload.
            let first = read_book(first_response).await?;
            let second = read_book(second_response).await?;
            if first != expected || second != expected { return Err("Concurrent native requests returned different archive bytes".into()); }
        }
        (&mut server).await.map_err(|error| error.to_string())??;
        Ok::<(), String>(())
    }).await;
    // Retire actual I/O and the owned advertisement even when the deadline or
    // native exchange fails; assertions below retain the original failure.
    cancel.cancel();
    if let Some(client) = client { client.shutdown().await; }
    if !server.is_finished() && tokio::time::timeout(Duration::from_secs(5), &mut server).await.is_err() {
        server.abort();
    }
    let withdrawal = discovery.unadvertise(&advertisement).await;
    let shutdown = discovery.shutdown().await;
    exchange.expect("Native inventory exchange exceeded its 20-second deadline").expect("Native production inventory exchange failed");
    withdrawal.unwrap(); shutdown.unwrap();
}
