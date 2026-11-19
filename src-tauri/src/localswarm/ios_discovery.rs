//! Apple DNS-SD uses mDNSResponder, including routerless peer-to-peer interfaces.
//! QUIC libraries and ordinary swarms use system-host records, including AWDL.
//!
//! Native references and poll sets belong to one worker. The callback cookie is an integer.
//! Callbacks only copy bounded events; they never mutate worker state, acquire
//! snapshot locks, deallocate references, or call back into DNS-SD. Callbacks
//! run inside the operation that creates or processes the native reference.
//! One address registration can own multiple A records on the same reference.
use super::{protocol, truncate_utf8, library_addresses, library_instance, valid_library_session_id, Manifest, Peer, MAX_ADVERTISEMENTS, MAX_PEERS, MAX_TXT, SERVICE};
use std::{cell::RefCell, collections::{BTreeMap, VecDeque}, ffi::{c_char, c_void, CStr, CString},
    ptr, rc::Rc, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU8, Ordering}},
    thread, time::{Duration, Instant}};
use tokio::sync::{mpsc, oneshot};

const REGTYPE: &[u8] = b"_localswarm._tcp\0";
const DOMAIN: &[u8] = b"local.\0";
const ADD: u32 = 2;
const INCLUDE_PEERS: u32 = 0x20000 | 0x100000; // IncludeP2P | IncludeAWDL.
const MAX_SOURCES: usize = MAX_PEERS * 2;
const MAX_EVENTS: usize = 1024;
const DNS_SERVICE_NAME_CAPACITY: usize = 1009; // kDNSServiceMaxDomainName
const BROWSE_CALLBACK_NAME_CAPACITY: usize = 256;
const MAX_BROWSE_ID_BYTES: usize = 255;
/// An unresolved record is expired after this long. A Watch event is the only
/// other trigger for a resolve, so a transient timeout needs a bounded retry.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESOLVE_RETRIES: u8 = 3;
type Ref = *mut c_void;
type BrowseCallback = unsafe extern "C" fn(Ref, u32, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type RegisterCallback = unsafe extern "C" fn(Ref, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type ResolveCallback = unsafe extern "C" fn(Ref, u32, u32, i32, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);
type QueryCallback = unsafe extern "C" fn(Ref, u32, u32, i32, *const c_char, u16, u16, u16, *const c_void, u32, *mut c_void);
type RecordCallback = unsafe extern "C" fn(Ref, Ref, u32, i32, *mut c_void);

#[link(name = "System")]
unsafe extern "C" {
    fn DNSServiceBrowse(sd: *mut Ref, flags: u32, interface: u32, ty: *const c_char, domain: *const c_char, callback: BrowseCallback, context: *mut c_void) -> i32;
    fn DNSServiceRegister(sd: *mut Ref, flags: u32, interface: u32, name: *const c_char, ty: *const c_char, domain: *const c_char, host: *const c_char, port: u16, txt_len: u16, txt: *const c_void, callback: RegisterCallback, context: *mut c_void) -> i32;
    fn DNSServiceResolve(sd: *mut Ref, flags: u32, interface: u32, name: *const c_char, ty: *const c_char, domain: *const c_char, callback: ResolveCallback, context: *mut c_void) -> i32;
    fn DNSServiceQueryRecord(sd: *mut Ref, flags: u32, interface: u32, name: *const c_char, ty: u16, class: u16, callback: QueryCallback, context: *mut c_void) -> i32;
    fn DNSServiceCreateConnection(sd: *mut Ref) -> i32;
    fn DNSServiceRegisterRecord(sd: Ref, record: *mut Ref, flags: u32, interface: u32, name: *const c_char, ty: u16, class: u16, len: u16, data: *const c_void, ttl: u32, callback: RecordCallback, context: *mut c_void) -> i32;
    fn DNSServiceConstructFullName(full: *mut c_char, name: *const c_char, ty: *const c_char, domain: *const c_char) -> i32;
    fn DNSServiceRefSockFD(sd: Ref) -> i32;
    fn DNSServiceProcessResult(sd: Ref) -> i32;
    fn DNSServiceRefDeallocate(sd: Ref);
}

#[derive(Default)]
struct Snapshot { peers: BTreeMap<String, Peer>, error: Option<String> }
struct Worker {
    commands: mpsc::Sender<Command>, stop: Arc<AtomicBool>, browsing: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.try_send(Command::Wake);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

pub struct Discovery { peer_id: String, snapshot: Arc<Mutex<Snapshot>>, worker: Option<Worker>, direct: bool }
impl Discovery {
    pub fn new(peer_id: String) -> Result<Self, String> {
        if !protocol::valid_peer_id(&peer_id) { return Err("Invalid LocalSwarm peer ID".into()); }
        Ok(Self { peer_id, snapshot: Arc::new(Mutex::new(Snapshot::default())), worker: None, direct: false })
    }
    /// Apple-only Nearby preference: publish the peer-to-peer (AWDL/llw)
    /// resolved source when one exists. Off keeps the historical first-source
    /// choice exactly.
    pub async fn set_direct(&mut self, direct: bool) -> Result<(), String> {
        self.direct = direct;
        if let Some(worker) = &self.worker {
            if worker.thread.as_ref().is_some_and(|thread| thread.is_finished()) { return Ok(()); }
            let (tx, rx) = oneshot::channel();
            let _ = worker.commands.send(Command::Direct(direct, tx)).await;
            let _ = rx.await;
        }
        Ok(())
    }
    fn worker(&mut self) -> Result<&Worker, String> {
        if self.worker.is_none() {
            let (commands, receiver) = mpsc::channel(32);
            let stop = Arc::new(AtomicBool::new(false)); let browsing = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone(); let thread_browsing = browsing.clone();
            let snapshot = self.snapshot.clone(); let peer_id = self.peer_id.clone(); let direct = self.direct;
            let thread = thread::Builder::new().name("localswarm-bonjour".into()).spawn(move || {
                Engine::new(peer_id, snapshot, thread_browsing, direct).run(receiver, thread_stop);
            }).map_err(|error| format!("Cannot start Bonjour discovery: {error}"))?;
            self.worker = Some(Worker { commands, stop, browsing, thread: Some(thread) });
        }
        self.worker.as_ref().ok_or_else(|| "LocalSwarm discovery is stopped".into())
    }
    pub fn is_browsing(&self) -> bool {
        self.worker.as_ref().is_some_and(|worker| !worker.thread.as_ref().is_some_and(|thread| thread.is_finished()) && worker.browsing.load(Ordering::Acquire))
    }
    pub fn error(&self) -> Option<String> { self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).error.clone() }
    pub fn peers(&self) -> Vec<Peer> { self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).peers.values().cloned().collect() }
    pub fn library_peer(&self, id: &str) -> Result<Peer, String> {
        if !self.is_browsing() { return Err("Find nearby before browsing a shared library".into()); }
        let peer = self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).peers.get(id).cloned()
            .ok_or("The shared library is no longer nearby; find nearby and browse again")?;
        super::library_peer(peer)
    }
    pub fn selected_library_peer(&self, id: &str) -> Result<Option<Peer>, String> {
        self.snapshot.lock().unwrap_or_else(|error| error.into_inner()).peers.get(id).cloned()
            .map(super::library_peer).transpose()
    }
    pub async fn start_browse(&mut self) -> Result<(), String> {
        let admission = AdmissionGuard::new();
        let (tx, rx) = oneshot::channel();
        self.worker()?.commands.send(Command::Browse(tx, admission.0.clone())).await.map_err(|_| "Bonjour discovery worker stopped")?;
        let result = rx.await.map_err(|_| "Bonjour discovery worker stopped")?;
        admission.accept();
        result
    }
    pub async fn stop_browse(&mut self) -> Result<(), String> {
        if self.worker.as_ref().is_some_and(|worker| worker.thread.as_ref().is_some_and(|thread| thread.is_finished())) {
            self.worker.take();
            self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).peers.clear();
            return Ok(());
        }
        if let Some(worker) = &self.worker {
            let (tx, rx) = tokio::sync::oneshot::channel();
            worker.commands.send(Command::StopBrowse(tx)).await.map_err(|_| "Bonjour discovery worker stopped")?;
            rx.await.map_err(|_| "Bonjour discovery worker stopped")??;
        }
        Ok(())
    }
    pub async fn advertise_addresses(&mut self, manifest: &Manifest, port: u16, locked: bool, _addresses: &[String], publisher_name: &str) -> Result<String, String> {
        manifest.validate()?;
        if publisher_name.is_empty() || !crate::publisher::valid_name(publisher_name) { return Err("Invalid publisher name".into()); }
        if port == 0 { return Err("LocalSwarm advertisement limit or invalid port".into()); }
        // Network.framework listens on infrastructure and peer-to-peer interfaces.
        // Let mDNSResponder publish scoped system-host records, including AWDL.
        let name = format!("Kiwix-{}", uuid::Uuid::new_v4().simple());
        let mut txt = Vec::new();
        append_txt(&mut txt, "device", publisher_name)?;
        for (key, value) in [("sid", manifest.swarm_id.clone()),
            ("name", if locked { "Protected share".into() } else { truncate_utf8(&manifest.name, 240) }),
            ("bytes", manifest.total_bytes.to_string()), ("chunks", manifest.chunk_hashes.len().to_string()),
            ("pid", self.peer_id.clone()), ("tp", "quic".into()), ("pv", "1".into()), ("awdl", "1".into())] {
            append_txt(&mut txt, key, &value)?;
        }
        if locked { append_txt(&mut txt, "lk", "1")?; }
        let admission = AdmissionGuard::new();
        let (tx, rx) = oneshot::channel();
        self.worker()?.commands.send(Command::Advertise { name, port, txt, addresses: None, reply: tx, admission: admission.0.clone() }).await.map_err(|_| "Bonjour discovery worker stopped")?;
        let result = rx.await.map_err(|_| "Bonjour discovery worker stopped")?;
        admission.accept();
        result
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn advertise_library(&mut self, session_id: &str, port: u16, count: usize, addresses: &[String], publisher_name: &str, generation: u64, native: bool) -> Result<String, String> {
        if !valid_library_session_id(session_id) || port == 0 || count == 0 || count > 10_000 || generation == 0 { return Err("Invalid library session, count or listener port".into()); }
        if publisher_name.is_empty() || !crate::publisher::valid_name(publisher_name) { return Err("Invalid publisher name".into()); }
        let count = count.to_string();
        let generation = generation.to_string();
        let addresses = if native { None } else { Some(library_addresses(addresses)?) };
        // A fresh session must be a fresh DNS-SD record; a restarted sender
        // otherwise leaves receivers holding the old session TXT until TTL.
        let name = library_instance(&self.peer_id, session_id);
        let mut txt = Vec::new();
        append_txt(&mut txt, "device", publisher_name)?;
        for (key, value) in [("sid", session_id), ("pid", self.peer_id.as_str()),
            ("name", "Shared library"), ("count", count.as_str()), ("gen", generation.as_str()),
            ("bytes", "0"), ("chunks", "0"), ("tp", "quic-library"), ("pv", "1")] {
            append_txt(&mut txt, key, value)?;
        }
        if native { append_txt(&mut txt, "awdl", "1")?; }
        let admission = AdmissionGuard::new();
        let (tx, rx) = oneshot::channel();
        self.worker()?.commands.send(Command::Advertise { name, port, txt, addresses, reply: tx, admission: admission.0.clone() }).await.map_err(|_| "Bonjour discovery worker stopped")?;
        let result = rx.await.map_err(|_| "Bonjour discovery worker stopped")?;
        admission.accept();
        result
    }
    pub async fn unadvertise(&mut self, id: &str) -> Result<(), String> {
        if let Some(worker) = &self.worker {
            let (tx, rx) = tokio::sync::oneshot::channel();
            worker.commands.send(Command::Withdraw(id.to_string(), tx)).await.map_err(|_| "Bonjour discovery worker stopped")?;
            rx.await.map_err(|_| "Bonjour discovery worker stopped")??;
        }
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some(worker) = self.worker.take() {
            worker.stop.store(true, Ordering::Release);
            // A cancelled shutdown must not let the retiring worker clear the
            // snapshot of a newly started discovery generation.
            self.snapshot = Arc::new(Mutex::new(Snapshot { peers: BTreeMap::new(), error: self.error() }));
            // The blocking join remains owned even if this future is cancelled.
            tokio::task::spawn_blocking(move || drop(worker)).await.map_err(|e| format!("Joining Bonjour shutdown: {e}"))?;
        }
        Ok(())
    }
}
type Reply<T> = oneshot::Sender<Result<T, String>>;
const ADMISSION_PENDING: u8 = 0;
const ADMISSION_ACCEPTED: u8 = 1;
const ADMISSION_CANCELLED: u8 = 2;
/// A successful channel send is not ownership transfer: the receiving task
/// can be dropped with an unread result still buffered in its oneshot.
struct AdmissionGuard(Arc<AtomicU8>);
impl AdmissionGuard {
    fn new() -> Self { Self(Arc::new(AtomicU8::new(ADMISSION_PENDING))) }
    fn accept(self) { self.0.store(ADMISSION_ACCEPTED, Ordering::Release); }
}
impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        let _ = self.0.compare_exchange(ADMISSION_PENDING, ADMISSION_CANCELLED, Ordering::AcqRel, Ordering::Acquire);
    }
}
enum Rollback { Browse(u64), Advertisement(String) }
struct PendingAdmission { state: Arc<AtomicU8>, rollback: Rollback }
enum Command {
    Wake,
    Direct(bool, Reply<()>),
    Browse(Reply<()>, Arc<AtomicU8>), StopBrowse(Reply<()>),
    Advertise { name: String, port: u16, txt: Vec<u8>, addresses: Option<Vec<std::net::Ipv4Addr>>, reply: Reply<String>, admission: Arc<AtomicU8> }, Withdraw(String, Reply<()>),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourceKey { id: String, interface: u32 }
struct Source {
    name: CString, ty: CString, domain: CString,
    watches: [u64; 2], present: [bool; 2], resolve: Option<(u64, Instant)>, retries: u8,
    peer: Option<Peer>,
}
#[derive(Clone)]
enum Kind { Browse, Advertisement(String), Watch(SourceKey, usize), Resolve(SourceKey) }
struct Context { tag: u64, queue: Rc<RefCell<Queue>> }
thread_local! {
    static ACTIVE_CALLBACK: RefCell<Option<Context>> = const { RefCell::new(None) };
}

// No borrow of the scope survives the call into DNS-SD. Restore the previous
// operation if a nested call returns or Rust unwinds outside a native callback.
fn with_callbacks<T>(tag: u64, queue: &Rc<RefCell<Queue>>, call: impl FnOnce() -> T) -> T {
    struct Restore<'a>(&'a RefCell<Option<Context>>, Option<Context>);
    impl Drop for Restore<'_> {
        fn drop(&mut self) { self.0.replace(self.1.take()); }
    }
    ACTIVE_CALLBACK.with(|active| {
        let previous = active.replace(Some(Context { tag, queue: Rc::clone(queue) }));
        let _restore = Restore(active, previous);
        call()
    })
}

