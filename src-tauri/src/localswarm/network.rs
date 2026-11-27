//! Private, exact-pinned QUIC transport. The shared identity is not publisher trust.
#![forbid(unsafe_code)]
use super::protocol::{self, Manifest, Message};
use crate::transfer_policy::{self, LocalBinding};
use quinn::{Connection, Endpoint};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, atomic::{AtomicU64, AtomicUsize, Ordering}};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

const IO_TIMEOUT: Duration = Duration::from_secs(45);
// A seeder may be blocked by the receiver's own prefetch/disk backpressure while
// writing a chunk. Give each chunk write the same bounded allowance the receive
// side already grants a payload (MAX_SHARED_CONSUMERS + IO_TIMEOUT).
const CHUNK_WRITE_ALLOWANCE: Duration = Duration::from_secs(MAX_SHARED_CONSUMERS + IO_TIMEOUT.as_secs());
const PIECE: usize = 16 * 1024;
// Unlimited uploads amortize native dispatch/completion overhead without
// queueing a whole (up to 16 MiB) chunk at the Apple bridge.
pub(super) const UNLIMITED_PIECE: usize = 256 * 1024;
const MAX_CONNECTIONS: usize = 16;
const MAX_CONTROLS: usize = 128;
// Receiver window: chunkRequests in flight per connection. The reference
// leecher uses 48; 32 removes the per-chunk round trip while keeping the
// prefetched window bounded for low-end devices. Requests carry no payload, so
// prefetched bytes are bounded separately by PREFETCH_BYTES.
pub(super) const PIPELINE_WINDOW: usize = 32;
// Chunk buffers a pipeline task may hold ahead of the writer, shared by every
// connection of one job (each task also holds the chunk it is handing over).
// Count-bounded by the channel and byte-bounded by PREFETCH_BYTES, so a deeper
// pipe keeps the socket busy through a slow write/fsync without unbounded RAM.
pub(super) const PIPELINE_BUFFERED: usize = 8;
// Hard cap on prefetched chunk payload per receive job: channel contents plus
// chunks already read but not yet written. At most three receive jobs run at
// once, so the worst case is three times this cap.
pub(super) const PREFETCH_BYTES: usize = 64 * 1024 * 1024;
// Accounting unit for PREFETCH_BYTES; permits are acquired per chunk length.
pub(super) const PREFETCH_UNIT: usize = 64 * 1024;
// Seeder window per peer, matching the reference engine: at most this many
// chunk reads/responses outstanding so a pipelined leecher is not serialized
// behind one source read plus one transport send.
const SERVE_WINDOW: usize = 16;
// Hard cap on chunk payload one peer may make the seeder hold: read-ahead plus
// bytes handed to the transport but not yet accepted. A peer requesting every
// chunk of a 127k-chunk share therefore buffers at most this much, never the
// whole file.
const SERVE_BUFFER_BYTES: usize = 16 * 1024 * 1024;
// Current manager bounds: 64 receive jobs * 4 sources; 8 seeds * 16 peers.
// Progress deadlines support a global minimum 1 B/s shared by this population.
const MAX_SHARED_CONSUMERS: u64 = 256;

pub(super) enum SendStream {
    Quinn(quinn::SendStream),
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    Apple(super::apple_transport::Connection),
}
pub(super) enum RecvStream {
    Quinn(quinn::RecvStream),
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    Apple(super::apple_transport::Connection),
}
impl SendStream {
    pub(super) async fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Quinn(stream) => stream.write_all(bytes).await.map_err(|error| error.to_string()),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            Self::Apple(stream) => stream.write_all(bytes).await,
        }
    }
}
impl RecvStream {
    pub(super) async fn read(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, String> {
        match self {
            Self::Quinn(stream) => stream.read(bytes).await.map_err(|error| error.to_string()),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            Self::Apple(stream) => stream.read(bytes).await,
        }
    }
    pub(super) async fn read_exact(&mut self, mut bytes: &mut [u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            let count = self.read(bytes).await?.filter(|count| *count != 0).ok_or("Truncated LocalSwarm stream")?;
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
}

struct BudgetState { limit: u64, tokens: f64, updated: Instant }
pub struct RateBudget { state: Mutex<BudgetState>, turn: AsyncMutex<()>, changed: Notify }
impl RateBudget {
    pub fn new(limit: u64) -> Self {
        Self { state: Mutex::new(BudgetState { limit, tokens: 0.0, updated: Instant::now() }), turn: AsyncMutex::new(()), changed: Notify::new() }
    }
    /// An unlimited budget must not shape the read loop: reserving in 16 KiB
    /// pieces costs a mutex, a timer and an async wake per piece, which capped
    /// real transfers at single-digit MB/s while the transport was idle.
    fn is_unlimited(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).limit == 0
    }
    pub fn set_limit(&self, limit: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.limit == limit { return; }
        state.limit = limit; state.tokens = 0.0; state.updated = Instant::now();
        drop(state); self.changed.notify_waiters();
    }
    #[cfg(test)]
    pub async fn acquire(&self, bytes: usize, cancel: &CancellationToken) -> Result<(), String> {
        let mut remaining = bytes;
        while remaining != 0 { remaining -= self.reserve(remaining, cancel).await?; }
        Ok(())
    }
    // Return a bounded portion as soon as credit exists. A mid-frame limit
    // change must not accumulate a whole old-size piece before making progress.
    pub(super) async fn reserve(&self, maximum: usize, cancel: &CancellationToken) -> Result<usize, String> {
        let _turn = tokio::select! {
            _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
            turn = self.turn.lock() => turn,
        };
        loop {
            if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let delay = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.limit == 0 { return Ok(maximum.min(UNLIMITED_PIECE)); }
                let now = Instant::now();
                let capacity = state.limit.div_ceil(10).min(PIECE as u64).max(1);
                state.tokens = (state.tokens + now.duration_since(state.updated).as_secs_f64() * state.limit as f64)
                    .min(capacity as f64);
                state.updated = now;
                let consumed = maximum.min(PIECE).min(state.tokens.floor() as usize);
                if consumed != 0 { state.tokens -= consumed as f64; return Ok(consumed); }
                Duration::from_secs_f64(((maximum.min(capacity as usize).max(1) as f64 - state.tokens)
                    / state.limit as f64).max(0.001))
            };
            tokio::select! {
                _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
                _ = tokio::time::sleep(delay) => {},
                _ = &mut notified => {},
            }
        }
    }
}

pub(super) fn identity() -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), String> {
    let certificates = CertificateDer::pem_slice_iter(include_bytes!("identity.cert.pem"))
        .collect::<Result<Vec<_>, _>>().map_err(|_| "Invalid LocalSwarm certificate")?;
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("identity.key.pem")).map_err(|error| match error {
        rustls::pki_types::pem::Error::NoItemsFound => "Missing LocalSwarm key",
        _ => "Invalid LocalSwarm key",
    })?;
    if certificates.len() != 1 { return Err("Invalid LocalSwarm certificate chain".into()); }
    Ok((certificates, key))
}

#[derive(Debug)]
struct ExactPin { certificate: CertificateDer<'static>, provider: Arc<rustls::crypto::CryptoProvider> }
impl rustls::client::danger::ServerCertVerifier for ExactPin {
    fn verify_server_cert(&self, leaf: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>, _ocsp: &[u8], _now: UnixTime)
        -> Result<rustls::client::danger::ServerCertVerified, rustls::Error>
    {
        if leaf.as_ref() != self.certificate.as_ref() || !intermediates.is_empty() {
            return Err(rustls::Error::General("LocalSwarm exact certificate pin mismatch".into()));
        }
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, signature: &rustls::DigitallySignedStruct)
        -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
    {
        rustls::crypto::verify_tls12_signature(message, cert, signature, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, signature: &rustls::DigitallySignedStruct)
        -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
    {
        rustls::crypto::verify_tls13_signature(message, cert, signature, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn transport() -> Arc<quinn::TransportConfig> {
    let mut config = quinn::TransportConfig::default();
    config.max_concurrent_bidi_streams(1u32.into());
    config.max_concurrent_uni_streams(0u32.into());
    // One maximum-sized chunk must fit even while the receiver deliberately
    // reads slowly. This avoids flow-control stalls at low download limits.
    config.stream_receive_window((32u32 * 1024 * 1024).into());
    config.receive_window((64u32 * 1024 * 1024).into());
    config.max_idle_timeout(Some(Duration::from_secs(120).try_into().expect("constant idle timeout")));
    config.keep_alive_interval(Some(Duration::from_secs(10)));
    Arc::new(config)
}

pub(super) fn client_config() -> Result<quinn::ClientConfig, String> {
    let (certificates, key) = identity()?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let pin = Arc::new(ExactPin { certificate: certificates[0].clone(), provider: provider.clone() });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13]).map_err(|e| e.to_string())?
        .dangerous().with_custom_certificate_verifier(pin)
        .with_client_auth_cert(certificates, key).map_err(|e| e.to_string())?;
    tls.alpn_protocols = vec![b"localswarm-v1".to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(transport());
    Ok(config)
}

pub(super) fn server_config() -> Result<quinn::ServerConfig, String> {
    let (certificates, key) = identity()?;
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13]).map_err(|e| e.to_string())?
        .with_no_client_auth().with_single_cert(certificates, key).map_err(|e| e.to_string())?;
    tls.alpn_protocols = vec![b"localswarm-v1".to_vec()];
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(transport());
    config.max_incoming(MAX_CONNECTIONS * 2);
    #[cfg(mobile)]
    config.migration(false);
    Ok(config)
}

pub(super) async fn guarded<T>(cancel: &CancellationToken, future: impl Future<Output = Result<T, String>>)
    -> Result<T, String>
{
    guarded_for(cancel, IO_TIMEOUT, future).await
}

async fn guarded_for<T>(cancel: &CancellationToken, allowance: Duration, future: impl Future<Output = Result<T, String>>)
    -> Result<T, String>
{
    tokio::select! {
        _ = cancel.cancelled() => Err("LocalSwarm operation cancelled".into()),
        value = tokio::time::timeout(allowance, future) => value.map_err(|_| "LocalSwarm peer made no progress".to_string())?,
    }
}

async fn read_exact(recv: &mut RecvStream, bytes: &mut [u8], cancel: &CancellationToken) -> Result<(), String> {
    guarded(cancel, async { recv.read_exact(bytes).await.map_err(|e| format!("LocalSwarm read failed: {e}")) }).await
}
async fn read_payload(recv: &mut RecvStream, mut bytes: &mut [u8], cancel: &CancellationToken) -> Result<(), String> {
    let mut progress = PayloadProgress::new(bytes.len(), tokio::time::Instant::now());
    while !bytes.is_empty() {
        if progress.expired(tokio::time::Instant::now()) { return Err("LocalSwarm payload progress deadline exceeded".into()); }
        let count = tokio::select! {
            _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
            result = tokio::time::timeout_at(progress.deadline(), recv.read(bytes)) => result
                .map_err(|_| "LocalSwarm payload progress deadline exceeded".to_string())?
                .map_err(|e| format!("LocalSwarm read failed: {e}"))?
                .ok_or_else(|| "Truncated LocalSwarm payload".to_string())?,
        };
        if count == 0 { return Err("Truncated LocalSwarm payload".into()); }
        progress.received(tokio::time::Instant::now());
        bytes = &mut bytes[count..];
    }
    Ok(())
}

struct PayloadProgress {
    total_deadline: tokio::time::Instant,
    idle_deadline: tokio::time::Instant,
}
impl PayloadProgress {
    fn new(bytes: usize, now: tokio::time::Instant) -> Self {
        Self { total_deadline: now + next_request_allowance(bytes), idle_deadline: now + Self::idle_allowance() }
    }
    fn idle_allowance() -> Duration { Duration::from_secs(MAX_SHARED_CONSUMERS) + IO_TIMEOUT }
    fn received(&mut self, now: tokio::time::Instant) { self.idle_deadline = now + Self::idle_allowance(); }
    fn deadline(&self) -> tokio::time::Instant { self.total_deadline.min(self.idle_deadline) }
    fn expired(&self, now: tokio::time::Instant) -> bool { now >= self.deadline() }
}
async fn write_all(send: &mut SendStream, bytes: &[u8], cancel: &CancellationToken) -> Result<(), String> {
    guarded(cancel, async { send.write_all(bytes).await.map_err(|e| format!("LocalSwarm write failed: {e}")) }).await
}
async fn send_message(send: &mut SendStream, message: &Message, cancel: &CancellationToken) -> Result<(), String> {
    let bytes = message.encode()?;
    write_all(send, &bytes, cancel).await
}

/// True when a session error is the byte stream itself dying — the connection
/// was lost, the peer restarted, or a frame arrived truncated — rather than a
/// protocol, integrity, policy or cancellation decision. Only these failures
/// may be retried by redialing the same peer: no chunk was accepted, so the
/// verified prefix is untouched, and a fresh dial revalidates the local link and
/// repeats the handshake, manifest and bitfield before any request. Everything
/// else (checksum, cross-swarm, flood, auth, policy, cancel) stays terminal.
///
/// A stalled but open stream (`payload progress deadline exceeded`) is
/// deliberately not included: the transport has not reported a loss, so
/// redialing a silent peer would multiply its stall deadline instead of
/// recovering a link flap.
pub(super) fn transient_transport_failure(error: &str) -> bool {
    error.starts_with("LocalSwarm read failed: ") || error.starts_with("LocalSwarm write failed: ")
        || error == "Truncated LocalSwarm payload"
}

// The type is read before allocating, and payload receives are rate-limited in
// small pieces. Waiting for our own bandwidth budget never consumes an I/O deadline.
async fn receive(recv: &mut RecvStream, cancel: &CancellationToken, budget: &RateBudget,
    expected_chunk: Option<(&Manifest, u32)>, allowed: &[u8]) -> Result<Message, String>
{
    let message = receive_frame(recv, cancel, budget, expected_chunk, allowed, true).await?;
    if let (Some((manifest, index)), Message::ChunkResponse { data, .. }) = (expected_chunk, &message) {
        let measured = super::profile::start();
        let matches = chunk_matches(data, &manifest.chunk_hashes[index as usize]);
        super::profile::record(4, data.len(), measured);
        if !matches { return Err("LocalSwarm chunk checksum mismatch".into()); }
    }
    Ok(message)
}

// Framing only: a chunk payload is returned before its SHA-256 check, which
// `receive` performs and which the receive loop runs on the blocking pool while
// the next response is read.
async fn receive_frame(recv: &mut RecvStream, cancel: &CancellationToken, budget: &RateBudget,
    expected_chunk: Option<(&Manifest, u32)>, allowed: &[u8], header_deadline: bool) -> Result<Message, String>
{
    let mut length = [0; 4];
    if header_deadline { read_exact(recv, &mut length, cancel).await?; }
    else {
        tokio::select! {
            _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
            result = recv.read_exact(&mut length) => result.map_err(|e| format!("LocalSwarm read failed: {e}"))?,
        }
    }
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > protocol::MAX_FRAME { return Err("Invalid LocalSwarm frame length".into()); }
    let mut kind = [0]; read_exact(recv, &mut kind, cancel).await?;
    if length > protocol::body_cap(kind[0])? { return Err("LocalSwarm frame exceeds type limit".into()); }
    if !allowed.contains(&kind[0]) { return Err("Unexpected LocalSwarm frame for this session phase".into()); }
    if kind[0] == 3 {
        let mut json_len = [0; 4]; read_exact(recv, &mut json_len, cancel).await?;
        let count = u32::from_be_bytes(json_len) as usize;
        if count > protocol::MAX_MANIFEST || length != 5 + count { return Err("Invalid manifest frame length".into()); }
        let mut json = vec![0; count];
        for piece in json.chunks_mut(PIECE) { read_exact(recv, piece, cancel).await?; }
        return Ok(Message::ManifestResponse(json));
    }
    if kind[0] == 7 {
        let (manifest, expected) = expected_chunk.ok_or("Unsolicited chunk response")?;
        let mut sid_len = [0; 4]; read_exact(recv, &mut sid_len, cancel).await?;
        if u32::from_be_bytes(sid_len) != 64 || length < 77 { return Err("Malformed chunk response".into()); }
        let mut header = [0; 72]; read_exact(recv, &mut header, cancel).await?;
        let index = u32::from_be_bytes(header[64..68].try_into().map_err(|_| "Bad chunk index")?);
        let count = u32::from_be_bytes(header[68..72].try_into().map_err(|_| "Bad chunk length")?) as usize;
        if &header[..64] != manifest.swarm_id.as_bytes() || index != expected
            || count != manifest.chunk_len(index)? || length != 77 + count
        { return Err("Unsolicited or inconsistent chunk response".into()); }
        let mut data = vec![0; count];
        if budget.is_unlimited() {
            read_payload(recv, &mut data, cancel).await?;
        } else {
            let mut offset = 0;
            while offset < data.len() {
                let count = budget.reserve((data.len() - offset).min(PIECE), cancel).await?;
                read_payload(recv, &mut data[offset..offset + count], cancel).await?;
                offset += count;
            }
        }
        // The chunk SHA-256 check belongs to the caller: the receive loop runs
        // it on the blocking pool while the next response is read. Bytes are
        // never handed to the writer before that check passes.
        return Ok(Message::ChunkResponse { swarm_id: manifest.swarm_id.clone(), index, data });
    }
    let mut body = vec![0; length]; body[0] = kind[0];
    read_exact(recv, &mut body[1..], cancel).await?;
    Message::decode(&body)
}

pub(super) fn peer_uses_internet(binding: LocalBinding, peer: IpAddr, revision: u64) -> Result<bool, String> {
    let internet = !binding.is_local_peer(peer)?;
    if internet { transfer_policy::require_internet_since(revision)?; }
    Ok(internet)
}

pub(super) async fn peer_policy_lost(internet: bool, revision: u64) {
    if internet { transfer_policy::internet_denied_since(revision).await; }
    else { std::future::pending::<()>().await; }
}

pub(super) async fn peer_guarded<T>(binding: LocalBinding, peer: IpAddr, revision: u64, local_revision: u64,
    future: impl Future<Output = Result<T, String>>) -> Result<T, String>
{
    transfer_policy::require_local_since(local_revision)?;
    let internet = peer_uses_internet(binding, peer, revision)?;
    let value = tokio::select! {
        biased;
        _ = transfer_policy::local_denied_since(local_revision) => return Err("Local network policy interrupted LocalSwarm".into()),
        _ = peer_policy_lost(internet, revision) => return Err("Mobile network policy interrupted LocalSwarm".into()),
        value = future => value?,
    };
    transfer_policy::require_local_since(local_revision)?;
    peer_uses_internet(binding, peer, revision)?;
    Ok(value)
}

struct ReceiveAdmission { accepting: bool, dials: CancellationToken }
pub(super) struct ReceiveActivity {
    admission: Mutex<ReceiveAdmission>,
    active: AtomicUsize,
}
impl Default for ReceiveActivity {
    fn default() -> Self {
        Self { admission: Mutex::new(ReceiveAdmission { accepting: true, dials: CancellationToken::new() }),
            active: AtomicUsize::new(0) }
    }
}
impl ReceiveActivity {
    #[cfg(test)]
    pub fn active_sessions(&self) -> usize { self.active.load(Ordering::Acquire) }
    pub fn accepting(&self) -> bool {
        self.admission.lock().unwrap_or_else(|error| error.into_inner()).accepting
    }
    #[cfg(any(target_os = "ios", test))]
    pub fn set_accepting(&self, accepting: bool) {
        let mut admission = self.admission.lock().unwrap_or_else(|error| error.into_inner());
        if admission.accepting == accepting { return; }
        admission.accepting = accepting;
        if accepting { admission.dials = CancellationToken::new(); }
        else { admission.dials.cancel(); }
    }
    pub fn dial_token(&self) -> Option<CancellationToken> {
        let admission = self.admission.lock().unwrap_or_else(|error| error.into_inner());
        admission.accepting.then(|| admission.dials.child_token())
    }
    fn session(self: &Arc<Self>) -> Result<ReceiveSession, String> {
        let admission = self.admission.lock().unwrap_or_else(|error| error.into_inner());
        if !admission.accepting { return Err("LocalSwarm is not accepting new receive sessions".into()); }
        self.active.fetch_add(1, Ordering::AcqRel);
        Ok(ReceiveSession(self.clone()))
    }
}
struct ReceiveSession(Arc<ReceiveActivity>);
impl Drop for ReceiveSession {
    fn drop(&mut self) { self.0.active.fetch_sub(1, Ordering::AcqRel); }
}

pub struct Session {
    endpoint: Option<Endpoint>,
    connection: Option<Connection>,
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    native: Option<super::apple_transport::Connection>,
    send: SendStream,
    recv: RecvStream,
    manifest: Arc<Manifest>,
    /// The peer's advertised chunks. Shared with the receiver's scheduler so a
    /// session handoff costs one refcount instead of a chunk-count byte copy;
    /// control messages copy only when a snapshot is still outstanding.
    available: Arc<Vec<bool>>,
    cancel: CancellationToken,
    budget: Arc<RateBudget>,
    heartbeat: Vec<u8>,
    binding: Option<LocalBinding>,
    policy_revision: u64,
    local_revision: u64,
    policy_cancel: CancellationToken,
    policy_task: Mutex<Option<JoinHandle<()>>>,
    receive_activity: Option<ReceiveSession>,
    /// The dial target this session was opened with. A transport loss reconnects
    /// through the same validated path, never through a cached socket.
    target: Option<(String, u16)>,
    /// Bounded evidence of the route actually used: interface/local address for
    /// Network.framework, local -> peer address for QUIC.
    route: Option<String>,
}
impl Session {
    pub(super) fn track_receive(&mut self, activity: &Arc<ReceiveActivity>) -> Result<(), String> {
        if self.receive_activity.is_none() { self.receive_activity = Some(activity.session()?); }
        Ok(())
    }
    pub fn manifest(&self) -> &Manifest { &self.manifest }
    /// Snapshot of the peer's advertised chunks. The receiver schedules batches
    /// without holding the session, which lives in its own pipeline task.
    pub(super) fn availability(&self) -> Arc<Vec<bool>> { self.available.clone() }
    /// Where to dial this peer again after a transient transport loss.
    pub(super) fn retry_target(&self) -> Option<(&str, u16)> {
        self.target.as_ref().map(|(host, port)| (host.as_str(), *port))
    }
    /// Bounded route receipt captured when this session became usable.
    pub fn route(&self) -> Option<&str> { self.route.as_deref() }
    pub fn close(&self) {
        if let Some(connection) = &self.connection { connection.close(0u32.into(), b"stopped"); }
        if let Some(endpoint) = &self.endpoint { endpoint.close(0u32.into(), b"stopped"); }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if let Some(connection) = &self.native { connection.close(); }
        self.policy_cancel.cancel();
    }
    pub async fn shutdown(&self) {
        self.close();
        let task = self.policy_task.lock().unwrap_or_else(|error| error.into_inner()).take();
        if let Some(task) = task { let _ = task.await; }
        if let Some(endpoint) = &self.endpoint { endpoint.wait_idle().await; }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if let Some(connection) = &self.native { connection.shutdown().await; }
    }
    pub async fn into_manifest(self) -> Arc<Manifest> { self.shutdown().await; self.manifest.clone() }
    pub async fn get_chunk(&mut self, index: u32) -> Result<Vec<u8>, String> {
        let measured = super::profile::start();
        let result = if let (Some(binding), Some(connection)) = (self.binding, &self.connection) {
            let peer = connection.remote_address().ip();
            peer_guarded(binding, peer, self.policy_revision, self.local_revision, self.fetch_chunk(index)).await
        } else { self.fetch_chunk(index).await };
        if result.is_err() { self.close(); }
        if let Ok(bytes) = &result { super::profile::record(3, bytes.len(), measured); }
        result
    }
    async fn fetch_chunk(&mut self, index: u32) -> Result<Vec<u8>, String> {
        let mut pipeline = self.pipeline(&[index]).await?;
        pipeline.next().await?.map(|(_, data)| data)
            .ok_or_else(|| "LocalSwarm peer stopped before the requested chunk".into())
    }
    /// Send up to `PIPELINE_WINDOW` chunkRequests before reading the first
    /// response. The stream is ordered and every request names one index, so
    /// responses are matched positionally: the next response belongs to the
    /// oldest outstanding request. Requests carry only indices; payload is held
    /// one chunk at a time and its prefetch accounting is charged by the
    /// pipeline task, so dropping the pipeline or the session releases both.
    pub(super) async fn pipeline(&mut self, indices: &[u32]) -> Result<ChunkPipeline<'_>, String> {
        if indices.is_empty() || indices.len() > PIPELINE_WINDOW {
            return Err("Invalid LocalSwarm chunk pipeline window".into());
        }
        for &index in indices { self.manifest.chunk_len(index)?; }
        let swarm_id = self.manifest.swarm_id.clone();
        for &index in indices {
            send_message(&mut self.send, &Message::ChunkRequest { swarm_id: swarm_id.clone(), index }, &self.cancel).await?;
        }
        let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(15), Duration::from_secs(15));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Ok(ChunkPipeline {
            session: self,
            outstanding: indices.iter().copied().collect(),
            controls: 0,
            heartbeat,
        })
    }
}
impl Drop for Session { fn drop(&mut self) { self.close(); } }

