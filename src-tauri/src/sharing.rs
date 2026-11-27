// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Opt-in read-only sharing. Public listeners never expose app IPC or sensors.
//! Each shared book gets a root-serving listener so root-relative assets work.
//! Service-worker registration is rejected at the HTTP edge, not merely by CSP.
//! The script-free index searches only selected archives (256 characters / 1024
//! bytes per query, 20 hits per archive, 100 total, two concurrent native searches).
//! Revocation cancels listener tasks, active socket I/O and response bodies,
//! and drains native archive calls before reporting that sharing has stopped.
//! Capability activation uses a script-free explicit same-site Continue link,
//! preserving Strict cookies even when the initial navigation was cross-site.
use crate::{
    archive::{Archive, SearchResults},
    localization::Localizer,
    server::{encode_path, parse_range, ArchiveWorkers},
    service::Service,
    store::{Book, Store},
};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use socket2::{Domain, Protocol, Socket, Type};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock, Weak,
    },
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{watch, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};
use uuid::Uuid;

#[path = "sharing_download.rs"]
pub(crate) mod download;
#[path = "sharing_reader.rs"]
mod reader;
#[cfg(test)]
#[path = "sharing_interop_tests.rs"]
mod interop_tests;

const QUERY_BYTES: usize = 1024;
const QUERY_CHARS: usize = 256;
const RESULTS_PER_BOOK: u32 = 20;
const RESULTS_TOTAL: usize = 100;
const OPDS_PAGE_DEFAULT: usize = 50;
const OPDS_PAGE_MAX: usize = 200;
#[cfg(not(mobile))]
const LIBRARY_ARCHIVE_CACHE: usize = 32;
#[cfg(mobile)]
const LIBRARY_ARCHIVE_CACHE: usize = 8;
#[cfg(not(mobile))]
const MAX_LIBRARY_HOSTS: usize = 32;
#[cfg(mobile)]
const MAX_LIBRARY_HOSTS: usize = 8;
/// Availability results are reused for this long, then re-stat'ed, so a file
/// removed or replaced while sharing is refused within two seconds.
const LIBRARY_AVAILABILITY_TTL: Duration = Duration::from_secs(2);
const LIBRARY_AVAILABILITY_MAX: usize = 8192;
const BOOTSTRAP_BODY_BYTES: usize = 256;
const SHARING_MAX_CONNECTIONS: usize = 128;
/// Longest silence tolerated inside a response body. A stalled client stops
/// making progress, not merely elapsed time, so a slow link keeps streaming.
const SHARING_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Absolute budget for delivering a complete request head. Connections are
/// admitted before a client is authenticated, so a slowloris must not hold one
/// of the bounded permits for the long body deadline.
const SHARING_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Absolute budget for a declared request body once its head arrived. Sharing
/// only accepts small PIN/bootstrap forms; handlers also bound these bodies.
const SHARING_BODY_TIMEOUT: Duration = Duration::from_secs(30);
/// Request head bytes kept for framing. Hyper's default HTTP/1 read buffer
/// accepts a head up to about 408 KiB, so a head that outgrows this storage cap
/// is not rejected here: the scan keeps looking for its terminator without
/// keeping the bytes, and a completed oversized head is treated as unaccounted
/// rather than misparsed.
const SHARING_HEAD_BYTES: usize = 16 * 1024;
/// HTTP/2 clients (prior knowledge) multiplex requests on one connection, so
/// the HTTP/1 request-phase machine does not apply to them.
const HTTP2_PREFACE: &[u8] = b"PRI * HTTP/2.0";
const PIN_ATTEMPTS: usize = 5;
const PIN_WINDOW: Duration = Duration::from_secs(30);
/// A spent PIN budget locks the form for this long, doubling on each new
/// exhausted budget. The ceiling keeps a legitimate owner from waiting forever
/// while a guesser gains at most five attempts per window.
const PIN_LOCKOUT_MAX: Duration = Duration::from_secs(15 * 60);
/// Shortest accepted share PIN. A four-digit PIN is a hundredfold cheaper to
/// guess offline and online, so the minimum is raised to six digits.
const PIN_MIN_DIGITS: usize = 6;
const PIN_MAX_DIGITS: usize = 12;
/// Archive responses are never legitimately framed. Cookies ignore ports, so a
/// page on another loopback/LAN port of this host must not frame reader content.
const ARCHIVE_CSP: &str = "default-src 'self' blob: data:; script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob:; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-src 'none'; object-src 'none'; form-action 'self'; base-uri 'self'; frame-ancestors 'none'";

/// Starting a session is the owner's explicit consent. Selected scope serves
/// only its admitted installations; library scope follows the live library.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SharingScope {
    Selected,
    #[default]
    Library,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum SharingBinding {
    #[serde(rename = "loopback")]
    Loopback {},
    #[serde(rename = "ipv4All")]
    Ipv4All {},
    #[serde(rename = "ipv6All")]
    Ipv6All {},
    #[serde(rename = "dualStackAll")]
    DualStackAll {},
    /// Serve through the Wi-Fi access point this phone is hosting. The address
    /// is the live hotspot interface, never the client Wi-Fi link that the
    /// hotspot replaced; it is resolved at start and revalidated as a local
    /// interface. The listener is retired with the reservation.
    #[serde(rename = "hotspot")]
    Hotspot {},
    #[serde(rename = "interface")]
    Interface { address: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct SharingInterface {
    name: String,
    address: String,
    family: &'static str,
}

// Scoped/link-local addresses are deliberately unavailable: links without an
// interface scope are unreachable, and browser support for scoped URL hosts is
// inconsistent. No DNS or caller-supplied hostnames enter the binding path.
fn safe_interface(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_link_local()
                && ip.octets()[0] != 0
        }
        IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && (ip.segments()[0] & 0xffc0) != 0xfe80
                && ip.to_ipv4_mapped().is_none()
        }
    }
}

pub fn interfaces() -> Result<Vec<SharingInterface>, String> {
    let mut addresses: Vec<_> = if_addrs::get_if_addrs()
        .map_err(|error| format!("Cannot list local network interfaces: {error}"))?
        .into_iter()
        .map(|interface| { let address = interface.ip(); (interface.name, address) })
        .filter(|(_, address)| safe_interface(*address))
        .map(|(name, address)| SharingInterface {
            name,
            address: address.to_string(),
            family: if address.is_ipv4() { "ipv4" } else { "ipv6" },
        })
        .collect();
    addresses.sort_by(|a, b| (&a.name, &a.address).cmp(&(&b.name, &b.address)));
    addresses.dedup_by(|a, b| a.name == b.name && a.address == b.address);
    Ok(addresses)
}

/// The one-click default binds one private LAN interface, never a public,
/// wildcard, loopback or unscoped link-local address. Advanced bindings remain
/// available as explicit choices.
pub fn default_binding() -> Result<SharingBinding, String> {
    let available = crate::localswarm::reachable_addresses()?;
    auto_lan_binding(&available, crate::localswarm::routed_local_address())
}

/// `available` holds only real LAN interfaces. Among them, the address the
/// system is actually routing traffic from wins, so a second private adapter
/// that merely sorts first is not handed to the other device as a dead link.
/// `routed` grants nothing by itself: an address that is not a private LAN
/// candidate (a VPN or carrier source address) is ignored.
fn auto_lan_binding(available: &[IpAddr], routed: Option<IpAddr>) -> Result<SharingBinding, String> {
    let eligible = |address: &IpAddr| matches!(address, IpAddr::V4(ip) if ip.is_private() && !ip.is_loopback())
        && safe_interface(*address);
    routed.filter(|address| eligible(address) && available.contains(address))
        .or_else(|| available.iter().copied().find(eligible))
        .map(|address| SharingBinding::Interface { address: address.to_string() })
        .ok_or_else(|| "Connect to a trusted local Wi-Fi or Ethernet network before sharing, or choose an advanced network binding".into())
}

fn binding_addresses(
    service: Option<&Service>,
    binding: &SharingBinding,
) -> Result<(IpAddr, Vec<IpAddr>), String> {
    let v4 = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
    if *binding == (SharingBinding::Loopback {}) {
        return Ok((v4, vec![v4]));
    }
    let mut local: Vec<IpAddr> = interfaces()?
        .into_iter()
        .filter_map(|interface| interface.address.parse().ok())
        .collect();
    // Loopback remains a real, reachable access URL for wildcard listeners.
    local.extend([v4, v6]);
    local.sort_by_key(|address| (address.is_loopback(), address.is_ipv6(), *address));
    local.dedup();
    let bind = match binding {
        SharingBinding::Loopback {} => unreachable!(),
        SharingBinding::Ipv4All {} => {
            local.retain(IpAddr::is_ipv4);
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        }
        SharingBinding::Ipv6All {} => {
            local.retain(IpAddr::is_ipv6);
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        }
        SharingBinding::DualStackAll {} => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        SharingBinding::Hotspot {} => {
            let service = service.ok_or(
                "Sharing over a Wi-Fi hotspot requires the running application's hotspot reservation",
            )?;
            let address = service.hotspot_binding_address()?;
            if !safe_interface(address) {
                return Err("The hotspot interface has no usable IPv4 address; restart the hotspot and try again".into());
            }
            local = vec![address];
            address
        }
        SharingBinding::Interface { address } => {
            let address: IpAddr = address
                .parse()
                .map_err(|_| "Choose a literal IP address from a local interface".to_string())?;
            if !safe_interface(address) || !local.contains(&address) {
                return Err("The selected IP address is not an available local interface; refresh interfaces and choose again".into());
            }
            local = vec![address];
            address
        }
    };
    Ok((bind, local))
}

fn bind_listener(
    address: IpAddr,
    port: u16,
    binding: &SharingBinding,
) -> Result<TcpListener, String> {
    let socket = Socket::new(
        if address.is_ipv6() {
            Domain::IPV6
        } else {
            Domain::IPV4
        },
        Type::STREAM,
        Some(Protocol::TCP),
    )
    .map_err(|error| format!("Cannot create sharing socket: {error}"))?;
    socket
        .set_reuse_address(true)
        .map_err(|error| format!("Cannot configure sharing socket for restart: {error}"))?;
    if address.is_ipv6() {
        socket
            .set_only_v6(*binding != (SharingBinding::DualStackAll {}))
            .map_err(|error| format!("Cannot configure IPv6 sharing mode: {error}"))?;
    }
    let target = SocketAddr::new(address, port);
    socket.bind(&target.into()).map_err(|error| format!(
        "Cannot bind sharing address {target}: {error}. Choose an available port or another local interface."
    ))?;
    socket
        .listen(128)
        .map_err(|error| format!("Cannot listen on sharing address {target}: {error}"))?;
    socket
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    TcpListener::from_std(socket.into()).map_err(|error| error.to_string())
}

fn access_url(host: IpAddr, port: u16, path: &str) -> String {
    format!("http://{}{path}", SocketAddr::new(host, port))
}

/// Revocation also drains native calls: dropping a blocking-task future alone
/// cannot cancel native archive reads already queued on the blocking pool.
#[derive(Clone)]
struct SharingCancellation(Arc<CancellationState>);
struct CancellationState {
    stopped: AtomicBool,
    native: RwLock<()>,
    signal: watch::Sender<bool>,
    workers: Arc<ArchiveWorkers>,
    search_cancel: tokio_util::sync::CancellationToken,
    searches: Arc<Semaphore>,
    connections: Arc<Semaphore>,
}
impl SharingCancellation {
    fn new() -> Self {
        Self(Arc::new(CancellationState {
            stopped: AtomicBool::new(false),
            native: RwLock::new(()),
            signal: watch::channel(false).0,
            workers: Arc::new(ArchiveWorkers::default()),
            search_cancel: tokio_util::sync::CancellationToken::new(),
            searches: Arc::new(Semaphore::new(2)),
            connections: Arc::new(Semaphore::new(SHARING_MAX_CONNECTIONS)),
        }))
    }
    fn is_cancelled(&self) -> bool {
        self.0.stopped.load(Ordering::Acquire)
    }
    async fn cancelled(self) {
        let mut signal = self.0.signal.subscribe();
        let _ = signal.wait_for(|stopped| *stopped).await;
    }
    fn native<T>(&self, work: impl FnOnce() -> T) -> Result<T, io::Error> {
        let _guard = self
            .0
            .native
            .read()
            .unwrap_or_else(|error| error.into_inner());
        if self.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Sharing stopped",
            ));
        }
        Ok(work())
    }
    fn begin_cancel(&self) {
        self.0.search_cancel.cancel();
        self.0.workers.close();
        self.0.stopped.store(true, Ordering::Release);
        self.0.signal.send_replace(true);
    }
    fn drain_native(&self) {
        // No native work can begin after revocation, including work whose
        // spawn_blocking task had not started. Finish any in-flight call before
        // sharing_stop/privacy/read-only can report that sharing is off.
        drop(
            self.0
                .native
                .write()
                .unwrap_or_else(|error| error.into_inner()),
        );
    }
    fn cancel(&self) {
        self.begin_cancel();
        self.drain_native();
    }
}

struct SharingListener {
    listener: TcpListener,
    cancellation: SharingCancellation,
}
impl axum::serve::Listener for SharingListener {
    type Io = SharingIo;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (socket, address) = axum::serve::Listener::accept(&mut self.listener).await;
            if let Ok(permit) = self.cancellation.0.connections.clone().try_acquire_owned() {
                return (SharingIo::new(socket, &self.cancellation, permit, SHARING_TIMEOUTS), address);
            }
            // Do not let one listener queue an unbounded number of sockets
            // or spin while the session's active connections are saturated.
            drop(socket);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// The index just past an HTTP/1 request head terminator, tolerating a bare
/// line feed so a hand-written client is not misclassified.
fn request_head_end(head: &[u8]) -> Option<usize> {
    head.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| at + 4)
        .or_else(|| {
            head.windows(2)
                .position(|window| window == b"\n\n")
                .map(|at| at + 2)
        })
}

/// What the request head declared about its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeclaredBody {
    /// No body, or an explicit zero length.
    None,
    /// A declared number of body bytes.
    Length(u64),
    /// Chunked, repeated, malformed or beyond the head storage cap. Reads can
    /// no longer be attributed to the next request, so the connection never
    /// re-enters a request phase.
    Unaccounted,
}

/// A request body framing decision. Hyper rejects repeated or malformed
/// lengths itself; a handler that still reads such a body bounds it with its
/// own body limit and timeout.
fn declared_body(head: &[u8]) -> DeclaredBody {
    let Ok(text) = std::str::from_utf8(head) else {
        return DeclaredBody::Unaccounted;
    };
    let mut length = None;
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        let Some((name, value)) = line.split_once(':') else { continue };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if value.to_ascii_lowercase().contains("chunked") {
                return DeclaredBody::Unaccounted;
            }
        } else if name.eq_ignore_ascii_case("content-length") {
            // A repeated length is ambiguous and Hyper rejects it.
            if length.is_some() {
                return DeclaredBody::Unaccounted;
            }
            let Ok(declared) = value.trim().parse::<u64>() else {
                return DeclaredBody::Unaccounted;
            };
            length = Some(declared);
        }
    }
    match length {
        Some(0) | None => DeclaredBody::None,
        Some(declared) => DeclaredBody::Length(declared),
    }
}

