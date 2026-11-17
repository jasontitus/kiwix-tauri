// SPDX-License-Identifier: GPL-3.0-or-later
//! Public BLE connection assistance. Only trusted Share all / selected Connect
//! actions authorize native radio configuration. Metadata reads never authorize it.
use super::*;
use crate::{localswarm::library::LibraryPeerDescriptor, nearby_public_link::{Framing, Kind, PublicLibrary, HostRequest, ReverseReply}};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// A stop sequence runs while callers may hold the nearby gate: native stop,
/// route cancellation and the retained worker join must all finish within it.
const NEARBY_STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct NearbyState {
    inner: Mutex<NearbyInner>,
    pub(super) gate: tokio::sync::Mutex<()>,
    pub(super) native_ready: AtomicBool,
}
#[derive(Default)]
struct NearbyInner { publisher: Option<Owner>, consumer: Option<Owner>, hotspot: Option<PublicHotspot> }
struct PublicHotspot {
    address: String,
    join: crate::nearby_public_link::TemporaryNetwork,
    cancel: CancellationToken,
}
struct Owner {
    work: WorkOwner,
    #[cfg_attr(not(mobile), expect(dead_code, reason = "The native mobile BLE callback forwards events to the owner task"))]
    events: mpsc::Sender<BleEvent>,
    actions: mpsc::Sender<Select>,
    status: OwnerStatus,
    peers: Vec<PublicPeer>,
    descriptor: Option<LibraryPeerDescriptor>,
    join_lease: Option<String>,
    selected_cancel: Option<CancellationToken>,
    reverse_route: Option<crate::transfer_policy::JoinedRoute>,
    library_cancel: Option<CancellationToken>,
    hotspot_claim: bool,
    scanning: bool,
    connect_pending: bool,
    #[cfg_attr(not(mobile), expect(dead_code, reason = "The native mobile BLE callback enforces monotonically ordered events"))]
    sequence: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnerStatus { owner: String, phase: String, peer_id: Option<String>, error: Option<String> }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicPeer {
    id: String, peer_id: String, transport: &'static str, discovery: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    publisher_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    library_session_id: Option<String>,
    #[serde(skip)]
    native: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BleEvent {
    #[cfg_attr(not(mobile), expect(dead_code, reason = "The native mobile BLE callback authenticates the receiving owner"))]
    owner: String,
    #[cfg_attr(not(mobile), expect(dead_code, reason = "The native mobile BLE callback validates event ordering"))]
    sequence: u64,
    kind: String,
    peer: Option<String>, connection: Option<String>,
    // Retain the native event schema under deny_unknown_fields.
    #[serde(rename = "info")]
    _info: Option<String>,
    name: Option<String>,
    packet_budget: Option<u16>, bytes: Option<String>, error: Option<String>,
}
struct Select { peer: String, reply: oneshot::Sender<Result<Value, String>> }
struct PublicExchange { connection: String, framing: Framing, deadline: Instant, delivered: bool, hosting: bool }
#[derive(Clone)]
struct WorkOwner { id: String, label: String, privacy_generation: u64, admission_generation: u64, cancel: CancellationToken }
impl NearbyInner {
    fn owner(&self, id: &str) -> Option<&Owner> {
        self.publisher.as_ref().filter(|owner| owner.work.id == id)
            .or_else(|| self.consumer.as_ref().filter(|owner| owner.work.id == id))
    }
    fn owner_mut(&mut self, id: &str) -> Option<&mut Owner> {
        if self.publisher.as_ref().is_some_and(|owner| owner.work.id == id) { self.publisher.as_mut() }
        else { self.consumer.as_mut().filter(|owner| owner.work.id == id) }
    }
}
impl NearbyState {
    pub(super) fn cancel_all(&self) {
        let state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        for owner in [&state.publisher, &state.consumer].into_iter().flatten() {
            owner.work.cancel.cancel();
            if let Some(cancel) = &owner.library_cancel { cancel.cancel(); }
        }
        if let Some(hotspot) = &state.hotspot { hotspot.cancel.cancel(); }
    }
}
impl Service {
    #[cfg(mobile)]
    pub(crate) fn nearby_listeners_ready(&self) {
        self.nearby.native_ready.store(true, Ordering::SeqCst);
    }
    #[cfg_attr(not(mobile), expect(dead_code, reason = "Registered by the native mobile BLE event listener"))]
    pub(crate) fn nearby_bluetooth_event(&self, payload: Value) {
        let Ok(event) = serde_json::from_value::<BleEvent>(payload) else { self.nearby.cancel_all(); return; };
        let mut state = self.nearby.inner.lock().unwrap_or_else(|error| error.into_inner());
        let Some(owner) = state.owner_mut(&event.owner) else { return; };
        if owner.work.cancel.is_cancelled() { return; }
        if event.sequence <= owner.sequence || event.bytes.as_ref().is_some_and(|bytes| bytes.len() > 684)
            || event.name.as_ref().is_some_and(|name| name.len() > 256) {
            owner.work.cancel.cancel(); return;
        }
        owner.sequence = event.sequence;
        if owner.events.try_send(event).is_err() { owner.work.cancel.cancel(); }
    }
    #[cfg_attr(not(mobile), expect(dead_code, reason = "Registered by the native mobile joined-route loss listener"))]
    pub(crate) fn nearby_join_event(&self, payload: Value) {
        let Some(id) = payload.get("owner").and_then(Value::as_str) else { return; };
        let mut state = self.nearby.inner.lock().unwrap_or_else(|error| error.into_inner());
        let Some(owner) = state.owner_mut(id) else { return; };
        if payload.get("kind").and_then(Value::as_str) == Some("lost")
            && payload.get("lease").and_then(Value::as_str) == owner.join_lease.as_deref() {
            if let Some(cancel) = &owner.selected_cancel { cancel.cancel(); }
            if let Some(descriptor) = owner.descriptor.take() {
                descriptor.cancel.cancel();
                if let Some(route) = descriptor.route { route.cancel(); }
            }
            owner.status.phase = "failed".into();
            owner.status.error = Some("The sender’s Wi-Fi connection ended. Connect again.".into());
        }
    }
    fn nearby_live(&self, owner: &WorkOwner) -> Result<(), String> {
        if owner.cancel.is_cancelled() { return Err("Nearby connection stopped".into()); }
        self.ensure_running()?;
        self.ensure_window_active(&owner.label)?;
        self.normal_writable()?;
        if self.launch_generation() != owner.privacy_generation { return Err("Nearby connection expired after a privacy transition".into()); }
        #[cfg(mobile)]
        {
            let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            if !state.active || !state.desired_active { return Err(crate::user_error::suspended()); }
        }
        if self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?.owner(&owner.id).is_none() {
            return Err("Nearby connection stopped".into());
        }
        Ok(())
    }
    fn nearby_status_update(&self, owner: &WorkOwner, phase: &str, error: Option<String>) {
        let mut state = self.nearby.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(current) = state.owner_mut(&owner.id) { current.status.phase = phase.into(); current.status.error = error; }
    }
    async fn nearby_native(&self, command: &'static str, payload: Value) -> Result<Value, String> {
        #[cfg(mobile)]
        return self.mobile_call(command, payload).await;
        #[cfg(not(mobile))]
        { let _ = (command, payload); Err("Bluetooth discovery requires the iPhone, iPad or Android application. Use the same local network on this device.".into()) }
    }
    async fn nearby_call(&self, owner: &WorkOwner, command: &'static str, payload: Value) -> Result<Value, String> {
        self.nearby_live(owner)?;
        tokio::select! {
            biased;
            _ = owner.cancel.cancelled() => Err("Nearby connection stopped".into()),
            result = self.nearby_native(command, payload) => result,
        }
    }
    async fn nearby_send(&self, owner: &WorkOwner, connection: &str, packets: Vec<Vec<u8>>) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(15), async {
            for packet in packets {
                self.nearby_call(owner, "nearbyBluetoothSend", json!({"owner":owner.id,"connection":connection,"bytes":STANDARD.encode(packet)})).await?;
            }
            Ok::<_,String>(())
        }).await.map_err(|_|"The public Nearby information exchange timed out")?
    }
    async fn nearby_stop_owner(&self, id: Option<&str>) -> Result<(), String> {
        crate::nearby_log!("[nearby] stop owner id={id:?}");
        let owners = {
            let mut state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
            let mut owners = Vec::new();
            if state.publisher.as_ref().is_some_and(|owner| id.is_none_or(|id| id == owner.work.id)) { owners.push(state.publisher.take().unwrap()); }
            if state.consumer.as_ref().is_some_and(|owner| id.is_none_or(|id| id == owner.work.id)) { owners.push(state.consumer.take().unwrap()); }
            for owner in &owners { owner.work.cancel.cancel(); }
            owners
        };
        let mut errors = Vec::new();
        #[cfg(target_os = "android")]
        let retired_any = !owners.is_empty();
        for mut owner in owners {
            if let Some(descriptor) = &owner.descriptor { descriptor.cancel.cancel(); }
            if let Some(cancel) = &owner.library_cancel { cancel.cancel(); }
            let owner_id = owner.work.id.clone();
            let mut task = owner.task.take();
            // The caller may hold the nearby gate: a native stop that never
            // returns, or a retained worker that never finishes, must not pin it.
            let stop = async {
                let mut failures = Vec::new();
                if let Err(error) = self.nearby_native("nearbyBluetoothStop", json!({"owner": &owner_id})).await { failures.push(error); }
                if let Err(error) = self.nearby_native("nearbyJoinCancel", json!({"owner": &owner_id})).await { failures.push(error); }
                failures
            };
            match tokio::time::timeout(NEARBY_STOP_TIMEOUT, stop).await {
                Ok(failures) => errors.extend(failures),
                Err(_) => errors.push("The Nearby connection did not stop in time".into()),
            }
            if let Some(mut handle) = task.take() {
                let joined = tokio::time::timeout(NEARBY_STOP_TIMEOUT, &mut handle).await;
                match joined {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => errors.push(error.to_string()),
                    Err(_) => {
                        // Retained ownership ends here: abort instead of detaching
                        // a worker that outlived its stop bound.
                        handle.abort();
                        errors.push("The Nearby connection did not stop in time".into());
                    }
                }
            }
        }
        #[cfg(target_os = "android")]
        if retired_any {
            if let Err(error) = self.nearby_release_hotspot_if_unused().await { errors.push(error); }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
    pub(super) async fn nearby_stop_publisher(&self) -> Result<(), String> {
        let id = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?.publisher.as_ref().map(|owner| owner.work.id.clone());
        if let Some(id) = id { self.nearby_stop_owner(Some(&id)).await } else { Ok(()) }
    }
    pub(super) async fn nearby_retire(&self) -> Result<(), String> {
        self.nearby.cancel_all();
        let _gate = self.nearby.gate.lock().await;
        let mut errors = Vec::new();
        if let Err(error) = self.nearby_stop_owner(None).await { errors.push(error); }
        self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.hotspot = None;
        #[cfg(mobile)]
        if let Err(error) = self.nearby_native("nearbyRetire", json!({})).await { errors.push(error); }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
    fn nearby_insert(&self, label: String, publisher: bool) -> Result<(WorkOwner, mpsc::Receiver<BleEvent>, mpsc::Receiver<Select>), String> {
        let (admission_generation, cancel) = self.localswarm_admission_token(None)?;
        let work = WorkOwner { id:Uuid::new_v4().to_string(),label,
            privacy_generation:self.launch_generation(),admission_generation,cancel };
        let (events, receive_events) = mpsc::channel(64);
        let (actions, receive_actions) = mpsc::channel(1);
        let owner = Owner { work:work.clone(),events,actions,status:OwnerStatus { owner:work.id.clone(),phase:"starting".into(),peer_id:None,error:None },
            peers:Vec::new(),descriptor:None,join_lease:None,selected_cancel:None,reverse_route:None,library_cancel:None,hotspot_claim:false,scanning:false,connect_pending:false,sequence:0,task:None };
        let mut state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
        let slot = if publisher { &mut state.publisher } else { &mut state.consumer };
        if slot.is_some() { return Err("Nearby radio owner is already active".into()); }
        *slot = Some(owner);
        Ok((work, receive_events, receive_actions))
    }
    fn nearby_task(&self, work: &WorkOwner, task: tokio::task::JoinHandle<()>) {
        if let Some(owner) = self.nearby.inner.lock().unwrap_or_else(|error| error.into_inner()).owner_mut(&work.id) { owner.task = Some(task); }
        else { task.abort(); }
    }
    /// Share all may call this after explicitly starting an app-owned Android
    /// hotspot. Never starts a hotspot, borrows a home password, or guesses an AP IP.
    pub(super) async fn nearby_library_socket(self: &Arc<Self>, label: &str)
        -> Result<Option<(std::net::UdpSocket, Vec<String>, CancellationToken)>, String> {
        #[cfg(target_os = "android")]
        {
            use tauri_plugin_kiwix_mobile::MobileExt;
            self.ensure_window_active(label)?;
            self.normal_writable()?;
            if !self.mobile_hotspot_hosting() { return Ok(None); }
            let generation = self.launch_generation();
            let info = self.hotspot.lock().map_err(|_|"Hotspot state unavailable")?.info().cloned()
                .ok_or("The temporary hotspot is not ready")?;
            let app = self.app.lock().map_err(|_|"Application handle unavailable")?.clone().ok_or("Native adapter unavailable")?;
            let (socket, address, lease) = tokio::task::spawn_blocking(move || {
                let deadline = Instant::now()+Duration::from_secs(10);
                let reply = loop {
                    match app.kiwix_mobile().call_sync("hotspotQuicRoute",json!({})) {
                        Ok(reply) => break reply,
                        Err(error) if (error.contains("EXACT_INTERFACE_UNAVAILABLE") || error.contains("NEARBY_VISIBLE_ACTIVITY_REQUIRED")) && Instant::now()<deadline => std::thread::sleep(Duration::from_millis(200)),
                        Err(error) => return Err(error),
                    }
                };
                let address = text(&reply,"address")?.to_string();
                let socket = crate::transfer_policy::JoinedRoute::hotspot_socket(&reply,None)?;
                let lease = text(&reply,"lease")?.to_string();
                let id = Uuid::parse_str(&lease).map_err(|_|"Invalid hotspot interface lease")?;
                if id.to_string()!=lease { return Err("Invalid hotspot interface lease".into()); }
                socket.set_nonblocking(true).map_err(|error|error.to_string())?;
                Ok::<_,String>((socket,address,lease))
            }).await.map_err(|error|error.to_string())??;
            self.ensure_window_active(label)?;
            self.normal_writable()?;
            if self.launch_generation()!=generation { return Err("Nearby sharing was retired".into()); }
            let cancel = CancellationToken::new();
            {
                let mut state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
                if let Some(existing) = state.hotspot.as_ref().filter(|existing|existing.address==address
                    && existing.join.reservation_id==lease && !existing.cancel.is_cancelled()) {
                    // Sharing and receiving may legitimately retain the same AP.
                    // Reusing its exact lease must not retire the existing listener.
                    return Ok(Some((socket,vec![address],existing.cancel.clone())));
                }
                if let Some(old) = state.hotspot.take() { old.cancel.cancel(); }
                state.hotspot = Some(PublicHotspot { address:address.clone(),join:crate::nearby_public_link::TemporaryNetwork {
                    reservation_id:lease.clone(),ssid:info.ssid,passphrase:zeroize::Zeroizing::new(info.passphrase),
                },cancel:cancel.clone() });
            }
            let service = self.clone(); let label = label.to_string(); let watch = cancel.clone();
            tokio::spawn(async move {
                loop {
                    let available = service.nearby_native("hotspotQuicState",json!({"lease":lease})).await
                        .ok().and_then(|reply|reply.get("available").and_then(Value::as_bool))==Some(true);
                    let foreground = service.mobile.state.lock().is_ok_and(|state|state.active && state.desired_active);
                    if !available || !foreground || service.ensure_window_active(&label).is_err()
                        || service.normal_writable().is_err() || service.launch_generation()!=generation {
                        watch.cancel(); return;
                    }
                    // Fallback detector: a retired reservation and a lifecycle
                    // transition both arrive as events, and explicit cancellation
                    // is immediate through `watch`, so this bridge round trip only
                    // needs to catch a lease that vanished silently.
                    tokio::select! { _ = watch.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
                }
            });
            Ok(Some((socket,vec![address],cancel)))
        }
        #[cfg(not(target_os = "android"))]
        { let _ = label; Ok(None) }
    }
    /// Called only after the live QUIC library listener has been created by Share all.
    pub(super) async fn nearby_publish_library(self: &Arc<Self>, label: &str, descriptor: LibraryPeerDescriptor) -> Result<(), String> {
        if !cfg!(mobile) { return Ok(()); }
        crate::nearby_log!("[nearby] publish library start");
        if !self.nearby.native_ready.load(Ordering::SeqCst) { return Err("Native Nearby listeners are not ready yet. Try again.".into()); }
        let _gate = self.nearby.gate.lock().await;
        crate::nearby_log!("[nearby] publish library gate acquired");
        self.nearby_stop_publisher().await?;
        let (work, events, _) = self.nearby_insert(label.to_string(), true)?;
        self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&work.id)
            .ok_or("Nearby sharing stopped")?.library_cancel = Some(descriptor.cancel.clone());
        let join = {
            let state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
            state.hotspot.as_ref().filter(|hotspot|!hotspot.cancel.is_cancelled()
                && descriptor.addresses.len()==1 && descriptor.addresses[0]==hotspot.address).map(|hotspot|hotspot.join.clone())
        };
        let public = PublicLibrary { version:1,public_id:work.id.clone(),session_id:descriptor.session_id.clone(),peer_id:descriptor.peer_id.clone(),
            name:descriptor.name.clone(),port:descriptor.port,
            addresses:descriptor.addresses.iter().filter(|raw|crate::nearby_public_link::metadata_address(raw))
                .take(16).cloned().collect(),join,can_join:cfg!(target_os = "ios") };
        let encoded = match public.encode() {
            Ok(encoded) => encoded,
            Err(error) => { self.nearby_stop_owner(Some(&work.id)).await?; return Err(error); }
        };
        let service = self.clone(); let task_work = work.clone();
        let task = tokio::spawn(async move {
            let result = service.nearby_publish(&task_work, public, encoded, descriptor.cancel, events).await;
            if let Err(error) = result {
                crate::nearby_log!("[nearby] publish failed: {error}");
                if !task_work.cancel.is_cancelled() { service.nearby_status_update(&task_work,"failed",Some(error)); }
                // A Bluetooth failure alone does not revoke an accepted QUIC
                // source. Its originating window still owns the live listener.
                if service.nearby_live(&task_work).is_err() {
                    task_work.cancel.cancel();
                    if let Ok(state) = service.nearby.inner.lock() {
                        if let Some(cancel) = state.owner(&task_work.id).and_then(|owner|owner.library_cancel.as_ref()) { cancel.cancel(); }
                    }
                }
            } else {
                service.nearby_status_update(&task_work,"stopped",None);
                task_work.cancel.cancel();
            }
            let _ = service.nearby_native("nearbyBluetoothStop", json!({"owner":task_work.id})).await;
        });
        self.nearby_task(&work,task);
        Ok(())
    }
    pub(super) fn nearby_status(&self) -> Result<Value, String> {
        let state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
        Ok(json!({"supported":cfg!(mobile),"publisher":state.publisher.as_ref().map(|owner| &owner.status),
            "nativeReady":self.nearby.native_ready.load(Ordering::SeqCst),
            "consumer":state.consumer.as_ref().map(|owner| &owner.status),
            "discovery":state.consumer.as_ref().is_some_and(|owner|owner.scanning && !owner.work.cancel.is_cancelled()),
            "peers":state.consumer.as_ref().filter(|owner|!owner.work.cancel.is_cancelled()).map(|owner|owner.peers.clone()).unwrap_or_default()}))
    }
    pub(super) fn nearby_resolve(&self, peer_id: &str) -> Result<LibraryPeerDescriptor, String> {
        let (work, descriptor) = {
            let state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
            let owner = state.consumer.as_ref().ok_or("Choose and connect to the nearby sender first")?;
            if !owner.peers.iter().any(|peer| peer.id == peer_id && peer.library_session_id.is_some()) { return Err("Choose and connect to this nearby sender first".into()); }
            (owner.work.clone(), owner.descriptor.clone().ok_or("The Nearby sender is not connected")?)
        };
        self.nearby_live(&work)?;
        if descriptor.cancel.is_cancelled() { return Err("The Nearby connection ended. Connect again.".into()); }
        if let Some(route) = &descriptor.route { route.check()?; }
        Ok(descriptor)
    }
    pub(super) async fn nearby_command(self: Arc<Self>, command: &str, args: Value) -> Result<Value, String> {
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main").to_string();
        self.ensure_window_active(&label)?;
        if command == "nearby_stop" {
            let _gate = self.nearby.gate.lock().await;
            self.nearby_stop_owner(args.get("owner").and_then(Value::as_str)).await?;
            return Ok(Value::Null);
        }
        #[cfg(mobile)]
        self.mobile_admit(command,&args)?;
        self.ensure_generation(&args)?;
        self.normal_writable()?;
        if command == "nearby_status" { return self.nearby_status(); }
        if command == "nearby_connect" {
            let peer = text(&args,"peerId")?.to_string();
            let (actions, cancel) = {
                let mut state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
                let owner = state.consumer.as_mut().filter(|owner| owner.work.label == label && !owner.work.cancel.is_cancelled()).ok_or("Open Nearby to find the sender again")?;
                if !owner.peers.iter().any(|row| row.id == peer) { return Err("The selected nearby sender is no longer available".into()); }
                if owner.connect_pending { return Err("A Nearby connection is already in progress".into()); }
                owner.connect_pending = true;
                (owner.actions.clone(),owner.work.cancel.clone())
            };
            let (reply, response) = oneshot::channel();
            if actions.try_send(Select { peer,reply }).is_err() {
                if let Ok(mut state) = self.nearby.inner.lock() {
                    if let Some(owner) = state.consumer.as_mut().filter(|owner| owner.work.label == label) { owner.connect_pending = false; }
                }
                return Err("A Nearby connection is already in progress".into());
            }
            return tokio::select! { _ = cancel.cancelled() => Err("Nearby connection stopped".into()), result = response => result.map_err(|_|"Nearby connection ended")? };
        }
        if command != "nearby_discover" { return Err("Unknown Nearby command".into()); }
        let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let existing = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.consumer.as_ref()
            .filter(|owner|owner.work.label==label && !owner.work.cancel.is_cancelled())
            .map(|owner|(owner.work.clone(),owner.status.phase.clone(),owner.scanning,owner.descriptor.is_some()));
        if let Some((work,phase,scanning,accepted)) = existing {
            if (!enabled && phase=="starting") || (enabled && phase=="failed" && !accepted) {
                let _gate = self.nearby.gate.lock().await;
                self.nearby_stop_owner(Some(&work.id)).await?;
            } else {
                if enabled || scanning {
                    self.nearby_call(&work,"nearbyBluetoothScan",json!({"owner":work.id,"enabled":enabled})).await?;
                    if let Some(current) = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&work.id) { current.scanning = enabled; }
                }
                return self.nearby_status();
            }
        }
        if !enabled { return self.nearby_status(); }
        if !cfg!(mobile) { return self.nearby_status(); }
        if !self.nearby.native_ready.load(Ordering::SeqCst) { return Err("Native Nearby listeners are not ready yet. Try again.".into()); }
        let _mode = self.privacy_gate.read().await;
        let _gate = self.nearby.gate.lock().await;
        let old = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?.consumer.as_ref().map(|owner|(owner.work.id.clone(),owner.status.phase.clone(),owner.work.cancel.is_cancelled()));
        if let Some((id,phase,cancelled)) = old {
            if !cancelled && phase != "failed" { return self.nearby_status(); }
            self.nearby_stop_owner(Some(&id)).await?;
        }
        let (work,events,actions) = self.nearby_insert(label,false)?;
        let service = self.clone(); let task_work = work.clone();
        let task = tokio::spawn(async move {
            let result = service.nearby_discover(&task_work,events,actions).await;
            if let Err(error) = result { if !task_work.cancel.is_cancelled() { service.nearby_status_update(&task_work,"failed",Some(error)); } }
            task_work.cancel.cancel();
            #[cfg(target_os = "android")]
            {
                let claim = service.nearby.inner.lock().is_ok_and(|state|state.owner(&task_work.id).is_some_and(|owner|owner.hotspot_claim));
                if claim { let _ = service.nearby_release_hotspot_if_unused().await; }
            }
            let _ = service.nearby_native("nearbyBluetoothStop",json!({"owner":task_work.id})).await;
        });
        self.nearby_task(&work,task);
        self.nearby_status()
    }
    async fn nearby_next(&self, owner: &WorkOwner, events: &mut mpsc::Receiver<BleEvent>, deadline: Instant) -> Result<BleEvent,String> {
        loop {
            self.nearby_live(owner)?;
            tokio::select! {
                biased;
                _ = owner.cancel.cancelled() => return Err("Nearby connection stopped".into()),
                _ = tokio::time::sleep_until(deadline.into()) => return Err("The nearby sender did not respond. Try again.".into()),
                event = events.recv() => return event.ok_or_else(||"Bluetooth connection ended".into()),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        }
    }
    async fn nearby_publish(self: &Arc<Self>, owner: &WorkOwner, mut public: PublicLibrary, mut encoded: zeroize::Zeroizing<Vec<u8>>,
        mut sharing_cancel: CancellationToken, mut events: mpsc::Receiver<BleEvent>) -> Result<(),String> {
        crate::nearby_log!("[nearby] publisher bluetooth start");
        self.nearby_call(owner,"nearbyBluetoothStart",json!({"owner":owner.id,"role":"peripheral","publicId":public.public_id,"name":public.name})).await?;
        crate::nearby_log!("[nearby] publisher bluetooth started");
        self.nearby_status_update(owner,"advertising",None);
        let mut active: Option<PublicExchange> = None;
        let mut reverse_attempted = false;
        loop {
            let deadline = active.as_ref().map(|exchange|exchange.deadline).unwrap_or_else(||Instant::now()+Duration::from_secs(60));
            let event = tokio::select! {
                _ = sharing_cancel.cancelled() => return Ok(()),
                event = self.nearby_next(owner,&mut events,deadline) => event,
            };
            let event = match event {
                Ok(event) => event,
                Err(_) if self.nearby_live(owner).is_ok() => {
                    if let Some(exchange) = active.take() { let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":exchange.connection})).await; }
                    continue;
                }
                Err(error) => return Err(error),
            };
            match event.kind.as_str() {
                "connected" if active.is_none() => {
                    if let (Some(connection),Some(budget)) = (event.connection,event.packet_budget) {
                        if let Ok(framing) = Framing::new(budget) {
                            active = Some(PublicExchange { connection,framing,deadline:Instant::now()+Duration::from_secs(15),delivered:false,hosting:false });
                        }
                    }
                }
                "bytes" => {
                    let Some(exchange) = active.as_mut() else { continue; };
                    if event.connection.as_deref()!=Some(exchange.connection.as_str()) { continue; }
                    let result = decode_packet(event.bytes.as_deref()).and_then(|bytes|exchange.framing.receive(Kind::Request,&bytes).map_err(|error|error.to_string()));
                    match result {
                        Ok(Some(request)) if !exchange.delivered && request.as_slice()==public.public_id.as_bytes() => {
                            if sharing_cancel.is_cancelled() { return Ok(()); }
                            let packets = exchange.framing.fragment(Kind::Metadata,&encoded).map_err(|error|error.to_string())?;
                            exchange.delivered = true;
                            // Queue capacity is not indication delivery. Receiver ACK
                            // retires only this exchange, never the public share.
                            exchange.deadline = Instant::now()+Duration::from_secs(45);
                            if self.nearby_send(owner,&exchange.connection,packets).await.is_err() {
                                let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":exchange.connection})).await;
                                active = None; self.nearby_live(owner)?;
                            }
                        }
                        Ok(Some(request)) if exchange.delivered && request.as_slice()==b"done" => {
                            let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":exchange.connection})).await;
                            active = None;
                        }
                        Ok(Some(request)) if exchange.delivered && !exchange.hosting && public.can_join
                            && !reverse_attempted && request.as_slice()==b"hosting" => {
                            // Waiting is passive. No network configuration is performed
                            // until a complete, bounded current-exchange offer arrives.
                            exchange.hosting = true;
                            exchange.deadline = Instant::now()+Duration::from_secs(120);
                        }
                        Ok(Some(request)) if exchange.delivered && exchange.hosting && public.can_join && !reverse_attempted => {
                            let request = serde_json::from_slice::<HostRequest>(&request).ok().filter(|request|request.validate(&public).is_ok());
                            let Some(request) = request else {
                                let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":exchange.connection})).await;
                                active = None; continue;
                            };
                            // Share all authorizes at most one reverse-join OS prompt
                            // per active owner. Public subscribers cannot prompt-spam.
                            reverse_attempted = true; public.can_join = false;
                            let result = self.nearby_reverse_publish(owner,&public,&request,sharing_cancel.clone()).await;
                            let reply = match result {
                                Ok(descriptor) => {
                                    self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&owner.id)
                                        .ok_or("Nearby sharing stopped")?.library_cancel = Some(descriptor.cancel.clone());
                                    public.addresses = descriptor.addresses; public.port = descriptor.port;
                                    public.join = Some(request.join); sharing_cancel = descriptor.cancel;
                                    self.nearby_status_update(owner,"advertising",None);
                                    ReverseReply { library:Some(public.clone()),error:None }
                                }
                                Err(error) => {
                                    let _ = self.nearby_native("nearbyJoinCancel",json!({"owner":owner.id})).await;
                                    {
                                        let mut state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
                                        if let Some(current) = state.owner_mut(&owner.id) {
                                            if let Some(cancel) = current.selected_cancel.take() { cancel.cancel(); }
                                            current.join_lease = None; current.reverse_route = None;
                                        }
                                    }
                                    self.nearby_status_update(owner,"advertising",Some(error.clone()));
                                    ReverseReply { library:None,error:Some(error.chars().take(256).collect()) }
                                }
                            };
                            encoded = public.encode()?;
                            let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&reply).map_err(|error|error.to_string())?);
                            if bytes.len()>4096 { return Err("The temporary-network response is too large".into()); }
                            let packets = exchange.framing.fragment(Kind::Metadata,&bytes).map_err(|error|error.to_string())?;
                            exchange.deadline = Instant::now()+Duration::from_secs(30);
                            if self.nearby_send(owner,&exchange.connection,packets).await.is_err() {
                                let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":exchange.connection})).await;
                                active = None; self.nearby_live(owner)?;
                            }
                        }
                        Ok(None) => {},
                        _ => {
                            let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":exchange.connection})).await;
                            active = None;
                        }
                    }
                }
                "disconnected" => { if active.as_ref().is_some_and(|exchange|event.connection.as_deref()==Some(exchange.connection.as_str())) { active = None; } }
                "failed" => return Err(event.error.unwrap_or_else(||"Bluetooth is unavailable. Check its permissions.".into())),
                _ => {},
            }
        }
    }
    async fn nearby_reverse_publish(&self, owner: &WorkOwner, public: &PublicLibrary, request: &HostRequest,
        prior_cancel: CancellationToken) -> Result<LibraryPeerDescriptor,String> {
        if !cfg!(target_os = "ios") { return Err("This sender cannot join a receiver-hosted temporary network".into()); }
        self.nearby_live(owner)?;
        if prior_cancel.is_cancelled() { return Err("Library sharing stopped".into()); }
        let route_cancel = owner.cancel.child_token();
        {
            let mut state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
            state.owner_mut(&owner.id).ok_or("Nearby sharing stopped")?.selected_cancel = Some(route_cancel.clone());
        }
        self.nearby_status_update(owner,"joining",None);
        let join = self.nearby_call(owner,"nearbyJoin",json!({"owner":owner.id,"reservationId":request.join.reservation_id,
            "ssid":request.join.ssid,"security":"wpa2","passphrase":request.join.passphrase.as_str()}));
        let reply = tokio::select! { _ = prior_cancel.cancelled() => Err("Library sharing stopped".into()), result = join => result };
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => { route_cancel.cancel(); let _ = self.nearby_native("nearbyJoinCancel",json!({"owner":owner.id})).await; return Err(error); }
        };
        let lease = text(&reply,"lease")?.to_string();
        let route = crate::transfer_policy::JoinedRoute::new(owner.id.clone(),lease.clone(),route_cancel.clone())?;
        {
            let mut state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
            state.owner_mut(&owner.id).ok_or("Nearby sharing stopped")?.join_lease = Some(lease);
        }
        let endpoint = format!("{}:{}",request.address,public.port).parse().map_err(|_|"Invalid temporary-network endpoint")?;
        let (socket,addresses) = route.listener_socket(endpoint).await?;
        self.nearby_live(owner)?;
        let manager = self.localswarm_manager(Some(owner.admission_generation)).await?;
        let descriptor = manager.rebind_library_on_socket(&public.session_id,socket,addresses,route_cancel).await?;
        self.nearby_live(owner)?;
        route.check()?;
        self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&owner.id)
            .ok_or("Nearby sharing stopped")?.reverse_route = Some(route);
        Ok(descriptor)
    }
    fn nearby_peer(&self, owner: &WorkOwner, event: BleEvent) {
        let Some(native) = event.peer else { return; };
        let mut state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
        let Some(current) = state.owner_mut(&owner.id) else { return; };
        let name = event.name.filter(|name|!name.is_empty() && name.len()<=256 && !name.chars().any(char::is_control));
        if let Some(peer) = current.peers.iter_mut().find(|peer|peer.native==native) {
            // A later BLE scan response can supply the initially absent name.
            // Keep the original row identity and any accepted catalog identity.
            if peer.name.is_none() && peer.library_session_id.is_none() && name.is_some() {
                peer.publisher_name = name.clone(); peer.name = name;
            }
            return;
        }
        if current.peers.len() >= 16 { return; }
        let id = Uuid::new_v4().to_string();
        crate::nearby_log!("[nearby] peer native={native} name={name:?}");
        current.peers.push(PublicPeer { id:id.clone(),peer_id:id,transport:"quic-library",discovery:"bluetooth",publisher_name:name.clone(),name,library_session_id:None,native });
    }
    fn nearby_selected_native(&self, owner: &WorkOwner) -> Option<String> {
        let mut state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
        let current = state.owner_mut(&owner.id)?;
        let selected = current.status.peer_id.clone()?;
        current.peers.iter().find(|peer| peer.id == selected).map(|peer| peer.native.clone())
    }
    fn nearby_lost(&self, owner: &WorkOwner, event: BleEvent) {
        let Some(native) = event.peer else { return; };
        let mut state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
        crate::nearby_log!("[nearby] peer lost native={native}");
        if let Some(current) = state.owner_mut(&owner.id) {
            current.peers.retain(|peer| peer.native != native);
        }
    }
    async fn nearby_discover(self: &Arc<Self>, owner: &WorkOwner, mut events: mpsc::Receiver<BleEvent>, mut actions: mpsc::Receiver<Select>) -> Result<(),String> {
        self.nearby_call(owner,"nearbyBluetoothStart",json!({"owner":owner.id,"role":"central"})).await?;
        self.nearby_status_update(owner,"discovering",None);
        if let Some(current) = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&owner.id) { current.scanning = true; }
        loop {
            self.nearby_live(owner)?;
            tokio::select! {
                _ = owner.cancel.cancelled() => return Ok(()),
                action = actions.recv() => {
                    let action = action.ok_or("Nearby discovery stopped")?;
                    let result = self.nearby_connect_selected(owner,&action.peer,&mut events).await;
                    if let Err(error) = &result {
                        let hotspot_claim = {
                            let mut state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
                            state.owner_mut(&owner.id).is_some_and(|current| {
                                if let Some(cancel) = current.selected_cancel.take() { cancel.cancel(); }
                                current.descriptor = None; current.join_lease = None;
                                std::mem::take(&mut current.hotspot_claim)
                            })
                        };
                        #[cfg(target_os = "android")]
                        if hotspot_claim { let _ = self.nearby_release_hotspot_if_unused().await; }
                        #[cfg(not(target_os = "android"))]
                        let _ = hotspot_claim;
                        crate::nearby_log!("[nearby] connect failed cancel: {error}");
                        let _ = self.nearby_native("nearbyJoinCancel",json!({"owner":owner.id})).await;
                        self.nearby_status_update(owner,"failed",Some(error.clone()));
                    }
                    if let Ok(mut state) = self.nearby.inner.lock() {
                        if let Some(current) = state.owner_mut(&owner.id) { current.connect_pending = false; }
                    }
                    let _ = action.reply.send(result);
                }
                event = events.recv() => {
                    let event = event.ok_or("Bluetooth discovery stopped")?;
                    match event.kind.as_str() {
                        "peer" => self.nearby_peer(owner,event),
                        "lost" => self.nearby_lost(owner,event),
                        "failed" => {
                            let mut state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
                            let current = state.owner_mut(&owner.id).ok_or("Nearby discovery stopped")?;
                            current.scanning = false;
                            if current.descriptor.is_some() {
                                current.status.error = Some("Bluetooth discovery stopped; the accepted library connection remains available.".into());
                            } else { return Err(event.error.unwrap_or_else(||"Bluetooth discovery is unavailable".into())); }
                        }
                        _ => {},
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    let descriptor = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner(&owner.id).and_then(|current|current.descriptor.clone());
                    if let Some(descriptor) = descriptor {
                        if let Err(error) = descriptor.check() {
                            // A rejected read-only proof is not confirmed loss.
                            // Only the native route/authority revocation or an
                            // explicit unavailable response retires the owner.
                            let confirmed = descriptor.cancel.is_cancelled() || descriptor.authority.check().is_err();
                            if confirmed {
                                let claim = {
                                    let mut state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
                                    state.owner_mut(&owner.id).is_some_and(|current| {
                                        if let Some(cancel) = current.selected_cancel.take() { cancel.cancel(); }
                                        current.descriptor = None; current.join_lease = None;
                                        current.status.phase = "failed".into();
                                        current.status.error = Some("The selected Nearby network connection ended. Connect again.".into());
                                        std::mem::take(&mut current.hotspot_claim)
                                    })
                                };
                                #[cfg(target_os = "android")]
                                if claim { let _ = self.nearby_release_hotspot_if_unused().await; }
                                #[cfg(not(target_os = "android"))]
                                let _ = claim;
                            } else {
                                crate::nearby_log!("[nearby] transient route proof failed: {error}");
                            }
                        }
                    }
                },
            }
        }
    }
    async fn nearby_connect_selected(self: &Arc<Self>, owner: &WorkOwner, peer_id: &str, events: &mut mpsc::Receiver<BleEvent>) -> Result<Value,String> {
        self.nearby_live(owner)?;
        let (native,hosted) = {
            let mut state = self.nearby.inner.lock().map_err(|_| "Nearby state unavailable")?;
            let current = state.owner_mut(&owner.id).ok_or("Nearby discovery stopped")?;
            let native = current.peers.iter().find(|peer|peer.id==peer_id).ok_or("Nearby sender is no longer available")?.native.clone();
            if let Some(descriptor) = current.descriptor.take() { descriptor.cancel.cancel(); }
            for peer in &mut current.peers { peer.library_session_id = None; }
            current.join_lease = None;
            if let Some(cancel) = current.selected_cancel.take() { cancel.cancel(); }
            current.status.peer_id = Some(peer_id.to_string());
            (native,std::mem::take(&mut current.hotspot_claim))
        };
        #[cfg(target_os = "android")]
        if hosted { self.nearby_release_hotspot_if_unused().await?; }
        #[cfg(not(target_os = "android"))]
        let _ = hosted;
        crate::nearby_log!("[nearby] connect selected cancel previous peer={peer_id}");
        self.nearby_call(owner,"nearbyJoinCancel",json!({"owner":owner.id})).await?;
        self.nearby_status_update(owner,"connecting",None);
        crate::nearby_log!("[nearby] connect peer={peer_id} native={native}");
        let reply = self.nearby_call(owner,"nearbyBluetoothConnect",json!({"owner":owner.id,"peer":native})).await?;
        let connection = text(&reply,"connection")?.to_string();
        crate::nearby_log!("[nearby] ble connected connection={connection}");
        let result = self.nearby_connect_exchange(owner,&connection,&reply,events).await;
        if let Err(error) = &result { crate::nearby_log!("[nearby] connect exchange failed: {error}"); }
        let _ = self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":connection})).await;
        let (descriptor,public) = result?;
        self.nearby_live(owner)?;
        if let Err(error) = descriptor.check() {
            let confirmed = descriptor.cancel.is_cancelled() || descriptor.authority.check().is_err();
            if confirmed { return Err(error); }
            crate::nearby_log!("[nearby] transient outer route proof failed: {error}");
        }
        let mut state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
        let current = state.owner_mut(&owner.id).ok_or("Nearby connection stopped")?;
        let peer = current.peers.iter_mut().find(|peer|peer.id==peer_id).ok_or("Nearby sender stopped")?;
        peer.library_session_id = Some(public.session_id.clone()); peer.name = Some(public.name.clone()); peer.publisher_name = Some(public.name.clone());
        current.descriptor = Some(descriptor); current.status.phase = "connected".into(); current.status.error = None;
        Ok(json!({"peerId":peer_id,"librarySessionId":public.session_id,"publisherName":public.name}))
    }
    #[cfg_attr(not(target_os = "android"), expect(dead_code, reason = "Android hotspot retirement must retain a reservation claimed by Nearby"))]
    pub(super) fn nearby_uses_hotspot(&self) -> bool {
        let state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
        let holds_reservation = |owner: &Owner| owner.hotspot_claim && !owner.work.cancel.is_cancelled();
        state.publisher.as_ref().is_some_and(holds_reservation)
            || state.consumer.as_ref().is_some_and(holds_reservation)
    }
    async fn nearby_connect_exchange(self: &Arc<Self>, owner: &WorkOwner, connection: &str, reply: &Value,
        events: &mut mpsc::Receiver<BleEvent>) -> Result<(LibraryPeerDescriptor,PublicLibrary),String> {
        let (public,mut framing) = self.nearby_read_selected(owner,connection,reply,events).await?;
        let cancel = owner.cancel.child_token();
        self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&owner.id)
            .ok_or("Nearby connection stopped")?.selected_cancel = Some(cancel.clone());
        let may_reverse = cfg!(target_os = "android") && public.can_join && public.join.is_none();
        if !may_reverse {
            let packets = framing.fragment(Kind::Request,b"done").map_err(|error|error.to_string())?;
            let _ = self.nearby_send(owner,connection,packets).await;
            self.nearby_call(owner,"nearbyBluetoothDisconnect",json!({"owner":owner.id,"connection":connection})).await?;
        }
        let route = if let Some(join) = &public.join {
            self.nearby_status_update(owner,"joining",None);
            let reply = self.nearby_call(owner,"nearbyJoin",json!({"owner":owner.id,"reservationId":join.reservation_id,
                "ssid":join.ssid,"security":"wpa2","passphrase":join.passphrase.as_str()})).await?;
            let lease = text(&reply,"lease")?.to_string();
            let route = crate::transfer_policy::JoinedRoute::new(owner.id.clone(),lease.clone(),cancel.clone())?;
            self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&owner.id).ok_or("Nearby connection stopped")?.join_lease = Some(lease);
            // The join invoke already resolved on a verified association. A
            // second immediate native proof can race Wi-Fi info settling and
            // would cancel the join; the retrying browse and final descriptor
            // check still validate the route before it is used.
            Some(route)
        } else { None };
        let make_descriptor = |public: &PublicLibrary,route| LibraryPeerDescriptor {
            session_id:public.session_id.clone(),peer_id:public.peer_id.clone(),name:public.name.clone(),port:public.port,
            addresses:public.addresses.clone(),native_services:Vec::new(),direct:false,route,cancel:cancel.clone(),authority:Default::default(),
        };
        let joined = route.is_some();
        self.nearby_status_update(owner,"connecting",None);
        let descriptor = make_descriptor(&public,route);
        let direct = if public.addresses.is_empty() { Err("The sender has no common local-network endpoint".into()) }
            else if !joined {
                // No joined network to settle (including the reverse fallback):
                // retain the original single direct-probe budget.
                tokio::time::timeout(Duration::from_secs(10),
                    crate::localswarm::library::browse(&descriptor,cancel.child_token()))
                    .await.map_err(|_|"The sender is not reachable on the current Wi-Fi".to_string())
                    .and_then(|result|result.map(|_|()))
            } else {
                // The join resolves on a verified SSID/BSSID, but DHCP and the
                // sender's listener can lag. Retry within one absolute window
                // instead of cancelling the join and prompting for Wi-Fi again.
                let deadline = Instant::now() + Duration::from_secs(30);
                let mut last: Option<String> = None;
                loop {
                    if cancel.is_cancelled() {
                        break Err("The selected Nearby network connection ended. Connect again.".into());
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break Err(last.unwrap_or_else(|| "The sender is not reachable on the current Wi-Fi".into()));
                    }
                    match tokio::time::timeout(remaining.min(Duration::from_secs(10)),
                        crate::localswarm::library::browse(&descriptor,cancel.child_token())).await {
                        Ok(Ok(_)) => break Ok(()),
                        Ok(Err(error)) => last = Some(error),
                        Err(_) => last = Some("The sender is not reachable on the current Wi-Fi".into()),
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break Err(last.unwrap_or_else(|| "The sender is not reachable on the current Wi-Fi".into()));
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => break Err("The selected Nearby network connection ended. Connect again.".into()),
                        _ = tokio::time::sleep(remaining.min(Duration::from_millis(500))) => {},
                    }
                }
            };
        let (descriptor,public) = if let Err(error) = direct {
            crate::nearby_log!("[nearby] direct route failed: {error}");
            if !may_reverse { return Err(error); }
            #[cfg(target_os = "android")]
            {
                let (rebound,host_route) = self.nearby_reverse_receive(owner,connection,&public,&mut framing,events,cancel.clone()).await?;
                let public = rebound;
                let descriptor = make_descriptor(&public,Some(host_route));
                self.nearby_status_update(owner,"connecting",None);
                crate::localswarm::library::browse(&descriptor,cancel.child_token()).await.map(|_|())?;
                (descriptor,public)
            }
            #[cfg(not(target_os = "android"))]
            return Err(error);
        } else { (descriptor,public) };
        if may_reverse {
            let packets = framing.fragment(Kind::Request,b"done").map_err(|error|error.to_string())?;
            let _ = self.nearby_send(owner,connection,packets).await;
        }
        self.nearby_live(owner)?;
        if let Err(error) = descriptor.check() {
            let confirmed = descriptor.cancel.is_cancelled() || descriptor.authority.check().is_err();
            if confirmed { return Err(error); }
            crate::nearby_log!("[nearby] transient final route proof failed: {error}");
        }
        Ok((descriptor,public))
    }
    #[cfg(target_os = "android")]
    async fn nearby_reverse_receive(self: &Arc<Self>, owner: &WorkOwner, connection: &str, public: &PublicLibrary, framing: &mut Framing,
        events: &mut mpsc::Receiver<BleEvent>,cancel:CancellationToken) -> Result<(PublicLibrary,crate::transfer_policy::JoinedRoute),String> {
        self.nearby_live(owner)?;
        let hosting = framing.fragment(Kind::Request,b"hosting").map_err(|error|error.to_string())?;
        if let Err(error) = self.nearby_send(owner,connection,hosting).await {
            crate::nearby_log!("[nearby] hosting request failed: {error}");
            return Err(error);
        }
        crate::nearby_log!("[nearby] hosting request sent");
        self.nearby_status_update(owner,"joining",None);
        {
            let _hotspot_use = self.hotspot_use.lock().await;
            self.nearby_live(owner)?;
            self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?.owner_mut(&owner.id)
                .ok_or("Nearby connection stopped")?.hotspot_claim = true;
            if !self.mobile_hotspot_hosting() {
                tokio::select! {
                    _ = owner.cancel.cancelled() => return Err("Nearby connection stopped".into()),
                    result = tokio::time::timeout(Duration::from_secs(90),self.mobile_hotspot_start()) => {
                        result.map_err(|_|"Android did not finish creating the temporary Wi-Fi network")??;
                    }
                }
            }
        }
        let (socket,_,host_cancel) = self.nearby_library_socket(&owner.label).await?
            .ok_or("Android has no app-owned temporary hotspot")?;
        drop(socket);
        crate::nearby_log!("[nearby] reverse hotspot ready; sending join request");
        let request = {
            let state = self.nearby.inner.lock().map_err(|_|"Nearby state unavailable")?;
            let hotspot = state.hotspot.as_ref().filter(|hotspot|!hotspot.cancel.is_cancelled()).ok_or("The temporary hotspot stopped")?;
            HostRequest { version:1,public_id:public.public_id.clone(),address:hotspot.address.clone(),join:hotspot.join.clone() }
        };
        request.validate(public)?;
        let host_route = crate::transfer_policy::JoinedRoute::hotspot(owner.id.clone(),request.join.reservation_id.clone(),cancel.clone())?;
        let watch = cancel.clone();
        tokio::spawn(async move { tokio::select! { _ = host_cancel.cancelled() => watch.cancel(), _ = watch.cancelled() => {} } });
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&request).map_err(|error|error.to_string())?);
        self.nearby_send(owner,connection,framing.fragment(Kind::Request,&bytes).map_err(|error|error.to_string())?).await?;
        crate::nearby_log!("[nearby] join request sent; awaiting sender reply");
        let bytes = tokio::select! {
            _ = cancel.cancelled() => return Err("The temporary Wi-Fi connection ended while the sender was joining".into()),
            bytes = self.nearby_read_metadata(owner,connection,framing,events,Instant::now()+Duration::from_secs(150)) => bytes?,
        };
        let reply: ReverseReply = serde_json::from_slice(&bytes).map_err(|_|"Invalid sender temporary-network response")?;
        crate::nearby_log!("[nearby] sender reverse reply received library={} error={:?}", reply.library.is_some(), reply.error.is_some());
        let rebound = match (reply.library,reply.error) {
            (Some(library),None) => library,
            (None,Some(error)) if error.len()<=1024 => return Err(format!("The sender could not join the temporary Wi-Fi network: {error}")),
            _ => return Err("Invalid sender temporary-network response".into()),
        };
        rebound.validate(&public.public_id)?;
        if rebound.session_id!=public.session_id || rebound.peer_id!=public.peer_id { return Err("The sender's public library changed while connecting".into()); }
        host_route.check()?;
        Ok((rebound,host_route))
    }
    async fn nearby_read_selected(&self, owner: &WorkOwner, connection: &str, reply: &Value, events: &mut mpsc::Receiver<BleEvent>) -> Result<(PublicLibrary,Framing),String> {
        let info = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text(reply,"info")?).map_err(|_|"Invalid public Nearby information")?;
        if info.len()!=18 || info[..2]!=[2,0] { return Err("This sender uses an unsupported Nearby version".into()); }
        let public_id = Uuid::from_slice(&info[2..]).map_err(|_|"Invalid public Nearby share identity")?.to_string();
        let budget = reply.get("packetBudget").and_then(Value::as_u64).and_then(|budget|u16::try_from(budget).ok()).ok_or("Invalid Bluetooth packet budget")?;
        let mut framing = Framing::new(budget).map_err(|error|error.to_string())?;
        self.nearby_send(owner,connection,framing.fragment(Kind::Request,public_id.as_bytes()).map_err(|error|error.to_string())?).await?;
        let bytes = self.nearby_read_metadata(owner,connection,&mut framing,events,Instant::now()+Duration::from_secs(15)).await?;
        let public: PublicLibrary = serde_json::from_slice(&bytes).map_err(|_|"Invalid public Nearby connection information")?;
        public.validate(&public_id)?;
        Ok((public,framing))
    }
    async fn nearby_read_metadata(&self, owner: &WorkOwner, connection: &str, framing: &mut Framing,
        events: &mut mpsc::Receiver<BleEvent>, deadline: Instant) -> Result<zeroize::Zeroizing<Vec<u8>>,String> {
        loop {
            let event = self.nearby_next(owner,events,deadline).await?;
            if event.kind=="peer" { self.nearby_peer(owner,event); continue; }
            if event.kind=="lost" {
                // A `lost` advertisement names a peer, not a connection. Only
                // the sender being connected to ends this exchange; another
                // device leaving range (or a stale failure from an earlier
                // attempt) just drops its row.
                let selected = event.peer.is_some() && self.nearby_selected_native(owner) == event.peer;
                let error = event.error.clone();
                self.nearby_lost(owner,event);
                if selected { return Err(error.unwrap_or_else(||"The selected sender disconnected before sending connection information".into())); }
                continue;
            }
            if event.connection.as_deref().is_some_and(|id|id!=connection) { continue; }
            match event.kind.as_str() {
                "bytes" => if let Some(bytes) = framing.receive(Kind::Metadata,&decode_packet(event.bytes.as_deref())?).map_err(|error|error.to_string())? { return Ok(bytes); },
                "connected" => {},
                _ => return Err(event.error.unwrap_or_else(||"The selected sender disconnected before sending connection information".into())),
            }
        }
    }
    /// Capture under the synchronous callback's ownership, before any spawned
    /// cleanup can be delayed behind a replacement window/owner.
    pub(crate) fn nearby_cancel_window(&self, label: &str) -> Vec<String> {
        let state = self.nearby.inner.lock().unwrap_or_else(|error|error.into_inner());
        let mut ids = Vec::new();
        for owner in [&state.publisher,&state.consumer].into_iter().flatten().filter(|owner|owner.work.label==label) {
            owner.work.cancel.cancel();
            if let Some(cancel) = &owner.library_cancel { cancel.cancel(); }
            ids.push(owner.work.id.clone());
        }
        ids
    }
    pub(crate) async fn nearby_release_owners(&self, ids: Vec<String>) -> Result<(),String> {
        let _gate = self.nearby.gate.lock().await;
        let mut errors = Vec::new();
        for id in ids { if let Err(error) = self.nearby_stop_owner(Some(&id)).await { errors.push(error); } }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
    pub(crate) async fn nearby_release_window(&self, label: &str) {
        let ids = self.nearby_cancel_window(label);
        let _ = self.nearby_release_owners(ids).await;
    }
}
fn decode_packet(encoded: Option<&str>) -> Result<Vec<u8>,String> {
    let encoded = encoded.ok_or("Bluetooth packet is missing")?;
    let bytes = STANDARD.decode(encoded).map_err(|_|"Invalid Bluetooth packet encoding")?;
    if !(9..=512).contains(&bytes.len()) || STANDARD.encode(&bytes)!=encoded { return Err("Invalid Bluetooth packet length or encoding".into()); }
    Ok(bytes)
}