/// Bounded, in-order window over one connection. See `Session::pipeline`.
pub(super) struct ChunkPipeline<'a> {
    session: &'a mut Session,
    outstanding: std::collections::VecDeque<u32>,
    controls: usize,
    heartbeat: tokio::time::Interval,
}
impl ChunkPipeline<'_> {
    /// Next response in request order, or `None` once the window is drained.
    /// A failed response retires the whole connection: the remaining window is
    /// dropped and no further request can be issued on it.
    pub(super) async fn next(&mut self) -> Result<Option<(u32, Vec<u8>)>, String> {
        let Some((index, data)) = self.next_unverified().await? else { return Ok(None) };
        let measured = super::profile::start();
        let matches = chunk_matches(&data, &self.session.manifest.chunk_hashes[index as usize]);
        super::profile::record(4, data.len(), measured);
        if !matches {
            self.session.close();
            return Err("LocalSwarm chunk checksum mismatch".into());
        }
        Ok(Some((index, data)))
    }
    /// Next response in request order without its chunk SHA-256 check, which
    /// the caller must complete before the bytes are used. The receive loop
    /// runs that check on the blocking pool while the next response is read, so
    /// hashing never sits between the socket and the writer.
    pub(super) async fn next_unverified(&mut self) -> Result<Option<(u32, Vec<u8>)>, String> {
        let result = self.read().await;
        if result.is_err() { self.session.close(); }
        result
    }
    /// Availability controls never count as progress, so the same bounded flood
    /// limit applies per response as it did for a single request.
    async fn read(&mut self) -> Result<Option<(u32, Vec<u8>)>, String> {
        let Some(&index) = self.outstanding.front() else { return Ok(None) };
        loop {
            if self.controls >= MAX_CONTROLS { return Err("LocalSwarm control message flood".into()); }
            let measured = super::profile::start();
            let message = {
                let session = &mut *self.session;
                let pending = receive_frame(&mut session.recv, &session.cancel, &session.budget,
                    Some((&session.manifest, index)), &[4, 5, 7], true);
                tokio::pin!(pending);
                loop {
                    tokio::select! {
                        message = &mut pending => break message?,
                        _ = self.heartbeat.tick() => write_all(&mut session.send, &session.heartbeat, &session.cancel).await?,
                    }
                }
            };
            super::profile::record(8, 0, measured);
            match message {
                Message::ChunkResponse { data, .. } => {
                    self.outstanding.pop_front();
                    self.controls = 0;
                    return Ok(Some((index, data)));
                }
                Message::Bitfield { swarm_id, bits } if swarm_id == self.session.manifest.swarm_id && bits.len() == self.session.available.len() => {
                    self.session.available = Arc::new(bits);
                    self.controls += 1;
                }
                Message::Have { swarm_id, index: have } if swarm_id == self.session.manifest.swarm_id && (have as usize) < self.session.available.len() => {
                    Arc::make_mut(&mut self.session.available)[have as usize] = true;
                    self.controls += 1;
                }
                _ => return Err("Unexpected LocalSwarm message while fetching chunk".into()),
            }
        }
    }
}

/// A chunk whose SHA-256 already matched the manifest. It is constructed only
/// after the receive loop's verification completed, so the staging writer never
/// re-hashes a chunk that just crossed that check.
pub(super) struct VerifiedChunk {
    pub index: u32,
    pub data: Vec<u8>,
    /// Prefetch accounting; released once the chunk has been written.
    pub permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// One pipeline handoff: the source slot, then the next verified chunk in
/// request order.
pub(super) type PipelineItem = (usize, Result<VerifiedChunk, String>);

/// A response whose payload is being hashed on the blocking pool. Its prefetch
/// permit travels with the verified bytes until the writer releases them, so
/// the one-response lookahead costs no extra accounting headroom.
struct PendingChunk {
    index: u32,
    permit: tokio::sync::OwnedSemaphorePermit,
    hash: JoinHandle<(Vec<u8>, bool)>,
    measured: Option<Instant>,
}

/// Finish a pending verification and queue the chunk, or report that the
/// receive job is going away. Verification normally overlaps the next read;
/// budget pressure hands it off sooner so the writer can free capacity.
async fn hand_off(source: usize, pending: PendingChunk, sender: &tokio::sync::mpsc::Sender<PipelineItem>,
    shutdown: &CancellationToken) -> Result<bool, String>
{
    let PendingChunk { index, permit, hash, measured } = pending;
    let (data, matches) = tokio::select! {
        biased;
        _ = shutdown.cancelled() => return Ok(false),
        hashed = hash => hashed.map_err(|_| "LocalSwarm chunk verification stopped".to_string())?,
    };
    super::profile::record(4, data.len(), measured);
    if !matches { return Err("LocalSwarm chunk checksum mismatch".into()); }
    let chunk = VerifiedChunk { index, data, permit: Some(permit) };
    Ok(tokio::select! {
        biased;
        _ = shutdown.cancelled() => false,
        sent = sender.send((source, Ok(chunk))) => sent.is_ok(),
    })
}

/// Keep lookahead when it fits, but never wait for more budget while retaining
/// a chunk that only this task can deliver to the writer. Every claim remains
/// attached to its bytes through verification, queueing and the write.
async fn reserve_prefetch(source: usize, pending: &mut Option<PendingChunk>,
    sender: &tokio::sync::mpsc::Sender<PipelineItem>, prefetch: &Arc<tokio::sync::Semaphore>,
    units: u32, shutdown: &CancellationToken)
    -> Result<Option<tokio::sync::OwnedSemaphorePermit>, String>
{
    if shutdown.is_cancelled() { return Ok(None); }
    let measured = super::profile::start();
    let permit = match prefetch.clone().try_acquire_many_owned(units) {
        Ok(permit) => permit,
        Err(tokio::sync::TryAcquireError::Closed) => return Err("LocalSwarm prefetch budget closed".into()),
        Err(tokio::sync::TryAcquireError::NoPermits) => {
            if let Some(previous) = pending.take() {
                if !hand_off(source, previous, sender, shutdown).await? { return Ok(None); }
            }
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => return Ok(None),
                permit = prefetch.clone().acquire_many_owned(units) => permit
                    .map_err(|_| "LocalSwarm prefetch budget closed".to_string())?,
            }
        }
    };
    super::profile::record(11, 0, measured);
    Ok(Some(permit))
}