/// Per-connection deadlines. A client must deliver its request head and any
/// declared body within short absolute windows; only a response in progress
/// uses the long idle deadline. Time a handler spends between the request and
/// its first response byte is therefore not charged against the client.
#[derive(Clone, Copy)]
struct SharingTimeouts {
    request: Duration,
    body: Duration,
    idle: Duration,
}
const SHARING_TIMEOUTS: SharingTimeouts = SharingTimeouts {
    request: SHARING_REQUEST_TIMEOUT,
    body: SHARING_BODY_TIMEOUT,
    idle: SHARING_IDLE_TIMEOUT,
};

/// Result of feeding request-head bytes.
enum HeadFeed {
    /// The head terminator has not arrived yet.
    Pending,
    /// The client speaks HTTP/2 with prior knowledge.
    Http2,
    /// The head is complete; `arrived` bytes of the following section came in
    /// the same read.
    Complete { declared: DeclaredBody, arrived: u64 },
}

#[derive(Clone, Copy)]
enum RequestState {
    /// A request head is being read under the short absolute budget.
    Head,
    /// A declared body is still outstanding under the absolute body budget.
    Body { remaining: u64 },
    /// The current request is fully framed; further bytes are the next
    /// request, which gets a fresh head budget.
    Framed,
    /// The current request's body framing cannot be accounted for (chunked,
    /// malformed, oversized head), so reads may be a body drain and are never
    /// treated as the next request. The response idle bound governs.
    Unaccounted,
}

/// Wake and abort the connection even if its client is not polling the body
/// (for example, a download blocked by TCP backpressure).
struct SharingIo {
    socket: TcpStream,
    _permit: OwnedSemaphorePermit,
    timeouts: SharingTimeouts,
    request: RequestState,
    /// Head/body budget while a request is still incomplete.
    request_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    /// Response idle bound. Armed at the first response byte, reset on write
    /// progress, and kept until the next request's head completes.
    response_idle: Option<Pin<Box<tokio::time::Sleep>>>,
    /// Request head bytes kept for framing, at most `SHARING_HEAD_BYTES`.
    head: Vec<u8>,
    /// Last bytes seen after the storage cap, for terminator detection only.
    head_tail: Vec<u8>,
    /// The head outgrew the storage cap, so its framing is unknown.
    head_truncated: bool,
    /// A response byte was already written for the current request.
    responded: bool,
    /// The current response has been flushed; bytes after a framed request may
    /// now be treated as the next request.
    response_done: bool,
    /// Pipelined bytes held until the current response is flushed.
    next_request: Vec<u8>,
    /// This connection speaks HTTP/2 and keeps the plain idle bound.
    http2: bool,
    stopped: Pin<Box<dyn Future<Output = ()> + Send>>,
    cancelled: bool,
}
impl SharingIo {
    fn new(socket: TcpStream, cancellation: &SharingCancellation, permit: OwnedSemaphorePermit, timeouts: SharingTimeouts) -> Self {
        Self {
            socket,
            _permit: permit,
            timeouts,
            request: RequestState::Head,
            request_deadline: Some(Box::pin(tokio::time::sleep(timeouts.request))),
            response_idle: None,
            head: Vec::new(),
            head_tail: Vec::new(),
            head_truncated: false,
            responded: false,
            response_done: true,
            next_request: Vec::new(),
            http2: false,
            stopped: Box::pin(cancellation.clone().cancelled()),
            cancelled: false,
        }
    }

    // The arm/reset helpers read the period inside the method: calling
    // `self.arm_request(self.timeouts.request)` would borrow `self` both
    // mutably (the `Pin<&mut Self>` deref) and immutably (the argument).
    fn arm_request(&mut self) {
        let request = self.timeouts.request;
        self.request_deadline = Some(Box::pin(tokio::time::sleep(request)));
    }

    fn arm_body(&mut self) {
        let body = self.timeouts.body;
        self.request_deadline = Some(Box::pin(tokio::time::sleep(body)));
    }

    fn disarm_request(&mut self) {
        self.request_deadline = None;
    }

    fn arm_response(&mut self) {
        let idle = self.timeouts.idle;
        self.response_idle = Some(Box::pin(tokio::time::sleep(idle)));
    }

    fn reset_response(&mut self) {
        let idle = self.timeouts.idle;
        match self.response_idle.as_mut() {
            Some(deadline) => deadline.as_mut().reset(tokio::time::Instant::now() + idle),
            None => self.arm_response(),
        }
    }

    fn disarm_response(&mut self) {
        self.response_idle = None;
    }

    /// Account for request bytes. A fully framed request may be followed by the
    /// next request; an unaccounted body may not, because its bytes could be a
    /// chunked body still being drained.
    fn received(&mut self, bytes: &[u8]) {
        if self.http2 {
            self.reset_response();
            return;
        }
        match self.request {
            RequestState::Head => match self.feed_head(bytes) {
                HeadFeed::Pending => {}
                HeadFeed::Http2 => {
                    self.http2 = true;
                    self.head.clear();
                    self.head_tail.clear();
                    self.request = RequestState::Unaccounted;
                    self.disarm_request();
                    // An idle multiplexed connection is still bounded.
                    self.arm_response();
                }
                HeadFeed::Complete { declared, arrived } => self.head_complete(declared, arrived),
            },
            RequestState::Body { remaining } => {
                let remaining = remaining.saturating_sub(bytes.len() as u64);
                if remaining > 0 {
                    self.request = RequestState::Body { remaining };
                } else {
                    // The declared body is complete; the handler owns the
                    // connection until it writes, with no connection budget.
                    self.request = RequestState::Framed;
                    self.disarm_request();
                }
            }
            RequestState::Framed => {
                // Do not let a pipelined request start a fresh head budget
                // while the previous response is still streaming.
                if !self.response_done {
                    if self.next_request.len().saturating_add(bytes.len())
                        > SHARING_HEAD_BYTES.saturating_mul(4)
                    {
                        self.next_request.clear();
                        self.request = RequestState::Unaccounted;
                        self.disarm_request();
                    } else {
                        self.next_request.extend_from_slice(bytes);
                    }
                    return;
                }
                // The next request may only start after the previous response
                // completed. Keep that response's idle bound armed until this
                // head completes, then hand the connection to the new handler.
                self.request = RequestState::Head;
                self.head.clear();
                self.head_tail.clear();
                self.head_truncated = false;
                self.arm_request();
                self.received(bytes);
            }
            RequestState::Unaccounted => {
                // Never re-enter a request phase here: these bytes may be an
                // unread chunked body. The response idle bound governs.
            }
        }
    }

    /// Feed request-head bytes, keeping at most `SHARING_HEAD_BYTES` for
    /// framing while still finding the terminator of a larger head.
    fn feed_head(&mut self, bytes: &[u8]) -> HeadFeed {
        if !self.head_truncated {
            let room = SHARING_HEAD_BYTES.saturating_sub(self.head.len());
            let stored = bytes.len().min(room);
            self.head.extend_from_slice(&bytes[..stored]);
            if self.head.starts_with(HTTP2_PREFACE) {
                return HeadFeed::Http2;
            }
            if let Some(end) = request_head_end(&self.head) {
                let declared = declared_body(&self.head[..end]);
                // Body bytes may arrive in the same read as the head; the
                // declared length must not outlive them.
                let arrived = (self.head.len() - end + (bytes.len() - stored)) as u64;
                return HeadFeed::Complete { declared, arrived };
            }
            if stored < bytes.len() {
                // The head continues past the storage cap. Seed the tail from
                // the stored bytes so a terminator spanning the boundary is
                // still seen, then keep scanning without keeping the bytes.
                self.head_truncated = true;
                self.head_tail.clear();
                let tail = &self.head[self.head.len().saturating_sub(3)..];
                self.head_tail.extend_from_slice(tail);
                return self.feed_truncated(&bytes[stored..]);
            }
            return HeadFeed::Pending;
        }
        self.feed_truncated(bytes)
    }

    fn feed_truncated(&mut self, bytes: &[u8]) -> HeadFeed {
        let mut window = Vec::with_capacity(self.head_tail.len() + bytes.len());
        window.extend_from_slice(&self.head_tail);
        window.extend_from_slice(bytes);
        if let Some(end) = request_head_end(&window) {
            let arrived = (window.len() - end) as u64;
            return HeadFeed::Complete {
                declared: DeclaredBody::Unaccounted,
                arrived,
            };
        }
        self.head_tail.clear();
        self.head_tail.extend_from_slice(&window[window.len().saturating_sub(3)..]);
        HeadFeed::Pending
    }

    /// A request head completed. HTTP/1.1 serializes responses, so a new head
    /// means the previous response is complete: its idle bound ends here.
    fn head_complete(&mut self, declared: DeclaredBody, arrived: u64) {
        self.responded = false;
        let response_done = self.response_done;
        self.response_done = false;
        if response_done {
            self.disarm_response();
        }
        match declared {
            DeclaredBody::Length(remaining) if remaining > arrived => {
                self.request = RequestState::Body {
                    remaining: remaining - arrived,
                };
                self.arm_body();
            }
            DeclaredBody::Length(_) | DeclaredBody::None => {
                self.request = RequestState::Framed;
                self.disarm_request();
            }
            DeclaredBody::Unaccounted => {
                self.request = RequestState::Unaccounted;
                self.disarm_request();
            }
        }
    }

    fn poll_available(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if self.cancelled || self.stopped.as_mut().poll(cx).is_ready() {
            self.cancelled = true;
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Sharing stopped",
            ));
        }
        if let Some(deadline) = self.request_deadline.as_mut() {
            if deadline.as_mut().poll(cx).is_ready() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    match self.request {
                        RequestState::Head => "Sharing request head timed out",
                        RequestState::Body { .. } => "Sharing request body timed out",
                        RequestState::Framed | RequestState::Unaccounted => {
                            "Sharing request timed out"
                        }
                    },
                ));
            }
        }
        if let Some(idle) = self.response_idle.as_mut() {
            if idle.as_mut().poll(cx).is_ready() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Sharing connection is idle",
                ));
            }
        }
        Ok(())
    }
}
impl AsyncRead for SharingIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(error) = self.poll_available(cx) {
            return Poll::Ready(Err(error));
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut self.socket).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.received(&buf.filled()[before..]);
        }
        result
    }
}
impl AsyncWrite for SharingIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.poll_available(cx) {
            return Poll::Ready(Err(error));
        }
        // The first response byte starts the long idle bound; nothing before
        // it may charge the handler's own time against the client.
        if !self.responded {
            self.responded = true;
            self.response_done = false;
            self.arm_response();
        }
        let result = Pin::new(&mut self.socket).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(written)) if written > 0) {
            self.reset_response();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.poll_available(cx) {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.socket).poll_flush(cx);
        if matches!(result, Poll::Ready(Ok(()))) && !self.response_done {
            self.response_done = true;
            if !self.next_request.is_empty() {
                let pending = std::mem::take(&mut self.next_request);
                self.received(&pending);
            }
        }
        result
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

pub struct SharingServer {
    url: String,
    urls: Vec<String>,
    port: u16,
    binding: SharingBinding,
    bindings: Vec<String>,
    books: Vec<Value>,
    token: String,
    book_ids: Vec<String>,
    scope: SharingScope,
    session_id: String,
    locked: bool,
    catalog_url: Option<String>,
    catalog_urls: Vec<String>,
    library_count: usize,
    cancellation: SharingCancellation,
    servers: Vec<JoinHandle<Result<(), String>>>,
    // Connection tasks retain only weak references. Retirement drops these
    // owners after all registered native work, not merely Axum's close signal.
    archives: Vec<Arc<Archive>>,
    // Library mode allocates one listener per book lazily, when the index or a
    // reader first asks for it. Selected mode keeps every listener for its life.
    library: Option<LibraryHosts>,
    // The same host set every listener of this session checks `Host` against.
    // Status reads it so the links shown are the ones the server answers on.
    hosts: SessionHosts,
    // Last observation of whether an interface binding's address is still on
    // this device, so a status poll does not enumerate interfaces every time.
    address_lost: Mutex<(Option<Instant>, bool)>,
}
impl Drop for SharingServer {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(library) = &self.library {
            library.abort_all();
        }
        for server in self.servers.drain(..) {
            server.abort();
        }
    }
}
/// A selected-mode book keeps the archive opened at start (weakly owned by the
/// connection task). A library-mode book resolves and re-verifies its archive
/// against the live library record for every request.
#[derive(Clone)]
enum BookArchive {
    Selected { archive: Weak<Archive>, owner: Arc<Book>, store: Arc<Store>, stamp: Option<(u64, Option<SystemTime>)> },
    Library(Arc<LibraryBook>),
}
impl BookArchive {
    fn validate_selected(&self) -> Result<(), ResolveError> {
        if let Self::Selected { owner, store, stamp, .. } = self {
            let current = store.book(&owner.metadata.id).map_err(ResolveError::internal)?;
            if !same_owner(Some(owner), current.as_ref()) || archive_stamp(owner)? != *stamp {
                return Err(ResolveError::gone("This selected archive is no longer shared"));
            }
        }
        Ok(())
    }
    fn resolve(&self) -> Result<Arc<Archive>, ResolveError> {
        match self {
            Self::Selected { archive, .. } => {
                self.validate_selected()?;
                archive.upgrade().ok_or_else(|| ResolveError::forbidden(
                    "This shared file is no longer available. Stop sharing and start again"))
            }
            Self::Library(book) => library_archive(book),
        }
    }
}
#[derive(Clone)]
struct SharedBook {
    archive: BookArchive,
    token: String,
    authorities: AcceptedAuthorities,
    port: u16,
    store: Arc<Store>,
    cancellation: SharingCancellation,
    /// Tracks active reading traffic on library-mode per-book listeners so LRU
    /// eviction retires idle listeners instead of actively read books.
    activity: Option<(Arc<std::sync::atomic::AtomicU64>, Arc<std::sync::atomic::AtomicU64>)>,
}
#[derive(Clone)]
struct IndexBook {
    id: String,
    title: String,
    port: u16,
    archive: Weak<Archive>,
    fulltext: bool,
}
#[derive(Clone)]
struct Index {
    token: String,
    authorities: AcceptedAuthorities,
    entries: Arc<Vec<IndexBook>>,
    store: Arc<Store>,
    cancellation: SharingCancellation,
    library: LibraryIndex,
}

