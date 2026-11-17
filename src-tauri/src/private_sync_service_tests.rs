// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
struct Profile(PathBuf);
impl Profile {
    fn new() -> Self { Self(std::env::temp_dir().join(format!("kiwix-private-native-{}", Uuid::new_v4()))) }
    fn service(&self) -> Arc<Service> { Service::new(self.0.clone(), false, false).unwrap() }
}
impl Drop for Profile { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn enabled(service: &Arc<Service>) {
    service.store.update(|state| { state.reading_sync.enabled = true; state.reading_sync.categories = Categories { bookmarks: true, positions: true }; Ok(()) }).unwrap();
}
fn manager(service: &Arc<Service>) -> Arc<PrivateSyncManager> {
    let manager = Arc::new(PrivateSyncManager::new(crate::private_sync_identity::load_or_create(service.store.data_dir()).unwrap()));
    assert!(service.private_sync.set(manager.clone()).is_ok());
    manager
}
fn receipt() -> Runtime {
    let socket = TcpSocket::new_v4().unwrap();
    Runtime { window: "main".into(), generation: 1, revision: 0, endpoints: vec!["127.0.0.1:43210".parse().unwrap()], binding: crate::transfer_policy::bind_local_tcp_socket(&socket).unwrap(), cancel: CancellationToken::new() }
}
fn session() -> Arc<PairSession> {
    Arc::new(PairSession { view: Mutex::new(PairView { id: Uuid::new_v4().to_string(), invitation: None, peer_identity: None, peer_name: None, verification_code: None, local_confirmed: false, remote_confirmed: false, expires_at: crate::timestamps::format(time::OffsetDateTime::now_utc() + time::Duration::seconds(20)), phase: "waiting".into(), error: None }), token: Mutex::new(Some(zeroize::Zeroizing::new("0".repeat(64)))), expires: Instant::now() + Duration::from_secs(20), claimed: AtomicBool::new(true), notify: tokio::sync::Notify::new(), cancel: CancellationToken::new() })
}
async fn channels(a: Arc<PrivateSyncManager>, b: Arc<PrivateSyncManager>) -> (Channel, Channel) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let address = listener.local_addr().unwrap(); let pin = b.identity.public_hex();
    let host = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        socket2::SockRef::from(&stream).set_recv_buffer_size(4096).unwrap();
        Channel::handshake(stream, &b.identity, false, None).await.unwrap()
    });
    let stream = TcpStream::connect(address).await.unwrap();
    socket2::SockRef::from(&stream).set_send_buffer_size(4096).unwrap();
    let client = Channel::handshake(stream, &a.identity, true, Some(&pin)).await.unwrap();
    (client, host.await.unwrap())
}
async fn wait_verification(session: &PairSession) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop { if session.view.lock().unwrap().phase == "verifying" { break; } tokio::task::yield_now().await; }
    }).await.unwrap();
}
#[tokio::test]
async fn both_explicit_confirmations_are_required_before_durable_enrollment() {
    let ap = Profile::new(); let bp = Profile::new(); let a = ap.service(); let b = bp.service(); enabled(&a); enabled(&b);
    let am = manager(&a); let bm = manager(&b); let ar = receipt(); let br = receipt(); let ass = session(); let bss = session();
    let (mut ac, mut bc) = channels(am.clone(), bm.clone()).await;
    let aw = a.clone(); let aam = am.clone(); let aar = ar.clone(); let aass = ass.clone();
    let at = tokio::spawn(async move { aw.private_sync_pair_exchange(&aam, &aar, &aass, &mut ac, "B".into(), aar.endpoints.clone()).await });
    let bw = b.clone(); let bbm = bm.clone(); let bbr = br.clone(); let bbss = bss.clone();
    let bt = tokio::spawn(async move { bw.private_sync_pair_exchange(&bbm, &bbr, &bbss, &mut bc, "A".into(), bbr.endpoints.clone()).await });
    wait_verification(&ass).await; wait_verification(&bss).await;
    assert_eq!(ass.view.lock().unwrap().verification_code, bss.view.lock().unwrap().verification_code);
    ass.view.lock().unwrap().local_confirmed = true; ass.notify.notify_one();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(a.store.read().unwrap().private_sync.peers.is_empty()); assert!(b.store.read().unwrap().private_sync.peers.is_empty());
    bss.view.lock().unwrap().local_confirmed = true; bss.notify.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async { at.await.unwrap().unwrap(); bt.await.unwrap().unwrap(); }).await.unwrap();
    assert_eq!(a.store.read().unwrap().private_sync.peers[0].identity, bm.identity.public_hex());
    assert_eq!(b.store.read().unwrap().private_sync.peers[0].identity, am.identity.public_hex());
    assert_eq!(ass.view.lock().unwrap().phase, "confirmed");
    assert!(ass.token.lock().unwrap().is_none()); assert!(bss.token.lock().unwrap().is_none());
}
#[tokio::test]
async fn normal_store_producers_capture_and_remote_apply_never_echoes() {
    let ap = Profile::new(); let bp = Profile::new(); let a = ap.service(); let b = bp.service();
    enabled(&a); enabled(&b);
    let bookmark = a.clone().dispatch("save_bookmark", json!({"bookmark":{"bookId":"missing","path":"A/Encoded%23literal?q=%2F#fragment","title":"Portable","note":"unicode λ"}})).await.unwrap();
    let captured = a.store.read().unwrap(); assert!(captured.reading_sync.counter > 0);
    let categories = Categories { bookmarks: true, positions: true };
    let payload = crate::reading_data::export(&captured, categories).unwrap();
    let own_counter = b.store.read().unwrap().reading_sync.counter;
    b.store.update_reading_sync(|state| { crate::reading_data::apply(state, payload.clone(), categories)?; Ok(()) }).unwrap();
    b.store.update_reading_sync(|state| { crate::reading_data::apply(state, payload, categories)?; Ok(()) }).unwrap();
    let applied = b.store.read().unwrap(); assert_eq!(applied.reading_sync.counter, own_counter); assert_eq!(applied.bookmarks.len(), 1); assert_eq!(applied.bookmarks[0].id, bookmark["id"].as_str().unwrap());
    b.store.update(|_| Ok(())).unwrap();
    assert_eq!(b.store.read().unwrap().reading_sync.counter, own_counter, "ordinary empty update must not echo remote materialization");
}
#[tokio::test]
async fn unpair_offline_cancels_owned_receipts_and_durable_identity_authority() {
    let profile = Profile::new(); let service = profile.service(); enabled(&service); let manager = manager(&service); let runtime = receipt();
    *manager.runtime.lock().unwrap() = Some(runtime.clone()); let pending = session(); *manager.session.lock().unwrap() = Some(pending.clone());
    let identity = "a".repeat(64);
    service.store.update(|state| { state.private_sync.peers.push(Peer { id: identity.clone(), identity: identity.clone(), name: "Owned peer".into(), endpoints: runtime.endpoints.clone(), enabled: true, automatic: false, last_sync: None, acknowledged_counter: 0 }); Ok(()) }).unwrap();
    service.clone().dispatch("private_sync_unpair", json!({"privacyGeneration":1,"peerId":identity})).await.unwrap();
    assert!(runtime.cancel.is_cancelled()); assert!(pending.cancel.is_cancelled()); assert!(service.store.read().unwrap().private_sync.peers.is_empty()); assert!(manager.session.lock().unwrap().is_none());
    assert!(pending.token.lock().unwrap().is_none());
    assert!(service.private_sync_trusted(&runtime, &identity).is_err());
}
#[tokio::test]
async fn private_and_stale_generation_commands_fail_before_identity_creation() {
    let profile = Profile::new(); let service = profile.service();
    assert!(service.clone().dispatch("private_sync_status", json!({})).await.is_err());
    assert!(!profile.0.join("private-reading-sync-v1").exists());
    service.clone().dispatch("save_settings", json!({"settings":{"privateMode":true}})).await.unwrap();
    assert!(service.clone().dispatch("private_sync_configure", json!({"privacyGeneration":1,"enabled":true,"categories":{"bookmarks":true,"positions":true}})).await.is_err());
    assert!(!profile.0.join("private-reading-sync-v1").exists());
}