/// Drive one connection in its own task so its request window never stops
/// between scheduling rounds: the task waits for index batches from the
/// receiver, pipelines each batch, and hands verified chunks back through a
/// byte-capped queue. The session is returned so the caller keeps ownership; a
/// failed response closes it and is reported once, exactly like the
/// single-chunk path.
pub(super) async fn pipeline_task(source: usize, mut session: Session,
    mut batches: tokio::sync::mpsc::Receiver<Vec<u32>>,
    sender: tokio::sync::mpsc::Sender<PipelineItem>,
    prefetch: Arc<tokio::sync::Semaphore>, shutdown: CancellationToken) -> Session
{
    let revision = session.policy_revision;
    let local_revision = session.local_revision;
    let policy = session.binding.zip(session.connection.as_ref().map(|connection| connection.remote_address().ip()));
    let fetch = async {
        let manifest = session.manifest.clone();
        loop {
            let indices = tokio::select! {
                biased;
                _ = shutdown.cancelled() => return Ok(()),
                batch = batches.recv() => match batch { Some(batch) => batch, None => return Ok(()) },
            };
            if indices.is_empty() { continue; }
            let mut pipeline = session.pipeline(&indices).await?;
            // One response ahead: its SHA-256 runs on the blocking pool while
            // the next response is read, so verification overlaps the socket
            // instead of sitting between the socket and the writer. The chunk
            // already holds its prefetch permit, so this adds concurrency, not
            // buffer headroom.
            let mut pending: Option<PendingChunk> = None;
            while let Some(&index) = pipeline.outstanding.front() {
                // Charge the cap before reading. If lookahead does not fit,
                // first deliver the pending chunk without releasing its claim.
                let units = manifest.chunk_len(index)?.div_ceil(PREFETCH_UNIT).max(1);
                let units = u32::try_from(units).unwrap_or(u32::MAX);
                let Some(permit) = reserve_prefetch(source, &mut pending, &sender, &prefetch,
                    units, &shutdown).await? else { return Ok(()); };
                let next = tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return Ok(()),
                    next = pipeline.next_unverified() => next?,
                };
                let Some((index, data)) = next else { break };
                let expected = manifest.chunk_hashes[index as usize].clone();
                let measured = super::profile::start();
                let hash = tokio::task::spawn_blocking(move || {
                    let matches = chunk_matches(&data, &expected);
                    (data, matches)
                });
                if let Some(previous) = pending.replace(PendingChunk { index, permit, hash, measured }) {
                    if !hand_off(source, previous, &sender, &shutdown).await? { return Ok(()); }
                }
            }
            if let Some(previous) = pending {
                if !hand_off(source, previous, &sender, &shutdown).await? { return Ok(()); }
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), String>(())
    };
    let result = match policy {
        Some((binding, peer)) => peer_guarded(binding, peer, revision, local_revision, fetch).await,
        None => fetch.await,
    };
    if let Err(error) = result {
        session.close();
        let _ = sender.send((source, Err(error))).await;
    }
    session
}

#[cfg(test)]
pub async fn connect(host: &str, port: u16, swarm_id: &str, pin: Option<&str>, peer_id: &str,
    cancel: CancellationToken, download_budget: Arc<RateBudget>) -> Result<Session, String>
{
    connect_with_route(host, port, swarm_id, pin, peer_id, cancel, download_budget, false).await
}

/// `direct` is the Apple-only Nearby preference: try the AWDL/peer-to-peer
/// route first, then fall back to infrastructure. It never relaxes the local
/// link policy, and non-Apple transports ignore it because they have no
/// equivalent peer-to-peer route.
#[expect(clippy::too_many_arguments, reason = "Transport boundary keeps endpoint, handshake credentials, cancellation, shared rate budget and Apple route preference explicit")]
pub async fn connect_with_route(host: &str, port: u16, swarm_id: &str, pin: Option<&str>, peer_id: &str,
    cancel: CancellationToken, download_budget: Arc<RateBudget>, direct: bool) -> Result<Session, String>
{
    if !protocol::valid_hash(swarm_id) || !protocol::valid_peer_id(peer_id) || port == 0
        || host.is_empty() || host.len() > 253 || host.chars().any(char::is_control)
        || pin.is_some_and(|pin| pin.len() > 1024)
    { return Err("Invalid LocalSwarm connection details".into()); }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    if host.starts_with("bonjour:") || host.starts_with("bonjour-lan:") {
        return connect_apple(host, port, swarm_id, pin, peer_id, cancel, download_budget, direct).await;
    }
    let _ = direct;
    let target = (host.to_string(), port);
    let host = host.strip_prefix('[').and_then(|host| host.strip_suffix(']')).unwrap_or(host);
    // Numeric discovery peers need no default-route DNS. Hostname resolution is
    // Internet-policy fenced; it must not silently use cellular for a LAN job.
    let policy_revision = transfer_policy::internet_revision()?;
    let local_revision = transfer_policy::local_revision()?;
    transfer_policy::require_local_since(local_revision)?;
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() { vec![SocketAddr::new(ip, port)] } else {
        transfer_policy::require_internet_since(policy_revision)?;
        tokio::select! {
            biased;
            _ = transfer_policy::local_denied_since(local_revision) => return Err("Local network policy interrupted address lookup".into()),
            _ = transfer_policy::internet_denied_since(policy_revision) => return Err("Mobile network policy interrupted address lookup".into()),
            result = guarded(&cancel, async {
                let addresses = tokio::net::lookup_host((host, port)).await.map_err(|e| format!("LocalSwarm address lookup failed: {e}"))?;
                let mut result = Vec::new();
                for address in addresses {
                    if result.len() == 16 { break; }
                    if !result.contains(&address) { result.push(address); }
                }
                if result.is_empty() { return Err("LocalSwarm host has no usable address".into()); }
                Ok(result)
            }) => result?,
        }
    };
    let (endpoint, connection, binding) = connect_addresses(&addresses, &cancel, client_config()?, policy_revision, local_revision).await?;
    let result = peer_guarded(binding, connection.remote_address().ip(), policy_revision, local_revision, async {
        let (send, recv) = guarded(&cancel, async { connection.open_bi().await.map_err(|e| e.to_string()) }).await?;
        handshake(SendStream::Quinn(send), RecvStream::Quinn(recv), swarm_id, pin, peer_id, &cancel, &download_budget).await
    }).await;
    match result {
        Ok((send, recv, manifest, available, heartbeat)) => {
            let policy_cancel = CancellationToken::new();
            let watch_cancel = policy_cancel.clone();
            let watched_connection = connection.clone();
            let internet = match peer_uses_internet(binding, connection.remote_address().ip(), policy_revision) {
                Ok(internet) => internet,
                Err(error) => {
                    connection.close(0u32.into(), b"policy interrupted");
                    endpoint.close(0u32.into(), b"policy interrupted"); endpoint.wait_idle().await;
                    return Err(error);
                }
            };
            // Quinn owns timers even while the caller is doing disk work or
            // waiting for another peer. Retire traffic independently of polling.
            let policy_task = tokio::spawn(async move {
                tokio::select! {
                    biased;
                    _ = watch_cancel.cancelled() => {},
                    _ = transfer_policy::local_denied_since(local_revision) => watched_connection.close(0u32.into(), b"local policy interrupted"),
                    _ = peer_policy_lost(internet, policy_revision) => watched_connection.close(0u32.into(), b"network policy interrupted"),
                }
            });
            let route = Some(format!("{} -> {}", endpoint.local_addr().map_or_else(|_| "?".into(), |address| address.ip().to_string()), connection.remote_address()));
            Ok(Session { endpoint: Some(endpoint), connection: Some(connection),
                #[cfg(any(target_os = "ios", target_os = "macos"))]
                native: None,
                receive_activity: None,
                send, recv, manifest: Arc::new(manifest), available: Arc::new(available), cancel, budget: download_budget, heartbeat,
                binding: Some(binding), policy_revision, local_revision, policy_cancel, policy_task: Mutex::new(Some(policy_task)),
                target: Some(target), route })
        },
        Err(error) => {
            connection.close(0u32.into(), b"protocol error"); endpoint.close(0u32.into(), b"protocol error");
            endpoint.wait_idle().await;
            Err(error)
        }
    }
}

async fn handshake(mut send: SendStream, mut recv: RecvStream, swarm_id: &str, pin: Option<&str>, peer_id: &str,
    cancel: &CancellationToken, download_budget: &RateBudget)
    -> Result<(SendStream, RecvStream, Manifest, Vec<bool>, Vec<u8>), String>
{
    send_message(&mut send, &Message::Handshake { peer_id: peer_id.into(), swarm_id: swarm_id.into() }, cancel).await?;
    match receive(&mut recv, cancel, download_budget, None, &[1]).await? {
        Message::Handshake { swarm_id: sid, peer_id } if sid == swarm_id && protocol::valid_peer_id(&peer_id) => {},
        _ => return Err("LocalSwarm peer did not handshake for the requested swarm".into()),
    }
    let mut availability = None;
    let mut early_haves = Vec::new();
    if let Some(pin) = pin.filter(|value| !value.is_empty()) {
        // The seeder proves the PIN before this client discloses anything
        // derived from it, so an endpoint that impersonates the dialed address
        // learns nothing it can verify offline. Both proofs are bound to fresh
        // per-connection nonces and cannot be replayed.
        let client_nonce = protocol::auth_nonce()?;
        send_message(&mut send, &Message::AuthChallenge { swarm_id: swarm_id.into(), nonce: client_nonce }, cancel).await?;
        // The seeder advertises its availability before it reads the challenge,
        // so those controls are collected here rather than rejected.
        let mut proof = None;
        for _ in 0..MAX_CONTROLS {
            match receive(&mut recv, cancel, download_budget, None, &[4, 5, 10]).await? {
                Message::AuthProof { swarm_id: sid, nonce, proof: value } if sid == swarm_id => { proof = Some((nonce, value)); break; }
                Message::Bitfield { swarm_id: sid, bits } if sid == swarm_id && availability.is_none() => availability = Some(bits),
                Message::Have { swarm_id: sid, index } if sid == swarm_id => early_haves.push(index),
                _ => return Err("Unexpected LocalSwarm message before the PIN proof".into()),
            }
        }
        let (server_nonce, proof) = proof.ok_or("LocalSwarm peer did not prove the shared PIN")?;
        if !constant_time_eq(&proof, &protocol::pin_proof(pin, "server", swarm_id, &client_nonce, &server_nonce)) {
            return Err("LocalSwarm peer did not prove the shared PIN".into());
        }
        send_message(&mut send, &Message::AuthResponse {
            swarm_id: swarm_id.into(),
            proof: protocol::pin_proof(pin, "client", swarm_id, &client_nonce, &server_nonce),
        }, cancel).await?;
    }
    send_message(&mut send, &Message::ManifestRequest { swarm_id: swarm_id.into() }, cancel).await?;
    let mut manifest = None;
    for _ in 0..MAX_CONTROLS {
        match receive(&mut recv, cancel, download_budget, None, &[3, 4, 5]).await? {
            Message::ManifestResponse(json) => {
                let value = Manifest::from_json(&json)?;
                if value.swarm_id != swarm_id { return Err("LocalSwarm manifest swarm mismatch".into()); }
                manifest = Some(value); break;
            }
            Message::Bitfield { swarm_id: sid, bits } if sid == swarm_id && availability.is_none() => availability = Some(bits),
            Message::Have { swarm_id: sid, index } if sid == swarm_id => early_haves.push(index),
            _ => return Err("Unexpected LocalSwarm message before manifest".into()),
        }
    }
    let manifest = manifest.ok_or("LocalSwarm control message flood before manifest")?;
    let mut available = availability.unwrap_or_else(|| vec![false; manifest.chunk_hashes.len()]);
    if available.len() != manifest.chunk_hashes.len() { return Err("LocalSwarm bitfield count does not match manifest".into()); }
    for index in early_haves { *available.get_mut(index as usize).ok_or("Out-of-range LocalSwarm have")? = true; }
    let heartbeat = Message::Bitfield { swarm_id: swarm_id.into(), bits: vec![false; manifest.chunk_hashes.len()] }.encode()?;
    write_all(&mut send, &heartbeat, cancel).await?;
    Ok((send, recv, manifest, available, heartbeat))
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[expect(clippy::too_many_arguments, reason = "Apple transport dispatch preserves the cross-platform endpoint, authentication, cancellation, budget and route-preference contract")]
async fn connect_apple(host: &str, port: u16, swarm_id: &str, pin: Option<&str>, peer_id: &str,
    cancel: CancellationToken, download_budget: Arc<RateBudget>, direct: bool) -> Result<Session, String>
{
    let (connection, route) = super::apple_transport::Connection::connect(host, &cancel, direct).await?;
    let result = handshake(SendStream::Apple(connection.clone()), RecvStream::Apple(connection.clone()),
        swarm_id, pin, peer_id, &cancel, &download_budget).await;
    match result {
        Ok((send, recv, manifest, available, heartbeat)) => {
            let policy_cancel = CancellationToken::new();
            let watch_cancel = policy_cancel.clone(); let lifetime = cancel.clone(); let watched = connection.clone();
            let task = tokio::spawn(async move {
                tokio::select! { _ = lifetime.cancelled() => watched.close(), _ = watch_cancel.cancelled() => {} }
            });
            // Prefer the native interface/local-address detail; the selected
            // adapter is authoritative even if path inspection is unavailable.
            let route = Some(match connection.route() {
                Some(detail) => format!("{route} {detail}"),
                None => route.to_string(),
            });
            Ok(Session { endpoint: None, connection: None, native: Some(connection), send, recv,
                manifest: Arc::new(manifest), available: Arc::new(available), cancel, budget: download_budget, heartbeat,
                receive_activity: None,
                binding: None, policy_revision: 0, local_revision: 0, policy_cancel, policy_task: Mutex::new(Some(task)),
                target: Some((host.to_string(), port)), route })
        }
        Err(error) => { connection.shutdown().await; Err(error) }
    }
}

pub(super) async fn connect_addresses(addresses: &[SocketAddr], cancel: &CancellationToken, config: quinn::ClientConfig, revision: u64, local_revision: u64)
    -> Result<(Endpoint, Connection, LocalBinding), String>
{
    let mut last_error = "LocalSwarm host has no usable address".to_string();
    let mut first_error: Option<String> = None;
    for address in addresses {
        if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
        transfer_policy::require_local_since(local_revision)?;
        crate::nearby_log!("[nearby] try {address}");
        let bind = if address.is_ipv4() { SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0) }
            else { SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0) };
        let socket = match std::net::UdpSocket::bind(bind) {
            Ok(socket) => socket,
            Err(error) => { crate::nearby_log!("[nearby] {address} bind failed: {error}"); last_error = format!("LocalSwarm socket failed: {error}"); continue; }
        };
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let binding = transfer_policy::bind_local_socket(&socket)?;
        let internet = peer_uses_internet(binding, address.ip(), revision)?;
        // UDP connect performs kernel route selection without sending a packet.
        // Use this policy-bound probe only to obtain its selected local address;
        // Quinn must receive a separate, unconnected socket bound to that address.
        if let Err(error) = socket.connect(*address) {
            crate::nearby_log!("[nearby] {address} route failed: {error}");
            last_error = format!("LocalSwarm route selection failed: {error}"); continue;
        }
        let local = socket.local_addr().map_err(|error| error.to_string())?;
        if local.ip().is_unspecified() { last_error = "LocalSwarm route has no local address".into(); continue; }
        drop(socket);
        let socket = match std::net::UdpSocket::bind(SocketAddr::new(local.ip(), 0)) {
            Ok(socket) => socket,
            Err(error) => { last_error = format!("LocalSwarm route binding failed: {error}"); continue; }
        };
        socket.set_nonblocking(true).map_err(|error| error.to_string())?;
        let actual_binding = transfer_policy::bind_local_socket(&socket)?;
        if actual_binding != binding { return Err("The selected LocalSwarm network changed while binding".into()); }
        let mut endpoint = match Endpoint::new(quinn::EndpointConfig::default(), None, socket, Arc::new(quinn::TokioRuntime)) {
            Ok(endpoint) => endpoint,
            Err(error) => { last_error = format!("LocalSwarm socket failed: {error}"); continue; }
        };
        endpoint.set_default_client_config(config.clone());
        let attempt = match endpoint.connect(*address, "localswarm.local") {
            Ok(attempt) => attempt,
            Err(error) => {
                endpoint.close(0u32.into(), b"failed"); endpoint.wait_idle().await;
                last_error = error.to_string(); continue;
            }
        };
        let result = tokio::select! {
            biased;
            _ = transfer_policy::local_denied_since(local_revision) => Err("Local network policy interrupted LocalSwarm".to_string()),
            _ = peer_policy_lost(internet, revision) => Err("Mobile network policy interrupted LocalSwarm".to_string()),
            _ = cancel.cancelled() => Err("LocalSwarm operation cancelled".to_string()),
            value = tokio::time::timeout(Duration::from_secs(8), attempt) => value
                .map_err(|_| "LocalSwarm connect timed out".to_string())
                .and_then(|v| v.map_err(|e| format!("LocalSwarm QUIC connection failed: {e}"))),
        };
        match result {
            Ok(connection) => {
                crate::nearby_log!("[nearby] connected {address} local {binding:?}");
                if let Err(error) = transfer_policy::require_local_since(local_revision) {
                    connection.close(0u32.into(), b"local policy interrupted");
                    endpoint.close(0u32.into(), b"local policy interrupted"); endpoint.wait_idle().await;
                    return Err(error);
                }
                if let Err(error) = peer_uses_internet(binding, connection.remote_address().ip(), revision) {
                    connection.close(0u32.into(), b"policy interrupted");
                    endpoint.close(0u32.into(), b"policy interrupted"); endpoint.wait_idle().await;
                    return Err(error);
                }
                return Ok((endpoint, connection, binding));
            }
            Err(error) => {
                crate::nearby_log!("[nearby] {address} failed: {error}");
                endpoint.close(0u32.into(), b"failed"); endpoint.wait_idle().await;
                if first_error.is_none() { first_error = Some(error.clone()); }
                last_error = error;
            }
        }
    }
    let failure = first_error.unwrap_or(last_error);
    crate::nearby_log!("[nearby] all addresses failed: {failure}");
    Err(failure)
}