/// A request-time identity failure. Selected-mode archives keep their previous
/// 403/404 behaviour; library-mode archives refuse missing or replaced files
/// with 410 Gone and a message the reader can understand.
#[derive(Debug)]
struct ResolveError {
    status: StatusCode,
    message: String,
}
impl ResolveError {
    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }
    fn gone(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GONE,
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

struct LibraryBook {
    id: String,
    installation_id: String,
    path: String,
    store: Arc<Store>,
    archives: Arc<LibraryArchives>,
}

/// Bounded, re-validated handles for library archives. Every request first
/// checks the live library record and the file stamp, so a removed or replaced
/// archive is refused even while a handle from an earlier request is cached.
struct LibraryArchives {
    cached: Mutex<VecDeque<CachedLibraryArchive>>,
    capacity: usize,
    #[cfg(test)]
    opens: std::sync::atomic::AtomicUsize,
}
impl Default for LibraryArchives {
    fn default() -> Self {
        Self {
            cached: Mutex::new(VecDeque::new()),
            capacity: LIBRARY_ARCHIVE_CACHE,
            #[cfg(test)]
            opens: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}
impl LibraryArchives {
    #[cfg(test)]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }
    fn record_open(&self) {
        #[cfg(test)]
        self.opens
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(test)]
    fn opens(&self) -> usize {
        self.opens.load(std::sync::atomic::Ordering::SeqCst)
    }
}
struct CachedLibraryArchive {
    id: String,
    installation_id: String,
    path: String,
    size: u64,
    modified: Option<SystemTime>,
    archive: Arc<Archive>,
}

/// Short-lived file-presence results for the library index, feeds and reader.
///
/// Every sharing request used to stat each archive; this keeps that syscall per
/// book to once per `LIBRARY_AVAILABILITY_TTL` instead of once per request.
/// Entries are keyed by path and expected size, and record the observed length
/// and mtime they were computed from, so a changed file simply becomes a new
/// check. The cache lives with the sharing server, never process-wide.
#[derive(Default)]
struct AvailabilityCache {
    entries: Mutex<HashMap<(String, u64), AvailabilityEntry>>,
    swept: Mutex<Option<Instant>>,
}

struct AvailabilityEntry {
    checked: Instant,
    present: bool,
    len: u64,
    modified: Option<SystemTime>,
}

impl AvailabilityCache {
    fn present(&self, book: &Book) -> bool {
        if book.source_reference.is_some() {
            return cfg!(mobile);
        }
        let key = (book.path.clone(), book.metadata.size);
        let now = Instant::now();
        {
            let entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(entry) = entries.get(&key) {
                if now.saturating_duration_since(entry.checked) < LIBRARY_AVAILABILITY_TTL {
                    return entry.present;
                }
            }
        }
        let observed = std::fs::metadata(&book.path)
            .ok()
            .map(|metadata| (metadata.len(), metadata.modified().ok()));
        let (len, modified) = observed.unwrap_or((0, None));
        let mut entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&key) {
            entry.checked = now;
            // The same length and mtime is the same file; keep its verdict.
            if entry.len != len || entry.modified != modified {
                entry.present = available_len(book, len);
                entry.len = len;
                entry.modified = modified;
            }
            return entry.present;
        }
        let present = available_len(book, len);
        if entries.len() >= LIBRARY_AVAILABILITY_MAX {
            let sweep = {
                let mut swept = self.swept.lock().unwrap_or_else(|error| error.into_inner());
                if swept.is_none_or(|at| now.saturating_duration_since(at) >= LIBRARY_AVAILABILITY_TTL) {
                    *swept = Some(now);
                    true
                } else {
                    false
                }
            };
            if sweep {
                entries.retain(|_, entry| {
                    now.saturating_duration_since(entry.checked) < LIBRARY_AVAILABILITY_TTL
                });
            }
            // Still full of fresh entries: skip caching rather than clearing
            // the whole map and re-statting the library on every request.
            if entries.len() >= LIBRARY_AVAILABILITY_MAX {
                return present;
            }
        }
        entries.insert(
            key,
            AvailabilityEntry {
                checked: now,
                present,
                len,
                modified,
            },
        );
        present
    }
}

/// `book_present`'s size rules for an already observed length.
fn available_len(book: &Book, len: u64) -> bool {
    len != 0
        && (book.path.to_ascii_lowercase().ends_with(".zimaa")
            || book.metadata.size == 0
            || len == book.metadata.size)
}

fn archive_stamp(book: &Book) -> Result<Option<(u64, Option<SystemTime>)>, ResolveError> {
    if book.source_reference.is_some() {
        return Ok(None);
    }
    let metadata = std::fs::metadata(&book.path).map_err(|_| {
        ResolveError::gone("This archive file is missing; restore it or import it again")
    })?;
    let split = book.path.to_ascii_lowercase().ends_with(".zimaa");
    if !split && book.metadata.size != 0 && metadata.len() != book.metadata.size {
        return Err(ResolveError::gone(
            "This archive file was replaced; import the new archive separately",
        ));
    }
    Ok(Some((metadata.len(), metadata.modified().ok())))
}

fn library_archive(book: &LibraryBook) -> Result<Arc<Archive>, ResolveError> {
    let current = book
        .store
        .book(&book.id)
        .map_err(ResolveError::internal)?
        .ok_or_else(|| {
            ResolveError::gone("This archive is no longer in the library")
        })?;
    library_archive_checked(book, &current, LibraryArchiveAccess::Direct)
}

enum LibraryArchiveAccess {
    Direct,
    Search,
}

/// Resolve a cached or fresh handle from a live library record the caller has
/// already read, so listing the library does not scan the store again per
/// archive.
fn library_archive_checked(book: &LibraryBook, current: &Book, access: LibraryArchiveAccess) -> Result<Arc<Archive>, ResolveError> {
    if !current.available {
        return Err(ResolveError::gone(
            "This archive is no longer available in the library",
        ));
    }
    if current.installation_id != book.installation_id || current.path != book.path {
        return Err(ResolveError::gone(
            "This archive was relinked or replaced; reopen it from the library index",
        ));
    }
    let stamp = archive_stamp(current)?;
    {
        let mut cached = book
            .archives
            .cached
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(index) = cached.iter().position(|item| item.id == book.id) {
            let item = cached.remove(index).expect("cached library archive");
            if item.installation_id == book.installation_id
                && item.path == book.path
                && stamp.as_ref().is_none_or(|(size, modified)| {
                    item.size == *size && item.modified == *modified
                })
            {
                let archive = item.archive.clone();
                cached.push_back(item);
                return Ok(archive);
            }
            // A stale handle must not indefinitely occupy a warmed search slot.
        }
    }
    book.archives.record_open();
    let archive = if current.source_reference.is_some() {
        crate::archive_source::retain(current).and_then(|source| source.open())
    } else {
        Archive::open(std::path::Path::new(&current.path))
    }
    .map_err(|error| {
        ResolveError::gone(format!(
            "This archive can no longer be opened ({error}); restore it or import it again"
        ))
    })?;
    if archive.metadata().map_err(ResolveError::internal)?.id != current.metadata.id {
        return Err(ResolveError::gone(
            "Archive file identity changed; import it separately or locate the original archive",
        ));
    }
    let (size, modified) = stamp.unwrap_or((current.metadata.size, None));
    let archive = Arc::new(archive);
    let mut cached = book
        .archives
        .cached
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Two concurrent searches can open the same miss before either inserts.
    // Keep one cache slot per archive without accepting another request's stamp.
    cached.retain(|item| item.id != book.id);
    if cached.len() >= book.archives.capacity {
        if matches!(access, LibraryArchiveAccess::Search) {
            // An all-library query may walk every archive in a fixed order.
            // Its misses must not evict the handles the next query can reuse.
            return Ok(archive);
        }
        cached.pop_front();
    }
    cached.push_back(CachedLibraryArchive {
        id: book.id.clone(),
        installation_id: book.installation_id.clone(),
        path: book.path.clone(),
        size,
        modified,
        archive: archive.clone(),
    });
    Ok(archive)
}

/// Resolve one library archive from a book record the caller already holds.
fn library_archive_record(
    state: &LibraryIndex,
    book: &Book,
) -> Result<Arc<Archive>, ResolveError> {
    library_archive_checked(
        &LibraryBook {
            id: book.metadata.id.clone(),
            installation_id: book.installation_id.clone(),
            path: book.path.clone(),
            store: state.store.clone(),
            archives: state.archives.clone(),
        },
        book,
        LibraryArchiveAccess::Search,
    )
}

type RetiredLibraryHosts = Arc<Mutex<Vec<JoinHandle<Result<(), String>>>>>;

#[derive(Clone)]
struct LibraryHosts {
    token: String,
    bind: IpAddr,
    binding: SharingBinding,
    hosts: SessionHosts,
    store: Arc<Store>,
    cancellation: SharingCancellation,
    archives: Arc<LibraryArchives>,
    entries: Arc<Mutex<HashMap<String, LibraryHost>>>,
    seq: Arc<std::sync::atomic::AtomicU64>,
    retired: RetiredLibraryHosts,
}
struct LibraryHost {
    port: u16,
    installation_id: String,
    path: String,
    used_seq: Arc<std::sync::atomic::AtomicU64>,
    handle: JoinHandle<Result<(), String>>,
}
impl LibraryHosts {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, LibraryHost>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }
    fn push_retired(&self, handle: JoinHandle<Result<(), String>>) {
        let mut retired = self
            .retired
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        retired.retain(|existing| !existing.is_finished());
        retired.push(handle);
    }
    /// Retire hosts whose book disappeared, then bind a listener only for the
    /// books a reader actually asked to open. Index and feed requests stay
    /// allocation-free even for a large library, and idle listeners are evicted
    /// LRU above `MAX_LIBRARY_HOSTS`.
    fn ensure(
        &self,
        available: &[Book],
        requested: &[Book],
    ) -> Result<HashMap<String, u16>, String> {
        let mut entries = self.lock();
        if self.cancellation.is_cancelled() {
            return Err("Sharing has stopped".into());
        }
        let current: HashSet<_> = available
            .iter()
            .map(|book| book.metadata.id.as_str())
            .collect();
        let requested_ids: HashSet<_> = requested
            .iter()
            .map(|book| book.metadata.id.as_str())
            .collect();
        let stale: Vec<_> = entries
            .keys()
            .filter(|id| !current.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale {
            if let Some(host) = entries.remove(&id) {
                host.handle.abort();
                self.push_retired(host.handle);
            }
        }
        let mut ports = HashMap::new();
        for book in requested {
            let id = book.metadata.id.clone();
            if let Some(host) = entries.get_mut(&id) {
                if host.installation_id == book.installation_id && host.path == book.path {
                    host.used_seq.store(self.next_seq(), std::sync::atomic::Ordering::Relaxed);
                    ports.insert(id, host.port);
                    continue;
                }
                if let Some(host) = entries.remove(&id) {
                    host.handle.abort();
                    self.push_retired(host.handle);
                }
            }
            while entries.len() >= MAX_LIBRARY_HOSTS {
                let evict = entries
                    .iter()
                    .filter(|(candidate, _)| !requested_ids.contains(candidate.as_str()))
                    .min_by_key(|(_, host)| host.used_seq.load(std::sync::atomic::Ordering::Relaxed))
                    .map(|(candidate, _)| candidate.clone());
                let Some(evict) = evict else { break };
                if let Some(host) = entries.remove(&evict) {
                    host.handle.abort();
                    self.push_retired(host.handle);
                }
            }
            let listener = bind_listener(self.bind, 0, &self.binding)?;
            let port = listener.local_addr().map_err(|error| error.to_string())?.port();
            let used_seq = Arc::new(std::sync::atomic::AtomicU64::new(self.next_seq()));
            let state = SharedBook {
                archive: BookArchive::Library(Arc::new(LibraryBook {
                    id: id.clone(),
                    installation_id: book.installation_id.clone(),
                    path: book.path.clone(),
                    store: self.store.clone(),
                    archives: self.archives.clone(),
                })),
                token: self.token.clone(),
                authorities: self.hosts.for_port(port),
                port,
                store: self.store.clone(),
                cancellation: self.cancellation.clone(),
                activity: Some((self.seq.clone(), used_seq.clone())),
            };
            let router = Router::new().fallback(any(shared_request)).with_state(state);
            let listener = SharingListener {
                listener,
                cancellation: self.cancellation.clone(),
            };
            let stopped = self.cancellation.clone();
            let handle = tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(stopped.cancelled())
                    .await
                    .map_err(|error| format!("Library book listener failed: {error}"))
            });
            entries.insert(
                id.clone(),
                LibraryHost {
                    port,
                    installation_id: book.installation_id.clone(),
                    path: book.path.clone(),
                    used_seq,
                    handle,
                },
            );
            ports.insert(id, port);
        }
        Ok(ports)
    }
    fn abort_all(&self) {
        for (_, host) in self.lock().drain() {
            host.handle.abort();
        }
        for handle in self.retired.lock().unwrap_or_else(|error| error.into_inner()).drain(..) {
            handle.abort();
        }
    }
    async fn shutdown(&self) -> Result<(), String> {
        let mut handles: Vec<_> = self
            .lock()
            .drain()
            .map(|(_, host)| host.handle)
            .collect();
        handles.extend(self.retired.lock().unwrap_or_else(|error| error.into_inner()).drain(..));
        let mut errors = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(())) => {}
                Err(error) if error.is_cancelled() => {}
                Ok(Err(error)) => errors.push(error),
                Err(error) => errors.push(format!("Joining library book listener: {error}")),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

#[derive(Clone)]
struct LibraryIndex {
    token: String,
    authorities: AcceptedAuthorities,
    port: u16,
    store: Arc<Store>,
    hosts: LibraryHosts,
    archives: Arc<LibraryArchives>,
    availability: Arc<AvailabilityCache>,
    cancellation: SharingCancellation,
    session: Arc<SharingSession>,
}

struct SharingSession {
    id: String,
    pin_hash: Option<[u8; 32]>,
    attempts: Mutex<PinAttempts>,
    selected: Option<HashMap<String, Book>>,
}

/// One protected session's failed-PIN budget. A spent budget locks the form for
/// an escalating window instead of resetting on a fixed timer, and a successful
/// unlock restores the initial budget.
struct PinAttempts { next: Instant, remaining: usize, lockout: Duration }
impl PinAttempts {
    fn new() -> Self { Self { next: Instant::now(), remaining: PIN_ATTEMPTS, lockout: PIN_WINDOW } }
    fn attempt(&mut self) -> bool {
        let now = Instant::now();
        if now < self.next { return false; }
        if self.remaining == 0 {
            // The previous budget is spent: escalate before refilling it.
            self.lockout = (self.lockout * 2).min(PIN_LOCKOUT_MAX);
            self.remaining = PIN_ATTEMPTS;
        }
        self.remaining -= 1;
        if self.remaining == 0 { self.next = now + self.lockout; }
        true
    }
    fn succeeded(&mut self) { *self = Self::new(); }
    fn retry_after(&self) -> u64 {
        let millis = self.next.saturating_duration_since(Instant::now()).as_millis();
        u64::try_from(millis.div_ceil(1000)).unwrap_or(u64::MAX).max(1)
    }
}

impl SharingSession {
    fn new(pin: Option<&str>, selected: Option<HashMap<String, Book>>) -> Result<Self, String> {
        let pin = pin.filter(|pin| !pin.is_empty());
        if pin.is_some_and(|pin| !(PIN_MIN_DIGITS..=PIN_MAX_DIGITS).contains(&pin.len()) || !pin.bytes().all(|byte| byte.is_ascii_digit())) {
            return Err(format!("The sharing PIN must contain {PIN_MIN_DIGITS}–{PIN_MAX_DIGITS} digits"));
        }
        let id = Uuid::new_v4().simple().to_string();
        let pin_hash = pin.map(|pin| Self::hash_pin(&id, pin));
        Ok(Self { id, pin_hash, attempts: Mutex::new(PinAttempts::new()), selected })
    }

    fn hash_pin(id: &str, pin: &str) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(id.as_bytes());
        hash.update(pin.as_bytes());
        hash.finalize().into()
    }

    fn locked(&self) -> bool { self.pin_hash.is_some() }

    fn name(&self) -> &'static str {
        if self.locked() { "Protected library" } else { "Shared library" }
    }

    fn attempt(&self) -> bool {
        let mut attempts = self.attempts.lock().unwrap_or_else(|error| error.into_inner());
        attempts.attempt()
    }

    fn succeeded(&self) {
        let mut attempts = self.attempts.lock().unwrap_or_else(|error| error.into_inner());
        attempts.succeeded();
    }

    fn retry_after(&self) -> u64 {
        self.attempts.lock().unwrap_or_else(|error| error.into_inner()).retry_after()
    }

    fn authenticates(&self, pin: Option<&str>) -> bool {
        match self.pin_hash.as_ref() {
            None => true,
            Some(expected) => {
                let actual = Self::hash_pin(&self.id, pin.unwrap_or(""));
                bool::from(expected.ct_eq(&actual))
            }
        }
    }

    fn permits(&self, book: &Book) -> bool {
        self.selected.as_ref().is_none_or(|selected| {
            same_owner(selected.get(&book.metadata.id), Some(book))
        })
    }
}
impl SharingServer {
    /// Retire every listener and worker in place. The caller keeps the server
    /// visible until this returns, so a concurrent library removal cannot
    /// unlink an archive that is still being streamed.
    pub(crate) async fn shutdown(&mut self) -> Result<(), String> {
        // Close admission and fire graceful-shutdown signals synchronously so
        // no new request can enter while waiting for the blocking pool, then
        // drain in-flight native archive searches off the Tokio async thread.
        self.cancellation.begin_cancel();
        let cancellation = self.cancellation.clone();
        if tokio::task::spawn_blocking(move || cancellation.drain_native())
            .await
            .is_err()
        {
            self.cancellation.drain_native();
        }
        let mut errors = Vec::new();
        if let Some(library) = &self.library {
            if let Err(error) = library.shutdown().await {
                errors.push(error);
            }
        }
        for server in self.servers.drain(..) {
            match server.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(error),
                Err(error) if error.is_cancelled() => {}
                Err(error) => errors.push(format!("Joining sharing listener: {error}")),
            }
        }
        if let Err(error) = self.cancellation.0.workers.drain().await {
            errors.push(error);
        }
        self.archives.clear();
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }

    /// The listener lives on the access-point address, so the reservation and
    /// the server must be released together and neither may be retired while
    /// the other is still in use.
    pub fn uses_hotspot(&self) -> bool {
        matches!(self.binding, SharingBinding::Hotspot {})
    }

    /// Whether this selected-scope share currently serves the archive.
    ///
    /// Library scope re-checks the live library record on every request, but a
    /// selected share owns a strong archive handle for its whole lifetime.
    /// Removal and relinking therefore have to release it explicitly first.
    pub fn serves_selected(&self, id: &str) -> bool {
        self.scope == SharingScope::Selected && self.book_ids.iter().any(|shared| shared == id)
    }

    /// Drop a whole-library share's cached handles for one archive. Windows
    /// cannot move or delete an open file, so removal releases them first; a
    /// later request revalidates against the library and reopens if needed.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn evict_library_archive(&self, id: &str) {
        if let Some(library) = &self.library {
            library
                .archives
                .cached
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|item| item.id != id);
        }
    }

    /// A loopback share still serves this device after the local link is gone,
    /// so network retirement leaves it running.
    #[cfg_attr(not(mobile), allow(dead_code))]
    pub fn loopback_only(&self) -> bool {
        matches!(self.binding, SharingBinding::Loopback {})
    }

    /// Start an explicitly requested session. Command integration supplies
    /// Library/port 0/default_binding() when those optional arguments are absent.
    pub async fn start_session(
        service: Arc<Service>,
        scope: SharingScope,
        ids: Vec<String>,
        port: u16,
        binding: SharingBinding,
        pin: Option<String>,
    ) -> Result<Self, String> {
        let mut session = SharingSession::new(pin.as_deref(), None)?;
        if scope == SharingScope::Library {
            return Self::start_library(service, port, binding, Arc::new(session)).await;
        }
        let state = service.store.books()?;
        let selected: HashSet<_> = ids.into_iter().collect();
        let books: Vec<_> = state
            .into_iter()
            .filter(|b| {
                selected.contains(&b.metadata.id)
                    && shareable_library_book(b)
                    && b.available
            })
            .collect();
        if books.len() != selected.len() || books.is_empty() {
            return Err("One or more files you chose to share are missing. Choose the files again and start sharing".into());
        }
        session.selected = Some(books.iter().map(|book| (book.metadata.id.clone(), book.clone())).collect());
        let session = Arc::new(session);
        let (bind, hosts) = binding_addresses(Some(service.as_ref()), &binding)?;
        let index_listener = bind_listener(bind, port, &binding)?;
        let index_address = index_listener.local_addr().map_err(|e| e.to_string())?;
        let port = index_address.port();
        let token = Uuid::new_v4().simple().to_string();
        let mut entries = Vec::new();
        let mut book_ids = Vec::new();
        let mut book_status = Vec::new();
        let mut prepared = Vec::new();
        let mut archives = Vec::new();
        let cancellation = SharingCancellation::new();
        let session_hosts = SessionHosts::new(&hosts, &binding);
        // Prepare every archive and socket before spawning anything. Any error
        // drops all sockets, rather than leaving a partially started collection.
        for book in books {
            let owner = Arc::new(book.clone());
            let stamp = archive_stamp(&book).map_err(|error| error.message)?;
            let id = book.metadata.id.clone();
            let s = service.clone();
            let (archive, fulltext) = tokio::task::spawn_blocking(move || {
                let archive = s.archive(&id)?;
                let (fulltext, _) = archive.search_capabilities()?;
                Ok::<_, String>((archive, fulltext))
            })
                .await
                .map_err(|e| e.to_string())??;
            let archive = Arc::new(archive);
            let listener = bind_listener(bind, 0, &binding)?;
            let book_port = listener.local_addr().map_err(|e| e.to_string())?.port();
            book_status.push(json!({
                "id": book.metadata.id,
                "port": book_port,
                "urls": hosts.iter().map(|host| access_url(*host, book_port, &format!("/.kiwix-access/{token}"))).collect::<Vec<_>>(),
            }));
            entries.push(IndexBook {
                id: book.metadata.id.clone(),
                title: book.metadata.title,
                port: book_port,
                archive: Arc::downgrade(&archive),
                fulltext,
            });
            book_ids.push(book.metadata.id);
            prepared.push((
                listener,
                SharedBook {
                    archive: BookArchive::Selected { archive: Arc::downgrade(&archive), owner, store: service.store.clone(), stamp },
                    token: token.clone(),
                    authorities: session_hosts.for_port(book_port),
                    port: book_port,
                    store: service.store.clone(),
                    cancellation: cancellation.clone(),
                    activity: None,
                },
            ));
            archives.push(archive);
        }
        let mut servers = Vec::new();
        let mut bindings = vec![index_address.to_string()];
        for (listener, state) in prepared {
            bindings.push(SocketAddr::new(bind, state.port).to_string());
            let router = Router::new()
                .fallback(any(shared_request))
                .with_state(state);
            let listener = SharingListener {
                listener,
                cancellation: cancellation.clone(),
            };
            let stopped = cancellation.clone();
            servers.push(tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(stopped.cancelled())
                    .await.map_err(|error| format!("Sharing listener failed: {error}"))
            }));
        }
        let library = LibraryHosts {
            token: token.clone(),
            bind,
            binding: binding.clone(),
            hosts: session_hosts.clone(),
            store: service.store.clone(),
            cancellation: cancellation.clone(),
            archives: Arc::new(LibraryArchives::default()),
            entries: Arc::new(Mutex::new(HashMap::new())),
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            retired: Arc::new(Mutex::new(Vec::new())),
        };
        let index_authorities = session_hosts.for_port(port);
        let library_index = LibraryIndex {
            token: token.clone(),
            authorities: index_authorities.clone(),
            port,
            store: service.store.clone(),
            hosts: library.clone(),
            archives: library.archives.clone(),
            availability: Arc::new(AvailabilityCache::default()),
            cancellation: cancellation.clone(),
            session: session.clone(),
        };
        let index = Index {
            token: token.clone(),
            authorities: index_authorities,
            entries: Arc::new(entries),
            store: service.store.clone(),
            cancellation: cancellation.clone(),
            library: library_index,
        };
        let router = Router::new().fallback(any(index_request)).with_state(index);
        let listener = SharingListener {
            listener: index_listener,
            cancellation: cancellation.clone(),
        };
        let stopped = cancellation.clone();
        servers.push(tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled())
                .await.map_err(|error| format!("Sharing listener failed: {error}"))
        }));
        let urls: Vec<_> = hosts
            .iter()
            .map(|host| access_url(*host, port, &format!("/nearby/{}", session.id)))
            .collect();
        Ok(Self {
            url: urls[0].clone(),
            urls,
            port,
            binding,
            bindings,
            books: book_status,
            token,
            book_ids,
            scope: SharingScope::Selected,
            session_id: session.id.clone(),
            locked: session.locked(),
            catalog_url: None,
            catalog_urls: Vec::new(),
            library_count: selected.len(),
            cancellation,
            servers,
            archives,
            library: Some(library),
            hosts: session_hosts,
            address_lost: Mutex::new((None, false)),
        })
    }

    /// Serve every currently available archive. Book listeners are allocated
    /// lazily from the index/catalog path, so importing many books does not
    /// open or bind anything until a reader asks for it.
    async fn start_library(
        service: Arc<Service>,
        port: u16,
        binding: SharingBinding,
        session: Arc<SharingSession>,
    ) -> Result<Self, String> {
        let available = service
            .store
            .books()?
            .into_iter()
            .filter(|book| shareable_library_book(book) && book.available && book_present(book))
            .collect::<Vec<_>>();
        if available.is_empty() {
            return Err("Import an available archive before serving the library".into());
        }
        let (bind, hosts) = binding_addresses(Some(service.as_ref()), &binding)?;
        let index_listener = bind_listener(bind, port, &binding)?;
        let index_address = index_listener.local_addr().map_err(|e| e.to_string())?;
        let port = index_address.port();
        let token = Uuid::new_v4().simple().to_string();
        let cancellation = SharingCancellation::new();
        let session_hosts = SessionHosts::new(&hosts, &binding);
        let library = LibraryHosts {
            token: token.clone(),
            bind,
            binding: binding.clone(),
            hosts: session_hosts.clone(),
            store: service.store.clone(),
            cancellation: cancellation.clone(),
            archives: Arc::new(LibraryArchives::default()),
            entries: Arc::new(Mutex::new(HashMap::new())),
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            retired: Arc::new(Mutex::new(Vec::new())),
        };
        let index = LibraryIndex {
            token: token.clone(),
            authorities: session_hosts.for_port(port),
            port,
            store: service.store.clone(),
            hosts: library.clone(),
            archives: library.archives.clone(),
            availability: Arc::new(AvailabilityCache::default()),
            cancellation: cancellation.clone(),
            session: session.clone(),
        };
        let router = Router::new().fallback(any(library_request)).with_state(index);
        let listener = SharingListener {
            listener: index_listener,
            cancellation: cancellation.clone(),
        };
        let stopped = cancellation.clone();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled())
                .await
                .map_err(|error| format!("Library sharing listener failed: {error}"))
        });
        let urls: Vec<_> = hosts
            .iter()
            .map(|host| access_url(*host, port, &format!("/nearby/{}", session.id)))
            .collect();
        let catalog_urls: Vec<_> = hosts
            .iter()
            .map(|host| {
                access_url(
                    *host,
                    port,
                    &format!("/{token}/catalog/v2/root.xml"),
                )
            })
            .collect();
        Ok(Self {
            url: urls[0].clone(),
            urls,
            port,
            binding,
            bindings: vec![index_address.to_string()],
            books: Vec::new(),
            token,
            book_ids: Vec::new(),
            scope: SharingScope::Library,
            session_id: session.id.clone(),
            locked: session.locked(),
            catalog_url: catalog_urls.first().cloned(),
            catalog_urls,
            library_count: available.len(),
            cancellation,
            servers: vec![handle],
            archives: Vec::new(),
            library: Some(library),
            hosts: session_hosts,
            address_lost: Mutex::new((None, false)),
        })
    }

    pub fn status(&self) -> Value {
        let mut status = json!({
            "running": true,
            "scope": self.scope,
            "sessionId": self.session_id,
            "locked": self.locked,
            "url": self.url,
            "urls": self.urls,
            "port": self.port,
            "binding": self.binding,
            "bindings": self.bindings,
            "books": self.books,
            "token": self.token,
            "bookIds": self.book_ids,
            "catalogUrl": self.catalog_url,
            "catalogUrls": self.catalog_urls,
            "libraryCount": self.library_count,
        });
        // The listener outlives the network it started on: a laptop changes
        // Wi-Fi or its lease while this still says "running". Report what is
        // true now. This describes the server only; which `Host` values it
        // answers is decided by `SessionHosts`, and is not changed here.
        match &self.binding {
            SharingBinding::Interface { address } => {
                status["addressLost"] = json!(self.interface_address_lost(address));
            }
            SharingBinding::Ipv4All {} | SharingBinding::Ipv6All {} | SharingBinding::DualStackAll {} => {
                // A wildcard listener already answers on the refreshed set.
                let hosts = self.hosts.current();
                if !hosts.is_empty() {
                    let links = |port: u16, path: &str| -> Vec<String> {
                        hosts.iter().map(|host| access_url(*host, port, path)).collect()
                    };
                    let urls = links(self.port, &format!("/nearby/{}", self.session_id));
                    status["url"] = json!(urls[0]);
                    status["urls"] = json!(urls);
                    if self.catalog_url.is_some() {
                        let catalogs = links(self.port, &format!("/{}/catalog/v2/root.xml", self.token));
                        status["catalogUrl"] = json!(catalogs[0]);
                        status["catalogUrls"] = json!(catalogs);
                    }
                    if let Some(books) = status["books"].as_array_mut() {
                        for book in books {
                            let port = book["port"].as_u64().and_then(|port| u16::try_from(port).ok());
                            if let Some(port) = port {
                                book["urls"] = json!(links(port, &format!("/.kiwix-access/{}", self.token)));
                            }
                        }
                    }
                }
            }
            SharingBinding::Loopback {} | SharingBinding::Hotspot {} => {}
        }
        status
    }

    /// An interface binding names one literal address. When that address is
    /// no longer on any local interface the listener cannot be reached, and
    /// only stopping and starting again can bind the new one.
    fn interface_address_lost(&self, address: &str) -> bool {
        let mut seen = self.address_lost.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        if !seen.0.is_some_and(|at| now.saturating_duration_since(at) < AUTHORITY_REFRESH_TTL) {
            seen.0 = Some(now);
            // An enumeration failure proves nothing; keep the last answer.
            if let Ok(current) = interfaces() {
                seen.1 = address_absent(address, &current);
            }
        }
        seen.1
    }
}
fn address_absent(address: &str, current: &[SharingInterface]) -> bool {
    let bound = address.parse::<IpAddr>().ok();
    !current.iter().any(|interface| interface.address.parse::<IpAddr>().ok() == bound)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn excerpt(s: &str, limit: usize) -> String {
    escape(&s.chars().take(limit).collect::<String>())
}
fn archive_url(path: &str) -> String {
    // The shared encoder preserves leading archive slashes as encoded bytes,
    // so the one literal slash here can never introduce a foreign authority.
    format!("/{}", encode_path(path))
}
fn deny() -> Response {
    (StatusCode::FORBIDDEN, "Sharing capability required").into_response()
}
fn authorities(hosts: &[IpAddr], port: u16) -> Arc<Vec<(IpAddr, String)>> {
    Arc::new(
        hosts
            .iter()
            .map(|host| (*host, SocketAddr::new(*host, port).to_string()))
            .collect(),
    )
}

fn accepted_host(
    headers: &HeaderMap,
    uri: &Uri,
    authorities: &[(IpAddr, String)],
) -> Option<IpAddr> {
    let mut values = headers.get_all("host").iter();
    let authority = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let host = authorities
        .iter()
        .find_map(|(host, accepted)| (accepted == authority).then_some(*host))?;
    // Absolute-form requests may not smuggle another authority past Host.
    if uri
        .authority()
        .is_some_and(|value| value.as_str() != authority)
        || uri.scheme_str().is_some_and(|scheme| scheme != "http")
    {
        return None;
    }
    let mut origins = headers.get_all("origin").iter();
    if let Some(origin) = origins.next() {
        if origin.to_str().ok()?.strip_prefix("http://") != Some(authority)
            || origins.next().is_some()
        {
            return None;
        }
    }
    Some(host)
}
const AUTHORITY_REFRESH_TTL: Duration = Duration::from_secs(2);
type HostList = Arc<Vec<IpAddr>>;
type LiveHosts = Arc<std::sync::RwLock<(Option<Instant>, HostList)>>;

/// Shared allowed-IP set for one sharing session. Explicit, loopback, and
/// hotspot bindings stay pinned to their startup addresses; wildcard bindings
/// share one TTL-refreshed live IP list across the index listener and every
/// per-book listener so post-DHCP `/open/{id}` redirects succeed immediately
/// on newly bound book ports without per-listener `get_if_addrs()` storms.
#[derive(Clone)]
struct SessionHosts {
    binding: SharingBinding,
    static_hosts: HostList,
    live: Option<LiveHosts>,
}

impl SessionHosts {
    fn new(hosts: &[IpAddr], binding: &SharingBinding) -> Self {
        let host_list = Arc::new(hosts.to_vec());
        let wildcard = matches!(
            binding,
            SharingBinding::Ipv4All {}
                | SharingBinding::Ipv6All {}
                | SharingBinding::DualStackAll {}
        );
        Self {
            binding: binding.clone(),
            static_hosts: host_list.clone(),
            live: wildcard
                .then(|| Arc::new(std::sync::RwLock::new((Some(Instant::now()), host_list)))),
        }
    }

    #[cfg(test)]
    fn new_expired(hosts: &[IpAddr], binding: &SharingBinding) -> Self {
        let this = Self::new(hosts, binding);
        if let Some(live) = &this.live {
            if let Ok(mut guard) = live.write() {
                guard.0 = None;
            }
        }
        this
    }

    fn current(&self) -> HostList {
        let Some(live) = &self.live else {
            return self.static_hosts.clone();
        };
        let now = Instant::now();
        {
            let guard = live.read().unwrap_or_else(|error| error.into_inner());
            if guard
                .0
                .is_some_and(|at| now.saturating_duration_since(at) < AUTHORITY_REFRESH_TTL)
            {
                return guard.1.clone();
            }
        }
        let mut guard = live.write().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        if !guard
            .0
            .is_some_and(|at| now.saturating_duration_since(at) < AUTHORITY_REFRESH_TTL)
        {
            guard.0 = Some(now);
            if let Ok((_, hosts)) = binding_addresses(None, &self.binding) {
                guard.1 = Arc::new(hosts);
            }
        }
        guard.1.clone()
    }

    fn for_port(&self, port: u16) -> AcceptedAuthorities {
        AcceptedAuthorities {
            hosts: self.clone(),
            port,
        }
    }
}

#[derive(Clone)]
struct AcceptedAuthorities {
    hosts: SessionHosts,
    port: u16,
}

impl AcceptedAuthorities {
    #[cfg(test)]
    fn new(hosts: &[IpAddr], binding: &SharingBinding, port: u16) -> Self {
        SessionHosts::new(hosts, binding).for_port(port)
    }

    fn check(&self, headers: &HeaderMap, uri: &Uri) -> Option<IpAddr> {
        let hosts = self.hosts.current();
        let accepted = authorities(&hosts, self.port);
        accepted_host(headers, uri, &accepted)
    }
}
fn service_worker_request(headers: &HeaderMap) -> bool {
    headers.contains_key("service-worker")
        || headers
            .get_all("sec-fetch-dest")
            .iter()
            .any(|value| value.as_bytes().eq_ignore_ascii_case(b"serviceworker"))
}
fn page(body: String) -> Response {
    Response::builder().header("Content-Type","text/html; charset=utf-8")
        .header("Content-Security-Policy","default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'; worker-src 'none'")
        .header("Permissions-Policy","camera=(), geolocation=(), microphone=(), display-capture=()")
        .header("Referrer-Policy","no-referrer").header("Cache-Control","no-store").header("X-Content-Type-Options","nosniff")
        .body(Body::from(body)).unwrap()
}
fn request_locale(store: &Store, headers: &HeaderMap) -> Result<Localizer, String> {
    // `locale()` reads the same setting without cloning bookmarks, history or
    // reader state on every rendered page.
    let setting = store.locale()?;
    let languages = headers
        .get_all("accept-language")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    Ok(Localizer::new(&setting, &languages))
}

fn activation_target(uri: &Uri) -> Result<String, &'static str> {
    let raw = uri.query().unwrap_or("");
    if raw.len() > 64 * 1024 {
        return Err("Article URL is too long");
    }
    let mut target = None;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if key != "path" || target.is_some() {
            return Err("Invalid article target");
        }
        // This is a local navigation, never a foreign authority or URL suffix.
        if !value.starts_with('/')
            || value.starts_with("//")
            || value.contains(['\\', '?', '#'])
            || value.chars().any(char::is_control)
            || !value.is_ascii()
        {
            return Err("Invalid article target");
        }
        target = Some(value.into_owned());
    }
    Ok(target.unwrap_or_else(|| "/".into()))
}