#[tokio::test]
async fn stale_conflict_resolution_never_discards_newly_arrived_alternatives() {
    let ap = Profile::new(); let bp = Profile::new(); let a = ap.service(); let b = bp.service();
    enabled(&a); enabled(&b);
    let categories = Categories { bookmarks: true, positions: true };
    let saved = a.clone().dispatch("save_bookmark", json!({"bookmark":{"bookId":"missing","path":"A/Article","title":"Title","note":"base"}})).await.unwrap();
    let seed = crate::reading_data::export(&a.store.read().unwrap(), categories).unwrap();
    b.store.update_reading_sync(|state| { crate::reading_data::apply(state, seed, categories)?; Ok(()) }).unwrap();
    for (service, note) in [(&a, "note A"), (&b, "note B")] {
        service.clone().dispatch("save_bookmark", json!({"bookmark":{"id":saved["id"],"bookId":"missing","path":"A/Article","note":note}})).await.unwrap();
    }
    let payload = crate::reading_data::export(&b.store.read().unwrap(), categories).unwrap();
    a.store.update_reading_sync(|state| { crate::reading_data::apply(state, payload, categories)?; Ok(()) }).unwrap();
    let presented = crate::reading_data::conflicts(&a.store.read().unwrap()).into_iter().find(|value|value.field == "note").unwrap();
    let payload = crate::reading_data::export(&a.store.read().unwrap(), categories).unwrap();
    b.store.update_reading_sync(|state| { crate::reading_data::apply(state, payload, categories)?; Ok(()) }).unwrap();
    b.clone().dispatch("save_bookmark", json!({"bookmark":{"id":saved["id"],"bookId":"missing","path":"A/Article","note":"third edit"}})).await.unwrap();
    let payload = crate::reading_data::export(&b.store.read().unwrap(), categories).unwrap();
    a.store.update_reading_sync(|state| { crate::reading_data::apply(state, payload, categories)?; Ok(()) }).unwrap();
    let before = serde_json::to_value(crate::reading_data::conflicts(&a.store.read().unwrap())).unwrap();
    let stale = a.clone().dispatch("private_sync_resolve", json!({"privacyGeneration":1,"conflictId":presented.id,"alternativeId":presented.alternatives[0].id,"alternativeIds":presented.alternatives.iter().map(|value|value.id.clone()).collect::<Vec<_>>()})).await;
    assert!(stale.is_err());
    assert_eq!(serde_json::to_value(crate::reading_data::conflicts(&a.store.read().unwrap())).unwrap(), before);
    let current = crate::reading_data::conflicts(&a.store.read().unwrap()).into_iter().find(|value|value.field == "note").unwrap();
    a.clone().dispatch("private_sync_resolve", json!({"privacyGeneration":1,"conflictId":current.id,"alternativeId":current.alternatives[0].id,"alternativeIds":current.alternatives.iter().map(|value|value.id.clone()).collect::<Vec<_>>()})).await.unwrap();
    assert!(crate::reading_data::conflicts(&a.store.read().unwrap()).is_empty());
}