pub type ChunkReader = Arc<dyn Fn(u32) -> Result<Vec<u8>, String> + Send + Sync>;
pub(super) type SeedSourceFailure = Arc<dyn Fn(&str) + Send + Sync>;
// The high bit closes admission; lower bits count sessions that have requested
// authenticated payload. One atomic transition prevents an inspected, idle peer
// from starting a new transfer after background admission has been closed.
const ADMISSION_CLOSED: usize = 1usize << (usize::BITS - 1);
pub struct SeedStats {
    uploaded: AtomicU64, peers: AtomicUsize, activity: AtomicUsize,
    #[cfg(test)] peak_serve_bytes: AtomicUsize,
    /// Test-only: accepted connections, so a test can force a mid-transfer
    /// link loss without touching the listener that must accept the redial.
    #[cfg(test)] connections: Mutex<Vec<Connection>>,
}
impl Default for SeedStats {
    fn default() -> Self {
        Self { uploaded: AtomicU64::new(0), peers: AtomicUsize::new(0),
            activity: AtomicUsize::new(if cfg!(target_os = "ios") { ADMISSION_CLOSED } else { 0 }),
            #[cfg(test)] peak_serve_bytes: AtomicUsize::new(0),
            #[cfg(test)] connections: Mutex::new(Vec::new()) }
    }
}
impl SeedStats {
    pub fn uploaded(&self) -> u64 { self.uploaded.load(Ordering::Relaxed) }
    /// Count payload the transport accepted. Apple writes report this from the
    /// native completion callback so retirement cannot lose the last window.
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    pub(super) fn record_upload(&self, bytes: u64) { self.uploaded.fetch_add(bytes, Ordering::Relaxed); }
    /// Test-only: peak chunk payload buffered for one peer (read-ahead plus
    /// transport-accepted-but-unconfirmed sends).
    #[cfg(test)]
    fn observe_serve_bytes(&self, bytes: usize) { self.peak_serve_bytes.fetch_max(bytes, Ordering::Relaxed); }
    #[cfg(test)]
    fn peak_serve_bytes(&self) -> usize { self.peak_serve_bytes.load(Ordering::Relaxed) }
    pub fn peer_count(&self) -> usize { self.peers.load(Ordering::Relaxed) }
    /// Test-only: drop every connection accepted so far, as a Wi-Fi roam or
    /// AWDL flap would. The listener keeps accepting, so the redial can land.
    #[cfg(test)]
    pub(super) fn drop_connections(&self) {
        for connection in self.connections.lock().unwrap_or_else(|error| error.into_inner()).drain(..) {
            connection.close(0u32.into(), b"test link loss");
        }
    }
    #[cfg(test)]
    pub(super) fn connection_count(&self) -> usize {
        self.connections.lock().unwrap_or_else(|error| error.into_inner()).len()
    }
    #[cfg(any(target_os = "ios", test))]
    pub fn active_transfers(&self) -> usize { self.activity.load(Ordering::Acquire) & !ADMISSION_CLOSED }
    pub fn accepting(&self) -> bool { self.activity.load(Ordering::Acquire) & ADMISSION_CLOSED == 0 }
    #[cfg(any(target_os = "ios", test))]
    pub fn set_accepting(&self, accepting: bool) {
        if accepting { self.activity.fetch_and(!ADMISSION_CLOSED, Ordering::AcqRel); }
        else { self.activity.fetch_or(ADMISSION_CLOSED, Ordering::AcqRel); }
    }
    fn begin_transfer(&self) -> Result<ActiveTransfer<'_>, String> {
        let mut state = self.activity.load(Ordering::Acquire);
        loop {
            if state & ADMISSION_CLOSED != 0 {
                return Err("LocalSwarm is not accepting new transfers".into());
            }
            match self.activity.compare_exchange_weak(state, state + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(observed) => state = observed,
            }
        }
        Ok(ActiveTransfer { stats: self, _power: crate::power_activity::Guard::acquire("Serving a LocalSwarm transfer") })
    }
}
struct ActiveTransfer<'a> {
    stats: &'a SeedStats,
    /// Releases the process activity assertion with the last live transfer.
    _power: crate::power_activity::Guard,
}
impl Drop for ActiveTransfer<'_> {
    fn drop(&mut self) { self.stats.activity.fetch_sub(1, Ordering::AcqRel); }
}
pub struct SeedServer {
    pub port: u16,
    pub addresses: Vec<String>,
    pub stats: Arc<SeedStats>,
    cancel: CancellationToken,
    endpoint: Option<Endpoint>,
    task: Option<JoinHandle<()>>,
}
impl SeedServer {
    pub fn is_finished(&self) -> bool { self.task.as_ref().is_none_or(JoinHandle::is_finished) }
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(endpoint) = &self.endpoint { endpoint.close(0u32.into(), b"stopped"); }
        if let Some(task) = self.task.take() { let _ = task.await; }
        if let Some(endpoint) = &self.endpoint { endpoint.wait_idle().await; }
    }
}
impl Drop for SeedServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(endpoint) = &self.endpoint { endpoint.close(0u32.into(), b"stopped"); }
    }
}

/// One local interface address, with the adapter facts the LAN classifier needs.
pub struct InterfaceCandidate<'a> {
    pub name: &'a str,
    pub address: IpAddr,
    pub point_to_point: bool,
    pub loopback: bool,
}

/// The addresses another device on the same local network can dial, in a
/// stable order. A VPN tunnel, a VM or container bridge and a carrier data
/// link are reachable from this machine but are the wrong path for a nearby
/// peer: advertising one makes a phone dial into the tunnel instead of the
/// Wi-Fi link, and binding Web sharing to one produces a dead link.
pub fn lan_addresses<'a>(candidates: impl IntoIterator<Item = InterfaceCandidate<'a>>) -> Vec<IpAddr> {
    let mut addresses: Vec<IpAddr> = candidates.into_iter()
        .filter(|candidate| transfer_policy::lan_interface(candidate.name, candidate.point_to_point, candidate.loopback)
            && !transfer_policy::cellular_interface(candidate.name))
        .map(|candidate| candidate.address).filter(|address| usable_address(*address)).collect();
    addresses.sort(); addresses.dedup(); addresses.truncate(32);
    addresses
}

pub fn reachable_addresses() -> Result<Vec<IpAddr>, String> {
    let interfaces = if_addrs::get_if_addrs().map_err(|e| format!("Cannot enumerate local interfaces: {e}"))?;
    Ok(lan_addresses(interfaces.iter().map(|interface| InterfaceCandidate {
        name: &interface.name, address: interface.ip(),
        point_to_point: interface.is_p2p(), loopback: interface.is_loopback(),
    })))
}

/// The local IPv4 address the kernel currently routes ordinary traffic from:
/// the interface this device is really using, when several have an address.
/// UDP connect performs route selection without sending a packet (the same
/// probe `connect_addresses` uses), and the documentation-range target is
/// never contacted. `None` when there is no IPv4 route at all.
pub fn routed_local_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(203, 0, 113, 1), 9)).ok()?;
    let local = socket.local_addr().ok()?.ip();
    (!local.is_unspecified()).then_some(local)
}

pub fn usable_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_multicast() && !ip.is_broadcast(),
        IpAddr::V6(ip) => !ip.is_unspecified() && !ip.is_loopback() && !ip.is_multicast() && !ip.is_unicast_link_local()
            && ip.to_ipv4_mapped().is_none(),
    }
}

/// One immutable authenticated publication, shared by every accepted peer.
/// Keeping the identity and PIN here avoids cloning them per dial. The PIN is
/// memory-only, exactly like the legacy token it replaces; the nonce-bound
/// exchange needs the secret itself, not a deterministic hash of it.
struct SeedPublication {
    manifest: Arc<Manifest>,
    peer_id: String,
    pin: Option<String>,
    reader: ChunkReader,
    on_source_failure: Option<SeedSourceFailure>,
    budget: Arc<RateBudget>,
    frame: Arc<Vec<u8>>,
    stats: Arc<SeedStats>,
}

pub(super) async fn seed(manifest: Arc<Manifest>, peer_id: String, pin: Option<String>, reader: ChunkReader,
    upload_budget: Arc<RateBudget>, cancel: CancellationToken, on_source_failure: Option<SeedSourceFailure>) -> Result<SeedServer, String>
{
    if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
    manifest.validate()?;
    if !protocol::valid_peer_id(&peer_id) || pin.as_ref().is_some_and(|pin| pin.len() > 1024) {
        return Err("Invalid LocalSwarm seed credentials".into());
    }
    let json = serde_json::to_vec(manifest.as_ref()).map_err(|e| e.to_string())?;
    if json.len() > protocol::MAX_MANIFEST { return Err("Manifest JSON too large".into()); }
    let manifest_frame = Arc::new(Message::ManifestResponse(json).encode()?);
    #[cfg(all(any(target_os = "ios", target_os = "macos"), not(test)))]
    return seed_apple(manifest, peer_id, pin, reader, upload_budget, cancel, manifest_frame, on_source_failure).await;
    #[cfg(any(not(any(target_os = "ios", target_os = "macos")), test))]
    {
    let config = server_config()?;
    // Explicit dual-stack binding: advertise both families only when this socket
    // actually accepts both. Never claim IPv6 reachability for an IPv4 fallback.
    let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        .and_then(|socket| { socket.set_only_v6(false)?; socket.bind(&SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0).into())?; Ok(socket) });
    let (socket, ipv6) = match socket {
        Ok(socket) => (std::net::UdpSocket::from(socket), true),
        Err(_) => (std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| format!("LocalSwarm listen failed: {e}"))?, false),
    };
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    let local_revision = transfer_policy::local_revision()?;
    transfer_policy::require_local_since(local_revision)?;
    let binding = transfer_policy::bind_local_socket(&socket)?;
    let endpoint = Endpoint::new(quinn::EndpointConfig::default(), Some(config), socket, Arc::new(quinn::TokioRuntime))
        .map_err(|e| format!("LocalSwarm QUIC listener failed: {e}"))?;
    let port = endpoint.local_addr().map_err(|e| e.to_string())?.port();
    let addresses = reachable_addresses()?.into_iter().filter(|ip| ipv6 || ip.is_ipv4()).map(|ip| ip.to_string()).collect();
    let stats = Arc::new(SeedStats::default());
    let task_endpoint = endpoint.clone();
    let task_cancel = cancel.child_token();
    let server_cancel = task_cancel.clone();
    let task_stats = stats.clone();
    let pin = pin.filter(|pin| !pin.is_empty());
    let publication = Arc::new(SeedPublication {
        manifest, peer_id, pin, reader, on_source_failure, budget: upload_budget, frame: manifest_frame, stats: stats.clone(),
    });
    let task = tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                _ = transfer_policy::local_denied_since(local_revision) => break,
                _ = task_cancel.cancelled() => break,
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                incoming = task_endpoint.accept() => {
                    let Some(incoming) = incoming else { break; };
                    if !task_stats.accepting() || tasks.len() >= MAX_CONNECTIONS { incoming.refuse(); continue; }
                    let revision = match transfer_policy::internet_revision() {
                        Ok(revision) => revision, Err(_) => { incoming.refuse(); continue; }
                    };
                    let peer = incoming.remote_address().ip();
                    // Loopback is a local process (including host regressions),
                    // never an internet peer; on-link proof still applies to
                    // every other address. IPv4-mapped IPv6 peers normalize first.
                    let local_peer = peer.to_canonical().is_loopback()
                        || binding.is_on_link_peer(peer).unwrap_or(false);
                    if !local_peer { incoming.refuse(); continue; }
                    if peer_uses_internet(binding, peer, revision).is_err() { incoming.refuse(); continue; }
                    let publication = publication.clone();
                    let cancel = task_cancel.clone();
                    tasks.spawn(async move {
                        let connection = match peer_guarded(binding, peer, revision, local_revision,
                            guarded(&cancel, async { incoming.await.map_err(|e| e.to_string()) })).await {
                            Ok(connection) => connection, Err(_) => return,
                        };
                        let stats = &publication.stats;
                        #[cfg(test)]
                        stats.connections.lock().unwrap_or_else(|error| error.into_inner()).push(connection.clone());
                        stats.peers.fetch_add(1, Ordering::Relaxed);
                        let serve_cancel = cancel.child_token();
                        if let Ok(internet) = peer_uses_internet(binding, connection.remote_address().ip(), revision) {
                            let serving = serve_connection(&connection, &publication, &serve_cancel);
                            tokio::pin!(serving);
                            tokio::select! {
                                biased;
                                _ = transfer_policy::local_denied_since(local_revision) => {
                                    serve_cancel.cancel();
                                    connection.close(0u32.into(), b"local policy interrupted");
                                    let _ = serving.await;
                                }
                                _ = peer_policy_lost(internet, revision) => {
                                    serve_cancel.cancel();
                                    connection.close(0u32.into(), b"network policy interrupted");
                                    let _ = serving.await;
                                }
                                _ = &mut serving => {},
                            }
                        }
                        stats.peers.fetch_sub(1, Ordering::Relaxed);
                        connection.close(0u32.into(), b"session ended");
                    });
                }
            }
        }
        task_cancel.cancel(); task_endpoint.close(0u32.into(), b"stopped");
        while tasks.join_next().await.is_some() {}
    });
    Ok(SeedServer { port, addresses, stats, cancel: server_cancel, endpoint: Some(endpoint), task: Some(task) })
    }
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
pub(super) async fn seed_apple(manifest: Arc<Manifest>, peer_id: String, pin: Option<String>, reader: ChunkReader,
    upload_budget: Arc<RateBudget>, cancel: CancellationToken, frame: Arc<Vec<u8>>,
    on_source_failure: Option<SeedSourceFailure>) -> Result<SeedServer, String>
{
    let mut listener = super::apple_transport::Listener::new(&cancel).await?;
    let port = listener.port;
    // These are display-only hints. Bonjour advertises the system hostname and
    // its scoped AWDL records; an empty infrastructure address list is normal.
    let addresses = reachable_addresses()?.into_iter().map(|ip| ip.to_string()).collect();
    let stats = Arc::new(SeedStats::default());
    let task_stats = stats.clone();
    let task_cancel = cancel.child_token(); let server_cancel = task_cancel.clone();
    let pin = pin.filter(|pin| !pin.is_empty());
    let publication = Arc::new(SeedPublication {
        manifest, peer_id, pin, reader, on_source_failure, budget: upload_budget, frame, stats: stats.clone(),
    });
    let task = tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                _ = task_cancel.cancelled() => break,
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                incoming = listener.accept() => {
                    let Some((connection, ready)) = incoming else { break; };
                    if !task_stats.accepting() || tasks.len() >= MAX_CONNECTIONS { connection.shutdown().await; continue; }
                    let publication = publication.clone();
                    let cancel = task_cancel.clone();
                    tasks.spawn(async move {
                        let ready = guarded(&cancel, async { ready.await.map_err(|_| "Apple QUIC readiness callback lost".to_string())? }).await;
                        if ready.is_ok() {
                            let stats = &publication.stats;
                            stats.peers.fetch_add(1, Ordering::Relaxed);
                            let _ = serve_streams(SendStream::Apple(connection.clone()), RecvStream::Apple(connection.clone()),
                                &publication, &cancel).await;
                            stats.peers.fetch_sub(1, Ordering::Relaxed);
                        }
                        connection.shutdown().await;
                    });
                }
            }
        }
        task_cancel.cancel();
        listener.shutdown().await;
        while tasks.join_next().await.is_some() {}
    });
    Ok(SeedServer { port, addresses, stats, cancel: server_cancel, endpoint: None, task: Some(task) })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