fn allocate_cookie(counter: &Mutex<usize>) -> Result<usize, String> {
    let mut next = counter.lock().map_err(|_| "Bonjour cookie allocator unavailable")?;
    let cookie = *next;
    *next = cookie.checked_add(1).ok_or("Bonjour operation limit reached")?;
    Ok(cookie)
}
struct Operation {
    raw: Ref,
    kind: Kind,
}
impl Drop for Operation {
    fn drop(&mut self) {
        // Only the worker calls ProcessResult, and it has returned before any
        // operation is removed. Deallocate synchronously stops future callbacks.
        unsafe { DNSServiceRefDeallocate(self.raw); }
    }
}
struct Queue { events: VecDeque<(u64, Event)>, window: Instant, count: usize, overflow: bool }
enum Event {
    Error(i32), Browse { interface: u32, add: bool, name: String, ty: String, domain: String },
    Registered(bool), Watch { interface: u32, add: bool },
    Resolved { interface: u32, full: String, host: String, port: u16, txt: Vec<u8> },
    Invalid,
}
struct Engine {
    self_id: String, snapshot: Arc<Mutex<Snapshot>>, browsing: Arc<AtomicBool>, direct: bool,
    queue: Rc<RefCell<Queue>>, operations: BTreeMap<u64, Operation>,
    browse: Option<u64>, failed: bool, sources: BTreeMap<SourceKey, Source>, advertisements: BTreeMap<String, Vec<u64>>,
    pending_admissions: Vec<PendingAdmission>,
}
impl Engine {
    fn new(self_id: String, snapshot: Arc<Mutex<Snapshot>>, browsing: Arc<AtomicBool>, direct: bool) -> Self {
        Self { self_id, snapshot, browsing, direct, queue: Rc::new(RefCell::new(Queue {
            events: VecDeque::new(), window: Instant::now(), count: 0, overflow: false })),
            operations: BTreeMap::new(), browse: None, failed: false, sources: BTreeMap::new(), advertisements: BTreeMap::new(), pending_admissions: Vec::new() }
    }
    fn operation(&mut self, kind: Kind, create: impl FnOnce(*mut Ref, *mut c_void) -> i32) -> Result<u64, String> {
        static NEXT_COOKIE: Mutex<usize> = Mutex::new(1);
        let cookie = allocate_cookie(&NEXT_COOKIE)?;
        let tag = cookie as u64;
        let mut raw = ptr::null_mut();
        let code = with_callbacks(tag, &self.queue, || create(&mut raw, ptr::without_provenance_mut(cookie)));
        if code != 0 { return Err(native_error(code)); }
        if raw.is_null() { return Err("Bonjour returned no operation".into()); }
        self.operations.insert(tag, Operation { raw, kind });
        Ok(tag)
    }
    fn error(&self, error: String) { self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).error = Some(error); }
    fn stop_browse(&mut self) {
        if let Some(tag) = self.browse.take() { self.operations.remove(&tag); }
        let sources = std::mem::take(&mut self.sources);
        for (_, source) in sources { self.retire(source); }
        self.browsing.store(false, Ordering::Release);
        self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).peers.clear();
    }
    fn retire(&mut self, source: Source) {
        for tag in source.watches { self.operations.remove(&tag); }
        if let Some((tag, _)) = source.resolve { self.operations.remove(&tag); }
    }
    fn retire_admissions(&mut self) {
        let mut index = 0;
        while index < self.pending_admissions.len() {
            match self.pending_admissions[index].state.load(Ordering::Acquire) {
                ADMISSION_PENDING => index += 1,
                ADMISSION_ACCEPTED => { self.pending_admissions.swap_remove(index); }
                _ => {
                    let pending = self.pending_admissions.swap_remove(index);
                    match pending.rollback {
                        Rollback::Browse(tag) if self.browse == Some(tag) => self.stop_browse(),
                        Rollback::Advertisement(id) => self.withdraw(&id),
                        Rollback::Browse(_) => (),
                    }
                }
            }
        }
    }
    fn command(&mut self, command: Command) {
        self.retire_admissions();
        match command {
            Command::Wake => (),
            Command::Direct(direct, reply) => {
                self.direct = direct;
                let ids: Vec<String> = self.sources.keys().map(|key| key.id.clone()).collect();
                for id in ids { self.publish(&id); }
                let _ = reply.send(Ok(()));
            }
            Command::Browse(reply, admission) => {
                if reply.is_closed() { return; }
                let started = self.browse.is_none();
                let result = if self.failed { Err("Stop the failed discovery session before restarting".into()) }
                    else if self.browse.is_some() { Ok(()) } else {
                    self.operation(Kind::Browse, |raw, context| unsafe {
                        DNSServiceBrowse(raw, INCLUDE_PEERS, 0, REGTYPE.as_ptr().cast(), DOMAIN.as_ptr().cast(), browse_callback, context)
                    }).map(|tag| {
                        self.browse = Some(tag); self.browsing.store(true, Ordering::Release);
                        self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).error = None;
                    })
                };
                if result.is_ok() && started {
                    if let Some(tag) = self.browse {
                        self.pending_admissions.push(PendingAdmission { state: admission, rollback: Rollback::Browse(tag) });
                    }
                }
                let _ = reply.send(result);
            }
            Command::StopBrowse(reply) => { self.stop_browse(); self.failed = false; let _ = reply.send(Ok(())); }
            Command::Advertise { name, port, txt, addresses, reply, admission } => {
                if reply.is_closed() { return; }
                let result = self.advertise(name, port, txt, addresses);
                if let Ok(id) = &result {
                    self.pending_admissions.push(PendingAdmission { state: admission, rollback: Rollback::Advertisement(id.clone()) });
                }
                let _ = reply.send(result);
            }
            Command::Withdraw(id, reply) => {
                self.withdraw(&id);
                let _ = reply.send(Ok(()));
            }
        }
    }
    fn withdraw(&mut self, id: &str) {
        if let Some(tags) = self.advertisements.remove(id) {
            for tag in tags { self.operations.remove(&tag); }
        }
    }
    fn advertise(&mut self, name: String, port: u16, txt: Vec<u8>, addresses: Option<Vec<std::net::Ipv4Addr>>) -> Result<String, String> {
        if self.advertisements.len() >= MAX_ADVERTISEMENTS { return Err("LocalSwarm advertisement limit or invalid port".into()); }
        let fullname = format!("{name}.{SERVICE}");
        if self.advertisements.keys().any(|id| id.strip_prefix(&fullname).is_some_and(|suffix| suffix.starts_with('#'))) {
            return Err("The previous announcement is still stopping; stop sharing before starting it again".into());
        }
        // A delayed withdrawal/cancel callback must not retire a newer session
        // that happens to publish the same stable DNS-SD instance.
        let id = format!("{fullname}#{}", uuid::Uuid::new_v4().simple());
        let instance = CString::new(name).map_err(|_| "Invalid Bonjour instance")?;
        let hostname = addresses.as_ref().map(|_| CString::new(format!("kiwix-{}.local.", uuid::Uuid::new_v4().simple())))
            .transpose().map_err(|_| "Invalid library hostname")?;
        let mut tags = Vec::new();
        if let Some(addresses) = &addresses {
            let tag = self.operation(Kind::Advertisement(id.clone()), |raw, context| unsafe {
                let code = DNSServiceCreateConnection(raw);
                if code != 0 { return code; }
                for address in addresses {
                    let mut record = ptr::null_mut();
                    let octets = address.octets();
                    let code = DNSServiceRegisterRecord(*raw, &mut record, 0x10, 0, hostname.as_ref().unwrap().as_ptr(), 1, 1, 4,
                        octets.as_ptr().cast(), 120, record_callback, context);
                    if code != 0 { DNSServiceRefDeallocate(*raw); *raw = ptr::null_mut(); return code; }
                }
                0
            })?;
            tags.push(tag);
        }
        let service = self.operation(Kind::Advertisement(id.clone()), |raw, context| unsafe {
            DNSServiceRegister(raw, 8 | if addresses.is_some() { 0 } else { INCLUDE_PEERS }, 0,
                instance.as_ptr(), REGTYPE.as_ptr().cast(), DOMAIN.as_ptr().cast(),
                hostname.as_ref().map_or(ptr::null(), |host| host.as_ptr()),
                port.to_be(), txt.len() as u16, txt.as_ptr().cast(), register_callback, context)
        });
        match service {
            Ok(tag) => { tags.push(tag); self.advertisements.insert(id.clone(), tags); Ok(id) }
            Err(error) => { for tag in tags { self.operations.remove(&tag); } Err(error) }
        }
    }
    fn drain_events(&mut self) {
        loop {
            let event = self.queue.borrow_mut().events.pop_front();
            let Some((tag, event)) = event else { break; };
            self.event(tag, event);
        }
        if self.queue.borrow().overflow {
            self.error("Local network discovery traffic limit exceeded; stop and restart discovery".into());
            self.stop_browse(); self.failed = true;
            self.operations.clear(); self.advertisements.clear();
            let mut queue = self.queue.borrow_mut(); queue.events.clear(); queue.overflow = false;
        }
    }
    fn run(mut self, mut commands: mpsc::Receiver<Command>, stop: Arc<AtomicBool>) {
        let mut refs = Vec::new(); let mut polls = Vec::new();
        while !stop.load(Ordering::Acquire) {
            self.retire_admissions();
            self.drain_events();
            if self.operations.is_empty() {
                match commands.blocking_recv() {
                    Some(command) if !stop.load(Ordering::Acquire) => self.command(command),
                    _ => break,
                }
                continue;
            }
            for _ in 0..32 {
                if stop.load(Ordering::Acquire) { break; }
                match commands.try_recv() {
                    Ok(command) => self.command(command),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => { stop.store(true, Ordering::Release); break; }
                }
            }
            if stop.load(Ordering::Acquire) { break; }
            self.drain_events();
            if self.operations.is_empty() { continue; }
            refs.clear(); refs.extend(self.operations.iter().map(|(tag, op)| (*tag, op.raw)));
            polls.clear(); polls.extend(refs.iter().map(|(_, raw)| libc::pollfd {
                fd: unsafe { DNSServiceRefSockFD(*raw) }, events: libc::POLLIN, revents: 0 }));
            let result = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 50) };
            if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                self.error(format!("Bonjour polling failed: {}", std::io::Error::last_os_error())); break;
            }
            for ((tag, raw), poll) in refs.iter().copied().zip(polls.iter()) {
                if stop.load(Ordering::Acquire) { break; }
                if !self.operations.contains_key(&tag) { continue; }
                if poll.fd < 0 || poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    self.event(tag, Event::Error(-65563));
                } else if poll.revents & libc::POLLIN != 0 {
                    // No Rust reference into the operations map or callback queue
                    // is borrowed while native code invokes the callback.
                    let code = with_callbacks(tag, &self.queue, || unsafe { DNSServiceProcessResult(raw) });
                    if code != 0 { self.event(tag, Event::Error(code)); }
                    self.drain_events();
                }
            }
            let expired: Vec<_> = self.sources.iter().filter_map(|(key, source)|
                source.resolve.filter(|(_, start)| start.elapsed() > RESOLVE_TIMEOUT).map(|_| key.clone())).collect();
            for key in expired { self.expire(&key); }
        }
        self.stop_browse(); self.operations.clear();
    }
    fn remove_source(&mut self, key: &SourceKey) {
        if let Some(source) = self.sources.remove(key) { self.retire(source); self.publish(&key.id); }
    }
    fn invalidate(&mut self, key: &SourceKey) {
        if let Some(source) = self.sources.get_mut(key) {
            if let Some((tag, _)) = source.resolve.take() { self.operations.remove(&tag); }
            source.peer = None;
        }
        self.publish(&key.id);
    }
    /// Whether an expired resolution should be re-issued: both watched records
    /// are still present, so the peer is still advertised, and the bounded
    /// retry budget is not spent.
    fn retry_expired(&self, key: &SourceKey) -> bool {
        self.sources.get(key).is_some_and(|source| source.present == [true; 2] && source.retries < MAX_RESOLVE_RETRIES)
    }
    /// Expire one resolution. Clearing it leaves `present` set and no Watch
    /// event is coming for an unchanged record, so a transient resolve timeout
    /// would leave a live peer undiscoverable until its next record change.
    fn expire(&mut self, key: &SourceKey) {
        self.invalidate(key);
        if !self.retry_expired(key) { return; }
        if let Some(source) = self.sources.get_mut(key) { source.retries += 1; }
        if let Err(error) = self.resolve(key) { self.error(error); self.remove_source(key); }
    }
    fn event(&mut self, tag: u64, event: Event) {
        let Some(kind) = self.operations.get(&tag).map(|op| op.kind.clone()) else { return; };
        if let Event::Error(code) = event {
            self.error(native_error(code));
            match kind {
                Kind::Browse => { self.stop_browse(); self.failed = true; }
                Kind::Advertisement(id) => self.withdraw(&id),
                Kind::Watch(key, _) | Kind::Resolve(key) => self.remove_source(&key),
            }
            return;
        }
        match (kind, event) {
            (Kind::Browse, Event::Browse { interface, add, name, ty, domain }) => {
                if ty.trim_end_matches('.') != "_localswarm._tcp" || !domain.eq_ignore_ascii_case("local.") || interface == 0 { return; }
                let Ok(name) = CString::new(name) else { return; };
                let Ok(ty) = CString::new(ty) else { return; }; let Ok(domain) = CString::new(domain) else { return; };
                let mut full = [0u8; DNS_SERVICE_NAME_CAPACITY];
                // SAFETY: full has the SDK's required capacity and the input C strings live through the call.
                if unsafe { DNSServiceConstructFullName(full.as_mut_ptr().cast(), name.as_ptr(), ty.as_ptr(), domain.as_ptr()) } != 0 { return; }
                let Some(id) = browse_id(&full) else { return; };
                let key = SourceKey { id: id.to_owned(), interface };
                if !add { self.remove_source(&key); return; }
                if self.sources.contains_key(&key) || self.sources.len() >= MAX_SOURCES { return; }
                let mut watches = [0; 2];
                for (index, rrtype) in [16, 33].into_iter().enumerate() {
                    let Ok(full) = CString::new(key.id.clone()) else { return; };
                    match self.operation(Kind::Watch(key.clone(), index), |raw, context| unsafe {
                        DNSServiceQueryRecord(raw, INCLUDE_PEERS, interface, full.as_ptr(), rrtype, 1, query_callback, context)
                    }) {
                        Ok(tag) => watches[index] = tag,
                        Err(error) => { self.error(error); for tag in watches { self.operations.remove(&tag); } return; }
                    }
                }
                self.sources.insert(key, Source { name, ty, domain, watches, present: [false; 2], resolve: None, retries: 0,
                    peer: None });
            }
            (Kind::Watch(key, index), Event::Watch { interface, add }) if interface == key.interface => {
                if let Some(source) = self.sources.get_mut(&key) {
                    source.present[index] = add;
                    // A record change is fresh state: the retry budget re-arms.
                    source.retries = 0;
                    if let Some((tag, _)) = source.resolve.take() { self.operations.remove(&tag); }
                    source.peer = None;
                }
                self.publish(&key.id);
                if let Err(error) = self.resolve(&key) { self.error(error); self.remove_source(&key); }
            }
            (Kind::Resolve(key), Event::Resolved { interface, full, host, port, txt }) => {
                self.operations.remove(&tag);
                if let Some(source) = self.sources.get_mut(&key) { source.resolve = None; source.retries = 0; }
                if interface != key.interface || full != key.id { self.invalidate(&key); return; }
                let Some((mut peer, peer_to_peer)) = parse_peer(full, port, &txt, &self.self_id) else { self.invalidate(&key); return; };
                if !host.to_ascii_lowercase().ends_with(".local.") { self.invalidate(&key); return; }
                if let Some(source) = self.sources.get_mut(&key) {
                    let Ok(instance) = source.name.to_str() else { return; };
                    peer.host = super::super::apple_transport::service_host(instance, peer_to_peer);
                    source.peer = Some(peer);
                }
                self.publish(&key.id);
            }
            (Kind::Advertisement(id), Event::Registered(false)) => {
                self.error("Bonjour advertisement was withdrawn by the system".into());
                self.withdraw(&id);
            }
            (Kind::Watch(key, _) | Kind::Resolve(key), Event::Invalid) => self.invalidate(&key),
            _ => (),
        }
    }
    fn resolve(&mut self, key: &SourceKey) -> Result<(), String> {
        let Some(source) = self.sources.get(key) else { return Ok(()); };
        if source.present != [true; 2] { return Ok(()); }
        let name = source.name.clone(); let ty = source.ty.clone(); let domain = source.domain.clone();
        let tag = self.operation(Kind::Resolve(key.clone()), |raw, context| unsafe {
            DNSServiceResolve(raw, INCLUDE_PEERS, key.interface, name.as_ptr(), ty.as_ptr(), domain.as_ptr(), resolve_callback, context)
        })?;
        if let Some(source) = self.sources.get_mut(key) { source.resolve = Some((tag, Instant::now())); }
        Ok(())
    }
    fn publish(&self, id: &str) {
        // Keep interface provenance independent. A Bonjour endpoint retains scope
        // and lets Network.framework create the peer-to-peer route on demand.
        // Direct mode prefers a source resolved on AWDL/llw; otherwise the first
        // source is exactly the historical choice.
        let mut candidates: Vec<(&SourceKey, &Peer)> = Vec::new();
        for (key, source) in &self.sources {
            if key.id != id { continue; }
            if let Some(peer) = &source.peer {
                if !peer.host.is_empty() { candidates.push((key, peer)); }
            }
        }
        let selected = if self.direct {
            candidates.iter().find(|(key, _)| direct_interface(key.interface)).or_else(|| candidates.first())
        } else { candidates.first() };
        let peer = selected.map(|(_, peer)| (*peer).clone());
        let mut snapshot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(peer) = peer {
            if !snapshot.peers.contains_key(id) && snapshot.peers.len() >= MAX_PEERS {
                if let Some(oldest) = snapshot.peers.keys().next().cloned() { snapshot.peers.remove(&oldest); }
            }
            snapshot.peers.insert(id.to_string(), peer);
        } else { snapshot.peers.remove(id); }
    }
}

