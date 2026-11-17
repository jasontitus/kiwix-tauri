// SPDX-License-Identifier: GPL-3.0-or-later
//! Owner-authenticated, foreground-only private TCP endpoint, isolated from public sharing.
use super::*;
use crate::private_sync::{Channel, Invitation, Message, Peer};
use crate::reading_data::Categories;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::net::{TcpSocket, TcpStream};
use tokio_util::sync::CancellationToken;
use serde::Serialize;

const PAIR_LIFETIME: Duration = Duration::from_secs(300);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest an idle automatic peer waits before one reconciliation exchange,
/// which also refreshes both sides' endpoints after a network change.
const IDLE_SYNC_REFRESH: Duration = Duration::from_secs(300);
fn private_sync_listener_socket() -> Result<TcpSocket, String> {
    let socket = TcpSocket::new_v4().map_err(|error| error.to_string())?;
    // A retired accepted connection can retain this local port in TIME_WAIT.
    // Unix SO_REUSEADDR permits its successor after the old listener is joined;
    // SO_REUSEPORT is deliberately never enabled, so live listeners stay exclusive.
    #[cfg(unix)]
    socket.set_reuseaddr(true).map_err(|error| error.to_string())?;
    Ok(socket)
}


#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairView {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    invitation: Option<zeroize::Zeroizing<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_code: Option<String>,
    local_confirmed: bool,
    remote_confirmed: bool,
    expires_at: String,
    phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
struct PairSession {
    view: Mutex<PairView>,
    token: Mutex<Option<zeroize::Zeroizing<String>>>,
    expires: Instant,
    claimed: AtomicBool,
    notify: tokio::sync::Notify,
    cancel: CancellationToken,
}
impl PairSession {
    fn retire(&self) {
        self.token.lock().unwrap_or_else(|error| error.into_inner()).take();
        let mut view = self.view.lock().unwrap_or_else(|error| error.into_inner());
        view.invitation = None;
        self.cancel.cancel();
        drop(view);
        self.notify.notify_waiters();
    }
}
#[derive(Clone)]
struct Runtime {
    window: String,
    generation: u64,
    revision: u64,
    endpoints: Vec<SocketAddr>,
    binding: crate::transfer_policy::LocalBinding,
    cancel: CancellationToken,
}