fn chunk_matches(data: &[u8], expected: &str) -> bool {
    let digest = Sha256::digest(data);
    let mut encoded = [0; 64];
    hex::encode_to_slice(digest, &mut encoded).is_ok() && encoded.as_slice() == expected.as_bytes()
}

#[cfg(any(not(any(target_os = "ios", target_os = "macos")), test))]
async fn serve_connection(connection: &Connection, publication: &SeedPublication, cancel: &CancellationToken) -> Result<(), String>
{
    let (send, recv) = guarded(cancel, async { connection.accept_bi().await.map_err(|e| e.to_string()) }).await?;
    serve_streams(SendStream::Quinn(send), RecvStream::Quinn(recv), publication, cancel).await
}

async fn serve_streams(mut send: SendStream, mut recv: RecvStream, publication: &SeedPublication,
    cancel: &CancellationToken) -> Result<(), String>
{
    let manifest = &publication.manifest;
    let peer_id = &publication.peer_id;
    let budget = &publication.budget;
    send_message(&mut send, &Message::Handshake { peer_id: peer_id.clone(), swarm_id: manifest.swarm_id.clone() }, cancel).await?;
    send_message(&mut send, &Message::Bitfield { swarm_id: manifest.swarm_id.clone(), bits: vec![true; manifest.chunk_hashes.len()] }, cancel).await?;
    match receive(&mut recv, cancel, budget, None, &[1]).await? {
        Message::Handshake { peer_id, swarm_id } if swarm_id == manifest.swarm_id && protocol::valid_peer_id(&peer_id) => {},
        _ => return Err("Missing LocalSwarm handshake".into()),
    }
    // A single reader owns framing for the connection's entire lifetime.
    // Cancelling mpsc::Receiver::recv does not discard partially read wire data.
    let reader_cancel = cancel.child_token();
    let task_cancel = reader_cancel.clone();
    let reader_budget = publication.budget.clone();
    let chunk_count = manifest.chunk_hashes.len();
    let swarm_id = manifest.swarm_id.clone();
    let (sender, messages) = tokio::sync::mpsc::channel(64);
    let read_task = tokio::spawn(async move {
        let mut limits = ClientFrameLimits::new(chunk_count);
        loop {
            let result = receive_frame(&mut recv, &task_cancel, &reader_budget, None, &[2, 4, 5, 6, 8, 9, 11], false)
                .await.and_then(|message| {
                    if message.swarm_id() != Some(swarm_id.as_str()) { return Err("Cross-swarm LocalSwarm frame".into()); }
                    limits.observe(&message)?; Ok(message)
                });
            let finished = result.is_err();
            tokio::select! {
                _ = task_cancel.cancelled() => break,
                result = sender.send(result) => if result.is_err() { break; },
            }
            if finished { break; }
        }
    });
    let result = serve_requests(send, messages, publication, cancel).await;
    reader_cancel.cancel();
    let _ = read_task.await;
    result
}

async fn next_client_message(messages: &mut tokio::sync::mpsc::Receiver<Result<Message, String>>) -> Result<Message, String> {
    messages.recv().await.ok_or_else(|| "LocalSwarm client reader stopped".to_string())?
}

struct PendingRead {
    index: u32, len: usize, task: JoinHandle<Result<Vec<u8>, String>>,
}
/// One chunk response handed to the transport but not yet accepted by it.
/// Only the Apple bridge can have several of these outstanding; Quinn's
/// flow-controlled write completes before `queue_chunk` returns.
struct PendingSend {
    bytes: usize, completions: std::collections::VecDeque<tokio::sync::oneshot::Receiver<Result<usize, String>>>,
}
impl PendingSend {
    /// Reap completions that have already fired. `true` once the transport has
    /// accepted every piece of this chunk.
    fn try_finish(&mut self) -> Result<bool, String> {
        loop {
            match self.completions.front_mut().map(|completion| completion.try_recv()) {
                None => return Ok(true),
                Some(Ok(Ok(_))) => { self.completions.pop_front(); }
                Some(Ok(Err(_))) => return Err("Apple QUIC connection failed or left the local link".into()),
                Some(Err(tokio::sync::oneshot::error::TryRecvError::Empty)) => return Ok(false),
                Some(Err(tokio::sync::oneshot::error::TryRecvError::Closed)) => return Err("Apple QUIC send callback lost".into()),
            }
        }
    }
    /// Wait for the transport to accept every remaining piece. Cancellation
    /// (or a dropped future) leaves unaccepted completions queued, so the
    /// caller can retry without losing track of the window.
    async fn wait(&mut self, cancel: &CancellationToken) -> Result<(), String> {
        while let Some(completion) = self.completions.front_mut() {
            tokio::select! {
                _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
                sent = completion => { sent.map_err(|_| "Apple QUIC send callback lost")??; }
            }
            self.completions.pop_front();
        }
        Ok(())
    }
}

/// Wait for the oldest outstanding transport send and remove it once accepted,
/// returning the payload bytes it accounted for.
async fn wait_front(sending: &mut std::collections::VecDeque<PendingSend>,
    cancel: &CancellationToken) -> Result<usize, String>
{
    let front = sending.front_mut().ok_or("No LocalSwarm send in flight")?;
    front.wait(cancel).await?;
    let bytes = front.bytes;
    sending.pop_front();
    Ok(bytes)
}

fn start_read(reader: &ChunkReader, manifest: &Arc<Manifest>, index: u32, len: usize,
    on_source_failure: Option<SeedSourceFailure>) -> PendingRead {
    let manifest = manifest.clone();
    let callback = reader.clone();
    let measured = super::profile::start();
    // The read and its integrity check run off the async runtime, so several
    // chunks are in flight while the previous response is still being written.
    let task = tokio::task::spawn_blocking(move || {
        let result = callback(index).and_then(|data| {
            if data.len() != len || !chunk_matches(&data, &manifest.chunk_hashes[index as usize]) {
                return Err("LocalSwarm source changed or returned corrupt bytes".into());
            }
            Ok(data)
        });
        if let (Err(error), Some(retire)) = (&result, on_source_failure) { retire(error); }
        let data = result?;
        super::profile::record(1, data.len(), measured);
        Ok(data)
    });
    PendingRead { index, len, task }
}

async fn serve_requests(mut send: SendStream, mut messages: tokio::sync::mpsc::Receiver<Result<Message, String>>,
    publication: &SeedPublication, cancel: &CancellationToken) -> Result<(), String>
{
    let mut reads = std::collections::VecDeque::new();
    let result = serve_window(&mut send, &mut messages, publication, cancel, &mut reads).await;
    // A blocking source read is joined even during cancellation: no detached
    // filesystem work may outlive stop/mode transitions.
    while let Some(read) = reads.pop_front() { let _ = read.task.await; }
    result
}

/// Answer chunk requests with a bounded window per peer, like the reference
/// engine: at most `SERVE_WINDOW` reads/responses outstanding and at most
/// `SERVE_BUFFER_BYTES` of chunk payload buffered. Requests beyond the window
/// stay in the bounded reader channel and backpressure the transport; none are
/// dropped, so positional response ordering is preserved. Responses are always
/// written in request order.
async fn serve_window(send: &mut SendStream, messages: &mut tokio::sync::mpsc::Receiver<Result<Message, String>>,
    publication: &SeedPublication, cancel: &CancellationToken,
    reads: &mut std::collections::VecDeque<PendingRead>) -> Result<(), String>
{
    let manifest = &publication.manifest;
    let pin = publication.pin.as_deref();
    let reader = &publication.reader;
    let budget = &publication.budget;
    let frame = &publication.frame;
    let stats = &publication.stats;
    let mut authed = pin.is_none();
    let mut auth_seen = false;
    let mut manifest_sent = false;
    // Nonce-bound PIN exchange state. The client challenge is answered with this
    // seeder's proof before the client discloses anything derived from the PIN.
    let mut challenge: Option<[u8; 32]> = None;
    let mut challenge_reply: Option<[u8; 32]> = None;
    // Keep ownership across inter-chunk disk/checkpoint/request gaps. An idle
    // inspection is not an active transfer and cannot claim background time.
    let mut active_transfer = None;
    let mut sending: std::collections::VecDeque<PendingSend> = std::collections::VecDeque::new();
    let mut deferred: Option<Message> = None;
    let mut read_bytes = 0usize;
    let mut send_bytes = 0usize;
    let mut request_deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    loop {
        if tokio::time::Instant::now() >= request_deadline {
            return Err("LocalSwarm next-request progress deadline exceeded".into());
        }
        // Reap transport sends the peer has already accepted; that frees the
        // window and keeps upload accounting current.
        while let Some(front) = sending.front_mut() {
            if !front.try_finish()? { break; }
            let bytes = front.bytes;
            sending.pop_front();
            send_bytes -= bytes;
        }
        // Admit requests that have already arrived, up to the window. This
        // never blocks, so a peer that pipelines its whole window cannot stall
        // the chunks already admitted.
        loop {
            if reads.len() + sending.len() >= SERVE_WINDOW { break; }
            let message = match deferred.take() {
                Some(message) => message,
                None => match messages.try_recv() {
                    Ok(Ok(message)) => message,
                    Ok(Err(error)) => return Err(error),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        return Err("LocalSwarm client reader stopped".into());
                    }
                },
            };
            if message.swarm_id() != Some(manifest.swarm_id.as_str()) { return Err("Cross-swarm LocalSwarm frame".into()); }
            match message {
                Message::Auth { token: supplied, .. } if !auth_seen && !manifest_sent && challenge.is_none() => {
                    // Legacy clients still send the deterministic token. Accept
                    // it so older builds and the reference engines interoperate,
                    // but never let a peer mix it with the nonce exchange.
                    auth_seen = true;
                    let matches = pin.is_none_or(|pin| constant_time_eq(
                        supplied.as_bytes(), protocol::auth_token(manifest.swarm_id.as_str(), pin).as_bytes()));
                    if !protocol::valid_hash(&supplied) || !matches {
                        return Err("LocalSwarm authentication failed".into());
                    }
                    authed = true;
                }
                Message::AuthChallenge { nonce, .. } if !auth_seen && !manifest_sent && challenge.is_none() => {
                    let Some(pin) = pin else { return Err("Unexpected LocalSwarm client message or authentication required".into()); };
                    let reply = protocol::auth_nonce()?;
                    let proof = protocol::pin_proof(pin, "server", manifest.swarm_id.as_str(), &nonce, &reply);
                    challenge = Some(nonce);
                    challenge_reply = Some(reply);
                    let frame = Message::AuthProof { swarm_id: manifest.swarm_id.clone(), nonce: reply, proof }.encode()?;
                    write_all(send, &frame, cancel).await?;
                }
                Message::AuthResponse { proof, .. } if challenge.is_some() && !authed => {
                    let Some(pin) = pin else { return Err("Unexpected LocalSwarm client message or authentication required".into()); };
                    let (Some(client_nonce), Some(reply)) = (challenge, challenge_reply) else {
                        return Err("LocalSwarm authentication failed".into());
                    };
                    if !constant_time_eq(&proof, &protocol::pin_proof(pin, "client", manifest.swarm_id.as_str(), &client_nonce, &reply)) {
                        return Err("LocalSwarm authentication failed".into());
                    }
                    authed = true;
                }
                message @ Message::ManifestRequest { .. } if authed && !manifest_sent => {
                    // Earlier requests keep their place: serve admitted reads
                    // and pipelined responses before the manifest frame.
                    if !reads.is_empty() || !sending.is_empty() {
                        deferred = Some(message);
                        break;
                    }
                    write_all(send, frame, cancel).await?; manifest_sent = true;
                }
                message @ Message::ChunkRequest { index, .. } if authed => {
                    let len = manifest.chunk_len(index)?;
                    // A request that does not fit the byte window waits for a
                    // transport completion instead of allocating past the cap.
                    if read_bytes + send_bytes + len > SERVE_BUFFER_BYTES {
                        deferred = Some(message);
                        break;
                    }
                    if active_transfer.is_none() { active_transfer = Some(stats.begin_transfer()?); }
                    reads.push_back(start_read(reader, manifest, index, len, publication.on_source_failure.clone()));
                    read_bytes += len;
                }
                Message::Bitfield { bits, .. } if bits.len() == manifest.chunk_hashes.len() => {},
                Message::Have { index, .. } if (index as usize) < manifest.chunk_hashes.len() => {},
                _ => return Err("Unexpected LocalSwarm client message or authentication required".into()),
            }
        }
        #[cfg(test)]
        stats.observe_serve_bytes(read_bytes + send_bytes);
        // Serve the oldest admitted read; later reads keep running in parallel.
        if let Some(read) = reads.pop_front() {
            read_bytes -= read.len;
            let data = read.task.await.map_err(|_| "LocalSwarm source reader stopped")??;
            if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
            let measured = super::profile::start();
            let header = chunk_header(manifest, read.index, data.len());
            if let Some(pending) = queue_chunk(send, &header, &data, budget, cancel, stats).await? {
                send_bytes += pending.bytes;
                sending.push_back(pending);
            }
            super::profile::record(2, data.len(), measured);
            request_deadline = tokio::time::Instant::now() + next_request_allowance(read.len);
            continue;
        }
        // The head request did not fit the byte window; only a completion can
        // make room, and later requests stay queued behind it.
        if deferred.is_some() {
            send_bytes -= wait_front(&mut sending, cancel).await?;
            continue;
        }
        // Nothing read-ahead: wait for the next request, a transport
        // completion, or the progress deadline.
        let message = if sending.is_empty() {
            Some(tokio::time::timeout_at(request_deadline, guarded(cancel, next_client_message(messages))).await
                .map_err(|_| "LocalSwarm next-request progress deadline exceeded")??)
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
                bytes = wait_front(&mut sending, cancel) => { send_bytes -= bytes?; None }
                message = tokio::time::timeout_at(request_deadline,
                    guarded(cancel, next_client_message(messages))) => Some(message
                        .map_err(|_| "LocalSwarm next-request progress deadline exceeded")??),
            }
        };
        if let Some(message) = message { deferred = Some(message); }
    }
}

fn chunk_header(manifest: &Manifest, index: u32, len: usize) -> [u8; 81] {
    let mut header = [0; 81];
    header[..4].copy_from_slice(&(77u32 + len as u32).to_be_bytes()); header[4] = 7;
    header[5..9].copy_from_slice(&64u32.to_be_bytes()); header[9..73].copy_from_slice(manifest.swarm_id.as_bytes());
    header[73..77].copy_from_slice(&index.to_be_bytes()); header[77..].copy_from_slice(&(len as u32).to_be_bytes());
    header
}

// Permit a minimum 1 B/s shared across the bounded native consumer population.
// This is a finite availability tradeoff, not an authentication/trust decision.
fn next_request_allowance(chunk_len: usize) -> Duration {
    Duration::from_secs(chunk_len as u64 * MAX_SHARED_CONSUMERS + IO_TIMEOUT.as_secs())
}

struct ClientFrameLimits {
    control_window: Instant, controls: usize,
    seen_have: Vec<bool>, bitfield_seen: bool,
}
impl ClientFrameLimits {
    fn new(chunk_count: usize) -> Self {
        Self { control_window: Instant::now(), controls: 0, seen_have: vec![false; chunk_count], bitfield_seen: false }
    }
    fn observe(&mut self, message: &Message) -> Result<(), String> {
        if self.control_window.elapsed() >= Duration::from_secs(60) { self.control_window = Instant::now(); self.controls = 0; }
        match message {
            Message::ChunkRequest { index, .. } => {
                if *index as usize >= self.seen_have.len() { return Err("Out-of-range LocalSwarm chunk request".into()); }
                // Useful requested bytes are bounded by queues and flow control,
                // not an arbitrary ceiling on unlimited LAN throughput.
                return Ok(());
            }
            Message::Have { index, .. } => {
                let seen = self.seen_have.get_mut(*index as usize).ok_or("Out-of-range LocalSwarm have")?;
                if !*seen { *seen = true; return Ok(()); }
            }
            Message::Bitfield { bits, .. } => {
                if bits.len() != self.seen_have.len() { return Err("Inconsistent LocalSwarm bitfield count".into()); }
                if !self.bitfield_seen { self.bitfield_seen = true; return Ok(()); }
            }
            Message::Auth { .. } | Message::AuthChallenge { .. } | Message::AuthResponse { .. } | Message::ManifestRequest { .. } => return Ok(()),
            _ => return Err("Unexpected LocalSwarm client frame".into()),
        }
        // Repeated availability controls make no new chunk progress.
        self.controls += 1;
        if self.controls > MAX_CONTROLS { return Err("LocalSwarm duplicate-control flood".into()); }
        Ok(())
    }
}