/// AWDL (`awdl0`) and low-latency WLAN (`llw0`) are Apple's peer-to-peer
/// interfaces. Interface indices from DNS-SD callbacks are stable for the
/// lifetime of the record.
fn direct_interface(index: u32) -> bool {
    if index == 0 { return false; }
    let mut output = [0u8; libc::IF_NAMESIZE];
    // SAFETY: if_indextoname writes at most IF_NAMESIZE bytes into the owned output buffer.
    if unsafe { libc::if_indextoname(index, output.as_mut_ptr().cast()) }.is_null() { return false; }
    CStr::from_bytes_until_nul(&output).is_ok_and(|name| {
        let name = name.to_bytes();
        name.starts_with(b"awdl") || name.starts_with(b"llw")
    })
}

fn browse_id(output: &[u8]) -> Option<&str> {
    let bounded = output.get(..MAX_BROWSE_ID_BYTES + 1).unwrap_or(output);
    CStr::from_bytes_until_nul(bounded).ok()?.to_str().ok()
}

fn append_txt(txt: &mut Vec<u8>, key: &str, value: &str) -> Result<(), String> {
    let len = key.len() + 1 + value.len();
    if len > 255 || txt.len() + 1 + len > MAX_TXT { return Err("LocalSwarm TXT exceeds limit".into()); }
    txt.push(len as u8); txt.extend_from_slice(key.as_bytes()); txt.push(b'='); txt.extend_from_slice(value.as_bytes()); Ok(())
}
fn parse_peer(id: String, port: u16, txt: &[u8], self_id: &str) -> Option<(Peer, bool)> {
    if id.len() > 255 || port == 0 || txt.len() > MAX_TXT { return None; }
    let mut properties = BTreeMap::new(); let mut rest = txt; let mut count = 0;
    while let Some((&len, tail)) = rest.split_first() {
        count += 1; if count > 16 { return None; }
        let value = tail.get(..len as usize)?; rest = tail.get(len as usize..)?;
        let split = value.iter().position(|b| *b == b'=')?;
        let key = std::str::from_utf8(&value[..split]).ok()?;
        if key.is_empty() || !key.bytes().all(|b| (0x20..=0x7e).contains(&b) && b != b'=') { return None; }
        let value = std::str::from_utf8(&value[split + 1..]).ok()?;
        if properties.insert(key.to_ascii_lowercase(), value).is_some() { return None; }
    }
    let text = |key: &str| properties.get(key).copied();
    if text("pv") != Some("1") { return None; }
    let publisher_name = crate::publisher::advertised_name(text("device")).ok()?;
    if text("tp") == Some("quic-library") {
        let session_id = text("sid")?; let peer_id = text("pid")?;
        let library_count = text("count")?.parse::<usize>().ok()?;
        let generation = text("gen").and_then(|value| value.parse::<u64>().ok());
        if !valid_library_session_id(session_id) || !protocol::valid_peer_id(peer_id) || peer_id == self_id
            || text("bytes") != Some("0") || text("chunks") != Some("0")
            || library_count == 0 || library_count > 10_000 || text("name") != Some("Shared library")
            || text("awdl").is_some_and(|value| value != "1")
            || properties.keys().any(|key| !["pv", "tp", "sid", "pid", "name", "bytes", "chunks", "count", "gen", "awdl", "device"].contains(&key.as_str()))
        { return None; }
        return Some((Peer { id, peer_id: peer_id.into(), swarm_id: String::new(),
            library_session_id: Some(session_id.into()), library_generation: generation, name: text("name")?.into(), total_bytes: 0, chunk_count: 0,
            publisher_name, library_count: Some(library_count),
            host: String::new(), port, locked: false, transport: "quic-library".into(), addresses: Vec::new() }, text("awdl") == Some("1")));
    }
    if text("tp") != Some("quic") { return None; }
    let swarm_id = text("sid")?; let peer_id = text("pid")?; let name = text("name")?;
    if !protocol::valid_hash(swarm_id) || !protocol::valid_peer_id(peer_id) || peer_id == self_id
        || name.is_empty() || name.len() > 240 || name.chars().any(char::is_control)
        || text("lk").is_some_and(|value| value != "0" && value != "1") { return None; }
    let total_bytes = text("bytes")?.parse::<u64>().ok()?; let chunk_count = text("chunks")?.parse::<usize>().ok()?;
    if chunk_count == 0 || chunk_count > protocol::MAX_CHUNKS || total_bytes == 0
        || total_bytes > protocol::MAX_CHUNK_SIZE as u64 * chunk_count as u64 { return None; }
    let locked = text("lk") == Some("1");
    Some((Peer { id, peer_id: peer_id.into(), swarm_id: swarm_id.into(), name: if locked { "Protected share".into() } else { name.into() },
        total_bytes, chunk_count, host: String::new(), port, locked, transport: "quic".into(), addresses: Vec::new(),
        publisher_name,
        library_session_id: None, library_generation: None, library_count: None }, text("awdl") == Some("1")))
}
fn native_error(code: i32) -> String {
    match code {
        -65570 | -65571 => format!("Local network permission denied; allow Local Network access for this app in system settings (Bonjour {code})"),
        _ => format!("Local network Bonjour operation failed ({code})"),
    }
}