fn activation_page(locale: &Localizer, target: &str, port: u16, token: &str) -> Response {
    let t = |source: &str| escape(&locale.text(source, &[]));
    // An explicit link starts a new, same-site navigation from this committed
    // document. A redirect (or automatic refresh) can retain the original
    // cross-site chain and omit the Strict cookie on the protected request.
    let body = format!(
        "<!doctype html><html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>html{{color-scheme:light}}body{{margin:0;background:#f6f2e8;color:#183b35;font:19px/1.6 Georgia,serif}}main{{max-width:38rem;margin:10vh auto;padding:2rem;border-top:8px solid #183b35}}h1{{font-size:clamp(2rem,7vw,3.5rem);line-height:1.1}}a{{display:inline-block;margin-top:1rem;padding:.65rem 1.5rem;background:#183b35;color:#f6f2e8;text-underline-offset:.2em}}a:focus-visible{{outline:3px solid #8b421b;outline-offset:4px}}</style></head><body><main><h1>{}</h1><p>{}</p><a href=\"{}\" rel=\"noreferrer\">{}</a></main></body></html>",
        escape(locale.lang()),
        escape(locale.dir()),
        t("Open shared item"),
        t("Open shared item"),
        t("Continue to the shared item."),
        escape(target),
        t("Continue"),
    );
    let mut response = page(body);
    response.headers_mut().insert(
        "Set-Cookie",
        format!("kiwix_share_{port}={token}; Path=/; HttpOnly; SameSite=Strict")
            .parse()
            .expect("generated sharing capability cookie"),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapCredentials {
    pin: Option<String>,
}

impl Drop for BootstrapCredentials {
    fn drop(&mut self) {
        if let Some(pin) = self.pin.as_mut() { zeroize::Zeroize::zeroize(pin); }
    }
}

fn json_response(value: Value) -> Response {
    let body = value.to_string();
    Response::builder()
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Content-Length", body.len())
        .header("Cache-Control", "no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(body)).unwrap()
}

fn scoped_books(state: &LibraryIndex) -> Result<Vec<Book>, String> {
    Ok(available_books(state)?.into_iter()
        .filter(|book| state.session.permits(book)).collect())
}

fn inventory(state: &LibraryIndex) -> Result<Vec<Value>, String> {
    Ok(scoped_books(state)?.into_iter().map(|book| json!({
        "id": book.metadata.id,
        "title": book.metadata.title,
        "fileName": download_name(&book.metadata.name, &book.metadata.id),
        "size": book.metadata.size,
        "archiveId": book.metadata.id,
    })).collect())
}

fn bootstrap_page(state: &LibraryIndex, headers: &HeaderMap, status: StatusCode, pin_error: Option<&str>) -> Response {
    let locale = match request_locale(&state.store, headers) {
        Ok(locale) => locale,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let t = |text: &str| escape(&locale.text(text, &[]));
    let action = if state.session.locked() {
        let (error, invalid) = match pin_error {
            Some(message) => (format!("<p id=\"pin-error\" role=\"alert\">{}</p>", t(message)), " aria-invalid=\"true\" aria-describedby=\"pin-error\""),
            None => (String::new(), ""),
        };
        format!("{error}<form method=\"post\" action=\"/nearby/{}\"><label for=\"pin\">{}</label><input id=\"pin\" name=\"pin\" type=\"password\" inputmode=\"numeric\" pattern=\"[0-9]{{{PIN_MIN_DIGITS},{PIN_MAX_DIGITS}}}\" minlength=\"{PIN_MIN_DIGITS}\" maxlength=\"{PIN_MAX_DIGITS}\" autocomplete=\"off\"{invalid} required><button type=\"submit\">{}</button></form>",
            state.session.id, t("Sharing PIN"), t("Open shared library"))
    } else {
        format!("<a href=\"/{}/\" rel=\"noreferrer\">{}</a>", state.token, t("Open shared library"))
    };
    let mut response = page(format!("<!doctype html><html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>body{{max-width:38rem;margin:10vh auto;padding:2rem;background:#f6f2e8;color:#183b35;font:19px/1.6 Georgia,serif}}h1{{border-top:8px solid;padding-top:1rem}}label{{display:block}}input,button,a{{font:inherit;padding:.5rem}}a{{color:inherit}}</style></head><body><h1>{}</h1><p>{}</p>{}</body></html>",
        escape(locale.lang()), escape(locale.dir()), t(state.session.name()), t(state.session.name()),
        t("Read-only sharing over an unencrypted local connection. Use only a trusted network."), action));
    *response.status_mut() = status;
    response
}

async fn bootstrap_request(state: &LibraryIndex, method: Method, uri: &Uri, headers: &HeaderMap, body: Body) -> Response {
    if uri.query().is_some() { return (StatusCode::BAD_REQUEST, "Unexpected bootstrap query").into_response(); }
    if method == Method::GET { return bootstrap_page(state, headers, StatusCode::OK, None); }
    if method != Method::POST { return deny(); }
    if headers.get_all("sec-fetch-site").iter().any(|value| value.as_bytes().eq_ignore_ascii_case(b"cross-site")) {
        return deny();
    }
    let content_type = headers.get("content-type").and_then(|value| value.to_str().ok())
        .unwrap_or("application/json").split(';').next().unwrap_or("").trim();
    let browser_form = content_type == "application/x-www-form-urlencoded";
    if state.session.locked() && !state.session.attempt() {
        let retry = state.session.retry_after().to_string();
        const LOCKED_OUT: &str = "Too many PIN attempts; try again shortly";
        if browser_form {
            let mut response = bootstrap_page(state, headers, StatusCode::TOO_MANY_REQUESTS, Some(LOCKED_OUT));
            response.headers_mut().insert("Retry-After", retry.parse().expect("number of seconds"));
            return response;
        }
        return (StatusCode::TOO_MANY_REQUESTS, [("Retry-After", retry)], LOCKED_OUT).into_response();
    }
    if headers.get_all("content-length").iter().any(|value| {
        value.to_str().ok().and_then(|value| value.parse::<usize>().ok()).is_none_or(|length| length > BOOTSTRAP_BODY_BYTES)
    }) {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Bootstrap body is too large").into_response();
    }
    let bytes = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, BOOTSTRAP_BODY_BYTES)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return (StatusCode::PAYLOAD_TOO_LARGE, "Bootstrap body is too large").into_response(),
        Err(_) => return (StatusCode::REQUEST_TIMEOUT, "Bootstrap body timed out").into_response(),
    };
    let credentials: BootstrapCredentials = if browser_form {
        let mut pin = None;
        for (key, value) in url::form_urlencoded::parse(&bytes) {
            if key != "pin" || pin.is_some() { return (StatusCode::BAD_REQUEST, "Invalid PIN form").into_response(); }
            pin = Some(value.into_owned());
        }
        BootstrapCredentials { pin }
    } else if content_type == "application/json" {
        match serde_json::from_slice(&bytes) {
            Ok(credentials) => credentials,
            Err(_) => return (StatusCode::BAD_REQUEST, "Invalid bootstrap JSON").into_response(),
        }
    } else {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Use JSON or a PIN form").into_response();
    };
    if !state.session.authenticates(credentials.pin.as_deref()) {
        if browser_form {
            return bootstrap_page(state, headers, StatusCode::FORBIDDEN, Some("The PIN was not accepted. Try again."));
        }
        return deny();
    }
    // A correct PIN restores the initial budget instead of carrying a lockout
    // escalation into the rest of the session.
    state.session.succeeded();
    if state.cancellation.is_cancelled() { return deny(); }
    if browser_form {
        return Response::builder().status(StatusCode::SEE_OTHER)
            .header("Location", format!("/{}/", state.token))
            .header("Cache-Control", "no-store").header("Referrer-Policy", "no-referrer")
            .body(Body::empty()).unwrap();
    }
    match inventory(state) {
        Ok(books) => json_response(json!({
            "sessionId": state.session.id,
            "name": state.session.name(),
            "token": state.token,
            "books": books,
        })),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn index_request(
    State(mut state): State<Index>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(host) = state.authorities.check(&headers, &uri) else {
        return deny();
    };
    if state.cancellation.is_cancelled() || service_worker_request(&headers) { return deny(); }
    if uri.path() == format!("/nearby/{}", state.library.session.id) {
        return bootstrap_request(&state.library, method, &uri, &headers, body).await;
    }
    let prefix = format!("/{}/", state.token);
    if let Some(rest) = uri.path().strip_prefix(&prefix) {
        if !rest.is_empty() && !matches!(rest, "reader" | "reader.js" | "search" | "suggest") {
            return library_request(State(state.library), method, uri, headers, body).await;
        }
    }
    let current = match scoped_books(&state.library) {
        Ok(books) => books,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let current: HashSet<_> = current.iter().map(|book| book.metadata.id.as_str()).collect();
    state.entries = Arc::new(state.entries.iter().filter(|entry| current.contains(entry.id.as_str())).cloned().collect());
    if method != Method::GET
        || state.cancellation.is_cancelled()
        || service_worker_request(&headers)
    {
        return deny();
    }
    let prefix = format!("/{}/", state.token);
    let Some(rest) = uri.path().strip_prefix(&prefix) else { return deny(); };
    if matches!(rest, "reader" | "reader.js" | "search" | "suggest") {
        return reader::request(reader::Scope::Selected(state), rest, host, &uri, &headers).await;
    }
    if !rest.is_empty() { return deny(); }
    let raw = uri.query().unwrap_or("");
    if raw.len() > 4096 {
        return (StatusCode::BAD_REQUEST, "Search URL is too long").into_response();
    }
    let mut query = None;
    let mut selected = None;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        match key.as_ref() {
            "q" if query.is_none() => query = Some(value.into_owned()),
            "book" if selected.is_none() => selected = Some(value.into_owned()),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Unknown or repeated search parameter",
                )
                    .into_response()
            }
        }
    }
    let query = query.unwrap_or_default();
    let selected = selected.unwrap_or_default();
    if query.len() > QUERY_BYTES
        || query.chars().count() > QUERY_CHARS
        || query.chars().any(char::is_control)
    {
        return (
            StatusCode::BAD_REQUEST,
            "Search must be at most 256 characters and 1024 bytes, without control characters",
        )
            .into_response();
    }
    if !selected.is_empty() && !state.entries.iter().any(|book| book.id == selected) {
        return (StatusCode::BAD_REQUEST, "Archive is not shared").into_response();
    }
    let locale = match request_locale(&state.store, &headers) {
        Ok(locale) => locale,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let t = |source: &str| escape(&locale.text(source, &[]));
    let mut body=format!("<!doctype html><html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>",escape(locale.lang()),escape(locale.dir()),t("Shared library · Kiwix"));
    body.push_str(r#"
:root{color-scheme:light;--ink:#183b35;--paper:#f6f2e8;--line:#c3cdc1}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:17px/1.6 Georgia,serif}main{max-width:960px;margin:auto;padding:3rem 1.5rem}header{border-top:8px solid var(--ink);padding-top:1rem}.eyebrow{font:700 .8rem/1.4 Verdana,sans-serif;letter-spacing:.18em;text-transform:uppercase}h1{font-size:clamp(2.5rem,8vw,4.5rem);line-height:1.05;margin:1rem 0}h2{font-size:1.6rem;margin-top:2.5rem}a{color:var(--ink);text-underline-offset:.2em}a:hover{color:#8b421b}form{padding:1.5rem 0;border-block:1px solid var(--line)}label{display:block;font:bold .85rem Verdana,sans-serif;margin-bottom:.5rem}.fields{display:flex;gap:.75rem;flex-wrap:wrap}.field{flex:1;min-width:12rem}input,select,button{width:100%;padding:.8rem;border:1px solid var(--ink);border-radius:0;background:#fffdf6;color:var(--ink);font:inherit}button{width:auto;align-self:end;background:var(--ink);color:var(--paper);cursor:pointer}a:focus-visible,input:focus-visible,select:focus-visible,button:focus-visible{outline:3px solid #bb6929;outline-offset:3px}.note{font-size:.9rem;color:#4c6259}.books{padding-left:1.3rem}.results{padding-left:1.5rem}.results li{padding:.6rem 0;border-bottom:1px solid var(--line)}.results p{margin:.3rem 0}.error{color:#8b271d}footer{border-top:1px solid var(--line);margin-top:3rem;padding-top:1rem}
</style></head><body><main><header>"#);
    body.push_str(&format!(
        "<p class=\"eyebrow\">{}</p><h1>{}</h1><p>{}</p></header>",
        t("Kiwix · Shared collection"),
        t("Knowledge, within reach."),
        t("Read and search the archives selected by this library’s owner.")
    ));
    body.push_str(&format!("<p><a href=\"/{}/reader\">{}</a></p>", state.token, t("Search articles with title suggestions")));
    body.push_str(&format!("<form method=\"get\" action=\"/{}/\" role=\"search\"><div class=\"fields\"><div class=\"field\"><label for=\"q\">{}</label><input id=\"q\" name=\"q\" type=\"search\" maxlength=\"256\" value=\"{}\" placeholder=\"{}\"></div><div class=\"field\"><label for=\"book\">{}</label><select id=\"book\" name=\"book\"><option value=\"\">{}</option>",state.token,t("Search this collection"),escape(&query),t("A subject, a place, an idea…"),t("Archive"),t("All shared items")));
    for book in state.entries.iter() {
        body.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(&book.id),
            if selected == book.id { " selected" } else { "" },
            escape(&book.title)
        ));
    }
    body.push_str(&format!("</select></div><button type=\"submit\">{}</button></div><p class=\"note\">{}</p></form>",t("Search"),t("Full-text search where available; otherwise title-prefix search. Up to 20 results per archive, 100 per search.")));
    if !query.trim().is_empty() {
        let permit = match state.cancellation.0.searches.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    "Search is busy; try again shortly",
                )
                    .into_response()
            }
        };
        let books = state.entries.clone();
        let search_query = query.trim().to_owned();
        let filter = selected.clone();
        let cancellation = state.cancellation.clone();
        let found = state.cancellation.0.workers.run(move || {
            // Keep the permit until native work ends, even if the HTTP request disconnects.
            let _permit = permit;
            let mut found = Vec::new();
            let mut remaining = RESULTS_TOTAL;
            for (index, book) in books
                .iter()
                .enumerate()
                .filter(|(_, book)| filter.is_empty() || book.id == filter)
            {
                // Still search every chosen archive after the display budget is
                // exhausted, so later books are not silently omitted.
                let archive = book.archive.upgrade().ok_or("This shared file is no longer available. Stop sharing and start again")?;
                let result = match cancellation.native(|| {
                    let mut matches = archive.search_cancellable(
                        &search_query,
                        book.fulltext,
                        0,
                        RESULTS_PER_BOOK.min(remaining.max(1) as u32),
                        true,
                        &cancellation.0.search_cancel,
                    )?;
                    matches.hits.truncate(remaining);
                    for hit in &mut matches.hits {
                        hit.snippet = crate::search::snippet_text(&hit.snippet, &cancellation.0.search_cancel)?;
                    }
                    Ok::<_, String>(matches)
                }) {
                    Ok(result) => result,
                    Err(_) => break,
                };
                if let Ok(results) = &result {
                    remaining = remaining.saturating_sub(results.hits.len());
                }
                found.push((index, result));
            }
            Ok(found)
        })
        .await;
        body.push_str(&format!(
            "<h2>{}</h2>",
            escape(&locale.text("Results for “{query}”", &[("query", query.trim())]))
        ));
        match found {
            Ok(found) => {
                for (index, result) in found {
                    let book = &state.entries[index];
                    body.push_str(&format!("<section><h3>{}</h3>", escape(&book.title)));
                    match result {
                        Ok(SearchResults { hits, total }) => {
                            if hits.is_empty() {
                                body.push_str(&format!("<p>{}</p>",t(if total==0 {"No matching articles."} else {"More matches are available. Select this archive above to see its results within the collection’s display limit."})));
                            } else {
                                let source = if book.fulltext {
                                    "Showing {shown} of approximately {count} matches."
                                } else {
                                    "Showing {shown} of approximately {count} matches by title prefix."
                                };
                                body.push_str(&format!(
                                    "<p class=\"note\">{}</p><ol class=\"results\">",
                                    escape(&locale.counted(
                                        source,
                                        total,
                                        hits.len() as u64
                                    ))
                                ));
                                for hit in hits {
                                    let path = archive_url(&hit.path);
                                    let target =
                                        url::form_urlencoded::Serializer::new(String::new())
                                            .append_pair("path", &path)
                                            .finish();
                                    body.push_str(&format!(
                                        "<li><a href=\"{}?{}\">{}</a><p>{}</p></li>",
                                        escape(&access_url(
                                            host,
                                            book.port,
                                            &format!("/.kiwix-access/{}", state.token)
                                        )),
                                        escape(&target),
                                        excerpt(
                                            if hit.title.is_empty() {
                                                &hit.path
                                            } else {
                                                &hit.title
                                            },
                                            512
                                        ),
                                        excerpt(&hit.snippet, 400)
                                    ));
                                }
                                body.push_str("</ol>");
                            }
                        }
                        Err(_) => body.push_str(&format!(
                            "<p class=\"error\">{}</p>",
                            t("This archive could not be searched.")
                        )),
                    }
                    body.push_str("</section>");
                }
            }
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Archive search worker stopped",
                )
                    .into_response()
            }
        }
    }
    body.push_str(&format!(
        "<h2>{}</h2><ul class=\"books\">",
        t("Browse the collection")
    ));
    for book in state.entries.iter() {
        body.push_str(&format!(
            "<li><a href=\"{}\">{}</a> · <a href=\"/{}/download/{}\" download>{}</a></li>",
            escape(&access_url(
                host,
                book.port,
                &format!("/.kiwix-access/{}", state.token)
            )),
            escape(&book.title),
            state.token,
            query_value(&book.id),
            t("Download"),
        ));
    }
    body.push_str(&format!(
        "</ul><footer class=\"note\">{}</footer></main></body></html>",
        t("Read-only sharing. This page has no access to the owner’s application controls.")
    ));
    page(body)
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LibraryRoute<'a> {
    Index,
    Inventory,
    Root,
    Entries,
    Categories,
    Languages,
    Open(&'a str),
    Download(&'a str),
    Reader(&'a str),
}

fn library_route(rest: &str) -> Option<LibraryRoute<'_>> {
    Some(match rest {
        "" => LibraryRoute::Index,
        "inventory" => LibraryRoute::Inventory,
        "catalog/v2/root.xml" => LibraryRoute::Root,
        "catalog/v2/entries" => LibraryRoute::Entries,
        "catalog/v2/categories" => LibraryRoute::Categories,
        "catalog/v2/languages" => LibraryRoute::Languages,
        "reader" | "reader.js" | "search" | "suggest" => LibraryRoute::Reader(rest),
        other if other.starts_with("open/") || other.starts_with("download/") => {
            let (open, id) = match other.strip_prefix("open/") {
                Some(id) => (true, id),
                None => (false, &other["download/".len()..]),
            };
            if id.is_empty() || id.contains('/') || id.contains(['\\', '?', '#']) {
                return None;
            }
            if open {
                LibraryRoute::Open(id)
            } else {
                LibraryRoute::Download(id)
            }
        }
        _ => return None,
    })
}

async fn library_request(
    State(state): State<LibraryIndex>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(host) = state.authorities.check(&headers, &uri) else {
        return deny();
    };
    if state.cancellation.is_cancelled()
        || service_worker_request(&headers)
    {
        return deny();
    }
    if uri.path() == format!("/nearby/{}", state.session.id) {
        return bootstrap_request(&state, method, &uri, &headers, body).await;
    }
    if !matches!(method, Method::GET | Method::HEAD)
    {
        return deny();
    }
    let prefix = format!("/{}/", state.token);
    let Some(rest) = uri.path().strip_prefix(&prefix) else {
        return deny();
    };
    let Some(route) = library_route(rest) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let mut response = match route {
        LibraryRoute::Index => library_index_page(&state, host, &uri, &headers),
        LibraryRoute::Inventory => match inventory(&state) {
            Ok(books) => json_response(json!({ "books": books })),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
        },
        LibraryRoute::Root => opds_root(&state, host, &headers),
        LibraryRoute::Entries => opds_entries(&state, host, &uri, &headers),
        LibraryRoute::Categories => opds_facets(&state, host, &headers, FacetKind::Categories),
        LibraryRoute::Languages => opds_facets(&state, host, &headers, FacetKind::Languages),
        LibraryRoute::Open(id) => library_open(&state, host, id, &uri),
        LibraryRoute::Download(id) => library_download(&state, id, &headers).await,
        LibraryRoute::Reader(rest) if state.session.selected.is_none() => reader::request(reader::Scope::Library(state.clone()), rest, host, &uri, &headers).await,
        LibraryRoute::Reader(_) => deny(),
    };
    if method == Method::HEAD {
        *response.body_mut() = Body::empty();
    }
    response
}

fn book_present(book: &Book) -> bool {
    if book.source_reference.is_some() {
        return cfg!(mobile);
    }
    std::fs::metadata(&book.path).is_ok_and(|metadata| available_len(book, metadata.len()))
}

/// Whole-library sharing serves persisted library entries. A non-persistent
/// provider reference lives only in the session overlay and must not become
/// LAN-visible; a persisted external-archive reference is an ordinary entry.
fn shareable_library_book(book: &Book) -> bool {
    book.source_reference
        .as_ref()
        .is_none_or(|source| source.persistent)
}

fn available_books(state: &LibraryIndex) -> Result<Vec<Book>, String> {
    Ok(state
        .store
        .books()?
        .into_iter()
        .filter(|book| {
            shareable_library_book(book)
                && book.available
                && state.availability.present(book)
        })
        .collect())
}

/// Resolve one identifier against the same availability rules the index uses.
fn available_book(state: &LibraryIndex, id: &str) -> Result<Book, String> {
    state
        .store
        .book(id)?
        .filter(|book| {
            shareable_library_book(book)
                && book.available
                && state.availability.present(book)
        })
        .ok_or_else(|| "Archive is not shared".to_string())
}

/// Stable title order with one lowercase key per book instead of two
/// allocations on every comparison.
fn sort_books_by_title(books: &mut Vec<Book>) {
    let mut keyed: Vec<(String, Book)> = std::mem::take(books)
        .into_iter()
        .map(|book| (book.metadata.title.to_lowercase(), book))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    books.extend(keyed.into_iter().map(|(_, book)| book));
}

fn index_page_start(uri: &Uri) -> Result<usize, String> {
    let raw = uri.query().unwrap_or("");
    if raw.len() > 64 {
        return Err("Invalid library page".into());
    }
    let mut start = 0usize;
    let mut seen = false;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if key != "start" || seen {
            return Err("Unknown or repeated library page parameter".into());
        }
        seen = true;
        start = value.parse().map_err(|_| "Invalid library page".to_string())?;
    }
    Ok(start)
}