struct PeerGreeting {
    token: Option<zeroize::Zeroizing<String>>,
    name: String,
    endpoints: Vec<SocketAddr>,
    categories: Categories,
}
#[derive(Default)]
struct PeerActivity {
    syncing: bool,
    error: Option<String>,
    attempted: Option<Instant>,
}
pub(super) struct PrivateSyncManager {
    identity: crate::private_sync_identity::Identity,
    revision: std::sync::atomic::AtomicU64,
    runtime: Mutex<Option<Runtime>>,
    listener_done: Mutex<Option<tokio::sync::watch::Receiver<bool>>>,
    session: Mutex<Option<Arc<PairSession>>>,
    activity: Mutex<HashMap<String, PeerActivity>>,
    serial: tokio::sync::Mutex<()>,
    permits: Arc<tokio::sync::Semaphore>,
}
impl PrivateSyncManager {
    fn new(identity: crate::private_sync_identity::Identity) -> Self {
        Self { identity, revision: std::sync::atomic::AtomicU64::new(0), runtime: Mutex::new(None), listener_done: Mutex::new(None), session: Mutex::new(None), activity: Mutex::new(HashMap::new()), serial: tokio::sync::Mutex::new(()), permits: Arc::new(tokio::sync::Semaphore::new(8)) }
    }
    pub(super) fn cancel(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        if let Some(runtime) = self.runtime.lock().unwrap_or_else(|error| error.into_inner()).take() { runtime.cancel.cancel(); }
        if let Some(session) = self.session.lock().unwrap_or_else(|error| error.into_inner()).take() { session.retire(); }
        self.activity.lock().unwrap_or_else(|error| error.into_inner()).clear();
    }
    fn runtime(&self) -> Result<Runtime, String> {
        self.runtime.lock().map_err(|_| "Private sync unavailable")?.clone()
            .filter(|runtime| !runtime.cancel.is_cancelled()).ok_or_else(|| "Private sync is paused. Return to the app and try again".into())
    }
    fn pair(&self, id: &str) -> Result<Arc<PairSession>, String> {
        self.session.lock().map_err(|_| "Private pairing unavailable")?.clone()
            .filter(|session| session.view.lock().is_ok_and(|view| view.id == id) && Instant::now() < session.expires && !session.cancel.is_cancelled())
            .ok_or_else(|| "This connection invitation has expired. Create a new invitation".into())
    }
    fn finish_pair_error(&self, session: &PairSession, error: String) {
        if let Ok(mut view) = session.view.lock() {
            if matches!(view.phase.as_str(), "confirmed" | "error") { return; }
            view.phase = "error".into(); view.error = Some(error); view.invitation = None;
            session.cancel.cancel();
        }
        session.retire();
    }
}
impl Service {
    pub(super) fn retire_private_sync(&self) {
        if let Some(manager) = self.private_sync.get() { manager.cancel(); }
    }
    fn private_sync_admit(&self, runtime: &Runtime) -> Result<(), String> {
        self.ensure_running()?;
        self.ensure_window_active(&runtime.window)?;
        self.ensure_generation(&json!({"privacyGeneration":runtime.generation}))?;
        if self.private_sync.get().is_none_or(|manager| manager.revision.load(Ordering::SeqCst) != runtime.revision) {
            return Err("Private sync admission was retired. Return to the app and try again".into());
        }
        if runtime.cancel.is_cancelled() || self.store.private_mode()? || self.store.read_only()? {
            return Err("Private sync is paused in private or read-only mode".into());
        }
        #[cfg(mobile)]
        if !self.mobile_active() { return Err("Return to the app before syncing private reading data".into()); }
        if !self.store.reading_sync_enabled()? { return Err("Private reading sync is disabled".into()); }
        Ok(())
    }
    async fn private_sync_manager(self: &Arc<Self>, window: &str, generation: u64) -> Result<Arc<PrivateSyncManager>, String> {
        let creation_policy = self.privacy_gate.read().await;
        self.ensure_running()?; self.ensure_window_active(window)?;
        self.ensure_generation(&json!({"privacyGeneration":generation}))?;
        if self.store.private_mode()? || self.store.read_only()? { return Err("Private sync is paused".into()); }
        #[cfg(mobile)]
        if !self.mobile_active() { return Err("Return to the app before syncing private reading data".into()); }
        let manager = self.private_sync.get_or_try_init(|| async {
            let identity = crate::private_sync_identity::load_or_create(self.store.data_dir())?;
            Ok::<_, String>(Arc::new(PrivateSyncManager::new(identity)))
        }).await?.clone();
        drop(creation_policy);
        let _serial = manager.serial.lock().await;
        if let Ok(runtime) = manager.runtime() {
            if runtime.generation == generation && self.private_sync_admit(&runtime).is_ok() { return Ok(manager.clone()); }
            manager.cancel();
        }
        let revision = manager.revision.load(Ordering::SeqCst);
        let previous = manager.listener_done.lock().map_err(|_| "Private sync retirement unavailable")?.clone();
        if let Some(mut done) = previous {
            if !*done.borrow() {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while !*done.borrow() { done.changed().await.map_err(|_| "Private endpoint retirement stopped")?; }
                    Ok::<(), String>(())
                }).await.map_err(|_| "Private endpoint is still stopping. Try again shortly")??;
            }
        }
        let socket = private_sync_listener_socket()?;
        let binding = crate::transfer_policy::bind_local_tcp_socket(&socket)?;
        let port = self.store.private_sync_listen_port()?;
        socket.bind(SocketAddr::from(([0, 0, 0, 0], port))).map_err(|error| format!("Cannot open the private Nearby endpoint: {error}"))?;
        let listener = socket.listen(16).map_err(|error| error.to_string())?;
        let port = listener.local_addr().map_err(|error| error.to_string())?.port();
        let mut endpoints = Vec::new();
        for interface in if_addrs::get_if_addrs().map_err(|error| error.to_string())? {
            let IpAddr::V4(address) = interface.ip() else { continue; };
            if (address.is_private() || address.is_link_local()) && binding.is_on_link_peer(IpAddr::V4(address))? {
                let endpoint = SocketAddr::new(IpAddr::V4(address), port);
                if !endpoints.contains(&endpoint) { endpoints.push(endpoint); }
            }
        }
        if endpoints.is_empty() { return Err("Connect both devices to the same Wi-Fi or Ethernet network before using private Nearby sync".into()); }
        endpoints.truncate(16);
        let runtime = Runtime { window: window.into(), generation, revision, endpoints, binding, cancel: CancellationToken::new() };
        {
            let _policy = self.privacy_gate.read().await;
            self.private_sync_admit(&runtime)?;
            self.store.update(|state| { state.private_sync.listen_port = port; Ok(()) })?;
            let mut slot = manager.runtime.lock().map_err(|_| "Private sync unavailable")?;
            if manager.revision.load(Ordering::SeqCst) != revision { return Err("Private sync startup was cancelled".into()); }
            *slot = Some(runtime.clone());
        }
        let weak = Arc::downgrade(self);
        let owned = manager.clone();
        let (finished, done) = tokio::sync::watch::channel(false);
        *manager.listener_done.lock().map_err(|_| "Private endpoint retirement unavailable")? = Some(done);
        tokio::spawn(async move {
            let worker = async {
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (stream, peer) = accepted.map_err(|error| error.to_string())?;
                            if !runtime.binding.is_on_link_peer(peer.ip())? { continue; }
                            let Ok(permit) = owned.permits.clone().try_acquire_owned() else { continue; };
                            let Some(service) = weak.upgrade() else { return Ok::<(),String>(()); };
                            let manager = owned.clone(); let receipt = runtime.clone();
                            tokio::spawn(async move {
                                let _permit = permit;
                                let cancelled = receipt.cancel.clone();
                                tokio::select! {
                                    _ = cancelled.cancelled() => {},
                                    _ = tokio::time::timeout(PAIR_LIFETIME + NETWORK_TIMEOUT, service.private_sync_accept(manager, receipt, stream)) => {},
                                }
                            });
                        },
                        _ = tokio::time::sleep(Duration::from_secs(15)) => {
                            let Some(service) = weak.upgrade() else { return Ok(()); };
                            service.private_sync_admit(&runtime)?;
                            let counter = service.store.reading_sync_counter()?;
                            for peer in service.store.private_sync_automatic_peers()?.iter().filter(|peer| peer.enabled) {
                                // The peer already acknowledged everything this device
                                // has recorded and its own endpoint runs the mirror
                                // check for the reverse direction, so there is nothing
                                // to push. A peer that never completed a sync, or one
                                // that has not been reached for a while, still connects
                                // so enrollment and endpoint refresh are not skipped.
                                if peer.acknowledged_counter >= counter && peer.last_sync.as_deref()
                                    .and_then(|last| crate::timestamps::parse(last).ok())
                                    .is_some_and(|last| (time::OffsetDateTime::now_utc() - last)
                                        < time::Duration::seconds(IDLE_SYNC_REFRESH.as_secs() as i64))
                                { continue; }
                                service.private_sync_spawn(owned.clone(), peer.id.clone(), false)?;
                            }
                        }
                    }
                }
            };
            tokio::select! { _ = runtime.cancel.cancelled() => {}, _ = worker => { runtime.cancel.cancel(); } }
            drop(listener);
            let _ = finished.send(true);
        });
        Ok(manager.clone())
    }
    fn private_sync_command_admit(&self, args: &Value, window: &str) -> Result<u64, String> {
        self.ensure_running()?;
        self.ensure_window_active(window)?;
        let generation = self.ensure_generation(args)?;
        if self.store.private_mode()? || self.store.read_only()? { return Err("Private sync is unavailable in private or read-only mode".into()); }
        #[cfg(mobile)]
        if !self.mobile_active() { return Err("Return to the app before syncing private reading data".into()); }
        Ok(generation)
    }
    pub(super) async fn private_sync_command(self: Arc<Self>, command: &str, args: Value) -> Result<Value, String> {
        let window = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main").to_string();
        let generation = {
            let _policy = self.privacy_gate.read().await;
            self.private_sync_command_admit(&args, &window)?
        };
        if command == "private_sync_configure" {
            let enabled = args.get("enabled").and_then(Value::as_bool).ok_or("Choose whether to enable private sync")?;
            let categories: Categories = serde_json::from_value(args.get("categories").cloned().ok_or("Choose reading data to sync")?).map_err(|_| "Invalid private sync categories")?;
            self.retire_private_sync();
            let _policy = self.privacy_gate.write().await;
            self.retire_private_sync();
            self.private_sync_command_admit(&args, &window)?;
            self.store.update(|state| {
                state.reading_sync.enabled = enabled;
                state.reading_sync.categories = categories;
                // A category change alters what the peer has actually seen.
                // Drop the idle-skip evidence so the next tick re-exchanges.
                for peer in &mut state.private_sync.peers { peer.last_sync = None; }
                Ok(())
            })?;
        }
        if matches!(command, "private_sync_peer_update" | "private_sync_unpair" | "private_sync_resolve" | "private_sync_resume" | "private_sync_pair_cancel" | "private_sync_rebootstrap" | "private_sync_clear_positions") {
            if matches!(command, "private_sync_peer_update" | "private_sync_unpair" | "private_sync_rebootstrap" | "private_sync_clear_positions") { self.retire_private_sync(); }
            let _policy = self.privacy_gate.write().await;
            if matches!(command, "private_sync_peer_update" | "private_sync_unpair" | "private_sync_rebootstrap" | "private_sync_clear_positions") { self.retire_private_sync(); }
            self.private_sync_command_admit(&args, &window)?;
            match command {
                "private_sync_peer_update" | "private_sync_unpair" => {
                    let id = text(&args, "peerId")?;
                    self.store.update(|state| {
                        if command == "private_sync_unpair" { state.private_sync.peers.retain(|peer| peer.id != id); }
                        else {
                            let peer = state.private_sync.peers.iter_mut().find(|peer| peer.id == id).ok_or("Connected device no longer exists")?;
                            if let Some(name) = args.get("name").and_then(Value::as_str) {
                                if name.trim().is_empty() || name.len() > 128 || name.contains('\0') { return Err("Device name is too long or empty".into()); }
                                peer.name = name.trim().into();
                            }
                            if let Some(enabled) = args.get("enabled").and_then(Value::as_bool) { peer.enabled = enabled; peer.last_sync = None; }
                            if let Some(automatic) = args.get("automatic").and_then(Value::as_bool) { peer.automatic = automatic; peer.last_sync = None; }
                        }
                        Ok(())
                    })?;
                },
                "private_sync_resolve" => {
                    let conflict_id = text(&args, "conflictId")?;
                    let alternative_id = text(&args, "alternativeId")?;
                    let presented: Vec<String> = serde_json::from_value(args.get("alternativeIds").cloned().ok_or("Review the conflict alternatives before resolving")?).map_err(|_| "Invalid conflict alternatives")?;
                    self.store.update_reading_sync(|state| {
                        let conflict = crate::reading_data::conflicts(state).into_iter().find(|conflict| conflict.id == conflict_id).ok_or("This conflict changed. Review its current alternatives before resolving")?;
                        let current: std::collections::BTreeSet<_> = conflict.alternatives.iter().map(|value| value.id.as_str()).collect();
                        let expected: std::collections::BTreeSet<_> = presented.iter().map(String::as_str).collect();
                        if expected.len() != presented.len() || current != expected { return Err("This conflict changed. Review every current alternative before resolving".into()); }
                        crate::reading_data::resolve(state, conflict_id, alternative_id)
                    })?;
                    self.notify_library_changed();
                },
                "private_sync_resume" => {
                    let id = text(&args, "positionId")?;
                    let state = self.store.read()?;
                    let candidate = crate::reading_data::positions(&state).into_iter().find(|position| position.id == id).ok_or("This reading position is no longer available")?;
                    let book = state.books.iter().find(|book| book.metadata.id == candidate.book_id && book.available).ok_or("Add this archive to your local library before resuming this reading position")?;
                    return Ok(json!({"bookId":book.metadata.id,"path":candidate.path,"title":candidate.title,"scroll":candidate.scroll}));
                },
                "private_sync_pair_cancel" => {
                    let id = text(&args, "sessionId")?;
                    if let Some(manager) = self.private_sync.get() {
                        let mut slot = manager.session.lock().map_err(|_| "Private pairing unavailable")?;
                        if slot.as_ref().is_some_and(|session| session.view.lock().is_ok_and(|view| view.id == id)) {
                            if let Some(session) = slot.take() { session.retire(); }
                        }
                    }
                },
                "private_sync_rebootstrap" | "private_sync_clear_positions" => {
                    if args.get("confirmed").and_then(Value::as_bool) != Some(true) { return Err("Confirm that all connected devices must reconnect before resetting sync history".into()); }
                    self.store.update_reading_sync(|state| {
                        if command == "private_sync_clear_positions" {
                            crate::reading_data::rebootstrap_forgetting_positions(state)?;
                        } else {
                            crate::reading_data::rebootstrap(state)?;
                        }
                        state.private_sync.peers.clear();
                        Ok(())
                    })?;
                    self.notify_library_changed();
                },
                _ => unreachable!(),
            }
            return self.private_sync_status(self.private_sync.get());
        }
        let enabled = self.store.reading_sync_enabled()?;
        if !enabled {
            if !matches!(command, "private_sync_status" | "private_sync_configure") { return Err("Enable private reading sync first".into()); }
            return self.private_sync_status(None);
        }
        let manager = match self.private_sync_manager(&window, generation).await {
            Ok(manager) => manager,
            Err(error) if matches!(command, "private_sync_status" | "private_sync_configure") => {
                let _policy = self.privacy_gate.read().await;
                self.private_sync_command_admit(&args, &window)?;
                let mut status = self.private_sync_status(self.private_sync.get())?;
                status["error"] = Value::String(error);
                return Ok(status);
            },
            Err(error) => return Err(error),
        };
        let _command_policy = self.privacy_gate.read().await;
        self.private_sync_command_admit(&args, &window)?;
        let runtime = manager.runtime()?;
        self.private_sync_admit(&runtime)?;
        match command {
            "private_sync_status" | "private_sync_configure" => {},
            "private_sync_pair_start" => {
                let mut bytes = [0u8; 32]; getrandom::fill(&mut bytes).map_err(|_| "Cannot create a secure connection invitation")?;
                let token = zeroize::Zeroizing::new(hex::encode(bytes));
                use zeroize::Zeroize; bytes.zeroize();
                let expires_at = crate::timestamps::format(time::OffsetDateTime::now_utc() + PAIR_LIFETIME);
                let invitation = zeroize::Zeroizing::new(serde_json::to_string(&Invitation { version: 1, identity: manager.identity.public_hex(), token: token.clone(), endpoints: runtime.endpoints.clone(), expires_at: expires_at.clone() }).map_err(|error| error.to_string())?);
                let session = Arc::new(PairSession { view: Mutex::new(PairView { id: Uuid::new_v4().to_string(), invitation: Some(invitation), peer_identity: None, peer_name: None, verification_code: None, local_confirmed: false, remote_confirmed: false, expires_at, phase: "waiting".into(), error: None }), token: Mutex::new(Some(token)), expires: Instant::now() + PAIR_LIFETIME, claimed: AtomicBool::new(false), notify: tokio::sync::Notify::new(), cancel: runtime.cancel.child_token() });
                if let Some(old) = manager.session.lock().map_err(|_| "Private pairing unavailable")?.replace(session.clone()) { old.retire(); }
                Self::private_sync_expire(manager.clone(), session);
            },
            "private_sync_pair_join" => {
                let invitation = text(&args, "invitation")?;
                if invitation.len() > 8192 { return Err("Connection invitation is too large".into()); }
                let invitation: Invitation = serde_json::from_str(invitation).map_err(|_| "Invalid private connection invitation")?;
                crate::private_sync::validate_identity(&invitation.identity)?;
                let expires = crate::timestamps::parse(&invitation.expires_at).map_err(|_| "Invalid invitation expiry")?;
                if invitation.version != 1 || invitation.identity == manager.identity.public_hex() || invitation.token.len() != 64
                    || !invitation.token.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    || expires <= time::OffsetDateTime::now_utc() || expires > time::OffsetDateTime::now_utc() + time::Duration::seconds(330)
                    || invitation.endpoints.is_empty() || invitation.endpoints.len() > 16 { return Err("This invitation is invalid or expired. Create a new invitation on the other device".into()); }
                let remaining = Duration::try_from(expires - time::OffsetDateTime::now_utc()).map_err(|_| "Invitation expired")?.min(PAIR_LIFETIME);
                let session = Arc::new(PairSession { view: Mutex::new(PairView { id: Uuid::new_v4().to_string(), invitation: None, peer_identity: Some(invitation.identity.clone()), peer_name: None, verification_code: None, local_confirmed: false, remote_confirmed: false, expires_at: invitation.expires_at.clone(), phase: "waiting".into(), error: None }), token: Mutex::new(Some(invitation.token.clone())), expires: Instant::now() + remaining, claimed: AtomicBool::new(true), notify: tokio::sync::Notify::new(), cancel: runtime.cancel.child_token() });
                if let Some(old) = manager.session.lock().map_err(|_| "Private pairing unavailable")?.replace(session.clone()) { old.retire(); }
                Self::private_sync_expire(manager.clone(), session.clone());
                let service = self.clone(); let owned = manager.clone();
                tokio::spawn(async move {
                    let work = service.private_sync_join(owned.clone(), runtime, session.clone(), invitation);
                    let result = tokio::select! { _ = session.cancel.cancelled() => return, result = tokio::time::timeout(remaining, work) => result.map_err(|_| "Private connection invitation expired".to_string()).and_then(|value|value) };
                    if let Err(error) = result { owned.finish_pair_error(&session, error); }
                });
            },
            "private_sync_pair_confirm" => {
                let session = manager.pair(text(&args, "sessionId")?)?;
                let mut view = session.view.lock().map_err(|_| "Private pairing unavailable")?;
                if view.phase != "verifying" || view.peer_identity.as_deref() != Some(text(&args, "peerIdentity")?) || view.verification_code.is_none() { return Err("Verify the current comparison code on both devices before connecting".into()); }
                view.local_confirmed = true;
                session.notify.notify_one();
            },
            "private_sync_now" => self.private_sync_spawn(manager.clone(), text(&args, "peerId")?.into(), true)?,
            _ => return Err(format!("Unknown private sync command: {command}")),
        }
        self.private_sync_status(Some(&manager))
    }
    fn private_sync_expire(manager: Arc<PrivateSyncManager>, session: Arc<PairSession>) {
        tokio::spawn(async move {
            let remaining = session.expires.saturating_duration_since(Instant::now());
            tokio::select! { _ = session.cancel.cancelled() => session.retire(), _ = tokio::time::sleep(remaining) => {
                manager.finish_pair_error(&session, "This invitation expired. Create a new invitation to connect".into());
            } }
        });
    }
    fn private_sync_status(&self, manager: Option<&Arc<PrivateSyncManager>>) -> Result<Value, String> {
        let state = self.store.read()?;
        let session = manager.and_then(|manager| manager.session.lock().ok()?.clone()).and_then(|session| session.view.lock().ok().map(|view| view.clone()));
        let activity = manager.and_then(|manager| manager.activity.lock().ok());
        let peers: Vec<_> = state.private_sync.peers.iter().map(|peer| {
            let status = activity.as_ref().and_then(|activity| activity.get(&peer.id));
            json!({"id":peer.id,"identity":peer.identity,"name":peer.name,"enabled":peer.enabled,"automatic":peer.automatic,"lastSync":peer.last_sync,"state":if !peer.enabled {"disabled"} else if status.is_some_and(|status| status.syncing) {"syncing"} else if status.is_some_and(|status| status.error.is_some()) {"error"} else {"idle"},"error":status.and_then(|status|status.error.as_ref())})
        }).collect();
        let acknowledged = state.private_sync.peers.iter().filter(|peer|peer.enabled).map(|peer|peer.acknowledged_counter).min().unwrap_or(state.reading_sync.counter);
        Ok(json!({"identity":manager.map(|manager|manager.identity.public_hex()).unwrap_or_default(),"enabled":state.reading_sync.enabled,"categories":state.reading_sync.categories,"pending":state.reading_sync.counter.saturating_sub(acknowledged),"session":session,"peers":peers,"conflicts":crate::reading_data::conflicts(&state),"positions":crate::reading_data::positions(&state)}))
    }
    fn private_sync_spawn(self: &Arc<Self>, manager: Arc<PrivateSyncManager>, id: String, explicit: bool) -> Result<(), String> {
        let runtime = manager.runtime()?; self.private_sync_admit(&runtime)?;
        let peer = self.store.private_sync_peer(&id)?.filter(|peer| peer.enabled).ok_or("This private device is disabled or no longer connected")?;
        {
            let mut activity = manager.activity.lock().map_err(|_| "Private sync status unavailable")?;
            let status = activity.entry(id.clone()).or_default();
            if status.syncing || (!explicit && status.attempted.is_some_and(|time|time.elapsed() < Duration::from_secs(15))) { return Ok(()); }
            status.syncing = true; status.error = None; status.attempted = Some(Instant::now());
        }
        let service = self.clone();
        tokio::spawn(async move {
            let result = tokio::select! { _ = runtime.cancel.cancelled() => Err("Private sync was cancelled".into()), result = tokio::time::timeout(NETWORK_TIMEOUT, service.private_sync_connect(manager.clone(), runtime.clone(), peer)) => result.map_err(|_| "Private sync timed out. Keep both apps open on the same network".to_string()).and_then(|value|value) };
            if let Ok(mut activity) = manager.activity.lock() {
                if let Some(status) = activity.get_mut(&id) { status.syncing = false; status.error = result.err(); }
            }
        });
        Ok(())
    }
    async fn private_sync_socket(&self, runtime: &Runtime, endpoints: &[SocketAddr]) -> Result<TcpStream, String> {
        for endpoint in endpoints {
            self.private_sync_admit(runtime)?;
            if !endpoint.is_ipv4() || endpoint.port() == 0 || !runtime.binding.is_on_link_peer(endpoint.ip())? { continue; }
            let socket = TcpSocket::new_v4().map_err(|error|error.to_string())?;
            let binding = crate::transfer_policy::bind_local_tcp_socket(&socket)?;
            if !binding.is_on_link_peer(endpoint.ip())? { continue; }
            if let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_secs(3), socket.connect(*endpoint)).await { return Ok(stream); }
        }
        Err("The connected device is unavailable. Keep both apps open on the same Wi-Fi network; create a new invitation if its network address changed".into())
    }
    fn private_sync_hello(&self, runtime: &Runtime, purpose: &str, token: Option<zeroize::Zeroizing<String>>) -> Result<Message, String> {
        let state = self.store.read()?;
        Ok(Message::Hello { purpose: purpose.into(), token, name: state.reading_sync.device_name.clone(), endpoints: runtime.endpoints.clone(), categories: state.reading_sync.categories })
    }
    fn private_sync_peer_info(message: Message, expected: &str) -> Result<PeerGreeting, String> {
        match message {
            Message::Hello { purpose, token, name, endpoints, categories } if purpose == expected && !name.trim().is_empty() && name.len() <= 128 && !name.contains('\0') && !endpoints.is_empty() && endpoints.len() <= 16 && endpoints.iter().all(|endpoint|endpoint.is_ipv4() && endpoint.port()>0 && !endpoint.ip().is_unspecified() && !endpoint.ip().is_multicast()) => Ok(PeerGreeting { token, name, endpoints, categories }),
            _ => Err("Invalid private device greeting".into()),
        }
    }
    async fn private_sync_join(self: Arc<Self>, manager: Arc<PrivateSyncManager>, runtime: Runtime, session: Arc<PairSession>, invitation: Invitation) -> Result<(), String> {
        let endpoints = invitation.endpoints;
        let identity = invitation.identity;
        drop(invitation.token);
        let stream = self.private_sync_socket(&runtime, &endpoints).await?;
        let mut channel = tokio::time::timeout(Duration::from_secs(10), Channel::handshake(stream, &manager.identity, true, Some(&identity))).await.map_err(|_| "Private handshake timed out")??;
        let token = session.token.lock().map_err(|_| "Private pairing unavailable")?.take().ok_or("Private invitation was cancelled")?;
        channel.send(&self.private_sync_hello(&runtime, "pair", Some(token))?).await?;
        let PeerGreeting { name, endpoints, .. } = Self::private_sync_peer_info(channel.receive().await?, "pair")?;
        self.private_sync_pair_exchange(&manager, &runtime, &session, &mut channel, name, endpoints).await
    }
    async fn private_sync_accept(self: Arc<Self>, manager: Arc<PrivateSyncManager>, runtime: Runtime, stream: TcpStream) -> Result<(), String> {
        self.private_sync_admit(&runtime)?;
        let mut channel = tokio::time::timeout(Duration::from_secs(10), Channel::handshake(stream, &manager.identity, false, None)).await.map_err(|_| "Private handshake timed out")??;
        let message = tokio::time::timeout(Duration::from_secs(10), channel.receive()).await.map_err(|_| "Private device greeting timed out")??;
        match &message {
            Message::Hello { purpose, .. } if purpose == "pair" => {
                let PeerGreeting { token, name, endpoints, .. } = Self::private_sync_peer_info(message, "pair")?;
                let session = manager.session.lock().map_err(|_| "Private pairing unavailable")?.clone().ok_or("No active private connection invitation")?;
                use subtle::ConstantTimeEq;
                {
                    let mut expected = session.token.lock().map_err(|_| "Private pairing unavailable")?;
                    if session.cancel.is_cancelled() || Instant::now() >= session.expires || expected.as_ref().is_none_or(|expected| token.as_deref().is_none_or(|token| token.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1))
                        || session.claimed.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() { return Err("Invalid or already used private invitation".into()); }
                    expected.take();
                }
                drop(token);
                channel.send(&self.private_sync_hello(&runtime, "pair", None)?).await?;
                let result = tokio::select! { _ = session.cancel.cancelled() => return Ok(()), result = self.private_sync_pair_exchange(&manager, &runtime, &session, &mut channel, name, endpoints) => result };
                if let Err(error) = result { manager.finish_pair_error(&session, error); }
                Ok(())
            },
            Message::Hello { purpose, .. } if purpose == "sync" => {
                let PeerGreeting { endpoints, categories: remote_categories, .. } = Self::private_sync_peer_info(message, "sync")?;
                self.private_sync_trusted(&runtime, &channel.identity)?;
                channel.send(&self.private_sync_hello(&runtime, "sync", None)?).await?;
                self.private_sync_exchange(&runtime, &mut channel, false, endpoints, remote_categories).await
            },
            _ => Err("Unknown private reading protocol".into()),
        }
    }
    async fn private_sync_pair_exchange(&self, manager: &Arc<PrivateSyncManager>, runtime: &Runtime, session: &Arc<PairSession>, channel: &mut Channel, name: String, endpoints: Vec<SocketAddr>) -> Result<(), String> {
        session.token.lock().map_err(|_| "Private pairing unavailable")?.take();
        {
            let mut view = session.view.lock().map_err(|_| "Private pairing unavailable")?;
            if session.cancel.is_cancelled() || Instant::now() >= session.expires {
                return Err("Private invitation expired or was cancelled".into());
            }
            view.peer_identity = Some(channel.identity.clone()); view.peer_name = Some(name.clone());
            view.verification_code = Some(channel.verification.clone()); view.phase = "verifying".into(); view.invitation = None;
        }
        // Do not race/recreate Channel::receive against a local UI notification:
        // a partially consumed encrypted frame is not cancellation-resumable.
        loop {
            if Instant::now() >= session.expires || session.cancel.is_cancelled() { return Err("Private invitation expired or was cancelled".into()); }
            self.private_sync_admit(runtime)?;
            if session.view.lock().map_err(|_| "Private pairing unavailable")?.local_confirmed { break; }
            tokio::select! {
                _ = session.notify.notified() => {},
                _ = session.cancel.cancelled() => return Err("Private pairing was cancelled".into()),
                _ = tokio::time::sleep(session.expires.saturating_duration_since(Instant::now())) => return Err("Private invitation expired".into()),
            }
        }
        channel.send(&Message::Confirm { identity: channel.identity.clone() }).await?;
        let confirmation = tokio::select! {
            message = channel.receive() => message?,
            _ = session.cancel.cancelled() => return Err("Private pairing was cancelled".into()),
            _ = tokio::time::sleep(session.expires.saturating_duration_since(Instant::now())) => return Err("Private invitation expired".into()),
        };
        match confirmation {
            Message::Confirm { identity } if identity == manager.identity.public_hex() => {
                session.view.lock().map_err(|_| "Private pairing unavailable")?.remote_confirmed = true;
            },
            _ => return Err("Invalid private device confirmation".into()),
        }
        // Ready means both sides finished confirming the displayed code. Wait
        // for the peer before changing a durable enrollment (including re-pairs).
        self.private_sync_admit(runtime)?;
        self.store.check_private_peer_capacity(&channel.identity)?;
        channel.send(&Message::Ready).await?;
        if !matches!(channel.receive().await?, Message::Ready) { return Err("Private device did not finish confirmation".into()); }
        {
            let _policy = self.privacy_gate.read().await;
            self.private_sync_admit(runtime)?;
            if session.cancel.is_cancelled() { return Err("Private pairing was cancelled".into()); }
            let mut view = session.view.lock().map_err(|_| "Private pairing unavailable")?;
            if Instant::now() >= session.expires { return Err("Private invitation expired".into()); }
            self.store.update_reading_sync(|state| {
                if runtime.cancel.is_cancelled() || session.cancel.is_cancelled() { return Err("Private pairing was cancelled".into()); }
                state.private_sync.check_capacity(&channel.identity)?;
                let automatic = state.private_sync.peers.iter().find(|peer|peer.id == channel.identity).is_some_and(|peer|peer.automatic);
                state.private_sync.peers.retain(|peer|peer.id != channel.identity);
                state.private_sync.peers.push(Peer { id: channel.identity.clone(), identity: channel.identity.clone(), name, endpoints, enabled: true, automatic, last_sync: None, acknowledged_counter: 0 });
                Ok(())
            })?;
            view.phase = "confirmed".into();
        }
        // Reading payloads are never sent over the enrollment session. A fresh
        // authenticated connection must find both identities durably enrolled.
        Ok(())
    }
    fn private_sync_trusted(&self, runtime: &Runtime, identity: &str) -> Result<(), String> {
        self.private_sync_admit(runtime)?;
        // Saved-peer validation enforces `id == identity`, so the id index is
        // the identity index; `enabled` stays the admission policy here.
        if self.store.private_sync_peer(identity)?.is_none_or(|peer| !peer.enabled) { return Err("This private device is not connected or was revoked".into()); }
        Ok(())
    }
    async fn private_sync_connect(self: Arc<Self>, manager: Arc<PrivateSyncManager>, runtime: Runtime, peer: Peer) -> Result<(), String> {
        self.private_sync_trusted(&runtime, &peer.identity)?;
        let stream = self.private_sync_socket(&runtime, &peer.endpoints).await?;
        let mut channel = tokio::time::timeout(Duration::from_secs(10), Channel::handshake(stream, &manager.identity, true, Some(&peer.identity))).await.map_err(|_| "Private handshake timed out")??;
        channel.send(&self.private_sync_hello(&runtime, "sync", None)?).await?;
        let PeerGreeting { endpoints, categories, .. } = Self::private_sync_peer_info(channel.receive().await?, "sync")?;
        self.private_sync_exchange(&runtime, &mut channel, true, endpoints, categories).await
    }
    async fn private_sync_send_snapshot(&self, runtime: &Runtime, channel: &mut Channel, snapshot: crate::reading_data::WireSnapshot) -> Result<(), String> {
        // The policy writer cannot complete revocation while an admitted sender
        // can still write personal bytes. Cancellation releases this read lease.
        let _policy = self.privacy_gate.read().await;
        self.private_sync_trusted(runtime, &channel.identity)?;
        let message = Message::Snapshot { snapshot };
        tokio::select! {
            _ = runtime.cancel.cancelled() => Err("Private sync was cancelled".into()),
            result = channel.send(&message) => result,
        }
    }
    async fn private_sync_exchange(&self, runtime: &Runtime, channel: &mut Channel, initiator: bool, endpoints: Vec<SocketAddr>, remote_categories: Categories) -> Result<(), String> {
        self.private_sync_trusted(runtime, &channel.identity)?;
        let state = self.store.read()?;
        let categories = Categories { bookmarks: state.reading_sync.categories.bookmarks && remote_categories.bookmarks, positions: state.reading_sync.categories.positions && remote_categories.positions };
        let sent_counter;
        if initiator {
            sent_counter = state.reading_sync.counter;
            let snapshot = crate::reading_data::export(&state, categories)?;
            self.private_sync_send_snapshot(runtime, channel, snapshot).await?;
            let Message::Snapshot { snapshot } = channel.receive().await? else { return Err("Private device sent an unexpected response".into()); };
            self.private_sync_apply(runtime, &channel.identity, snapshot, categories).await?;
            self.private_sync_trusted(runtime, &channel.identity)?;
            channel.send(&Message::Committed).await?;
            if !matches!(channel.receive().await?, Message::Committed) { return Err("Private device did not acknowledge durable sync".into()); }
        } else {
            let Message::Snapshot { snapshot } = channel.receive().await? else { return Err("Private device did not send reading data".into()); };
            self.private_sync_apply(runtime, &channel.identity, snapshot, categories).await?;
            let state = self.store.read()?; sent_counter = state.reading_sync.counter;
            let snapshot = crate::reading_data::export(&state, categories)?;
            self.private_sync_send_snapshot(runtime, channel, snapshot).await?;
            if !matches!(channel.receive().await?, Message::Committed) { return Err("Private device did not acknowledge durable sync".into()); }
            self.private_sync_trusted(runtime, &channel.identity)?;
            channel.send(&Message::Committed).await?;
        }
        let _policy = self.privacy_gate.read().await;
        self.private_sync_trusted(runtime, &channel.identity)?;
        self.store.update_reading_sync(|state| {
            if runtime.cancel.is_cancelled() { return Err("Private sync was cancelled".into()); }
            let peer = state.private_sync.peers.iter_mut().find(|peer|peer.identity == channel.identity && peer.enabled).ok_or("Private device was revoked")?;
            peer.endpoints = endpoints; peer.last_sync = Some(crate::timestamps::now()); peer.acknowledged_counter = sent_counter;
            Ok(())
        })?;
        self.notify_library_changed();
        Ok(())
    }
    async fn private_sync_apply(&self, runtime: &Runtime, identity: &str, snapshot: crate::reading_data::WireSnapshot, negotiated: Categories) -> Result<(), String> {
        let _policy = self.privacy_gate.read().await;
        self.private_sync_trusted(runtime, identity)?;
        self.store.update_reading_sync(|state| {
            if runtime.cancel.is_cancelled() || !state.private_sync.peers.iter().any(|peer|peer.identity == identity && peer.enabled) { return Err("Private device was revoked".into()); }
            let categories = Categories { bookmarks: negotiated.bookmarks && state.reading_sync.categories.bookmarks, positions: negotiated.positions && state.reading_sync.categories.positions };
            crate::reading_data::apply(state, snapshot, categories)?;
            Ok(())
        })?;
        self.notify_library_changed();
        Ok(())
    }
}

#[cfg(all(test, not(mobile)))]
#[path = "private_sync_service_tests.rs"]
mod tests;

#[cfg(all(test, mobile))]
#[path = "private_sync_mobile_tests.rs"]
mod mobile_tests;