#[tokio::test]
async fn disconnect_before_ready_does_not_enroll_the_peer() {
    let ap = Profile::new(); let bp = Profile::new();
    let a = ap.service(); let b = bp.service(); enabled(&a); enabled(&b);
    let am = manager(&a); let runtime = receipt(); let pending = session();
    let (mut local, mut remote) = channels(am.clone(), manager(&b)).await;
    pending.view.lock().unwrap().local_confirmed = true;
    let identity = am.identity.public_hex();
    let disconnect = tokio::spawn(async move {
        assert!(matches!(remote.receive().await.unwrap(), Message::Confirm { .. }));
        remote.send(&Message::Confirm { identity }).await.unwrap();
        assert!(matches!(remote.receive().await.unwrap(), Message::Ready));
        drop(remote);
    });
    assert!(a.private_sync_pair_exchange(&am, &runtime, &pending, &mut local, "B".into(), runtime.endpoints.clone()).await.is_err());
    disconnect.await.unwrap();
    assert!(a.store.read().unwrap().private_sync.peers.is_empty());
}

#[tokio::test]
async fn local_confirm_during_fragmented_remote_confirm_keeps_noise_framing() {
    let ap = Profile::new(); let bp = Profile::new(); let a = ap.service(); let b = bp.service(); enabled(&a); enabled(&b);
    let am = manager(&a); let bm = manager(&b); let runtime = receipt(); let pending = session();
    let (mut local, mut remote) = channels(am.clone(), bm).await;
    let service = a.clone(); let owner = am.clone(); let receipt = runtime.clone(); let pair = pending.clone();
    let pairing = tokio::spawn(async move { service.private_sync_pair_exchange(&owner, &receipt, &pair, &mut local, "B".into(), receipt.endpoints.clone()).await });
    wait_verification(&pending).await;
    let (sent, header) = tokio::sync::oneshot::channel(); let (release, resume) = tokio::sync::oneshot::channel();
    let identity = am.identity.public_hex();
    let remote = tokio::spawn(async move {
        crate::private_sync::tests::send_fragmented_confirmation(&mut remote, identity, sent, resume).await;
        assert!(matches!(remote.receive().await.unwrap(), Message::Confirm { .. }));
        assert!(matches!(remote.receive().await.unwrap(), Message::Ready));
        remote.send(&Message::Ready).await.unwrap();
    });
    header.await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    pending.view.lock().unwrap().local_confirmed = true; pending.notify.notify_one();
    tokio::time::sleep(Duration::from_millis(20)).await;
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async { pairing.await.unwrap().unwrap(); remote.await.unwrap(); }).await.unwrap();
    assert_eq!(a.store.read().unwrap().private_sync.peers.len(), 1);
}