fn library_index_page(
    state: &LibraryIndex,
    host: IpAddr,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {
    let mut books = match scoped_books(state) {
        Ok(books) => books,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let start = match index_page_start(uri) {
        Ok(start) => start,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    sort_books_by_title(&mut books);
    let total = books.len();
    let start = start.min(total);
    let visible = books
        .get(start..(start + OPDS_PAGE_MAX).min(total))
        .unwrap_or(&[]);
    let locale = match request_locale(&state.store, headers) {
        Ok(locale) => locale,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let t = |source: &str| escape(&locale.text(source, &[]));
    let catalog = access_url(
        host,
        state.port,
        &format!("/{}/catalog/v2/root.xml", state.token),
    );
    let mut body = format!(
        "<!doctype html><html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><link rel=\"alternate\" type=\"application/atom+xml\" href=\"{}\"><style>",
        escape(locale.lang()),
        escape(locale.dir()),
        t("Library server · Kiwix"),
        escape(&catalog),
    );
    body.push_str(r#"
:root{color-scheme:light;--ink:#183b35;--paper:#f6f2e8;--line:#c3cdc1}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:17px/1.6 Georgia,serif}main{max-width:960px;margin:auto;padding:3rem 1.5rem}header{border-top:8px solid var(--ink);padding-top:1rem}.eyebrow{font:700 .8rem/1.4 Verdana,sans-serif;letter-spacing:.18em;text-transform:uppercase}h1{font-size:clamp(2.5rem,8vw,4.5rem);line-height:1.05;margin:1rem 0}h2{font-size:1.5rem;margin-top:2.5rem}a{color:var(--ink);text-underline-offset:.2em}a:hover{color:#8b421b}.book{padding:1.25rem 0;border-top:1px solid var(--line)}.book h3{margin:0 0 .35rem;font-size:1.25rem}.meta{font:400 .85rem/1.5 Verdana,sans-serif;color:#4d625c;margin:.2rem 0}.note{color:#4d625c}.catalog{display:inline-block;margin-top:1rem;padding:.6rem 1.2rem;background:var(--ink);color:var(--paper);text-decoration:none}footer{margin-top:3rem;border-top:1px solid var(--line);padding-top:1rem}
"#);
    body.push_str(&format!(
        "</style></head><body><main><header><p class=\"eyebrow\">{}</p><h1>{}</h1><p>{}</p><a class=\"catalog\" href=\"{}\">{}</a></header><h2>{}</h2>",
        t("Kiwix · Library server"),
        t("Knowledge, within reach."),
        escape(&locale.counted(
            "Serving {count} archives from this device. Read-only; no application controls are exposed.",
            total as u64,
            total as u64
        )),
        escape(&catalog),
        t("Open the OPDS catalog"),
        t("Available archives"),
    ));
    body.push_str(&format!("<p><a href=\"/{}/reader\">{}</a></p>", state.token, t("Search articles with title suggestions")));
    for book in visible {
        let link = access_url(
            host,
            state.port,
            &format!(
                "/{}/open/{}",
                state.token,
                query_value(&book.metadata.id)
            ),
        );
        body.push_str(&format!(
            "<article class=\"book\" id=\"book-{}\"><h3><a href=\"{}\">{}</a></h3><p class=\"meta\">{} · {}</p><p>{}</p><p><a href=\"/{}/download/{}\" download>{}</a></p></article>",
            escape(&book.metadata.id),
            escape(&link),
            escape(&book.metadata.title),
            escape(&book.metadata.language),
            escape(&locale.counted("{count} articles", book.metadata.article_count, book.metadata.article_count)),
            excerpt(&book.metadata.description, 400),
            state.token,
            query_value(&book.metadata.id),
            t("Download"),
        ));
    }
    if start + visible.len() < total {
        let next = access_url(
            host,
            state.port,
            &format!("/{}/?start={}", state.token, start + visible.len()),
        );
        body.push_str(&format!(
            "<p class=\"note\"><a href=\"{}\" rel=\"next\">{}</a></p>",
            escape(&next),
            t("Next page"),
        ));
    }
    body.push_str(&format!(
        "<footer class=\"note\">{}</footer></main></body></html>",
        t("Read-only library sharing. Anyone with a link can read these items until you stop sharing.")
    ));
    page(body)
}

fn opds_updated() -> String {
    crate::timestamps::utc_seconds()
}

fn opds_document(body: String) -> Response {
    Response::builder()
        .header("Content-Type", "application/atom+xml; charset=utf-8")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
        )
        .header("Referrer-Policy", "no-referrer")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(body))
        .unwrap()
}

fn query_value(value: &str) -> String {
    // Unreserved characters work in both query values and opaque book-ID path
    // segments. Form encoding's '+' for a space is not valid for the latter.
    const ENCODE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-').remove(b'_').remove(b'.').remove(b'~');
    percent_encoding::utf8_percent_encode(value, ENCODE).to_string()
}

fn opds_root(state: &LibraryIndex, host: IpAddr, headers: &HeaderMap) -> Response {
    let locale = match request_locale(&state.store, headers) {
        Ok(locale) => locale,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let t = |source: &str| escape(&locale.text(source, &[]));
    let now = opds_updated();
    let base = access_url(
        host,
        state.port,
        &format!("/{}/catalog/v2", state.token),
    );
    let mut body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:opds=\"http://opds-spec.org/2010/catalog\" xmlns:thr=\"http://purl.org/syndication/thread/1.0\" xml:lang=\"{}\"><id>urn:kiwix:local-library:{}</id><title>{}</title><updated>{}</updated><author><name>Kiwix</name></author><link rel=\"self\" type=\"application/atom+xml;profile=opds-catalog;kind=navigation\" href=\"{}/root.xml\"/><link rel=\"start\" type=\"application/atom+xml;profile=opds-catalog;kind=navigation\" href=\"{}/root.xml\"/><link rel=\"search\" type=\"application/atom+xml\" href=\"{}/entries?q={{searchTerms}}\"/>",
        escape(locale.lang()),
        escape(&state.token),
        t("Local Kiwix library"),
        now,
        escape(&base),
        escape(&base),
        escape(&base),
    );
    for (path, title) in [
        ("entries", "All books"),
        ("categories", "Categories"),
        ("languages", "Languages"),
    ] {
        body.push_str(&format!(
            "<entry><id>urn:kiwix:local-library:{}:{}</id><title>{}</title><updated>{}</updated><content type=\"text\">{}</content><link rel=\"subsection\" type=\"application/atom+xml;profile=opds-catalog;kind=acquisition\" href=\"{}/{}\"/></entry>",
            escape(&state.token),
            path,
            t(title),
            now,
            t(title),
            escape(&base),
            path,
        ));
    }
    body.push_str("</feed>");
    opds_document(body)
}

struct EntriesQuery {
    q: String,
    lang: String,
    tag: String,
    start: usize,
    count: usize,
}

fn parse_entries_query(uri: &Uri) -> Result<EntriesQuery, &'static str> {
    let raw = uri.query().unwrap_or("");
    if raw.len() > 8192 {
        return Err("Catalog query is too long");
    }
    let mut q = None;
    let mut lang = None;
    let mut tag = None;
    let mut start = None;
    let mut count = None;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        let value = value.into_owned();
        if value.chars().any(char::is_control) {
            return Err("Invalid catalog filter");
        }
        match key.as_ref() {
            "q" if q.is_none() => q = Some(value),
            "lang" if lang.is_none() => lang = Some(value),
            "tag" | "category" if tag.is_none() => tag = Some(value),
            "start" if start.is_none() => {
                start = Some(value.parse::<usize>().map_err(|_| "Invalid catalog start")?)
            }
            "count" if count.is_none() => {
                count = Some(value.parse::<usize>().map_err(|_| "Invalid catalog count")?)
            }
            _ => {
                return Err("Unknown or repeated catalog parameter")
            }
        }
    }
    let query = EntriesQuery {
        q: q.unwrap_or_default(),
        lang: lang.unwrap_or_default(),
        tag: tag.unwrap_or_default(),
        start: start.unwrap_or(0),
        count: count.unwrap_or(OPDS_PAGE_DEFAULT),
    };
    if query.count == 0 || query.count > OPDS_PAGE_MAX {
        return Err("Catalog count must be between 1 and 200");
    }
    if query.q.len() > QUERY_BYTES
        || query.q.chars().count() > QUERY_CHARS
        || query.lang.chars().count() > 64
        || query.tag.chars().count() > 128
    {
        return Err("Catalog filter is too long");
    }
    Ok(query)
}

fn book_matches(book: &Book, query: &EntriesQuery) -> bool {
    if !query.lang.is_empty()
        && !book
            .metadata
            .language
            .split([',', ';'])
            .map(str::trim)
            .any(|language| language.eq_ignore_ascii_case(&query.lang))
    {
        return false;
    }
    if !query.tag.is_empty() {
        let category = format!("_category:{}", query.tag);
        if !book
            .metadata
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(&query.tag) || tag.eq_ignore_ascii_case(&category))
        {
            return false;
        }
    }
    if !query.q.is_empty() {
        let needle = query.q.to_lowercase();
        let haystack = format!(
            "{}\n{}\n{}",
            book.metadata.title, book.metadata.description, book.metadata.name
        )
        .to_lowercase();
        if !haystack.contains(&needle) {
            return false;
        }
    }
    true
}

fn book_entry_xml(book: &Book, index_url: &str, download_base: &str) -> String {
    let id = &book.metadata.id;
    let updated = if book.added_at.is_empty() {
        opds_updated()
    } else {
        book.added_at.clone()
    };
    let mut entry = format!(
        "<entry><id>urn:uuid:{}</id><title>{}</title><updated>{}</updated><summary>{}</summary><language>{}</language><name>{}</name><articleCount>{}</articleCount><mediaCount>{}</mediaCount><size>{}</size><dc:language>{}</dc:language><dc:publisher>{}</dc:publisher><dc:creator>{}</dc:creator><dc:issued>{}</dc:issued>",
        escape(id),
        escape(&book.metadata.title),
        escape(&updated),
        escape(&book.metadata.description),
        escape(&book.metadata.language),
        escape(&book.metadata.name),
        book.metadata.article_count,
        book.metadata.media_count,
        book.metadata.size,
        escape(&book.metadata.language),
        escape(&book.metadata.publisher),
        escape(&book.metadata.creator),
        escape(&book.metadata.date),
    );
    for tag in book.metadata.tags.iter().take(64) {
        entry.push_str(&format!(
            "<category term=\"{}\" label=\"{}\"/>",
            escape(tag),
            escape(tag)
        ));
    }
    entry.push_str(&format!(
        "<link rel=\"http://opds-spec.org/acquisition/open-access\" type=\"application/x-zim\" href=\"{}\" length=\"{}\"/>",
        escape(&format!("{download_base}/download/{}", query_value(id))),
        book.metadata.size,
    ));
    entry.push_str(&format!(
        "<link rel=\"alternate\" type=\"text/html\" href=\"{}open/{}\"/></entry>",
        escape(index_url),
        query_value(id),
    ));
    entry
}

fn opds_entries(
    state: &LibraryIndex,
    host: IpAddr,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {
    let query = match parse_entries_query(uri) {
        Ok(query) => query,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let locale = match request_locale(&state.store, headers) {
        Ok(locale) => locale,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let mut books = match scoped_books(state) {
        Ok(books) => books,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    books.retain(|book| book_matches(book, &query));
    sort_books_by_title(&mut books);
    let total = books.len();
    let start = query.start.min(total);
    let page = books
        .get(start..(start + query.count).min(total))
        .unwrap_or(&[]);
    let base = access_url(
        host,
        state.port,
        &format!("/{}/catalog/v2", state.token),
    );
    let index_url = access_url(host, state.port, &format!("/{}/", state.token));
    let download_base = index_url.trim_end_matches('/');
    let now = opds_updated();
    let mut body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:opds=\"http://opds-spec.org/2010/catalog\" xmlns:dc=\"http://purl.org/dc/terms/\" xmlns:opensearch=\"http://a9.com/-/spec/opensearch/1.1/\" xml:lang=\"{}\"><id>urn:kiwix:local-library:{}:entries</id><title>{}</title><updated>{}</updated><author><name>Kiwix</name></author><opensearch:totalResults>{}</opensearch:totalResults><opensearch:startIndex>{}</opensearch:startIndex><opensearch:itemsPerPage>{}</opensearch:itemsPerPage><link rel=\"self\" type=\"application/atom+xml;profile=opds-catalog;kind=acquisition\" href=\"{}/entries\"/><link rel=\"start\" type=\"application/atom+xml;profile=opds-catalog;kind=navigation\" href=\"{}/root.xml\"/>",
        escape(locale.lang()),
        escape(&state.token),
        escape(&locale.text("Local Kiwix library", &[])),
        now,
        total,
        start,
        page.len(),
        escape(&base),
        escape(&base),
    );
    if start + page.len() < total {
        let mut next = format!(
            "{}/entries?start={}&count={}",
            base,
            start + page.len(),
            query.count
        );
        if !query.q.is_empty() {
            next.push_str(&format!("&q={}", query_value(&query.q)));
        }
        if !query.lang.is_empty() {
            next.push_str(&format!("&lang={}", query_value(&query.lang)));
        }
        if !query.tag.is_empty() {
            next.push_str(&format!("&tag={}", query_value(&query.tag)));
        }
        body.push_str(&format!(
            "<link rel=\"next\" type=\"application/atom+xml;profile=opds-catalog;kind=acquisition\" href=\"{}\"/>",
            escape(&next)
        ));
    }
    for book in page {
        body.push_str(&book_entry_xml(book, &index_url, download_base));
    }
    body.push_str("</feed>");
    opds_document(body)
}

#[derive(Clone, Copy)]
enum FacetKind {
    Categories,
    Languages,
}

fn opds_facets(
    state: &LibraryIndex,
    host: IpAddr,
    headers: &HeaderMap,
    kind: FacetKind,
) -> Response {
    let locale = match request_locale(&state.store, headers) {
        Ok(locale) => locale,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let books = match scoped_books(state) {
        Ok(books) => books,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let mut facets: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for book in &books {
        match kind {
            FacetKind::Languages => {
                for language in book.metadata.language.split([',', ';']).map(str::trim) {
                    if !language.is_empty() {
                        *facets.entry(language.to_string()).or_default() += 1;
                    }
                }
            }
            FacetKind::Categories => {
                for tag in &book.metadata.tags {
                    if let Some(category) = tag.strip_prefix("_category:") {
                        if !category.is_empty() {
                            *facets.entry(category.to_string()).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    let (path, query_key, title) = match kind {
        FacetKind::Categories => ("categories", "tag", "Categories"),
        FacetKind::Languages => ("languages", "lang", "Languages"),
    };
    let base = access_url(
        host,
        state.port,
        &format!("/{}/catalog/v2", state.token),
    );
    let now = opds_updated();
    let mut body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:opds=\"http://opds-spec.org/2010/catalog\" xmlns:thr=\"http://purl.org/syndication/thread/1.0\" xml:lang=\"{}\"><id>urn:kiwix:local-library:{}:{}</id><title>{}</title><updated>{}</updated><author><name>Kiwix</name></author><opensearch:totalResults xmlns:opensearch=\"http://a9.com/-/spec/opensearch/1.1/\">{}</opensearch:totalResults><link rel=\"self\" type=\"application/atom+xml;profile=opds-catalog;kind=navigation\" href=\"{}/{}\"/><link rel=\"start\" type=\"application/atom+xml;profile=opds-catalog;kind=navigation\" href=\"{}/root.xml\"/>",
        escape(locale.lang()),
        escape(&state.token),
        path,
        escape(&locale.text(title, &[])),
        now,
        facets.len(),
        escape(&base),
        path,
        escape(&base),
    );
    for (value, count) in facets {
        body.push_str(&format!(
            "<entry><id>urn:kiwix:local-library:{}:{}:{}</id><title>{}</title><updated>{}</updated><content type=\"text\">{}</content><thr:count>{}</thr:count><link rel=\"subsection\" type=\"application/atom+xml;profile=opds-catalog;kind=acquisition\" href=\"{}/entries?{}={}\"/></entry>",
            escape(&state.token),
            path,
            escape(&value),
            escape(&value),
            now,
            escape(&value),
            count,
            escape(&base),
            query_key,
            query_value(&value),
        ));
    }
    body.push_str("</feed>");
    opds_document(body)
}

/// Produce a portable acquisition filename, including `.zim` within 120 bytes.
pub(crate) fn download_name(name: &str, id: &str) -> String {
    fn sanitized_stem(value: &str) -> String {
        let value = if value.get(value.len().saturating_sub(4)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".zim"))
        {
            &value[..value.len() - 4]
        } else {
            value
        };
        let mut stem = String::with_capacity(value.len().min(116) + 4);
        stem.extend(value.chars().take(116).map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        }));
        stem
    }

    // Sanitize metadata and its fallback alike, reserving four ASCII bytes for
    // the suffix before truncation so every advertised name stays usable.
    let mut base = sanitized_stem(name);
    if base.trim_matches(['.', '_', '-']).is_empty() {
        base = sanitized_stem(id);
    }
    if base.trim_matches(['.', '_', '-']).is_empty() {
        base = "archive".into();
    }
    base.push_str(".zim");
    if crate::acquisition::shared_file_name(&base).is_err() {
        // The bounded ASCII component can only fail the shared validator for
        // an OS-reserved stem. Prefix it without exceeding the 120-byte cap.
        base.truncate(base.len() - 4);
        base.truncate(115);
        base.insert(0, '_');
        base.push_str(".zim");
    }
    base
}

fn open_download(book: Book) -> Result<(download::Download, u64, String), ResolveError> {
    // Validate the same retained descriptors that will supply bytes. Reopening
    // by pathname after UUID validation would permit a replacement race.
    let source = crate::archive_source::retain(&book).map_err(ResolveError::gone)?;
    source.unchanged().map_err(ResolveError::gone)?;
    let identity = source.identity().map_err(ResolveError::gone)?;
    let metadata = source.open().and_then(|archive| archive.metadata()).map_err(ResolveError::gone)?;
    if metadata.id != book.metadata.id {
        return Err(ResolveError::gone("Archive file identity changed"));
    }
    let download = download::Download::new(source, metadata.size).map_err(ResolveError::gone)?;
    if download.identity() != &identity {
        return Err(ResolveError::gone("Archive source changed while opening"));
    }
    let size = download.size;
    Ok((download, size, download_name(&book.metadata.name, &book.metadata.id)))
}

pub(crate) fn open_library_download(book: Book) -> Result<(download::Download, u64, String), String> {
    open_download(book).map_err(|error| error.message)
}

pub(crate) fn same_owner(expected: Option<&Book>, current: Option<&Book>) -> bool {
    matches!((expected, current), (Some(expected), Some(current))
        if current.available && expected.installation_id == current.installation_id
            && expected.path == current.path && expected.source_reference == current.source_reference)
}

fn library_open(state: &LibraryIndex, host: IpAddr, raw_id: &str, uri: &Uri) -> Response {
    let target = match activation_target(uri) {
        Ok(target) => target,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let target = url::form_urlencoded::Serializer::new(String::new()).append_pair("path", &target).finish();
    let id = match percent_encoding::percent_decode_str(raw_id).decode_utf8() {
        Ok(id) => id.into_owned(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid archive identifier").into_response(),
    };
    let book = match state.store.book(&id) {
        Ok(Some(book)) if book.available => book,
        Ok(Some(_)) => {
            return (StatusCode::GONE, "This archive is no longer available in the library")
                .into_response()
        }
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Archive is not in the library").into_response()
        }
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    if !state.session.permits(&book) { return deny(); }
    if !book_present(&book) {
        return (
            StatusCode::GONE,
            "This archive file is missing or was replaced; restore it or import it again",
        )
            .into_response();
    }
    let books = match scoped_books(state) {
        Ok(books) => books,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let ports = match state.hosts.ensure(&books, std::slice::from_ref(&book)) {
        Ok(ports) => ports,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let Some(port) = ports.get(&id) else {
        return (StatusCode::NOT_FOUND, "Archive is not available").into_response();
    };
    Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header(
            "Location",
            access_url(host, *port, &format!("/.kiwix-access/{}?{target}", state.token)),
        )
        .header("Referrer-Policy", "no-referrer")
        .header("Cache-Control", "no-store")
        .body(Body::empty())
        .unwrap()
}

async fn library_download(
    state: &LibraryIndex,
    raw_id: &str,
    headers: &HeaderMap,
) -> Response {
    let id = match percent_encoding::percent_decode_str(raw_id).decode_utf8() {
        Ok(id) => id.into_owned(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid archive identifier").into_response(),
    };
    let book = match state.store.book(&id) {
        Ok(Some(book)) => book,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Archive is not in the library").into_response()
        }
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    if !state.session.permits(&book) { return deny(); }
    if !book.available {
        return (StatusCode::GONE, "This archive is no longer available in the library")
            .into_response();
    }
    let owner = Arc::new(book);
    let opened = state
        .cancellation
        .0
        .workers
        .run({
            let book = owner.clone();
            move || Ok(open_download((*book).clone()))
        })
        .await;
    let (file, size, name) = match opened {
        Ok(Ok(opened)) => opened,
        Ok(Err(error)) => return (error.status, error.message).into_response(),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Archive worker stopped").into_response()
        }
    };
    let current = match state.store.book(&id) {
        Ok(current) => current,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    if !same_owner(Some(owner.as_ref()), current.as_ref()) || state.cancellation.is_cancelled() {
        return (StatusCode::GONE, "This archive is no longer shared by its library owner").into_response();
    }
    let range = match headers
        .get("range")
        .and_then(|value| value.to_str().ok())
        .map(|value| parse_range(value, size))
        .transpose()
    {
        Ok(range) => range,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", format!("bytes */{size}"))
                .body(Body::empty())
                .unwrap()
        }
    };
    let (start, length) = range.unwrap_or((0, size));
    let cancellation = state.cancellation.clone();
    let store = state.store.clone();
    let chunks = stream::try_unfold(
        (file, start, length, cancellation.clone(), store, owner),
        |(mut file, offset, left, cancellation, store, owner)| async move {
            if left == 0 || cancellation.is_cancelled() {
                return Ok::<_, io::Error>(None);
            }
            let count = left.min(1024 * 1024);
            let stopped = cancellation.clone();
            let checked_store = store.clone();
            let checked_owner = owner.clone();
            let (file, bytes) = cancellation.0.workers.run(move || {
                if !checked_store.owner_matches(checked_owner.as_ref())? {
                    return Err("This archive is no longer shared by its library owner".into());
                }
                let bytes = stopped.native(|| file.read(offset, count))
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
                Ok((file, bytes))
            }).await.map_err(io::Error::other)?;
            Ok(Some((bytes, (file, offset + count, left - count, cancellation, store, owner))))
        },
    ).take_until(cancellation.cancelled());
    let mut response = Response::builder()
        .status(if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header("Content-Type", "application/x-zim")
        .header("Content-Length", length)
        .header("Accept-Ranges", "bytes")
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{name}\""),
        )
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "no-store");
    if range.is_some() {
        response = response.header(
            "Content-Range",
            format!("bytes {start}-{}/{size}", start + length - 1),
        );
    }
    response
        .body(Body::from_stream(chunks))
        .unwrap()
}

async fn shared_request(
    State(state): State<SharedBook>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if state.authorities.check(&headers, &uri).is_none()
        || state.cancellation.is_cancelled()
        || service_worker_request(&headers)
        || !matches!(method, Method::GET | Method::HEAD)
    {
        return deny();
    }
    if let Err(error) = state.archive.validate_selected() {
        return (error.status, error.message).into_response();
    }
    if uri.path() == format!("/.kiwix-access/{}", state.token) {
        let target = match activation_target(&uri) {
            Ok(target) => target,
            Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
        };
        let locale = match request_locale(&state.store, &headers) {
            Ok(locale) => locale,
            Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
        };
        let mut response = activation_page(&locale, &target, state.port, &state.token);
        if method == Method::HEAD {
            *response.body_mut() = Body::empty();
        }
        return response;
    }
    let cookie = format!("kiwix_share_{}={}", state.port, state.token);
    if !headers
        .get_all("cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .any(|c| c.split(';').any(|c| c.trim() == cookie))
    {
        return deny();
    }
    if let Some((seq, last_used)) = &state.activity {
        let next = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        last_used.store(next, std::sync::atomic::Ordering::Relaxed);
    }
    let path = match percent_encoding::percent_decode_str(uri.path().trim_start_matches('/'))
        .decode_utf8()
    {
        Ok(path) => path.into_owned(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8 archive path").into_response(),
    };
    let archive = match state
        .cancellation
        .0
        .workers
        .run({
            let archive = state.archive.clone();
            move || Ok(archive.resolve())
        })
        .await
    {
        Ok(Ok(archive)) => archive,
        Ok(Err(error)) => return (error.status, error.message).into_response(),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Archive worker stopped").into_response()
        }
    };
    let requested = path.clone();
    let cancellation = state.cancellation.clone();
    let entry = match state
        .cancellation
        .0
        .workers
        .run({
            let archive = archive.clone();
            move || {
                cancellation
                    .native(|| {
                        if requested.is_empty() {
                            archive.main_entry()
                        } else {
                            archive.entry(&requested)
                        }
                    })
                    .map_err(|error| error.to_string())
            }
        })
        .await
    {
        Ok(Ok(entry)) => entry,
        Ok(Err(error)) => {
            if state.cancellation.is_cancelled() {
                return deny();
            }
            return (StatusCode::NOT_FOUND, error).into_response();
        }
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Archive worker stopped").into_response()
        }
    };
    if entry.path != path {
        return Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(
                "Location",
                format!(
                    "{}{}",
                    archive_url(&entry.path),
                    uri.query().map(|q| format!("?{q}")).unwrap_or_default()
                ),
            )
            .header("Cache-Control", "no-store")
            .body(Body::empty())
            .unwrap();
    }
    let range = match headers
        .get("range")
        .and_then(|h| h.to_str().ok())
        .map(|r| parse_range(r, entry.size))
        .transpose()
    {
        Ok(range) => range,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", format!("bytes */{}", entry.size))
                .body(Body::empty())
                .unwrap()
        }
    };
    let (start, length) = range.unwrap_or((0, entry.size));
    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let size = entry.size;
    let stream = stream::try_unfold(
        (
            archive,
            entry.path,
            start,
            length,
            state.cancellation.clone(),
        ),
        |(archive, path, offset, left, cancellation)| async move {
            if left == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            let count = left.min(1024 * 1024);
            let a = archive.clone();
            let p = path.clone();
            let stopped = cancellation.clone();
            let bytes = cancellation.0.workers.run(move || {
                stopped.native(|| a.read(&p, offset, count))
                    .map_err(|error| error.to_string())?
            }).await.map_err(std::io::Error::other)?;
            if bytes.len() as u64 != count {
                return Err(std::io::Error::other("Truncated archive resource"));
            }
            Ok(Some((
                bytes,
                (archive, path, offset + count, left - count, cancellation),
            )))
        },
    )
    .take_until(state.cancellation.cancelled());
    let mime = if entry.mime.starts_with("text/") && !entry.mime.contains("charset=") {
        format!("{}; charset=utf-8", entry.mime)
    } else {
        entry.mime
    };
    let mut response=Response::builder().status(status).header("Content-Type",mime).header("Content-Length",length).header("Accept-Ranges","bytes")
        .header("Referrer-Policy","same-origin").header("Content-Security-Policy",ARCHIVE_CSP)
        .header("Permissions-Policy","camera=(), geolocation=(), microphone=(), display-capture=()")
        .header("X-Content-Type-Options","nosniff").header("Cache-Control","no-store");
    if range.is_some() {
        response = response.header(
            "Content-Range",
            format!("bytes {start}-{}/{size}", start + length - 1),
        );
    }
    response
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from_stream(stream)
        })
        .unwrap()
}

#[cfg(test)]
#[path = "sharing_tests.rs"]
mod tests;