/// Hand one encoded chunk response to the transport. Quinn's flow-controlled
/// write completes here; the Apple bridge copies and queues every piece and
/// returns completion handles so the caller can keep several chunks in flight.
async fn queue_chunk(send: &mut SendStream, header: &[u8], data: &[u8], budget: &RateBudget,
    cancel: &CancellationToken, stats: &Arc<SeedStats>) -> Result<Option<PendingSend>, String>
{
    match send {
        SendStream::Quinn(stream) => {
            // Waiting for our own bandwidth budget never consumes an I/O deadline.
            guarded_for(cancel, CHUNK_WRITE_ALLOWANCE, async {
                stream.write_all(header).await.map_err(|error| format!("LocalSwarm write failed: {error}"))
            }).await?;
            let mut offset = 0;
            while offset < data.len() {
                let count = budget.reserve(data.len() - offset, cancel).await?;
                guarded_for(cancel, CHUNK_WRITE_ALLOWANCE, async {
                    stream.write_all(&data[offset..offset + count]).await.map_err(|error| format!("LocalSwarm write failed: {error}"))
                }).await?;
                stats.uploaded.fetch_add(count as u64, Ordering::Relaxed);
                offset += count;
            }
            Ok(None)
        }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        SendStream::Apple(connection) => {
            if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
            let mut completions = connection.dispatch_write(header, None)?;
            let mut offset = 0;
            while offset < data.len() {
                let count = budget.reserve(data.len() - offset, cancel).await?;
                if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
                completions.extend(connection.dispatch_write(&data[offset..offset + count], Some(stats.clone()))?);
                offset += count;
            }
            Ok(Some(PendingSend { bytes: data.len(), completions: completions.into() }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier;
    /// A lost byte stream may be redialed; every protocol, integrity, policy or
    /// cancellation verdict must stay terminal with its own message.
    #[test]
    fn only_a_lost_byte_stream_is_retryable() {
        for error in [
            "LocalSwarm read failed: connection lost",
            "LocalSwarm read failed: LocalSwarm Apple QUIC connection failed or left the local link",
            "LocalSwarm read failed: Apple QUIC receive callback lost",
            "LocalSwarm write failed: Apple QUIC send callback lost",
            "Truncated LocalSwarm payload",
        ] { assert!(transient_transport_failure(error), "Should retry: {error}"); }
        for error in [
            "LocalSwarm operation cancelled",
            "LocalSwarm connection stopped",
            "Local network policy interrupted LocalSwarm",
            "Mobile network policy interrupted LocalSwarm",
            "LocalSwarm chunk checksum mismatch",
            "LocalSwarm source changed or returned corrupt bytes",
            "Cross-swarm LocalSwarm frame",
            "LocalSwarm control message flood",
            "Unexpected LocalSwarm message while fetching chunk",
            "LocalSwarm payload progress deadline exceeded",
        ] { assert!(!transient_transport_failure(error), "Must stay terminal: {error}"); }
    }
    #[test]
    fn receive_admission_cancels_dials_without_retiring_established_activity() {
        let activity = Arc::new(ReceiveActivity::default());
        let dial = activity.dial_token().unwrap();
        assert_eq!(activity.active_sessions(), 0);
        let session = activity.session().unwrap();
        assert_eq!(activity.active_sessions(), 1);
        activity.set_accepting(false);
        assert!(dial.is_cancelled());
        assert!(activity.dial_token().is_none());
        assert!(activity.session().is_err());
        assert_eq!(activity.active_sessions(), 1);
        activity.set_accepting(true);
        assert!(!activity.dial_token().unwrap().is_cancelled());
        assert!(dial.is_cancelled());
        drop(session);
        assert_eq!(activity.active_sessions(), 0);
    }

    #[test]
    fn closed_admission_keeps_existing_activity_but_rejects_new_activity() {
        let stats = SeedStats::default();
        stats.set_accepting(true);
        assert_eq!(stats.active_transfers(), 0);
        let active = stats.begin_transfer().unwrap();
        assert_eq!(stats.active_transfers(), 1);
        stats.set_accepting(false);
        assert!(!stats.accepting());
        assert!(stats.begin_transfer().is_err());
        assert_eq!(stats.active_transfers(), 1);
        drop(active);
        assert_eq!(stats.active_transfers(), 0);
        assert!(!stats.accepting());
        stats.set_accepting(true);
        drop(stats.begin_transfer().unwrap());
        assert_eq!(stats.active_transfers(), 0);
    }

    #[tokio::test]
    async fn unlimited_batches_are_bounded_and_live_caps_keep_small_credit() {
        let budget = RateBudget::new(0);
        let cancel = CancellationToken::new();
        assert_eq!(budget.reserve(usize::MAX, &cancel).await.unwrap(), UNLIMITED_PIECE);
        assert_eq!(budget.reserve(1, &cancel).await.unwrap(), 1);
        budget.set_limit(1024 * 1024 * 1024);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(budget.reserve(usize::MAX, &cancel).await.unwrap() <= PIECE);
        budget.set_limit(0);
        assert_eq!(budget.reserve(UNLIMITED_PIECE, &cancel).await.unwrap(), UNLIMITED_PIECE);
        cancel.cancel();
        assert!(budget.reserve(UNLIMITED_PIECE, &cancel).await.is_err());
    }

    #[test]
    fn exact_leaf_pin_rejects_any_other_certificate() {
        let (certificates, _) = identity().unwrap();
        let verifier = ExactPin { certificate: certificates[0].clone(), provider: Arc::new(rustls::crypto::ring::default_provider()) };
        let name = ServerName::try_from("localhost").unwrap();
        let now = UnixTime::since_unix_epoch(Duration::from_secs(1));
        assert!(verifier.verify_server_cert(&certificates[0], &[], &name, &[], now).is_ok());
        let mut changed = certificates[0].as_ref().to_vec(); changed[0] ^= 1;
        assert!(verifier.verify_server_cert(&CertificateDer::from(changed), &[], &name, &[], now).is_err());
        assert!(verifier.verify_server_cert(&certificates[0], &certificates, &name, &[], now).is_err());
    }
    #[tokio::test]
    async fn cancelled_budget_releases_waiters_and_zero_is_unlimited() {
        let budget = Arc::new(RateBudget::new(1));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(budget.acquire(PIECE, &cancel).await.is_err());
        let active = CancellationToken::new();
        budget.set_limit(0);
        budget.acquire(PIECE, &active).await.unwrap();
    }
    #[tokio::test]
    async fn live_waiters_wake_on_cancel_and_limit_change() {
        let budget = Arc::new(RateBudget::new(1));
        let cancel = CancellationToken::new();
        let waiter_budget = budget.clone(); let waiter_cancel = cancel.clone();
        let waiter = tokio::spawn(async move { waiter_budget.acquire(PIECE, &waiter_cancel).await });
        tokio::task::yield_now().await;
        cancel.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(2), waiter).await.unwrap().unwrap().is_err());
        let waiter_budget = budget.clone();
        let waiter = tokio::spawn(async move { waiter_budget.acquire(PIECE, &CancellationToken::new()).await });
        tokio::task::yield_now().await;
        budget.set_limit(0);
        tokio::time::timeout(Duration::from_secs(2), waiter).await.unwrap().unwrap().unwrap();
    }
    #[tokio::test]
    async fn fifo_budget_mixed_sizes_make_progress_without_exceeding_credit() {
        let budget = Arc::new(RateBudget::new(100));
        let turn = budget.turn.lock().await;
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        let started = Instant::now();
        let mut tasks = Vec::new();
        for (id, maximum) in [1usize, PIECE, 3].into_iter().enumerate() {
            let budget = budget.clone(); let sender = sender.clone();
            tasks.push(tokio::spawn(async move {
                for _ in 0..3 {
                    let granted = budget.reserve(maximum, &CancellationToken::new()).await.unwrap();
                    sender.send((id, granted, Instant::now())).await.unwrap();
                }
            }));
            tokio::task::yield_now().await;
        }
        drop(turn); drop(sender);
        let mut ids = Vec::new(); let mut total = 0usize;
        while let Some((id, granted, at)) = tokio::time::timeout(Duration::from_secs(5), receiver.recv()).await.unwrap() {
            assert!(granted > 0 && granted <= [1usize, PIECE, 3][id]);
            total += granted;
            assert!(total as f64 <= at.duration_since(started).as_secs_f64() * 100.0 + 1.0);
            ids.push(id);
        }
        for task in tasks { task.await.unwrap(); }
        assert_eq!(ids, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
    }

    #[tokio::test]
    async fn cancelling_queued_budget_waiter_releases_its_fifo_position() {
        let budget = Arc::new(RateBudget::new(100));
        let turn = budget.turn.lock().await;
        let mut tasks = Vec::new();
        let middle = CancellationToken::new();
        for id in 0..3 {
            let budget = budget.clone();
            let cancel = if id == 1 { middle.clone() } else { CancellationToken::new() };
            tasks.push(tokio::spawn(async move { budget.reserve(PIECE, &cancel).await }));
            tokio::task::yield_now().await;
        }
        middle.cancel();
        let cancelled = tasks.remove(1);
        assert!(tokio::time::timeout(Duration::from_secs(2), cancelled).await.unwrap().unwrap().is_err());
        drop(turn);
        for task in tasks {
            assert!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap() > 0);
        }
    }

    #[tokio::test]
    async fn immediate_address_failure_does_not_hide_a_usable_later_address() {
        let server = Endpoint::server(server_config().unwrap(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let address = server.local_addr().unwrap();
        let accepted_server = server.clone();
        let accepted = tokio::spawn(async move {
            let connection = accepted_server.accept().await.unwrap().await.unwrap();
            connection.closed().await;
        });
        let cancel = CancellationToken::new();
        let invalid = "0.0.0.0:0".parse().unwrap();
        let (endpoint, connection, _) = connect_addresses(&[invalid, address], &cancel, client_config().unwrap(), 0, 0).await.unwrap();
        assert_eq!(connection.remote_address(), address);
        assert_eq!(endpoint.local_addr().unwrap().ip(), address.ip(), "The client socket must retain its actual selected local address");
        connection.close(0u32.into(), b"test complete");
        endpoint.close(0u32.into(), b"test complete"); endpoint.wait_idle().await;
        server.close(0u32.into(), b"test complete");
        accepted.await.unwrap(); server.wait_idle().await;
        assert!(connect_addresses(&[invalid], &cancel, client_config().unwrap(), 0, 0).await.is_err());
    }

    #[test]
    fn bounded_control_rate_is_not_a_lifetime_heartbeat_limit() {
        let mut limits = ClientFrameLimits::new(2);
        let heartbeat = Message::Bitfield { swarm_id: "a".repeat(64), bits: vec![false; 2] };
        for _ in 0..3 {
            limits.control_window = Instant::now() - Duration::from_secs(61);
            for _ in 0..64 { limits.observe(&heartbeat).unwrap(); }
        }
        limits.control_window = Instant::now() - Duration::from_secs(61);
        for _ in 0..MAX_CONTROLS { limits.observe(&heartbeat).unwrap(); }
        assert!(limits.observe(&heartbeat).is_err());
    }

    #[test]
    fn useful_chunk_requests_and_first_haves_have_no_throughput_cap() {
        let mut limits = ClientFrameLimits::new(1024);
        for index in 0..1024 {
            limits.observe(&Message::Have { swarm_id: "a".repeat(64), index }).unwrap();
            limits.observe(&Message::ChunkRequest { swarm_id: "a".repeat(64), index }).unwrap();
        }
        let duplicate = Message::Have { swarm_id: "a".repeat(64), index: 0 };
        for _ in 0..MAX_CONTROLS { limits.observe(&duplicate).unwrap(); }
        assert!(limits.observe(&duplicate).is_err());
        assert!(limits.observe(&Message::ChunkRequest { swarm_id: "a".repeat(64), index: 1024 }).is_err());
    }

    fn transfer_fixture() -> Arc<Manifest> { transfer_fixture_with_size(1024 * 1024) }
    fn transfer_fixture_with_size(chunk_size: u32) -> Arc<Manifest> {
        let chunk_hash = hex::encode(Sha256::digest(vec![0x5a; chunk_size as usize]));
        let mut manifest = Manifest { zim: None,
            protocol_version: 1, swarm_id: String::new(), name: "wire-fixture.zim".into(),
            total_bytes: 2 * u64::from(chunk_size), chunk_size_bytes: chunk_size, chunk_hash_algo: "sha256".into(),
            files: vec![protocol::SwarmFile { path: "wire-fixture.zim".into(), size_bytes: 2 * u64::from(chunk_size),
                start_chunk_index: 0, end_chunk_index: 1 }],
            chunk_hashes: vec![chunk_hash.clone(), chunk_hash],
        };
        manifest.swarm_id = manifest.content_address();
        Arc::new(manifest)
    }

    #[tokio::test]
    async fn seeder_retires_source_on_manifest_mismatch_before_sending() {
        let manifest = transfer_fixture_with_size(16);
        let reader: ChunkReader = Arc::new(|index| Ok(vec![if index == 0 { 0x5a } else { 0 }; 16]));
        let cancel = CancellationToken::new();
        let offer = cancel.clone();
        let failure: SeedSourceFailure = Arc::new(move |_| offer.cancel());
        let valid = start_read(&reader, &manifest, 0, 16, Some(failure.clone())).task.await.unwrap();
        assert_eq!(valid.unwrap(), vec![0x5a; 16]);
        assert!(!cancel.is_cancelled());
        let corrupt = start_read(&reader, &manifest, 1, 16, Some(failure)).task.await.unwrap();
        assert!(corrupt.unwrap_err().contains("corrupt bytes"));
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    #[ignore = "real QUIC throttling regression takes at least 128 seconds"]
    async fn native_low_rate_chunks_survive_application_idle_deadline() {
        let manifest = transfer_fixture();
        let server = seed(manifest.clone(), "fixture-seeder".into(), Some("1234".into()),
            Arc::new(|_| Ok(vec![0x5a; 1024 * 1024])), Arc::new(RateBudget::new(0)),
            CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, Some("1234"), "fixture-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(16 * 1024))).await.unwrap();
        for index in 0..2 {
            let chunk = session.get_chunk(index).await.unwrap();
            assert_eq!(chunk.len(), 1024 * 1024);
            assert!(chunk.iter().all(|byte| *byte == 0x5a));
        }
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test]
    async fn closed_seed_admission_preserves_started_session_not_idle_inspection() {
        let manifest = transfer_fixture_with_size(4096);
        let server = seed(manifest.clone(), "admission-seeder".into(), None,
            Arc::new(|_| Ok(vec![0x5a; 4096])), Arc::new(RateBudget::new(0)),
            CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let stats = server.stats.clone();
        let mut active = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "active-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        let mut inspected = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "idle-inspector",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        assert_eq!(stats.active_transfers(), 0);
        assert_eq!(active.get_chunk(0).await.unwrap(), vec![0x5a; 4096]);
        assert_eq!(stats.active_transfers(), 1);
        stats.set_accepting(false);
        assert_eq!(active.get_chunk(1).await.unwrap(), vec![0x5a; 4096]);
        assert!(tokio::time::timeout(Duration::from_secs(5), inspected.get_chunk(0)).await.unwrap().is_err());
        assert_eq!(stats.active_transfers(), 1);
        inspected.shutdown().await;
        active.shutdown().await;
        server.shutdown().await;
        assert_eq!(stats.active_transfers(), 0);
        assert!(!stats.accepting());
    }

    #[tokio::test]
    async fn cancelling_real_throttled_receive_joins_cleanly() {
        let manifest = transfer_fixture();
        let server = seed(manifest.clone(), "fixture-seeder".into(), Some("1234".into()),
            Arc::new(|_| Ok(vec![0x5a; 1024 * 1024])), Arc::new(RateBudget::new(0)),
            CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let cancel = CancellationToken::new();
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, Some("1234"), "fixture-receiver",
            cancel.clone(), Arc::new(RateBudget::new(1))).await.unwrap();
        let transfer = tokio::spawn(async move {
            let result = session.get_chunk(0).await;
            session.shutdown().await;
            result
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(10), transfer).await.unwrap().unwrap().is_err());
        server.shutdown().await;
    }

    #[test]
    fn payload_progress_accepts_shared_minimum_rate_but_expires_real_stalls() {
        let start = tokio::time::Instant::now();
        let mut progress = PayloadProgress::new(PIECE, start);
        // Two upload consumers sharing 1 B/s: the old bytes+45 deadline
        // expired halfway through this otherwise continuously progressing piece.
        let mut now = start;
        for _ in 0..PIECE {
            now += Duration::from_secs(2);
            assert!(!progress.expired(now));
            progress.received(now);
        }
        assert!(now.duration_since(start) > Duration::from_secs(PIECE as u64 + 45));
        assert!(!progress.expired(now + PayloadProgress::idle_allowance() - Duration::from_nanos(1)));
        assert!(progress.expired(now + PayloadProgress::idle_allowance()));

        // The largest supported sharing population may legitimately have
        // more than 45 seconds between individual bytes.
        let mut progress = PayloadProgress::new(4, start);
        let mut now = start;
        for _ in 0..4 {
            now += Duration::from_secs(MAX_SHARED_CONSUMERS);
            assert!(!progress.expired(now));
            progress.received(now);
        }
        let absolute = start + next_request_allowance(4);
        assert_eq!(progress.deadline(), absolute);
        assert!(progress.expired(absolute));
    }

    #[tokio::test]
    async fn pipelined_requests_finish_without_any_additional_peer_frames() {
        let manifest = transfer_fixture_with_size(4096);
        let server = seed(manifest.clone(), "pipeline-seeder".into(), Some("1234".into()),
            Arc::new(|_| Ok(vec![0x5a; 4096])), Arc::new(RateBudget::new(4096)),
            CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, Some("1234"), "pipeline-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        // Go/Swift pipeline both requests before the throttled first response.
        for index in 0..2 {
            send_message(&mut session.send, &Message::ChunkRequest { swarm_id: manifest.swarm_id.clone(), index },
                &session.cancel).await.unwrap();
        }
        for index in 0..2 {
            let message = tokio::time::timeout(Duration::from_secs(8),
                receive(&mut session.recv, &session.cancel, &session.budget, Some((&manifest, index)), &[7]))
                .await.unwrap().unwrap();
            assert!(matches!(message, Message::ChunkResponse { index: received, data, .. }
                if received == index && data.len() == 4096 && data.iter().all(|byte| *byte == 0x5a)));
        }
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test]
    async fn partial_request_parser_survives_previous_response_completion() {
        let manifest = transfer_fixture_with_size(4096);
        let server = seed(manifest.clone(), "fragment-seeder".into(), None,
            Arc::new(|_| Ok(vec![0x5a; 4096])), Arc::new(RateBudget::new(4096)),
            CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "fragment-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        send_message(&mut session.send, &Message::ChunkRequest { swarm_id: manifest.swarm_id.clone(), index: 0 },
            &session.cancel).await.unwrap();
        let second = Message::ChunkRequest { swarm_id: manifest.swarm_id.clone(), index: 1 }.encode().unwrap();
        write_all(&mut session.send, &second[..2], &session.cancel).await.unwrap();
        let first = tokio::time::timeout(Duration::from_secs(8),
            receive(&mut session.recv, &session.cancel, &session.budget, Some((&manifest, 0)), &[7]))
            .await.unwrap().unwrap();
        assert!(matches!(first, Message::ChunkResponse { index: 0, .. }));
        write_all(&mut session.send, &second[2..], &session.cancel).await.unwrap();
        let second = tokio::time::timeout(Duration::from_secs(8),
            receive(&mut session.recv, &session.cancel, &session.budget, Some((&manifest, 1)), &[7]))
            .await.unwrap().unwrap();
        assert!(matches!(second, Message::ChunkResponse { index: 1, .. }));
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "Requires a live Wi-Fi/Ethernet interface for the production Apple local-link gate"]
    async fn apple_sender_to_concurrent_quinn_receivers_preserves_chunk_bytes() {
        let host = std::env::var("KIWIX_LOCALSWARM_TEST_HOST").unwrap_or_else(|_| {
            reachable_addresses().unwrap().into_iter().find(IpAddr::is_ipv4)
                .expect("A live local IPv4 interface is required").to_string()
        });
        let chunk_size = 1024 * 1024;
        let mut value = 0x47a9_d321_81ef_035bu64;
        let data = Arc::new((0..8 * chunk_size + 137).map(|_| {
            value ^= value << 13; value ^= value >> 7; value ^= value << 17; value as u8
        }).collect::<Vec<_>>());
        let hashes: Vec<_> = data.chunks(chunk_size).map(|chunk| hex::encode(Sha256::digest(chunk))).collect();
        let mut manifest = Manifest { zim: None,
            protocol_version: 1, swarm_id: String::new(), name: "Apple ownership regression".into(),
            total_bytes: data.len() as u64, chunk_size_bytes: chunk_size as u32, chunk_hash_algo: "sha256".into(),
            files: vec![protocol::SwarmFile { path: "ownership.zim".into(), size_bytes: data.len() as u64,
                start_chunk_index: 0, end_chunk_index: hashes.len() as u32 - 1 }], chunk_hashes: hashes,
        };
        manifest.swarm_id = manifest.content_address();
        manifest.validate().unwrap();
        let manifest = Arc::new(manifest);
        let frame = Arc::new(Message::ManifestResponse(serde_json::to_vec(manifest.as_ref()).unwrap()).encode().unwrap());
        let source = data.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            let start = index as usize * chunk_size;
            Ok(source[start..(start + chunk_size).min(source.len())].to_vec())
        });
        let server = seed_apple(manifest.clone(), "apple-ownership-seeder".into(), Some("738291".into()),
            reader, Arc::new(RateBudget::new(0)), CancellationToken::new(), frame, None).await.unwrap();
        let port = server.port;
        let mut receivers = JoinSet::new();
        for receiver in 0..4 {
            let host = host.clone(); let manifest = manifest.clone(); let expected = data.clone();
            receivers.spawn(async move {
                let mut session = connect(&host, port, &manifest.swarm_id, Some("738291"),
                    &format!("quinn-ownership-{receiver}"), CancellationToken::new(), Arc::new(RateBudget::new(0))).await?;
                assert!(session.connection.is_some(), "Receiver must exercise Quinn, not Apple");
                let result: Result<(), String> = async {
                    for offset in 0..manifest.chunk_hashes.len() {
                        let index = (offset + receiver) % manifest.chunk_hashes.len();
                        let bytes = session.get_chunk(index as u32).await?;
                        let start = index * chunk_size;
                        if bytes != expected[start..(start + chunk_size).min(expected.len())] {
                            return Err(format!("Receiver {receiver} changed chunk {index}"));
                        }
                    }
                    Ok(())
                }.await;
                session.shutdown().await;
                result
            });
        }
        let result: Result<(), String> = async {
            while let Some(result) = receivers.join_next().await { result.map_err(|error| error.to_string())??; }
            Ok(())
        }.await;
        let expected_upload = data.len() as u64 * 4;
        // A receiver can finish before the sender resumes its processed callback.
        let accounted = tokio::time::timeout(Duration::from_secs(5), async {
            while result.is_ok() && server.stats.uploaded() < expected_upload { tokio::task::yield_now().await; }
        }).await;
        let uploaded = server.stats.uploaded();
        server.shutdown().await;
        result.unwrap();
        accounted.expect("Sender did not finish accounting for verified payloads");
        assert_eq!(uploaded, expected_upload);
        println!("APPLE_QUINN_OWNERSHIP_PASS host={host} receivers=4 chunks={} bytes_per_receiver={} uploaded={uploaded} sha256={}",
            manifest.chunk_hashes.len(), data.len(), hex::encode(Sha256::digest(data.as_slice())));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "Bounded production Apple-to-Quinn throughput measurement; requires a live local interface"]
    async fn apple_sender_serial_quinn_throughput_measurement() {
        let host = std::env::var("KIWIX_LOCALSWARM_TEST_HOST").unwrap_or_else(|_| {
            reachable_addresses().unwrap().into_iter().find(IpAddr::is_ipv4)
                .expect("A live local IPv4 interface is required").to_string()
        });
        let manifest = transfer_fixture();
        let frame = Arc::new(Message::ManifestResponse(serde_json::to_vec(manifest.as_ref()).unwrap()).encode().unwrap());
        let server = seed_apple(manifest.clone(), "apple-measurement-seeder".into(), Some("738291".into()),
            Arc::new(|_| Ok(vec![0x5a; 1024 * 1024])), Arc::new(RateBudget::new(0)),
            CancellationToken::new(), frame, None).await.unwrap();
        let mut session = connect(&host, server.port, &manifest.swarm_id, Some("738291"),
            "quinn-measurement-receiver", CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        assert!(session.connection.is_some(), "Receiver must exercise Quinn, not Apple");
        // Serial cadence: one outstanding chunkRequest (the pre-pipeline path).
        for trial in 0..3 {
            let started = Instant::now();
            let mut received = 0u64;
            for chunk in 0..128 {
                let bytes = tokio::time::timeout(Duration::from_secs(30), session.get_chunk(chunk % 2))
                    .await.expect("Bounded benchmark chunk timed out").unwrap();
                assert_eq!(bytes.len(), 1024 * 1024);
                assert!(bytes.iter().all(|byte| *byte == 0x5a));
                received += bytes.len() as u64;
            }
            let elapsed = started.elapsed();
            println!("APPLE_QUINN_SERIAL_MEASUREMENT host={host} trial={trial} bytes={received} elapsed_ns={} bytes_per_second={:.0}",
                elapsed.as_nanos(), received as f64 / elapsed.as_secs_f64());
        }
        // Pipelined cadence: PIPELINE_WINDOW outstanding requests per connection,
        // consumed in request order. Same peer, same framing, same verification.
        for trial in 0..3 {
            let started = Instant::now();
            let mut received = 0u64;
            for batch in 0..(128 / PIPELINE_WINDOW) {
                let indices: Vec<u32> = (0..PIPELINE_WINDOW).map(|offset| ((batch * PIPELINE_WINDOW + offset) % 2) as u32).collect();
                let mut pipeline = session.pipeline(&indices).await.unwrap();
                while let Some((_, bytes)) = tokio::time::timeout(Duration::from_secs(30), pipeline.next())
                    .await.expect("Bounded benchmark batch timed out").unwrap() {
                    assert_eq!(bytes.len(), 1024 * 1024);
                    assert!(bytes.iter().all(|byte| *byte == 0x5a));
                    received += bytes.len() as u64;
                }
            }
            let elapsed = started.elapsed();
            println!("APPLE_QUINN_PIPELINED_MEASUREMENT host={host} trial={trial} window={PIPELINE_WINDOW} bytes={received} elapsed_ns={} bytes_per_second={:.0}",
                elapsed.as_nanos(), received as f64 / elapsed.as_secs_f64());
        }
        session.shutdown().await;
        server.shutdown().await;
    }

    /// Apple seeder → Quinn receiver serve-rate measurement. The receiver's
    /// request cadence is the only variable: one request at a time versus a
    /// full `PIPELINE_WINDOW`. If the seeder still answers strictly one
    /// chunkRequest at a time, both cadences deliver the same bytes/second.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "Bounded production Apple-to-Quinn serve-rate measurement; requires a live local interface"]
    async fn apple_sender_serve_rate_pipelined_client_measurement() {
        let host = std::env::var("KIWIX_LOCALSWARM_TEST_HOST").unwrap_or_else(|_| {
            reachable_addresses().unwrap().into_iter().find(IpAddr::is_ipv4)
                .expect("A live local IPv4 interface is required").to_string()
        });
        let chunks: u32 = std::env::var("KIWIX_LOCALSWARM_BENCH_CHUNKS").ok()
            .and_then(|value| value.parse().ok()).unwrap_or(32);
        let chunk_size = 1024 * 1024;
        let (manifest, data) = pipeline_fixture(chunks, chunk_size);
        let frame = Arc::new(Message::ManifestResponse(serde_json::to_vec(manifest.as_ref()).unwrap()).encode().unwrap());
        let source = data.clone();
        let reader: ChunkReader = Arc::new(move |index| Ok(source[index as usize].clone()));
        let server = seed_apple(manifest.clone(), "apple-serve-rate-seeder".into(), Some("738291".into()),
            reader, Arc::new(RateBudget::new(0)), CancellationToken::new(), frame, None).await.unwrap();
        let mut session = connect(&host, server.port, &manifest.swarm_id, Some("738291"),
            "quinn-serve-rate-receiver", CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        assert!(session.connection.is_some(), "Receiver must exercise Quinn, not Apple");
        let started = Instant::now();
        let mut serial_bytes = 0u64;
        for index in 0..chunks {
            let bytes = tokio::time::timeout(Duration::from_secs(120), session.get_chunk(index))
                .await.expect("Serial benchmark chunk timed out").unwrap();
            assert_eq!(bytes, data[index as usize]);
            serial_bytes += bytes.len() as u64;
        }
        let serial_elapsed = started.elapsed();
        let started = Instant::now();
        let mut pipelined_bytes = 0u64;
        for batch in 0..chunks.div_ceil(PIPELINE_WINDOW as u32) {
            let indices: Vec<u32> = (batch * PIPELINE_WINDOW as u32
                ..(batch * PIPELINE_WINDOW as u32 + PIPELINE_WINDOW as u32).min(chunks)).collect();
            let mut pipeline = session.pipeline(&indices).await.unwrap();
            while let Some((index, bytes)) = tokio::time::timeout(Duration::from_secs(120), pipeline.next())
                .await.expect("Pipelined benchmark chunk timed out").unwrap() {
                assert_eq!(bytes, data[index as usize]);
                pipelined_bytes += bytes.len() as u64;
            }
        }
        let pipelined_elapsed = started.elapsed();
        println!("APPLE_QUINN_SERVE_RATE host={host} chunks={chunks} window={PIPELINE_WINDOW} \
            serial_bytes={serial_bytes} serial_elapsed_ns={} serial_bytes_per_second={:.0} \
            pipelined_bytes={pipelined_bytes} pipelined_elapsed_ns={} pipelined_bytes_per_second={:.0}",
            serial_elapsed.as_nanos(), serial_bytes as f64 / serial_elapsed.as_secs_f64(),
            pipelined_elapsed.as_nanos(), pipelined_bytes as f64 / pipelined_elapsed.as_secs_f64());
        session.shutdown().await;
        server.shutdown().await;
    }

    fn pipeline_fixture(chunks: u32, chunk_size: u32) -> (Arc<Manifest>, Vec<Vec<u8>>) {
        let data: Vec<Vec<u8>> = (0..chunks).map(|index| (0..chunk_size)
            .map(|offset| (index.wrapping_mul(37).wrapping_add(offset)) as u8).collect()).collect();
        let total = u64::from(chunks) * u64::from(chunk_size);
        let mut manifest = Manifest { zim: None,
            protocol_version: 1, swarm_id: String::new(), name: "pipeline-fixture.zim".into(),
            total_bytes: total, chunk_size_bytes: chunk_size, chunk_hash_algo: "sha256".into(),
            files: vec![protocol::SwarmFile { path: "pipeline-fixture.zim".into(), size_bytes: total,
                start_chunk_index: 0, end_chunk_index: chunks - 1 }],
            chunk_hashes: data.iter().map(|chunk| hex::encode(Sha256::digest(chunk))).collect(),
        };
        manifest.swarm_id = manifest.content_address();
        manifest.validate().unwrap();
        (Arc::new(manifest), data)
    }

    #[tokio::test]
    async fn pipeline_holds_the_whole_window_before_the_first_response_is_consumed() {
        let (manifest, data) = pipeline_fixture(8, 4096);
        let served = Arc::new(AtomicUsize::new(0));
        let source = data.clone(); let counter = served.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            counter.fetch_add(1, Ordering::AcqRel);
            Ok(source[index as usize].clone())
        });
        let server = seed(manifest.clone(), "pipeline-window-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "pipeline-window-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        // The window is a hard bound, not a hint.
        assert!(session.pipeline(&[0u32; PIPELINE_WINDOW + 1]).await.is_err());
        let mut pipeline = session.pipeline(&[0, 1, 2, 3, 4, 5, 6, 7]).await.unwrap();
        // The peer serves every requested chunk while the receiver consumes none,
        // which is only possible if all eight requests were already outstanding.
        tokio::time::timeout(Duration::from_secs(10), async {
            while served.load(Ordering::Acquire) < 8 { tokio::task::yield_now().await; }
        }).await.expect("The whole window was not outstanding before the first response");
        for expected in 0..8u32 {
            let (index, bytes) = pipeline.next().await.unwrap().expect("ordered response");
            assert_eq!(index, expected, "Responses must arrive in request order");
            assert_eq!(bytes, data[expected as usize], "Positional response carried another chunk");
            assert_eq!(pipeline.outstanding.len(), 7 - expected as usize);
        }
        assert!(pipeline.next().await.unwrap().is_none());
        drop(pipeline);
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "seeder serve-cadence measurement"]
    async fn seeder_serve_cadence_measurement() {
        let chunks = 16u32;
        let (manifest, data) = pipeline_fixture(chunks, 64 * 1024);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let source = data.clone();
        let reader_active = active.clone(); let reader_peak = peak.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            let now = reader_active.fetch_add(1, Ordering::AcqRel) + 1;
            reader_peak.fetch_max(now, Ordering::AcqRel);
            std::thread::sleep(Duration::from_millis(50));
            reader_active.fetch_sub(1, Ordering::AcqRel);
            Ok(source[index as usize].clone())
        });
        let server = seed(manifest.clone(), "serve-cadence-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "serve-cadence-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        let indices: Vec<u32> = (0..chunks).collect();
        let started = Instant::now();
        let mut pipeline = session.pipeline(&indices).await.unwrap();
        while let Some((index, bytes)) = tokio::time::timeout(Duration::from_secs(30), pipeline.next())
            .await.expect("Serve-cadence window timed out").unwrap() {
            assert_eq!(bytes, data[index as usize]);
        }
        let elapsed = started.elapsed();
        let bytes = u64::from(chunks) * 64 * 1024;
        println!("SEEDER_SERVE_CADENCE chunks={chunks} chunk_bytes=65536 peak_reads={} elapsed_ns={} bytes_per_second={:.0} chunks_per_second={:.1}",
            peak.load(Ordering::Acquire), elapsed.as_nanos(), bytes as f64 / elapsed.as_secs_f64(),
            f64::from(chunks) / elapsed.as_secs_f64());
        drop(pipeline);
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn seeder_overlaps_pipelined_chunk_reads_and_bounds_the_window() {
        let chunks = (SERVE_WINDOW * 2) as u32;
        let (manifest, data) = pipeline_fixture(chunks, 4096);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let source = data.clone();
        let reader_active = active.clone(); let reader_peak = peak.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            let now = reader_active.fetch_add(1, Ordering::AcqRel) + 1;
            reader_peak.fetch_max(now, Ordering::AcqRel);
            std::thread::sleep(Duration::from_millis(20));
            reader_active.fetch_sub(1, Ordering::AcqRel);
            Ok(source[index as usize].clone())
        });
        let server = seed(manifest.clone(), "serve-window-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "serve-window-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        let indices: Vec<u32> = (0..chunks).collect();
        let started = Instant::now();
        let mut pipeline = session.pipeline(&indices).await.unwrap();
        while let Some((index, bytes)) = tokio::time::timeout(Duration::from_secs(10), pipeline.next())
            .await.expect("Pipelined window timed out").unwrap() {
            assert_eq!(bytes, data[index as usize]);
        }
        let elapsed = started.elapsed();
        let observed = peak.load(Ordering::Acquire);
        assert!(observed > 1, "Seeder serialized source reads (peak concurrency {observed})");
        assert!(observed <= SERVE_WINDOW, "Seeder exceeded its per-peer read window: {observed}");
        // Serialized reads would need chunks * 20 ms = 640 ms for this window.
        assert!(elapsed < Duration::from_millis(500),
            "Pipelined client was throttled by the serve cadence: {elapsed:?}");
        assert!(server.stats.peak_serve_bytes() <= SERVE_BUFFER_BYTES);
        drop(pipeline);
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pipelined_responses_keep_request_order_when_reads_complete_out_of_order() {
        let (manifest, data) = pipeline_fixture(16, 4096);
        let source = data.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            // Lower indices finish last, so completion order cannot be the
            // order responses are written.
            std::thread::sleep(Duration::from_millis(10 + u64::from(15 - index) * 3));
            Ok(source[index as usize].clone())
        });
        let server = seed(manifest.clone(), "serve-order-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "serve-order-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        let indices = [15u32, 3, 9, 0, 12, 5, 14, 1, 8, 2, 11, 4, 13, 6, 10, 7];
        let mut pipeline = session.pipeline(&indices).await.unwrap();
        for expected in indices {
            let (index, bytes) = tokio::time::timeout(Duration::from_secs(10), pipeline.next())
                .await.expect("Ordered window timed out").unwrap().expect("ordered response");
            assert_eq!(index, expected, "Responses must follow request order");
            assert_eq!(bytes, data[expected as usize]);
        }
        assert!(pipeline.next().await.unwrap().is_none());
        drop(pipeline);
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_peer_requesting_every_chunk_cannot_inflate_seeder_buffers() {
        let chunks = 64u32;
        let chunk_size = 1024 * 1024u32;
        let (manifest, data) = pipeline_fixture(chunks, chunk_size);
        let source = data.clone();
        let reader: ChunkReader = Arc::new(move |index| Ok(source[index as usize].clone()));
        let server = seed(manifest.clone(), "hostile-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "hostile-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        // Ask for the whole share up front and never read a response: the
        // seeder must stop at its per-peer window instead of buffering 64 MiB.
        for index in 0..chunks {
            send_message(&mut session.send, &Message::ChunkRequest { swarm_id: manifest.swarm_id.clone(), index },
                &session.cancel).await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            while server.stats.peak_serve_bytes() < SERVE_WINDOW * chunk_size as usize { tokio::task::yield_now().await; }
        }).await.expect("Seeder did not fill its serve window");
        assert!(server.stats.peak_serve_bytes() <= SERVE_BUFFER_BYTES,
            "Seeder buffered {} bytes for one peer", server.stats.peak_serve_bytes());
        assert!(server.stats.uploaded() < u64::from(chunks) * u64::from(chunk_size),
            "Seeder accepted the whole share without the peer reading");
        session.shutdown().await; drop(session); server.shutdown().await;
    }

    #[tokio::test]
    async fn pipeline_error_closes_the_session_and_releases_the_window() {
        let (manifest, data) = pipeline_fixture(8, 4096);
        let source = data.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            if index == 2 { return Err("LocalSwarm source changed".into()); }
            Ok(source[index as usize].clone())
        });
        let server = seed(manifest.clone(), "pipeline-error-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "pipeline-error-receiver",
            CancellationToken::new(), Arc::new(RateBudget::new(0))).await.unwrap();
        let mut pipeline = session.pipeline(&[0, 1, 2, 3, 4, 5, 6, 7]).await.unwrap();
        assert_eq!(pipeline.next().await.unwrap().unwrap().0, 0);
        assert_eq!(pipeline.next().await.unwrap().unwrap().0, 1);
        assert!(pipeline.next().await.is_err(), "A failed response must fail the window");
        drop(pipeline);
        tokio::time::timeout(Duration::from_secs(10), session.connection.as_ref().unwrap().closed())
            .await.expect("A failed response must close the session");
        assert!(session.pipeline(&[3]).await.is_err(), "A retired session must not accept a new window");
        tokio::time::timeout(Duration::from_secs(10), session.shutdown()).await.expect("Retired session must join cleanly");
        drop(session); server.shutdown().await;
    }

    /// The chunk SHA check moved off the socket, so the handoff is now the only
    /// place a response becomes a `VerifiedChunk`: bytes whose hash does not
    /// match must never reach the staging writer.
    #[tokio::test]
    async fn pipelined_handoff_never_queues_an_unverified_chunk() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let shutdown = CancellationToken::new();
        let permits = Arc::new(tokio::sync::Semaphore::new(2));
        let pending = |data: Vec<u8>, matches: bool| PendingChunk {
            index: 5,
            permit: permits.clone().try_acquire_many_owned(1).unwrap(),
            hash: tokio::task::spawn_blocking(move || (data, matches)),
            measured: None,
        };
        let error = hand_off(0, pending(vec![0x11; 4096], false), &sender, &shutdown).await.unwrap_err();
        assert_eq!(error, "LocalSwarm chunk checksum mismatch");
        assert!(receiver.try_recv().is_err(), "A mismatched chunk must not reach the writer");
        assert!(hand_off(0, pending(vec![0x5a; 4096], true), &sender, &shutdown).await.unwrap());
        let (source, chunk) = receiver.try_recv().expect("A verified chunk is queued");
        assert_eq!(source, 0);
        assert_eq!(chunk.unwrap().data, vec![0x5a; 4096]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn four_max_chunk_sources_make_progress_when_prefetch_is_full() {
        let manifest = transfer_fixture_with_size(protocol::MAX_CHUNK_SIZE);
        manifest.validate().unwrap();
        let first_response_gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let read_gate = first_response_gate.clone();
        let reader: ChunkReader = Arc::new(move |index| {
            if index == 0 {
                let (open, changed) = &*read_gate;
                let _open = changed.wait_while(open.lock().unwrap(), |open| !*open).unwrap();
            }
            Ok(vec![0x5a; protocol::MAX_CHUNK_SIZE as usize])
        });
        let server = seed(manifest.clone(), "full-prefetch-seeder".into(), None, reader,
            Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let capacity = PREFETCH_BYTES / PREFETCH_UNIT;
        let units = (protocol::MAX_CHUNK_SIZE as usize).div_ceil(PREFETCH_UNIT);
        let prefetch = Arc::new(tokio::sync::Semaphore::new(capacity));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(PIPELINE_BUFFERED);
        let shutdown = CancellationToken::new();
        let mut sessions = Vec::new();
        for source in 0..4 {
            let session = connect("127.0.0.1", server.port, &manifest.swarm_id, None,
                &format!("full-prefetch-receiver-{source}"), CancellationToken::new(),
                Arc::new(RateBudget::new(0))).await.unwrap();
            sessions.push(session);
        }
        let mut sources = Vec::new();
        for (source, session) in sessions.into_iter().enumerate() {
            let (batch_sender, batches) = tokio::sync::mpsc::channel(1);
            batch_sender.try_send(vec![0, 1]).unwrap();
            drop(batch_sender);
            sources.push(tokio::spawn(pipeline_task(source, session, batches, sender.clone(),
                prefetch.clone(), shutdown.clone())));
        }
        drop(sender);
        // No first response can arrive until every session has charged its
        // first 16 MiB. This fixes the scheduling boundary at four claims
        // exhausting 64 MiB, with a second response outstanding on each source.
        let fully_reserved = tokio::time::timeout(Duration::from_secs(120), async {
            while prefetch.available_permits() != 0 { tokio::task::yield_now().await; }
        }).await;
        let (open, changed) = &*first_response_gate;
        *open.lock().unwrap() = true;
        changed.notify_all();
        // Open the blocking readers even if startup failed, so a failed
        // regression cannot leave the runtime waiting on stranded threads.
        fully_reserved.expect("All four sessions must reserve their first chunk");

        let mut first_chunks = Vec::new();
        let mut first_seen = [false; 4];
        for _ in 0..4 {
            let (source, chunk) = tokio::time::timeout(Duration::from_secs(120), receiver.recv())
                .await.expect("Every source must deliver its first chunk without lookahead budget")
                .expect("The source queue closed");
            let chunk = chunk.unwrap();
            assert!(!first_seen[source]);
            first_seen[source] = true;
            assert_eq!(chunk.index, 0);
            assert_eq!(chunk.data.len(), protocol::MAX_CHUNK_SIZE as usize);
            assert!(chunk.data.iter().all(|byte| *byte == 0x5a));
            assert_eq!(chunk.permit.as_ref().expect("Writer owns the byte claim").num_permits(), units);
            first_chunks.push(chunk);
        }
        assert_eq!(prefetch.available_permits(), 0, "Queueing does not release byte accounting");
        assert!(receiver.try_recv().is_err(), "No second payload fits until the writer finishes one");

        // A single completed write must suffice to restart every source. Keep
        // the other three first chunks charged while writing second chunks.
        drop(first_chunks.pop());
        let mut second_seen = [false; 4];
        for _ in 0..4 {
            let (source, chunk) = tokio::time::timeout(Duration::from_secs(120), receiver.recv())
                .await.expect("Writer progress must unblock a waiting source")
                .expect("The source queue closed");
            let chunk = chunk.unwrap();
            assert!(!second_seen[source]);
            second_seen[source] = true;
            assert_eq!(chunk.index, 1);
            assert_eq!(chunk.data.len(), protocol::MAX_CHUNK_SIZE as usize);
            assert!(chunk.data.iter().all(|byte| *byte == 0x5a));
            assert_eq!(chunk.permit.as_ref().expect("Writer owns the byte claim").num_permits(), units);
            assert_eq!(prefetch.available_permits(), 0, "All four live payloads remain charged");
            drop(chunk);
        }
        for source in sources {
            let session = tokio::time::timeout(Duration::from_secs(120), source).await
                .expect("A drained source must exit").unwrap();
            // Drop closes the connection immediately; shutdown() would wait for
            // endpoint idle and add ~17 s per source to this regression.
            drop(session);
        }
        drop(first_chunks);
        assert_eq!(prefetch.available_permits(), capacity);
        assert!(receiver.recv().await.is_none());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn available_prefetch_preserves_hash_and_socket_lookahead() {
        let prefetch = Arc::new(tokio::sync::Semaphore::new(2));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let shutdown = CancellationToken::new();
        let mut pending = Some(PendingChunk {
            index: 0,
            permit: prefetch.clone().try_acquire_owned().unwrap(),
            hash: tokio::task::spawn_blocking(|| (vec![0x5a; 4096], true)),
            measured: None,
        });
        let lookahead = reserve_prefetch(0, &mut pending, &sender, &prefetch, 1, &shutdown)
            .await.unwrap().unwrap();
        assert!(pending.is_some(), "Available capacity must retain verification/socket overlap");
        assert!(receiver.try_recv().is_err());
        assert_eq!(prefetch.available_permits(), 0);
        assert!(hand_off(0, pending.take().unwrap(), &sender, &shutdown).await.unwrap());
        let (_, first) = receiver.recv().await.unwrap();
        assert_eq!(prefetch.available_permits(), 0, "Writer still owns the first claim");
        drop(first);
        assert_eq!(prefetch.available_permits(), 1);
        drop(lookahead);
        assert_eq!(prefetch.available_permits(), 2);
    }

    #[tokio::test]
    async fn cancelling_a_pipeline_releases_the_window_and_closes_the_session() {
        let (manifest, data) = pipeline_fixture(8, 4096);
        let source = data.clone();
        let reader: ChunkReader = Arc::new(move |index| Ok(source[index as usize].clone()));
        // Throttled so no response can be ready when cancellation lands.
        let server = seed(manifest.clone(), "pipeline-cancel-seeder".into(), None, reader,
            Arc::new(RateBudget::new(1024)), CancellationToken::new(), None).await.unwrap();
        server.stats.set_accepting(true);
        let cancel = CancellationToken::new();
        let mut session = connect("127.0.0.1", server.port, &manifest.swarm_id, None, "pipeline-cancel-receiver",
            cancel.clone(), Arc::new(RateBudget::new(0))).await.unwrap();
        let mut pipeline = session.pipeline(&[0, 1, 2, 3, 4, 5, 6, 7]).await.unwrap();
        cancel.cancel();
        assert!(pipeline.next().await.is_err(), "Cancellation must retire the window");
        drop(pipeline);
        tokio::time::timeout(Duration::from_secs(10), session.connection.as_ref().unwrap().closed())
            .await.expect("Cancellation must close the session");
        tokio::time::timeout(Duration::from_secs(10), session.shutdown()).await.expect("Cancelled session must join cleanly");
        drop(session); server.shutdown().await;
    }
}