#[tokio::test]
async fn unpair_waits_for_backpressured_personal_sender_to_release_policy_lease() {
    let ap = Profile::new(); let bp = Profile::new(); let a = ap.service(); let b = bp.service(); enabled(&a); enabled(&b);
    let am = manager(&a); let bm = manager(&b); let runtime = receipt();
    *am.runtime.lock().unwrap() = Some(runtime.clone());
    let identity = bm.identity.public_hex();
    a.store.update(|state| {
        state.private_sync.peers.push(Peer { id: identity.clone(), identity: identity.clone(), name: "B".into(), endpoints: runtime.endpoints.clone(), enabled: true, automatic: false, last_sync: None, acknowledged_counter: 0 });
        for index in 0..180 {
            state.bookmarks.push(Bookmark { id: Uuid::new_v4().to_string(), book_id: "missing".into(), path: format!("A/Article{index}"), title: "Title".into(), note: "x".repeat(8000), tags: vec![], highlights: vec![], created_at: crate::timestamps::now(), legacy_book: None });
        }
        Ok(())
    }).unwrap();
    let snapshot = crate::reading_data::export(&a.store.read().unwrap(), Categories { bookmarks: true, positions: true }).unwrap();
    let (mut sender, _not_reading) = channels(am, bm).await;
    let service = a.clone(); let receipt = runtime.clone();
    let send = tokio::spawn(async move { service.private_sync_send_snapshot(&receipt, &mut sender, snapshot).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop { if a.privacy_gate.try_write().is_err() { break; } tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(!send.is_finished(), "real socket backpressure must retain the admitted sender");
    a.clone().dispatch("private_sync_unpair", json!({"privacyGeneration":1,"peerId":identity})).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), send).await.unwrap().unwrap().is_err());
    assert!(a.privacy_gate.try_write().is_ok(), "revocation returns only after personal send lease is gone");
    assert!(a.store.read().unwrap().private_sync.peers.is_empty());
}

#[tokio::test]
async fn read_only_status_never_creates_identity_or_rewrites_profile() {
    let profile = Profile::new(); let service = profile.service();
    service.clone().dispatch("save_settings", json!({"settings":{"readOnly":true}})).await.unwrap();
    let before = std::fs::read(profile.0.join("state.json")).unwrap();
    assert!(service.clone().dispatch("private_sync_status", json!({"privacyGeneration":1})).await.is_err());
    assert_eq!(std::fs::read(profile.0.join("state.json")).unwrap(), before);
    assert!(!profile.0.join("private-reading-sync-v1").exists());
}

#[tokio::test]
async fn restart_retains_causal_identity_trust_and_no_echo_bookkeeping() {
    let profile = Profile::new(); let service = profile.service(); enabled(&service);
    service.clone().dispatch("save_bookmark", json!({"bookmark":{"bookId":"missing","path":"A/Restart?q=%2F#section","title":"Preserved","note":"native durable reading data"}})).await.unwrap();
    service.store.update(|state| {
        state.private_sync.peers.push(Peer { id: "b".repeat(64), identity: "b".repeat(64), name: "Approved device".into(), endpoints: vec!["127.0.0.1:43210".parse().unwrap()], enabled: true, automatic: false, last_sync: None, acknowledged_counter: state.reading_sync.counter });
        Ok(())
    }).unwrap();
    let identity = crate::private_sync_identity::load_or_create(service.store.data_dir()).unwrap().public_hex();
    let before = service.store.read().unwrap();
    drop(service);
    let reopened = profile.service(); let after = reopened.store.read().unwrap();
    assert_eq!(after.reading_sync, before.reading_sync);
    assert_eq!(after.private_sync, before.private_sync);
    assert_eq!(after.bookmarks, before.bookmarks);
    assert_eq!(after.reference_revision, before.reference_revision);
    assert_eq!(crate::private_sync_identity::load_or_create(reopened.store.data_dir()).unwrap().public_hex(), identity);
    let mut damaged = serde_json::to_value(&after).unwrap();
    damaged["readingSync"]["counter"] = json!(0);
    assert!(store::state_from_value(damaged).is_err(), "damaged causal counters cannot be silently salvaged/reset");
    let mut damaged = serde_json::to_value(&after).unwrap();
    damaged["privateSync"]["peers"][0]["identity"] = json!("not-an-identity");
    assert!(store::state_from_value(damaged).is_err(), "damaged durable trust cannot be silently accepted");
}

