// SPDX-License-Identifier: GPL-3.0-or-later
//! Public encrypted library sharing. A session snapshots installation membership,
//! not bytes: sources are lazily retained and identity-checked for every request.
#![forbid(unsafe_code)]
use super::{discovery::Peer, network::{self, RateBudget, RecvStream, SendStream}, library_transport as transport, library_wire as wire};
use crate::{store::{Book, Store}, transfer_policy::JoinedRoute};
use bytes::Bytes;
use reqwest::{header::{HeaderMap, HeaderValue}, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, HashMap, HashSet}, sync::{Arc, atomic::{AtomicU64, AtomicUsize, Ordering}}};
use tokio_util::sync::CancellationToken;
pub(crate) use super::library_route::LibraryRouteAuthority;

/// Absolute bound on waiting for the client's completion byte. The byte can
/// only follow a fully consumed response, so a peer that stays connected without
/// sending it is stalled, not slow.
const ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct LibraryPeerDescriptor {
    pub session_id: String, pub peer_id: String, pub name: String, pub port: u16,
    pub addresses: Vec<String>, pub native_services: Vec<String>,
    #[cfg_attr(not(any(target_os = "ios", target_os = "macos")), expect(dead_code, reason = "Only Apple's native QUIC adapter supports the shared descriptor's peer-to-peer route preference"))]
    pub direct: bool,
    pub authority: LibraryRouteAuthority,
    pub route: Option<JoinedRoute>, pub cancel: CancellationToken,
}
impl LibraryPeerDescriptor {
    pub(crate) fn from_peer(peer: &Peer) -> Result<Self, String> {
        let peer = super::discovery::library_peer(peer.clone())?;
        let (addresses, native_services) = if peer.host.starts_with("bonjour:") || peer.host.starts_with("bonjour-lan:") {
            (peer.addresses, vec![peer.host])
        } else { (peer.addresses, Vec::new()) };
        let descriptor = Self { session_id: peer.library_session_id.ok_or("Missing library session")?, peer_id: peer.peer_id,
            name: peer.publisher_name.unwrap_or(peer.name), port: peer.port, addresses, native_services, direct: false,
            route: None, cancel: CancellationToken::new(), authority: Default::default() };
        descriptor.validate()?; Ok(descriptor)
    }
    pub(crate) fn matches_peer(&self, peer: &Peer) -> bool {
        Self::from_peer(peer).is_ok_and(|current| self.session_id == current.session_id && self.peer_id == current.peer_id
            && self.port == current.port && self.addresses == current.addresses && self.native_services == current.native_services)
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() { return Err("The selected Nearby share retired".into()); }
        if let Some(route) = &self.route { route.check()?; }
        self.authority.check()
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        self.check()?;
        if self.cancel.is_cancelled() || !super::discovery::valid_library_session_id(&self.session_id)
            || !super::protocol::valid_peer_id(&self.peer_id) || self.port == 0
            || !crate::publisher::valid_name(&self.name) || self.name.is_empty()
            || self.addresses.len() > 32 || self.native_services.len() > 8
            || (self.addresses.is_empty() && self.native_services.is_empty())
            || self.addresses.iter().any(|s| !s.parse::<std::net::IpAddr>().is_ok_and(network::usable_address))
        { return Err("The selected Nearby library is no longer available".into()); }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        for service in &self.native_services {
            super::apple_transport::service_instance(service)?;
        }
        #[cfg(not(any(target_os = "ios", target_os = "macos")))]
        if !self.native_services.is_empty() {
            return Err("Native Apple service endpoints are unavailable on this platform".into());
        }
        if let Some(route) = &self.route { route.check()?; }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LibraryBook {
    pub id: String, pub title: String, pub file_name: String, pub size: u64,
    pub archive_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LibraryInventory {
    pub version: u8, pub session_id: String, pub peer_id: String, pub name: String, pub books: Vec<LibraryBook>,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibrarySharingStatus {
    pub running: bool, pub session_id: Option<String>, pub publisher_name: Option<String>,
    pub library_count: usize, pub uploaded: u64, pub peer_count: usize, pub error: Option<String>,
}
struct Sources {
    store: Arc<Store>, books: BTreeMap<String, Book>, session_id: String, peer_id: String, name: String,
    uploaded: AtomicU64, peers: AtomicUsize, budget: Arc<RateBudget>, cancel: CancellationToken,
    /// Inventory rows for this immutable installation snapshot. `books` never
    /// changes after construction, so browse replies reuse the derived rows
    /// instead of re-deriving names and cloning metadata per request.
    inventory_books: Vec<LibraryBook>,
}
impl Sources {
    fn new(store: Arc<Store>, books: BTreeMap<String, Book>, session_id: String, peer_id: String, name: String,
        budget: Arc<RateBudget>, cancel: CancellationToken) -> Self {
        let inventory_books = books.values().map(|book| LibraryBook { id: book.metadata.id.clone(),
            title: book.metadata.title.clone(), file_name: crate::sharing::download_name(&book.metadata.name, &book.metadata.id),
            size: book.metadata.size, archive_id: Some(book.metadata.id.clone()) }).collect();
        Self { store, books, session_id, peer_id, name, uploaded: AtomicU64::new(0), peers: AtomicUsize::new(0),
            budget, cancel, inventory_books }
    }
    fn principal(&self, expected: &Book) -> Result<(), String> {
        if self.cancel.is_cancelled() || self.store.private_mode()? || self.store.read_only()? {
            return Err("Nearby sharing stopped".into());
        }
        let current = self.store.book(&expected.metadata.id)?;
        if !crate::sharing::same_owner(Some(expected), current.as_ref()) {
            return Err("Archive installation is no longer shared".into());
        }
        Ok(())
    }
    fn principals(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() { return Err("Nearby sharing stopped".into()); }
        if self.store.private_mode()? || self.store.read_only()? { return Err("Nearby sharing stopped".into()); }
        // Index the installed snapshot once per check, then compare only the
        // shared set: the membership monitor runs this for the lifetime of
        // every running share, so it must not scan the whole library per
        // shared book. The index is rebuilt from the current snapshot on every
        // call, never retained across mutations.
        let installed = self.store.books()?;
        let mut by_id: HashMap<&str, &Book> = HashMap::with_capacity(installed.len());
        for book in &installed { by_id.insert(book.metadata.id.as_str(), book); }
        for expected in self.books.values() {
            let current = by_id.get(expected.metadata.id.as_str()).ok_or("Archive installation is no longer shared")?;
            if !crate::sharing::same_owner(Some(expected), Some(*current)) {
                return Err("Archive installation is no longer shared".into());
            }
        }
        Ok(())
    }
    fn inventory(&self) -> Result<LibraryInventory, String> {
        self.principals()?;
        Ok(LibraryInventory { version: wire::VERSION, session_id: self.session_id.clone(), peer_id: self.peer_id.clone(),
            name: self.name.clone(), books: self.inventory_books.clone() })
    }
    fn reply(&self, status: u16) -> wire::Reply {
        wire::Reply { version: wire::VERSION, session_id: self.session_id.clone(), peer_id: self.peer_id.clone(), status,
            length: 0, total: 0, start: 0, etag: None, inventory: None, error: None }
    }
}
pub(crate) struct LibraryServer {
    pub descriptor: LibraryPeerDescriptor,
    pub(super) generation: String,
    sources: Arc<Sources>, listener: Option<transport::Listener>, monitor: Option<tokio::task::JoinHandle<()>>,
}
impl LibraryServer {
    #[cfg(test)]
    pub(crate) async fn start_default(store: Arc<Store>, peer_id: String, name: String, cancel: CancellationToken) -> Result<Self, String> {
        Self::start(store, peer_id, name, Arc::new(RateBudget::new(0)), cancel).await
    }
    pub(crate) async fn start(store: Arc<Store>, peer_id: String, name: String, budget: Arc<RateBudget>, parent: CancellationToken) -> Result<Self, String> {
        Self::start_inner(store, peer_id, name, budget, parent, None).await
    }
    pub(super) async fn start_on_socket(store: Arc<Store>, peer_id: String, name: String, budget: Arc<RateBudget>, parent: CancellationToken,
        socket: std::net::UdpSocket, addresses: Vec<String>) -> Result<Self, String> {
        Self::start_inner(store, peer_id, name, budget, parent, Some((socket, addresses))).await
    }
    async fn start_inner(store: Arc<Store>, peer_id: String, name: String, budget: Arc<RateBudget>, parent: CancellationToken,
        socket: Option<(std::net::UdpSocket, Vec<String>)>) -> Result<Self, String> {
        if name.is_empty() || !crate::publisher::valid_name(&name) { return Err("Invalid publisher name".into()); }
        let books: Vec<_> = store.read()?.books.into_iter().filter(|b| b.available).collect();
        if books.is_empty() { return Err("Add an available archive before sharing your library".into()); }
        if books.len() > wire::MAX_BOOKS { return Err("The library exceeds the Nearby inventory limit".into()); }
        if books.iter().any(|b| !wire::valid_id(&b.metadata.id)) { return Err("Invalid installed archive identifier".into()); }
        let cancel = parent.child_token();
        let sources = Arc::new(Sources::new(store, books.into_iter().map(|b| (b.metadata.id.clone(), b)).collect(),
            uuid::Uuid::new_v4().simple().to_string(), peer_id, name, budget, cancel.clone()));
        let inventory = sources.inventory()?;
        if serde_json::to_vec(&inventory).map_err(|e| e.to_string())?.len() > wire::INVENTORY_LIMIT - 2048 { return Err("The library inventory exceeds its transport bound".into()); }
        let (listener, monitor) = library_listener(sources.clone(), socket).await?;
        let descriptor = LibraryPeerDescriptor { session_id: sources.session_id.clone(), peer_id: sources.peer_id.clone(), name: sources.name.clone(),
            port: listener.port, addresses: listener.addresses.clone(), native_services: Vec::new(), direct: false, route: None,
            cancel: listener.cancel.clone(), authority: Default::default() };
        Ok(Self { descriptor, generation: uuid::Uuid::new_v4().to_string(), sources, listener: Some(listener), monitor: Some(monitor) })
    }
    pub(super) async fn rebind(&mut self, socket: std::net::UdpSocket, addresses: Vec<String>) -> Result<(), String> {
        if !self.valid_sources() { return Err("The shared library session retired".into()); }
        if let Some(monitor) = self.monitor.take() { monitor.abort(); let _ = monitor.await; }
        if let Some(listener) = self.listener.take() { listener.shutdown().await; }
        let (listener, monitor) = match library_listener(self.sources.clone(), Some((socket, addresses))).await {
            Ok(listener) => listener,
            Err(error) => { self.cancel(); return Err(error); }
        };
        self.generation = uuid::Uuid::new_v4().to_string();
        self.descriptor.port = listener.port;
        self.descriptor.addresses = listener.addresses.clone();
        self.descriptor.cancel = listener.cancel.clone();
        self.descriptor.authority = Default::default();
        self.listener = Some(listener); self.monitor = Some(monitor);
        Ok(())
    }
    pub(crate) fn contains(&self, book: Option<&str>) -> bool { book.is_none_or(|id| self.sources.books.contains_key(id)) }
    pub(super) fn native_listener(&self) -> bool { self.listener.as_ref().is_some_and(|listener| listener.native) }
    pub(crate) fn valid_sources(&self) -> bool { self.sources.principals().is_ok() }
    pub(crate) fn status(&self) -> LibrarySharingStatus {
        let running = !self.sources.cancel.is_cancelled() && self.listener.as_ref().is_some_and(|l| !l.is_finished());
        LibrarySharingStatus { running, session_id: Some(self.sources.session_id.clone()), publisher_name: Some(self.sources.name.clone()),
            library_count: self.sources.books.len(), uploaded: self.sources.uploaded.load(Ordering::Relaxed), peer_count: self.sources.peers.load(Ordering::Relaxed),
            error: (!running).then(|| "Nearby sharing stopped; share your library again".into()) }
    }
    pub(crate) fn cancel(&self) { self.sources.cancel.cancel(); }
    pub(crate) async fn shutdown(mut self) {
        self.cancel();
        if let Some(listener) = self.listener.take() { listener.shutdown().await; }
        if let Some(monitor) = self.monitor.take() { let _ = monitor.await; }
    }
}
impl Drop for LibraryServer { fn drop(&mut self) { self.cancel(); } }
async fn library_listener(sources: Arc<Sources>, socket: Option<(std::net::UdpSocket, Vec<String>)>)
    -> Result<(transport::Listener, tokio::task::JoinHandle<()>), String> {
    let serving = sources.clone();
    let listener = transport::listen(Arc::new(move |send, recv, cancel| {
        let sources = serving.clone();
        Box::pin(async move { let _ = serve(sources, send, recv, cancel).await; })
    }), sources.cancel.clone(), socket).await?;
    // Membership watching owns no source descriptors/provider leases.
    let listener_cancel = listener.cancel.clone();
    let monitor = tokio::spawn(async move {
        loop { tokio::select! {
            _ = sources.cancel.cancelled() => break,
            _ = listener_cancel.cancelled() => { sources.cancel.cancel(); break; },
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                if sources.principals().is_err() { sources.cancel.cancel(); break; }
            }
        } }
        listener_cancel.cancel();
    });
    Ok((listener, monitor))
}
struct PeerCount(Arc<Sources>);
impl Drop for PeerCount { fn drop(&mut self) { self.0.peers.fetch_sub(1, Ordering::Relaxed); } }
async fn ack(recv: &mut RecvStream, cancel: &CancellationToken) -> Result<(), String> {
    // A sender can fill Quinn's receive window long before a rate-limited
    // consumer finishes reading it, so the ACK wait has no *progress* deadline.
    // It still needs an absolute one: a peer that keeps the connection alive
    // without ever sending the completion byte would otherwise pin this serve
    // task, its admission slot and its source ownership forever.
    let mut byte = [0];
    let deadline = tokio::time::Instant::now() + ACK_TIMEOUT;
    tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("LocalSwarm operation cancelled".into()),
        _ = tokio::time::sleep_until(deadline) => return Err("Library completion acknowledgement timed out".into()),
        result = recv.read_exact(&mut byte) => result?,
    }
    if byte != [1] { return Err("Invalid library completion acknowledgement".into()); } Ok(())
}
async fn serve(sources: Arc<Sources>, mut send: SendStream, mut recv: RecvStream, cancel: CancellationToken) -> Result<(), String> {
    sources.peers.fetch_add(1, Ordering::Relaxed); let _peer = PeerCount(sources.clone());
    let request: wire::Request = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        wire::receive(&mut recv, wire::CONTROL_LIMIT, &cancel),
    ).await.map_err(|_| "Library request setup timed out".to_string())??;
    request.validate()?;
    if request.session_id != sources.session_id || request.peer_id != sources.peer_id { return Err("Library session changed".into()); }
    let Some(book_id) = request.book_id else {
        let mut reply = sources.reply(200); reply.inventory = Some(sources.inventory()?);
        wire::send(&mut send, &reply, wire::INVENTORY_LIMIT, &cancel).await?;
        return ack(&mut recv, &cancel).await;
    };
    let Some(book) = sources.books.get(&book_id).cloned() else {
        wire::send(&mut send, &sources.reply(404), wire::CONTROL_LIMIT, &cancel).await?; return ack(&mut recv, &cancel).await;
    };
    let checked = sources.clone(); let owner = book.clone();
    let opened = tokio::task::spawn_blocking(move || {
        checked.principal(&owner)?;
        let opened = crate::sharing::open_library_download(owner.clone())?;
        checked.principal(&owner)?; Ok::<_, String>(opened)
    }).await.map_err(|e| e.to_string())?;
    if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
    let (mut file, size, _) = match opened {
        Ok(opened) => opened,
        Err(_) => { let mut reply = sources.reply(410); reply.error = Some("Shared archive is unavailable or changed".into());
            wire::send(&mut send, &reply, wire::CONTROL_LIMIT, &cancel).await?; return ack(&mut recv, &cancel).await; }
    };
    let etag = if file.has_strong_validator() { Some(source_etag(&book, &file)?) } else { None };
    let range = match request.range.as_deref() {
        Some(value) if etag.as_deref().is_some_and(|etag| request.if_range.as_deref().is_none_or(|validator| validator == etag)) => match wire::range(value, size) {
            Ok(range) => Some(range), Err(_) => { let mut reply = sources.reply(416); reply.total = size; reply.etag = etag.clone();
                drop(file);
                wire::send(&mut send, &reply, wire::CONTROL_LIMIT, &cancel).await?; return ack(&mut recv, &cancel).await; }
        }, _ => None,
    };
    let (mut offset, mut left) = range.unwrap_or((0, size));
    let mut reply = sources.reply(if range.is_some() { 206 } else { 200 });
    reply.length = left; reply.total = size; reply.start = offset; reply.etag = etag;
    wire::send(&mut send, &reply, wire::CONTROL_LIMIT, &cancel).await?;
    while left > 0 {
        let checked = sources.clone(); let owner = book.clone(); let count = left.min(wire::PIECE as u64);
        let read = tokio::task::spawn_blocking(move || {
            checked.principal(&owner)?;
            let bytes = file.read(offset, count).map_err(|e| e.to_string())?;
            checked.principal(&owner)?; Ok::<_, String>((file, bytes))
        }).await.map_err(|e| e.to_string())?;
        if cancel.is_cancelled() { return Err("LocalSwarm operation cancelled".into()); }
        let (returned, bytes) = match read {
            Ok(read) => read,
            Err(_) => { network::guarded(&cancel, send.write_all(&u32::MAX.to_be_bytes())).await?; return ack(&mut recv, &cancel).await; }
        };
        file = returned;
        let mut position = 0;
        network::guarded(&cancel, send.write_all(&(bytes.len() as u32).to_be_bytes())).await?;
        while position < bytes.len() {
            let allowed = sources.budget.reserve(bytes.len() - position, &cancel).await?;
            network::guarded(&cancel, send.write_all(&bytes[position..position + allowed])).await?;
            sources.uploaded.fetch_add(allowed as u64, Ordering::Relaxed); position += allowed;
        }
        offset += count; left -= count;
    }
    drop(file);
    network::guarded(&cancel, send.write_all(&0u32.to_be_bytes())).await?;
    ack(&mut recv, &cancel).await
}
fn source_etag(book: &Book, file: &crate::sharing::download::Download) -> Result<String, String> {
    // Only exact retained metadata and installation/provider authority are hashed.
    let bytes = serde_json::to_vec(&(&book.installation_id, &book.source_reference, file.identity())).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new(); hash.update(bytes);
    for name in file.source_names() { hash.update((name.len() as u64).to_be_bytes()); hash.update(name.as_bytes()); }
    Ok(format!("\"kiwix-source-{}\"", hex::encode(hash.finalize())))
}
fn validate_reply(peer: &LibraryPeerDescriptor, reply: &wire::Reply) -> Result<(), String> {
    if reply.version != wire::VERSION || reply.session_id != peer.session_id || reply.peer_id != peer.peer_id {
        return Err("The selected Nearby library session changed".into());
    }
    if !matches!(reply.status, 200 | 206 | 404 | 410 | 416) || reply.error.as_ref().is_some_and(|e| e.len() > 1024)
        || reply.etag.as_ref().is_some_and(|v| v.len() > 256 || !v.starts_with('"') || !v.ends_with('"') || v.chars().any(char::is_control))
        || (reply.status == 206 && (reply.length == 0 || reply.start.checked_add(reply.length).is_none_or(|end| end > reply.total)))
        || (reply.status == 200 && reply.inventory.is_none() && (reply.start != 0 || reply.length != reply.total))
        || (reply.inventory.is_some() && (reply.status != 200 || reply.length != 0 || reply.total != 0 || reply.start != 0))
        || (reply.status >= 400 && reply.length != 0)
    { return Err("Invalid library response metadata".into()); }
    Ok(())
}
async fn request(peer: &LibraryPeerDescriptor, book: Option<&str>, headers: HeaderMap, cancel: CancellationToken) -> Result<(transport::Transport, wire::Reply), String> {
    let mut stream = transport::connect(peer, &cancel).await?;
    let header = |key: &str| -> Result<Option<String>, String> { headers.get(key).map(|v| v.to_str().map(str::to_string).map_err(|_| "Invalid library request header".into())).transpose() };
    let request = wire::Request { version: wire::VERSION, session_id: peer.session_id.clone(), peer_id: peer.peer_id.clone(), book_id: book.map(str::to_string), range: header("range")?, if_range: header("if-range")? };
    request.validate()?; wire::send(&mut stream.send, &request, wire::CONTROL_LIMIT, &cancel).await?;
    let reply = wire::receive(&mut stream.recv, if book.is_some() { wire::CONTROL_LIMIT } else { wire::INVENTORY_LIMIT }, &cancel).await?;
    validate_reply(peer, &reply)?;
    if reply.status == 206 {
        let expected = request.range.as_deref().ok_or("Unsolicited library range response")?;
        if wire::range(expected, reply.total)? != (reply.start, reply.length)
            || request.if_range.as_ref().is_some_and(|expected| reply.etag.as_ref() != Some(expected)) {
            return Err("Library range response does not match the requested validator and bounds".into());
        }
    }
    Ok((stream, reply))
}
pub(crate) async fn browse(peer: &LibraryPeerDescriptor, cancel: CancellationToken) -> Result<LibraryInventory, String> {
    let (mut stream, reply) = request(peer, None, HeaderMap::new(), cancel.clone()).await?;
    if reply.status != 200 { return Err("Nearby library is unavailable".into()); }
    let inventory = reply.inventory.ok_or("Library inventory is missing")?;
    if inventory.version != wire::VERSION || inventory.session_id != peer.session_id || inventory.peer_id != peer.peer_id
        || inventory.name != peer.name || inventory.books.len() > wire::MAX_BOOKS { return Err("Invalid library inventory identity".into()); }
    let mut ids = HashSet::new();
    for book in &inventory.books {
        if !wire::valid_id(&book.id) || !ids.insert(&book.id) || book.title.len() > 4096 || book.title.chars().any(char::is_control)
            || book.size == 0 || crate::acquisition::shared_file_name(&book.file_name).is_err()
            || book.archive_id.as_ref().is_some_and(|id| !wire::valid_id(id)) { return Err("Invalid library inventory book".into()); }
    }
    stream.complete(&cancel).await?;
    Ok(inventory)
}
pub(crate) struct LibraryResponse {
    stream: transport::Transport, status: StatusCode, headers: HeaderMap, left: u64,
    cancel: CancellationToken, ended: bool,
    poisoned: bool,
}
struct PendingRead { cancel: CancellationToken, armed: bool }
impl Drop for PendingRead { fn drop(&mut self) { if self.armed { self.cancel.cancel(); } } }
impl LibraryResponse {
    pub(crate) fn status(&self) -> StatusCode { self.status }
    pub(crate) fn headers(&self) -> &HeaderMap { &self.headers }
    pub(crate) async fn chunk(&mut self) -> Result<Option<Bytes>, String> {
        if self.poisoned { return Err("Library response was interrupted".into()); }
        if self.ended { return Ok(None); }
        self.poisoned = true;
        let mut pending = PendingRead { cancel: self.cancel.clone(), armed: true };
        let result = self.read_chunk().await;
        if result.is_ok() { self.poisoned = false; pending.armed = false; }
        result
    }
    async fn read_chunk(&mut self) -> Result<Option<Bytes>, String> {
        if self.left == 0 { self.finish_body().await?; return Ok(None); }
        let mut frame = [0; 4]; network::guarded(&self.cancel, self.stream.recv.read_exact(&mut frame)).await?;
        let count = u32::from_be_bytes(frame);
        if count == u32::MAX { return Err("Shared archive changed or is no longer available".into()); }
        if count == 0 { return Err("Truncated library range".into()); }
        if count as usize > wire::PIECE || count as u64 > self.left { return Err("Invalid library data frame length".into()); }
        let mut bytes = vec![0; count as usize];
        let mut position = 0;
        while position < bytes.len() {
            let received = network::guarded(&self.cancel, self.stream.recv.read(&mut bytes[position..])).await?
                .filter(|count| *count != 0).ok_or("Truncated library range payload")?;
            position += received;
        }
        self.left -= count as u64;
        // Segmented consumers stop at Content-Length and never poll for EOF.
        // Complete framing/ACK before handing them the final payload bytes.
        if self.left == 0 { self.finish_body().await?; }
        Ok(Some(Bytes::from(bytes)))
    }
    async fn finish_body(&mut self) -> Result<(), String> {
        let mut frame = [0; 4]; network::guarded(&self.cancel, self.stream.recv.read_exact(&mut frame)).await?;
        match u32::from_be_bytes(frame) {
            0 => {},
            u32::MAX => return Err("Shared archive changed or is no longer available".into()),
            _ => return Err("Trailing library range payload".into()),
        }
        self.stream.complete(&self.cancel).await?;
        self.ended = true;
        Ok(())
    }
}
impl Drop for LibraryResponse { fn drop(&mut self) { self.cancel.cancel(); } }
pub(crate) async fn open(peer: &LibraryPeerDescriptor, book: &str, headers: HeaderMap, parent: CancellationToken) -> Result<LibraryResponse, String> {
    let cancel = parent.child_token();
    let (mut stream, reply) = request(peer, Some(book), headers, cancel.clone()).await?;
    if reply.inventory.is_some() || (matches!(reply.status, 206 | 416) && reply.etag.is_none()) { return Err("Invalid library range response".into()); }
    let status = StatusCode::from_u16(reply.status).map_err(|e| e.to_string())?;
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_str(&reply.length.to_string()).map_err(|e| e.to_string())?);
    if reply.etag.is_some() { headers.insert("accept-ranges", HeaderValue::from_static("bytes")); }
    headers.insert("content-type", HeaderValue::from_static("application/x-zim"));
    if let Some(etag) = reply.etag { headers.insert("etag", HeaderValue::from_str(&etag).map_err(|e| e.to_string())?); }
    if status == StatusCode::PARTIAL_CONTENT { headers.insert("content-range", HeaderValue::from_str(&format!("bytes {}-{}/{}", reply.start, reply.start + reply.length - 1, reply.total)).map_err(|e| e.to_string())?); }
    if status == StatusCode::RANGE_NOT_SATISFIABLE { headers.insert("content-range", HeaderValue::from_str(&format!("bytes */{}", reply.total)).map_err(|e| e.to_string())?); }
    let ended = status.as_u16() >= 400;
    if ended { stream.complete(&cancel).await?; }
    Ok(LibraryResponse { stream, status, headers, left: reply.length, cancel, ended, poisoned: false })
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;

#[cfg(all(test, any(target_os = "macos", target_os = "ios")))]
pub(super) async fn test_serve_native_request(
    store: Arc<Store>, peer_id: String, session_id: String, name: String,
    send: SendStream, recv: RecvStream, cancel: CancellationToken,
) -> Result<(), String> {
    let books = store.read()?.books.into_iter().filter(|book| book.available)
        .map(|book| (book.metadata.id.clone(), book)).collect();
    let sources = Arc::new(Sources::new(store, books, session_id, peer_id, name,
        Arc::new(RateBudget::new(0)), cancel.clone()));
    serve(sources, send, recv, cancel).await
}