/// A non-null value must be a NUL-terminated string borrowed for this callback.
/// Keep only the checked, owned text after returning to DNS-SD.
unsafe fn callback_string(value: *const c_char, capacity: usize) -> Option<String> {
    if value.is_null() { return None; }
    // SAFETY: successful DNS-SD callbacks provide a C string valid until return.
    let value = unsafe { CStr::from_ptr(value) };
    if value.to_bytes_with_nul().len() > capacity { return None; }
    value.to_str().ok().map(str::to_owned)
}

// DNS-SD echoes the cookie without dereferencing it. Decode borrowed payloads
// only after checking the active operation and its queue limits.
fn enqueue(cookie: *mut c_void, event: impl FnOnce() -> Event) {
    let _ = ACTIVE_CALLBACK.try_with(|active| {
        let active = active.borrow();
        let Some(context) = active.as_ref() else { return; };
        if context.tag != cookie.addr() as u64 { return; }
        let mut queue = context.queue.borrow_mut();
        if queue.window.elapsed() >= Duration::from_secs(60) { queue.window = Instant::now(); queue.count = 0; }
        queue.count = queue.count.saturating_add(1);
        if queue.overflow || queue.count > 8192 || queue.events.len() >= MAX_EVENTS { queue.overflow = true; return; }
        queue.events.push_back((context.tag, event()));
    });
}
unsafe extern "C" fn browse_callback(_: Ref, flags: u32, interface: u32, code: i32, name: *const c_char, ty: *const c_char, domain: *const c_char, context: *mut c_void) {
    enqueue(context, || {
        if code != 0 { return Event::Error(code); }
        // SAFETY: only a successful browse supplies these live C strings.
        match unsafe {
            (callback_string(name, BROWSE_CALLBACK_NAME_CAPACITY),
             callback_string(ty, BROWSE_CALLBACK_NAME_CAPACITY),
             callback_string(domain, BROWSE_CALLBACK_NAME_CAPACITY))
        } {
            (Some(name), Some(ty), Some(domain)) => Event::Browse { interface, add: flags & ADD != 0, name, ty, domain },
            _ => Event::Invalid,
        }
    });
}
extern "C" fn register_callback(_: Ref, flags: u32, code: i32, _: *const c_char, _: *const c_char, _: *const c_char, context: *mut c_void) {
    enqueue(context, || if code != 0 { Event::Error(code) } else { Event::Registered(flags & ADD != 0) });
}
unsafe extern "C" fn resolve_callback(_: Ref, _: u32, interface: u32, code: i32, full: *const c_char, host: *const c_char, port: u16, len: u16, txt: *const u8, context: *mut c_void) {
    enqueue(context, || {
        if code != 0 { return Event::Error(code); }
        let len = usize::from(len);
        if len > MAX_TXT || (len != 0 && txt.is_null()) { return Event::Invalid; }
        // SAFETY: only a successful resolve supplies these live C strings.
        let (Some(full), Some(host)) = (unsafe {
            (callback_string(full, DNS_SERVICE_NAME_CAPACITY), callback_string(host, DNS_SERVICE_NAME_CAPACITY))
        }) else { return Event::Invalid; };
        let txt = if len == 0 {
            Vec::new()
        } else {
            // SAFETY: a successful resolve lends len initialized TXT bytes
            // through this callback. The validated nonempty pointer is copied now.
            unsafe { std::slice::from_raw_parts(txt, len) }.to_vec()
        };
        Event::Resolved { interface, full, host, port: u16::from_be(port), txt }
    });
}
extern "C" fn query_callback(_: Ref, flags: u32, interface: u32, code: i32, _: *const c_char, _: u16, _: u16, len: u16, _: *const c_void, ttl: u32, context: *mut c_void) {
    enqueue(context, || {
        if code != 0 { Event::Error(code) } else if len as usize > MAX_TXT { Event::Invalid }
        else { Event::Watch { interface, add: flags & ADD != 0 && ttl != 0 } }
    });
}