#[tokio::test]
async fn queued_configure_cannot_persist_after_shutdown_starts() {
    let profile = Profile::new(); let service = profile.service();
    let before = service.store.read().unwrap().reading_sync;
    let held = service.privacy_gate.read().await;
    let operation = service.clone();
    let configure = tokio::spawn(async move { operation.dispatch("private_sync_configure", json!({"privacyGeneration":1,"enabled":true,"categories":{"bookmarks":true,"positions":true}})).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop { if service.privacy_gate.try_read().is_err() { break; } tokio::task::yield_now().await; }
    }).await.unwrap();
    let operation = service.clone(); let shutdown = tokio::spawn(async move { operation.shutdown().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !service.stopping.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
    }).await.unwrap();
    drop(held);
    assert!(configure.await.unwrap().is_err());
    shutdown.await.unwrap().unwrap();
    assert_eq!(service.store.read().unwrap().reading_sync, before);
}

#[tokio::test]
async fn explicit_position_capacity_recovery_preserves_local_reading_state() {
    let profile = Profile::new(); let service = profile.service(); enabled(&service);
    service.clone().dispatch("save_bookmark", json!({"bookmark":{"bookId":"missing","path":"A/Keep","title":"Keep","note":"keep local notes"}})).await.unwrap();
    service.clone().dispatch("record_visit", json!({"privacyGeneration":1,"bookId":"missing","path":"A/Keep","title":"Keep","scroll":640,"create":true})).await.unwrap();
    let before = service.store.read().unwrap(); assert_eq!(crate::reading_data::positions(&before).len(), 1);
    assert!(service.clone().dispatch("private_sync_clear_positions", json!({"privacyGeneration":1,"confirmed":false})).await.is_err());
    assert_eq!(service.store.read().unwrap().reading_sync, before.reading_sync);
    service.clone().dispatch("private_sync_clear_positions", json!({"privacyGeneration":1,"confirmed":true})).await.unwrap();
    let cleared = service.store.read().unwrap();
    assert_eq!(cleared.bookmarks, before.bookmarks);
    assert_eq!(serde_json::to_value(&cleared.history).unwrap(), serde_json::to_value(&before.history).unwrap());
    assert_eq!(serde_json::to_value(&cleared.tabs).unwrap(), serde_json::to_value(&before.tabs).unwrap());
    assert!(crate::reading_data::positions(&cleared).is_empty()); assert!(cleared.private_sync.peers.is_empty());
    service.store.update(|state| { state.settings.show_search_snippets = !state.settings.show_search_snippets; Ok(()) }).unwrap();
    assert!(crate::reading_data::positions(&service.store.read().unwrap()).is_empty(), "unrelated persistence cannot re-seed forgotten positions from unchanged history");
    service.clone().dispatch("record_visit", json!({"privacyGeneration":1,"bookId":"missing","path":"A/Keep","title":"Keep","scroll":720,"create":false})).await.unwrap();
    assert_eq!(crate::reading_data::positions(&service.store.read().unwrap())[0].scroll, 720.0);
}

#[cfg(unix)]
#[tokio::test]
async fn retired_connected_listener_rebinds_same_endpoint_without_sharing_live_listener() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let socket = private_sync_listener_socket().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = socket.listen(1).unwrap();
    let address = listener.local_addr().unwrap();
    let mut client = TcpStream::connect(address).await.unwrap();
    let (mut accepted, _) = listener.accept().await.unwrap();
    accepted.write_all(b"committed").await.unwrap();
    let mut bytes = [0u8; 9]; client.read_exact(&mut bytes).await.unwrap(); assert_eq!(&bytes, b"committed");
    // The accepted/server side actively closes first, exercising its local
    // endpoint's TIME_WAIT lifetime rather than merely dropping an idle listener.
    drop(accepted);
    assert_eq!(client.read(&mut [0u8; 1]).await.unwrap(), 0);
    drop(client); drop(listener);
    let replacement = private_sync_listener_socket().unwrap();
    replacement.bind(address).unwrap();
    let replacement = replacement.listen(1).unwrap();
    assert_eq!(replacement.local_addr().unwrap(), address);
    let contender = private_sync_listener_socket().unwrap();
    let failure = match contender.bind(address) {
        Err(error) => error,
        Ok(()) => match contender.listen(1) {
            Err(error) => error,
            Ok(_) => panic!("A live private listener must never share its endpoint"),
        },
    };
    assert_eq!(failure.kind(), std::io::ErrorKind::AddrInUse);
}