extern "C" fn record_callback(_: Ref, _: Ref, _: u32, code: i32, context: *mut c_void) {
    enqueue(context, || if code != 0 { Event::Error(code) } else { Event::Registered(true) });
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_id_checks_termination_utf8_and_byte_limit() {
        let name = "Kiwix\\. 中文._localswarm._tcp.local.";
        let mut output = [0xff; DNS_SERVICE_NAME_CAPACITY];
        output[..name.len()].copy_from_slice(name.as_bytes());
        output[name.len()] = 0;
        assert_eq!(browse_id(&output), Some(name));

        output.fill(b'a');
        output[MAX_BROWSE_ID_BYTES] = 0;
        assert_eq!(browse_id(&output).unwrap().len(), MAX_BROWSE_ID_BYTES);
        output[MAX_BROWSE_ID_BYTES] = b'a';
        output[MAX_BROWSE_ID_BYTES + 1] = 0;
        assert_eq!(browse_id(&output), None);
        assert_eq!(browse_id(b"unterminated"), None);
        assert_eq!(browse_id(b"invalid\xff\0"), None);
    }

    #[test]
    fn direct_interface_rejects_missing_and_ordinary_interfaces() {
        assert!(!direct_interface(0));
        assert!(!direct_interface(u32::MAX));
        let loopback = nix::net::if_::if_nametoindex("lo0").unwrap();
        assert!(!direct_interface(loopback));
    }

    #[test]
    fn callback_payloads_own_their_text_and_txt() {
        let engine = engine();
        let cookie = ptr::without_provenance_mut(17);
        let mut full = *b"fixture._localswarm._tcp.local.\0";
        let mut host = *b"fixture.local.\0";
        let mut txt = [3, b'x', b'=', b'y'];
        with_callbacks(17, &engine.queue, || {
            // SAFETY: the strings are terminated and the TXT array is initialized.
            unsafe {
                resolve_callback(ptr::null_mut(), 0, 4, 0,
                    full.as_ptr().cast(), host.as_ptr().cast(), 4242u16.to_be(),
                    txt.len() as u16, txt.as_ptr(), cookie);
            }
        });
        full.fill(0);
        host.fill(0);
        txt.fill(0);
        let (tag, event) = engine.queue.borrow_mut().events.pop_front().unwrap();
        assert_eq!(tag, 17);
        let Event::Resolved { interface, full, host, port, txt } = event else { panic!("expected resolve"); };
        assert_eq!((interface, port), (4, 4242));
        assert_eq!(full, "fixture._localswarm._tcp.local.");
        assert_eq!(host, "fixture.local.");
        assert_eq!(txt, [3, b'x', b'=', b'y']);
    }

    #[test]
    fn callback_boundaries_keep_limits_and_ignore_undefined_error_payloads() {
        let engine = engine();
        let cookie = ptr::without_provenance_mut(18);
        let mut max_name = [b'x'; BROWSE_CALLBACK_NAME_CAPACITY];
        *max_name.last_mut().unwrap() = 0;
        let mut over_name = [b'x'; BROWSE_CALLBACK_NAME_CAPACITY + 1];
        *over_name.last_mut().unwrap() = 0;
        let ty = c"_localswarm._tcp";
        let domain = c"local.";
        with_callbacks(18, &engine.queue, || {
            // SAFETY: successful string arguments are terminated. Missing TXT and
            // undefined error arguments exercise rejection without borrowing them.
            unsafe {
                browse_callback(ptr::null_mut(), ADD, 4, 0, max_name.as_ptr().cast(), ty.as_ptr(), domain.as_ptr(), cookie);
                browse_callback(ptr::null_mut(), ADD, 4, 0, over_name.as_ptr().cast(), ty.as_ptr(), domain.as_ptr(), cookie);
                browse_callback(ptr::null_mut(), ADD, 4, 0, c"\xff".as_ptr(), ty.as_ptr(), domain.as_ptr(), cookie);
                resolve_callback(ptr::null_mut(), 0, 4, 0, ty.as_ptr(), domain.as_ptr(), 0, 0, ptr::null(), cookie);
                resolve_callback(ptr::null_mut(), 0, 4, 0, ty.as_ptr(), domain.as_ptr(), 0, 1, ptr::null(), cookie);
                resolve_callback(ptr::null_mut(), 0, 4, -65570, ptr::null(), ptr::null(), 0, u16::MAX, ptr::null(), cookie);
            }
        });
        let mut queue = engine.queue.borrow_mut();
        assert!(matches!(queue.events.pop_front(), Some((18, Event::Browse { interface: 4, add: true, name, .. })) if name.len() == BROWSE_CALLBACK_NAME_CAPACITY - 1));
        assert!(matches!(queue.events.pop_front(), Some((18, Event::Invalid))));
        assert!(matches!(queue.events.pop_front(), Some((18, Event::Invalid))));
        assert!(matches!(queue.events.pop_front(), Some((18, Event::Resolved { txt, .. })) if txt.is_empty()));
        assert!(matches!(queue.events.pop_front(), Some((18, Event::Invalid))));
        assert!(matches!(queue.events.pop_front(), Some((18, Event::Error(-65570)))));
        assert!(queue.events.is_empty());
    }

    #[test]
    fn callback_cookie_only_routes_during_its_own_native_call() {
        let outer = engine();
        let inner = engine();
        let tag = 41;
        let cookie = ptr::without_provenance_mut(tag);
        let reject = || panic!("A stale callback decoded its payload");
        enqueue(cookie, reject);
        with_callbacks(tag as u64, &outer.queue, || {
            enqueue(cookie, || Event::Registered(true));
            with_callbacks(42, &inner.queue, || {
                enqueue(cookie, reject);
                enqueue(ptr::without_provenance_mut(42), || Event::Invalid);
            });
            thread::spawn(move || enqueue(ptr::without_provenance_mut(tag), reject)).join().unwrap();
            enqueue(cookie, || Event::Error(-65570));
        });
        enqueue(cookie, reject);
        assert_eq!(outer.queue.borrow().count, 2);
        assert_eq!(inner.queue.borrow().count, 1);
        assert!(matches!(outer.queue.borrow_mut().events.pop_front(), Some((41, Event::Registered(true)))));
        assert!(matches!(outer.queue.borrow_mut().events.pop_front(), Some((41, Event::Error(-65570)))));
        assert!(matches!(inner.queue.borrow_mut().events.pop_front(), Some((42, Event::Invalid))));
    }

    #[test]
    fn failed_creation_events_cannot_reach_a_new_operation() {
        let mut engine = engine();
        let mut previous = 0;
        for _ in 0..2 {
            let result = engine.operation(Kind::Browse, |_, cookie| {
                assert!(cookie.addr() > previous);
                previous = cookie.addr();
                unsafe { browse_callback(ptr::null_mut(), 0, 0, -65570, ptr::null(), ptr::null(), ptr::null(), cookie) };
                -65570
            });
            assert!(result.is_err());
        }
        assert_eq!(engine.queue.borrow().events.len(), 2);
        engine.drain_events();
        assert!(engine.queue.borrow().events.is_empty());
        assert!(engine.snapshot.lock().unwrap().error.is_none());
        assert!(!engine.failed);
    }

    #[test]
    fn exhausted_callback_cookies_never_repeat_and_overflow_stays_bounded() {
        let counter = Mutex::new(usize::MAX - 1);
        assert_eq!(allocate_cookie(&counter).unwrap(), usize::MAX - 1);
        assert!(allocate_cookie(&counter).is_err());
        assert!(allocate_cookie(&counter).is_err());
        let engine = engine();
        let cookie = ptr::without_provenance_mut(11);
        with_callbacks(11, &engine.queue, || {
            for _ in 0..MAX_EVENTS { enqueue(cookie, || Event::Invalid); }
            enqueue(cookie, || panic!("An overflowing queue decoded a payload"));
        });
        let queue = engine.queue.borrow();
        assert_eq!(queue.events.len(), MAX_EVENTS);
        assert!(queue.overflow);
    }

    fn txt(properties: &[(&str, &str)]) -> Vec<u8> {
        let mut txt = Vec::new();
        for (key, value) in properties { append_txt(&mut txt, key, value).unwrap(); }
        txt
    }
    fn library_txt() -> Vec<u8> {
        txt(&[("pv", "1"), ("tp", "quic-library"), ("sid", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("pid", "remote-peer"), ("name", "Shared library"), ("count", "12"), ("gen", "1"), ("bytes", "0"), ("chunks", "0")])
    }
    #[test]
    fn publisher_hint_never_replaces_identity_or_exposes_protected_titles() {
        let mut advertised = library_txt();
        append_txt(&mut advertised, "device", "図書館の iPad").unwrap();
        let (peer, _) = parse_peer("library".into(), 8090, &advertised, "self-peer").unwrap();
        assert_eq!(peer.publisher_name.as_deref(), Some("図書館の iPad"));
        assert_eq!(peer.peer_id, "remote-peer");
        assert_eq!(peer.name, "Shared library");
        for name in ["", " padded", "spoof\u{202e}", &"é".repeat(81)] {
            let mut advertised = library_txt();
            append_txt(&mut advertised, "device", name).unwrap();
            assert!(parse_peer("library".into(), 8090, &advertised, "self-peer").is_none());
        }
        let mut duplicate = advertised;
        append_txt(&mut duplicate, "DEVICE", "Another name").unwrap();
        assert!(parse_peer("library".into(), 8090, &duplicate, "self-peer").is_none());
    }
    fn engine() -> Engine {
        Engine::new("self-peer".into(), Arc::new(Mutex::new(Snapshot::default())), Arc::new(AtomicBool::new(false)), false)
    }
    #[test]
    fn library_parser_binds_public_session_and_explicit_awdl_capability() {
        let (peer, awdl) = parse_peer("library".into(), 8090, &library_txt(), "self-peer").unwrap();
        assert!(!awdl); assert_eq!(peer.transport, "quic-library");
        assert_eq!(peer.name, "Shared library"); assert!(peer.swarm_id.is_empty());
        assert_eq!(peer.library_session_id.as_deref(), Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        for (key, value) in [("awdl", "0"), ("token", "secret"), ("pin", "1234"), ("title", "private book"), ("count", "10001")] {
            let mut advertised = library_txt(); append_txt(&mut advertised, key, value).unwrap();
            assert!(parse_peer("library".into(), 8090, &advertised, "self-peer").is_none());
        }
        assert!(parse_peer("library".into(), 0, &library_txt(), "self-peer").is_none());
        assert!(parse_peer("library".into(), 8090, &library_txt(), "remote-peer").is_none());
        let mut duplicate = library_txt(); append_txt(&mut duplicate, "SID", "b").unwrap();
        assert!(parse_peer("library".into(), 8090, &duplicate, "self-peer").is_none());
        let quic = txt(&[("pv", "1"), ("tp", "quic"), ("sid", &"a".repeat(64)), ("pid", "remote-peer"),
            ("name", "Book"), ("bytes", "8"), ("chunks", "2"), ("awdl", "1")]);
        let (quic, awdl) = parse_peer("quic".into(), 1234, &quic, "self-peer").unwrap();
        assert!(awdl); assert_eq!(quic.transport, "quic"); assert!(quic.library_session_id.is_none());
    }
    #[test]
    fn library_requires_live_endpoint_and_browse_off_preserves_owned_advertisement() {
        let mut engine = engine();
        let (peer, _) = parse_peer("library".into(), 8090, &library_txt(), "self-peer").unwrap();
        let key = SourceKey { id: "library".into(), interface: 0 };
        engine.sources.insert(key.clone(), Source {
            name: CString::new("library").unwrap(), ty: CString::new("_localswarm._tcp").unwrap(),
            domain: CString::new("local.").unwrap(), watches: [0; 2], present: [true; 2], resolve: None, retries: 0,
            peer: Some(peer),
        });
        engine.publish("library");
        assert!(engine.snapshot.lock().unwrap().peers.is_empty());
        let peer = engine.sources.get_mut(&key).unwrap().peer.as_mut().unwrap();
        peer.host = "192.168.1.20".into(); peer.addresses.push(peer.host.clone());
        engine.publish("library");
        assert_eq!(engine.snapshot.lock().unwrap().peers.len(), 1);
        engine.invalidate(&key);
        assert!(engine.snapshot.lock().unwrap().peers.is_empty());
        engine.advertisements.insert("owned-library".into(), Vec::new());
        engine.stop_browse();
        assert!(engine.sources.is_empty());
        assert!(engine.advertisements.contains_key("owned-library"));
        engine.withdraw("owned-library");
        assert!(engine.advertisements.is_empty());
    }
    #[test]
    fn an_expired_resolution_is_retried_while_the_records_stay_present() {
        let mut engine = engine();
        let key = SourceKey { id: "library".into(), interface: 0 };
        engine.sources.insert(key.clone(), Source {
            name: CString::new("library").unwrap(), ty: CString::new("_localswarm._tcp").unwrap(),
            domain: CString::new("local.").unwrap(), watches: [0; 2], present: [true; 2],
            resolve: Some((7, Instant::now() - RESOLVE_TIMEOUT - Duration::from_secs(1))), retries: 0,
            peer: None,
        });
        // Invalidation keeps `present`, so an unchanged record gets no Watch
        // event: the expiry must re-issue the resolve itself.
        engine.invalidate(&key);
        assert_eq!(engine.sources[&key].present, [true; 2]);
        assert!(engine.retry_expired(&key));
        engine.sources.get_mut(&key).unwrap().retries = MAX_RESOLVE_RETRIES;
        assert!(!engine.retry_expired(&key));
        engine.sources.get_mut(&key).unwrap().present = [true, false];
        assert!(!engine.retry_expired(&key));
    }
    #[test]
    fn abandoned_library_admission_withdraws_owned_records() {
        let mut engine = engine();
        engine.advertisements.insert("cancelled-library".into(), Vec::new());
        let admission = AdmissionGuard::new();
        engine.pending_admissions.push(PendingAdmission { state: admission.0.clone(),
            rollback: Rollback::Advertisement("cancelled-library".into()) });
        engine.retire_admissions();
        assert!(engine.advertisements.contains_key("cancelled-library"));
        drop(admission);
        engine.retire_admissions();
        assert!(engine.advertisements.is_empty());
        assert!(engine.pending_admissions.is_empty());
    }
    #[tokio::test]
    async fn invalid_library_listener_never_opens_bonjour_and_stale_snapshot_is_rejected() {
        let mut discovery = Discovery::new("self-peer".into()).unwrap();
        for count in [0, 10_001] {
            assert!(discovery.advertise_library(&"a".repeat(32), 8090, count, &[], "Test sender", 1, true).await.is_err());
        }
        assert!(discovery.worker.is_none());
        let (mut peer, _) = parse_peer("library".into(), 8090, &library_txt(), "self-peer").unwrap();
        peer.host = "192.168.1.20".into(); peer.addresses.push(peer.host.clone());
        discovery.snapshot.lock().unwrap().peers.insert(peer.id.clone(), peer);
        assert!(discovery.library_peer("library").is_err());
    }
}
