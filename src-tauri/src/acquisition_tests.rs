// SPDX-License-Identifier: GPL-3.0-or-later
//! Unit tests for `acquisition`, kept beside it rather than inline.
use super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
/// A stateful native boundary for host orchestration regressions. Real
/// Rust workers, journal transitions, copy/hash/publication and Service
/// dispatch run unchanged; only the OS-owned transport is substituted.
#[cfg(all(unix, not(target_os = "ios")))]
pub(super) mod native_transport {
    use super::*;
    use crate::ios_background::{NativeRequest, NativeSnapshot};
    use std::sync::LazyLock;

    pub(super) struct Transport {
        pub snapshot: Mutex<NativeSnapshot>,
        pub starts: AtomicUsize,
        pub acknowledgements: AtomicUsize,
        pub stops: Mutex<Vec<bool>>,
        pub stop_entered: [tokio::sync::Semaphore; 2],
        pub stop_release: [Option<Arc<tokio::sync::Semaphore>>; 2],
        pub reject_cancel: bool,
    }

    fn registry() -> &'static Mutex<HashMap<String, Arc<Transport>>> {
        static REGISTRY: LazyLock<Mutex<HashMap<String, Arc<Transport>>>> =
            LazyLock::new(|| Mutex::new(HashMap::new()));
        &REGISTRY
    }

    pub(super) struct Registration(String, pub Arc<Transport>);
    impl Drop for Registration {
        fn drop(&mut self) {
            registry().lock().unwrap().remove(&self.0);
        }
    }

    pub(super) fn register(id: &str, transport: Transport) -> Registration {
        let transport = Arc::new(transport);
        assert!(registry().lock().unwrap().insert(id.into(), transport.clone()).is_none());
        Registration(id.into(), transport)
    }

    fn lookup(id: &str) -> Result<Arc<Transport>, String> {
        registry().lock().unwrap().get(id).cloned()
            .ok_or_else(|| "No test native transport is registered".into())
    }

    pub(crate) async fn snapshot(id: &str, _generation: &str) -> Result<NativeSnapshot, String> {
        Ok(lookup(id)?.snapshot.lock().unwrap().clone())
    }

    pub(crate) async fn start(request: &NativeRequest) -> Result<(), String> {
        let transport = lookup(&request.id)?;
        transport.starts.fetch_add(1, Ordering::SeqCst);
        transport.snapshot.lock().unwrap().state = "running".into();
        Ok(())
    }

    pub(crate) async fn stop(id: &str, _generation: &str, cancel: bool) -> Result<(), String> {
        let transport = lookup(id)?;
        transport.stops.lock().unwrap().push(cancel);
        let index = usize::from(cancel);
        transport.stop_entered[index].add_permits(1);
        if let Some(gate) = &transport.stop_release[index] {
            gate.acquire().await.unwrap().forget();
        }
        if cancel && transport.reject_cancel {
            return Err("Native cancellation did not acknowledge cleanup".into());
        }
        let mut snapshot = transport.snapshot.lock().unwrap();
        if cancel {
            if let Some(path) = snapshot.path.take() {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
            snapshot.state = "cancelled".into();
        } else if snapshot.state != "complete" {
            snapshot.state = "paused".into();
        }
        Ok(())
    }

    pub(crate) async fn acknowledge(id: &str, _generation: &str) -> Result<(), String> {
        let transport = lookup(id)?;
        transport.acknowledgements.fetch_add(1, Ordering::SeqCst);
        if let Some(path) = transport.snapshot.lock().unwrap().path.take() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}


#[test]
fn inner_metalink_policy_rejection_preserves_recovery_and_manual_stop_priority() {
    for (action, expected) in [
        (Signal::Run, Outcome::Interrupted),
        (Signal::Interrupt, Outcome::Interrupted),
        (Signal::Pause, Outcome::Paused),
        (Signal::Cancel, Outcome::Cancelled),
    ] {
        assert_eq!(
            settle_worker_result(Err("MOBILE_NETWORK_BLOCKED".into()), action),
            Ok(expected)
        );
    }
    assert_eq!(
        settle_worker_result(Err("Invalid Metalink".into()), Signal::Run),
        Err("Invalid Metalink".into())
    );
    for (action, expected) in [
        (Signal::Pause, Outcome::Paused),
        (Signal::Cancel, Outcome::Cancelled),
        (Signal::Interrupt, Outcome::Interrupted),
    ] {
        assert_eq!(
            settle_worker_result(Err("Download part changed during checksum".into()), action),
            Ok(expected)
        );
    }
    assert_eq!(
        settle_worker_result(Ok(Outcome::Complete), Signal::Cancel),
        Ok(Outcome::Complete)
    );
}

#[test]
fn rejects_unsafe_decoded_filenames() {
    for value in [
        "../a.zim",
        "..\\a.zim",
        "/a.zim",
        "CON.zim",
        "x.zim ",
        "x:stream.zim",
        "a\0.zim",
    ] {
        assert!(safe_name(value).is_err(), "{value:?}");
    }
    assert!(filename(&Url::parse("https://example.org/%2e%2e%2fa.zim").unwrap()).is_err());
    assert!(filename(&Url::parse("https://example.org/a%5cb.zim").unwrap()).is_err());
    assert_eq!(
        filename(&Url::parse("https://example.org/%C3%89cole.zim.meta4").unwrap()).unwrap(),
        "École.zim"
    );
}

#[tokio::test]
async fn shared_history_retains_owned_parts_without_restoring_network_authority() {
    let root = test_directory();
    let store = test_store(&root);
    let state_dir = root.join("state");
    let manager = DownloadManager::new(state_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let identity = format!("kiwix-library://{}/book", "a".repeat(32));
    let id = seed_partial(&manager, &root, identity.clone(), &zim_bytes(b'p', 32), 128);
    manager.inner.update(&id, |stored| { stored.job.interrupted = true; }).unwrap();
    drop(manager);
    let restored = DownloadManager::new(state_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    restored.resume_interrupted().await.unwrap();
    let row = restored.inner.snapshot(&id).unwrap();
    assert_eq!(row.job.status, "paused");
    assert_eq!(restored.list_snapshots().unwrap()[0].transport, DownloadTransport::Unknown,
        "Legacy whole-library HTTP used the same synthetic identity as QUIC");
    assert!(row.job.interrupted, "Internet reconnection must not consume local reauthorization");
    assert!(row.part_ownership.is_some());
    let part = paths(&row.job).unwrap().1;
    assert_eq!(fs::metadata(&part).unwrap().len(), 32);
    assert_eq!(restored.resume(&id, None).await.unwrap_err(), SHARED_BROWSE_AGAIN);
    assert_eq!(restored.queue_resume(&id, false, None, false, |_| ()).unwrap_err(), SHARED_BROWSE_AGAIN);
    assert_eq!(require_download_network(&restored.inner, &id, &identity).unwrap_err(), SHARED_BROWSE_AGAIN);
    assert_eq!(restored.start(identity, root.clone(), Some("other.zim".into()), Some(128), None).await.unwrap_err(), SHARED_BROWSE_AGAIN,
        "Ordinary HTTP admission must not consult Internet authorization for a local identity");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let internet_url = Url::parse(&format!("http://{}/fixture.zim", listener.local_addr().unwrap())).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2),
        download_request(&restored.inner, &id, internet_url, http::HeaderMap::new()))
        .await.expect("Missing local authority must fail before an Internet request").unwrap_err();
    assert_eq!(error, SHARED_BROWSE_AGAIN);
    tokio::time::timeout(Duration::from_secs(2), download_denied(&restored.inner, &id))
        .await.expect("A missing local grant must not wait on Internet policy");
    restored.cancel(&id).await.unwrap();
    assert!(!part.exists(), "Revoked grants do not discard durable cleanup ownership");
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[cfg(not(mobile))]
async fn shared_transport_provenance_survives_admission_completion_and_restore() {
    let root = test_directory();
    let store = test_store(&root);
    let state_dir = root.join("state");
    let manager = DownloadManager::new(state_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let permits = manager.inner.slots.clone().acquire_many_owned(3).await.unwrap();
    let bytes = zim_bytes(b'q', 128);
    let (grant, _) = crate::shared_library::test_download_grant(store.clone(), bytes.len() as u64);
    let job = manager.start_shared_library(grant, root.clone(), None, false).await.unwrap();
    assert_eq!(job.transport, DownloadTransport::QuicLibrary);
    manager.pause(&job.id).await.unwrap();
    drop(permits);
    drop(manager);

    let manager = DownloadManager::new(state_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    assert_eq!(manager.list_snapshots().unwrap()[0].transport, DownloadTransport::QuicLibrary);
    assert_eq!(manager.resume(&job.id, None).await.unwrap_err(), SHARED_BROWSE_AGAIN,
        "Persisted transport provenance is not network authority");
    // Model the durable checkpoint left after all range workers synced,
    // then use the real local recovery, verification and publication path.
    let (_, part) = paths(&job).unwrap();
    fs::write(&part, &bytes).unwrap();
    writable_options().open(&part).unwrap().sync_all().unwrap();
    manager.inner.update(&job.id, |stored| {
        stored.job.status = "downloading".into();
        stored.job.downloaded = 128;
        stored.transfer_url = Some(stored.job.url.clone());
        stored.etag = Some("\"stable\"".into());
        stored.segmented = true;
        stored.segments = Some(SegmentPlan {
            total: 128, validator: "\"stable\"".into(),
            ranges: vec![
                SegmentRange { start: 0, end: 63, done: 64 },
                SegmentRange { start: 64, end: 127, done: 64 },
            ],
        });
    }).unwrap();
    manager.recover_durable_local().await.unwrap();
    assert_eq!(manager.inner.snapshot(&job.id).unwrap().job.status, "complete");
    assert_eq!(fs::read(&job.destination).unwrap(), bytes);
    assert_eq!(manager.list_snapshots().unwrap()[0].transport, DownloadTransport::QuicLibrary);
    drop(manager);

    let restored = DownloadManager::new(state_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    assert_eq!(restored.list().unwrap()[0].status, "complete");
    assert_eq!(restored.list_snapshots().unwrap()[0].transport, DownloadTransport::QuicLibrary);
    let completed = restored.list_snapshots().unwrap().pop().unwrap();
    restored.remove_terminal(&job.id).unwrap();
    assert_eq!(restored.retained_snapshots().unwrap()[0].id, job.id);
    assert_eq!(restored.retained_snapshots().unwrap()[0].transport, DownloadTransport::QuicLibrary);
    let retained = restored.retained_snapshots().unwrap().pop().unwrap();
    restored.forget_terminal(&job.id).unwrap();
    assert!(restored.list_snapshots().unwrap().is_empty());
    assert!(restored.retained_snapshots().unwrap().is_empty());
    // Deterministic interleaving: a cleanup command retires the journal
    // after capture but before response/notification serialization.
    for (snapshot, status) in [(&job, "queued"), (&completed, "complete"), (&retained, "complete")] {
        let row = serde_json::to_value(snapshot).unwrap();
        assert_eq!(row["id"].as_str(), Some(job.id.as_str()));
        assert_eq!(row["status"], status);
        assert_eq!(row["transport"], "quic-library");
    }
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[cfg(not(mobile))]
async fn shared_grants_follow_actual_worker_completion_and_resumable_stop_ownership() {
    for outcome in ["complete", "paused", "cancelled", "failed"] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let bytes = zim_bytes(b'g', 128);
        let (grant, alive) = crate::shared_library::test_download_grant(store.clone(), bytes.len() as u64);
        let identity = grant.identity().unwrap();
        let id = Uuid::new_v4().to_string();
        // The sweep requires durable range/adoption proof, not merely a
        // single-response part whose visible byte count reached its total.
        let mut stored = durable_stored(&root, &id, &bytes, "downloading");
        stored.job.url = identity.clone();
        stored.transfer_url = Some(identity.clone());
        assert_eq!(durable_total(&stored), Some(bytes.len() as u64));
        assert!(durable_finalize_candidate(&stored));
        {
            let mut state = manager.inner.state().unwrap();
            state.jobs.insert(id.clone(), stored);
            manager.inner.save(&state).unwrap();
            manager.inner.shared_grants.lock().unwrap().insert(id.clone(), grant);
        }
        assert!(alive());
        assert!(!manager.has_active_shared_library_work().unwrap(),
            "A visible downloading row without a worker is not reception");
        let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let (release, releasing) = std::sync::mpsc::sync_channel(1);
        *manager.inner.finalize_gate.lock().unwrap() = Some(FinalizeGate { entered, release: releasing });
        let recovery = manager.clone();
        let sweep = tokio::spawn(async move { recovery.recover_durable_local().await });
        tokio::time::timeout(Duration::from_secs(10), entering).await
            .unwrap_or_else(|error| panic!(
                "{outcome} finalizer did not enter: {error}; sweep_finished={}; active_shared_work={:?}; row={:?}",
                sweep.is_finished(), manager.has_active_shared_library_work(), manager.inner.snapshot(&id),
            )).unwrap();
        assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "verifying");
        assert!(manager.has_active_shared_library_work().unwrap());
        if outcome == "failed" {
            fs::write(&destination, b"temporary publication blocker").unwrap();
        }
        if matches!(outcome, "paused" | "cancelled") {
            let action = if outcome == "paused" { Signal::Pause } else { Signal::Cancel };
            let stopping = manager.stop(&id, action);
            tokio::pin!(stopping);
            assert!(futures_util::poll!(&mut stopping).is_pending());
            assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, outcome);
            assert!(manager.has_active_shared_library_work().unwrap(),
                "A stop-intent row still owns its retiring finalizer");
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(10), &mut stopping).await.unwrap().unwrap();
        } else {
            release.send(()).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(10), sweep).await.unwrap().unwrap().unwrap();
        assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, outcome);
        assert!(!manager.has_active_shared_library_work().unwrap());
        match outcome {
            "complete" => {
                assert_eq!(fs::read(&destination).unwrap(), bytes);
                assert!(!part.exists());
                assert!(!alive(), "A published archive must release the actual capability");
                assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 1);
            }
            "cancelled" => {
                assert!(!part.exists());
                assert!(!alive(), "Acknowledged cancellation must release the capability");
            }
            "paused" | "failed" => {
                assert_eq!(fs::read(&part).unwrap(), bytes);
                assert!(alive(), "Resumable ownership must retain a valid capability");
                require_download_network(&manager.inner, &id, &identity).unwrap();
                if outcome == "paused" {
                    manager.cancel(&id).await.unwrap();
                    assert!(!part.exists());
                } else {
                    assert!(manager.inner.snapshot(&id).unwrap().job.error.unwrap().contains("Publishing verified archive"));
                    assert_eq!(fs::read(&destination).unwrap(), b"temporary publication blocker");
                    manager.forget_terminal(&id).unwrap();
                    assert_eq!(fs::read(&part).unwrap(), bytes,
                        "Forgetting releases network authority without deleting retained bytes");
                }
                assert!(!alive());
            }
            _ => unreachable!(),
        }
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
#[cfg(not(mobile))]
async fn shared_terminal_retirement_releases_capabilities_only_after_its_commit() {
    for action in ["remove", "forget", "delete"] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let (grant, alive) = crate::shared_library::test_download_grant(store.clone(), 128);
        let identity = grant.identity().unwrap();
        let bytes = zim_bytes(b'r', 32);
        let id = seed_partial(&manager, &root, identity.clone(), &bytes, 128);
        manager.inner.update(&id, |stored| stored.job.status = "failed".into()).unwrap();
        manager.inner.shared_grants.lock().unwrap().insert(id.clone(), grant);
        let part = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap().1;
        // A failed history commit must not revoke still-resumable ownership.
        let state_path = root.join("state/downloads.json");
        fs::remove_file(&state_path).unwrap();
        fs::create_dir(&state_path).unwrap();
        assert!(manager.remove_terminal(&id).is_err());
        assert!(!manager.inner.snapshot(&id).unwrap().history_dismissed);
        assert!(alive());
        require_download_network(&manager.inner, &id, &identity).unwrap();
        fs::remove_dir(&state_path).unwrap();
        match action {
            "remove" => {
                manager.remove_terminal(&id).unwrap();
                assert!(manager.inner.snapshot(&id).unwrap().history_dismissed);
                assert_eq!(fs::read(&part).unwrap(), bytes);
            }
            "forget" => {
                manager.forget_terminal(&id).unwrap();
                assert!(manager.inner.snapshot(&id).is_err());
                assert_eq!(fs::read(&part).unwrap(), bytes);
            }
            "delete" => {
                manager.delete_terminal(&id).await.unwrap();
                assert!(manager.inner.snapshot(&id).unwrap().files_deleted);
                assert!(!part.exists());
            }
            _ => unreachable!(),
        }
        assert!(!alive(), "{action} must release the actual capability, not just its row");
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
#[cfg(not(mobile))]
async fn a_retained_nearby_download_from_an_earlier_share_session_is_replaced_only_with_consent() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let _permits = manager.inner.slots.clone().acquire_many_owned(3).await.unwrap();
    // Production rows record the canonical destination (start_transport).
    let base = fs::canonicalize(&root).unwrap();
    let (grant, _) = crate::shared_library::test_download_grant(store.clone(), 128);
    let peer = "a".repeat(32);

    // Another device's (or another book's) retained download is never offered
    // for replacement: it may still resume by browsing that device.
    let other_peer = seed_partial(&manager, &base,
        crate::shared_library::download_identity(&"c".repeat(32), &"d".repeat(32), "book"), &zim_bytes(b'p', 32), 128);
    let error = manager.start_shared_library(grant.clone(), base.clone(), None, true).await.unwrap_err();
    assert!(!error.starts_with(SHARED_EARLIER_SESSION), "{error}");
    assert_eq!(manager.inner.snapshot(&other_peer).unwrap().job.status, "paused");
    manager.cancel(&other_peer).await.unwrap();

    // The sharer restarted: same peer and book, new session id, failed row.
    let id = seed_partial(&manager, &base,
        crate::shared_library::download_identity(&"c".repeat(32), &peer, "book"), &zim_bytes(b'p', 32), 128);
    manager.inner.state().unwrap().jobs.get_mut(&id).unwrap().job.status = "failed".into();
    let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
    // Without consent: a named refusal, and the retained row is untouched.
    let error = manager.start_shared_library(grant.clone(), base.clone(), None, false).await.unwrap_err();
    assert!(error.starts_with(SHARED_EARLIER_SESSION) && error.contains("fixture.zim"), "{error}");
    assert!(part.exists());
    // A refusal the new start would hit is checked before anything is
    // discarded, so the earlier partial download survives it.
    fs::write(&destination, b"unrelated").unwrap();
    let error = manager.start_shared_library(grant.clone(), base.clone(), None, true).await.unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert!(part.exists());
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "failed");
    fs::remove_file(&destination).unwrap();
    // With consent: the earlier download is cancelled (its part discarded)
    // and the new one is admitted for the same destination.
    let job = manager.start_shared_library(grant, base.clone(), None, true).await.unwrap();
    assert_ne!(job.id, id);
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "cancelled");
    assert!(!part.exists());
    manager.cancel(&job.id).await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[cfg(not(mobile))]
async fn shared_reauthorization_resumes_one_owned_row_and_rejects_changed_inventory() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    // Keep the scheduler stopped: this exercises admission, not a substitute transport.
    let permits = manager.inner.slots.clone().acquire_many_owned(3).await.unwrap();
    let (grant, _) = crate::shared_library::test_download_grant(store.clone(), 128);
    let identity = grant.identity().unwrap();
    let prefix = zim_bytes(b'p', 32);
    let id = seed_partial(&manager, &root, identity.clone(), &prefix, 128);
    let (_, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
    assert_eq!(manager.resume(&id, None).await.unwrap_err(), SHARED_BROWSE_AGAIN);
    let (changed, _) = crate::shared_library::test_download_grant(store.clone(), 129);
    assert!(manager.start_shared_library(changed, root.join("other"), None, false).await.unwrap_err().contains("archive changed"));
    assert_eq!(fs::read(&part).unwrap(), prefix);
    assert!(manager.inner.shared_grants.lock().unwrap().is_empty());
    let resumed = manager.start_shared_library(grant, root.join("other"), None, false).await.unwrap();
    assert_eq!(resumed.id, id);
    assert_eq!(resumed.transport, DownloadTransport::Unknown,
        "Reauthorization cannot relabel the retained historical prefix");
    assert_eq!(resumed.url, identity);
    assert_eq!(manager.list().unwrap().len(), 1);
    assert_eq!(resumed.destination, root.join("fixture.zim").to_str().unwrap());
    manager.pause(&id).await.unwrap();
    assert_eq!(fs::read(&part).unwrap(), prefix, "Admission cannot truncate a retained prefix");
    assert_eq!(manager.inner.snapshot(&id).unwrap().etag.as_deref(), Some("\"stable\""));
    manager.cancel(&id).await.unwrap();
    assert!(!part.exists());
    drop(permits); drop(manager); drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// Opt-in because the real listener binds the machine's physical LAN, not a fake response.
#[tokio::test]
#[cfg(not(mobile))]
#[ignore = "opens a real physical-LAN QUIC library listener"]
async fn quic_library_ranges_replacement_and_selected_cancellation_preserve_durable_jobs() {
    use crate::localswarm::{library::{self, LibraryServer}, LocalSwarmManager};
    use crate::shared_library::SharedLibraryClient;
    use tokio_util::sync::CancellationToken;
    let root = test_directory();
    let source = root.join("source.zim");
    fs::copy(fixture_archive(), &source).unwrap();
    let expected = fs::read(&source).unwrap();
    assert!(expected.len() > 97);
    let sender = Arc::new(Store::new(root.join("sender")).unwrap());
    register_installed_archive(&sender, &source);
    sender.update(|state| { state.books[0].metadata.name = "fixture".into(); Ok(()) }).unwrap();
    let server = LibraryServer::start_default(sender.clone(), "a".repeat(32), "QUIC test sender".into(), CancellationToken::new()).await.unwrap();
    for case in ["range", "replacement", "cancel-reauthorize"] {
        let receiver_root = root.join(case);
        fs::create_dir(&receiver_root).unwrap();
        let store = Arc::new(Store::new(receiver_root.join("profile")).unwrap());
        let manager = DownloadManager::new(receiver_root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let discovery = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
        let client = SharedLibraryClient::new(store.clone(), discovery);
        let mut descriptor = server.descriptor.clone();
        descriptor.cancel = CancellationToken::new();
        let selected_cancel = descriptor.cancel.clone();
        let inventory = client.browse_selected(descriptor.clone()).await.unwrap();
        assert_eq!(inventory.books.len(), 1);
        let book = &inventory.books[0];
        assert_eq!(book.file_name, "fixture.zim");
        let mut probe = library::open(&descriptor, &book.id, http::HeaderMap::new(), CancellationToken::new()).await.unwrap();
        let etag = probe.headers()[ETAG].to_str().unwrap().to_string();
        while probe.chunk().await.unwrap().is_some() {}
        drop(probe);
        let prefix = if case == "replacement" { vec![b'x'; 97] } else { expected[..97].to_vec() };
        let id = seed_partial(&manager, &receiver_root, book.download_identity.clone(), &prefix, expected.len() as u64);
        manager.inner.update(&id, |stored| stored.etag = Some(etag)).unwrap();
        if case == "replacement" {
            // Same archive bytes and pathname, new retained source identity: If-Range
            // must force a full response before replacing the deliberately wrong prefix.
            let replacement = root.join("replacement.zim");
            fs::copy(fixture_archive(), &replacement).unwrap();
            fs::rename(&replacement, &source).unwrap();
        }
        let permits = manager.inner.slots.clone().acquire_many_owned(3).await.unwrap();
        let job = client.download(&manager, receiver_root.clone(), &inventory.receipt_id, &book.id, None, false).await.unwrap();
        assert_eq!(job.id, id);
        assert_eq!(job.url, book.download_identity);
        if case == "cancel-reauthorize" {
            selected_cancel.cancel();
            drop(permits);
            let interrupted = finished_job(&manager, &id).await;
            assert!(matches!(interrupted.status.as_str(), "paused" | "failed"));
            let (_, part) = paths(&interrupted).unwrap();
            assert_eq!(fs::read(&part).unwrap(), prefix);
            assert!(manager.resume(&id, None).await.is_err(), "Saved URL cannot restore selected authority");
            let mut next = server.descriptor.clone();
            next.cancel = CancellationToken::new();
            let next = client.browse_selected(next).await.unwrap();
            let resumed = client.download(&manager, receiver_root.clone(), &next.receipt_id, &book.id, None, false).await.unwrap();
            assert_eq!(resumed.id, id);
        } else {
            drop(permits);
        }
        let complete = finished_job(&manager, &id).await;
        assert_eq!(complete.status, "complete", "{case}: {:?}", complete.error);
        assert_eq!(fs::read(&complete.destination).unwrap(), expected, "{case}");
        assert_eq!(manager.list().unwrap().len(), 1);
        let archive = crate::archive::Archive::open(Path::new(&complete.destination)).unwrap();
        archive.check_integrity().unwrap();
        drop(archive);
        client.invalidate();
        manager.shutdown().await.unwrap();
        drop(client); drop(manager); drop(store);
    }
    server.shutdown().await;
    drop(sender);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checks_range_boundaries_without_32_bit_truncation() {
    assert_eq!(
        parse_range("bytes 4294967296-8589934591/8589934592"),
        Some(ByteRange {
            start: 4_294_967_296,
            end: 8_589_934_591,
            total: 8_589_934_592
        })
    );
    for range in [
        "bytes 8-7/10",
        "bytes 0-10/10",
        "bytes 0-1/*",
        "items 0-1/3",
        "bytes 0-18446744073709551615/18446744073709551615",
    ] {
        assert!(parse_range(range).is_none());
    }
    assert_eq!(unsatisfied_range("bytes */9000000000"), Some(9_000_000_000));
}

#[test]
fn metalink_rejects_plaintext_and_external_filename_traversal() {
    let base = Url::parse("https://example.org/a.zim.meta4").unwrap();
    let xml = |name: &str, url: &str| {
        format!("<metalink xmlns=\"urn:ietf:params:xml:ns:metalink\"><file name=\"{name}\"><size>9000000000</size><url>{url}</url></file></metalink>")
    };
    assert!(parse_metalink(&xml("../a.zim", "https://example.org/a.zim"), &base).is_err());
    assert!(parse_metalink(&xml("a.zim", "http://example.org/a.zim"), &base).is_err());
    assert_eq!(
        parse_metalink(&xml("a.zim", "/a.zim"), &base).unwrap().size,
        9_000_000_000
    );
}

fn test_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!("kiwix-acquisition-test-{}", Uuid::new_v4()));
    crate::fs_ext::create_private_dir_all(&path).unwrap();
    fs::canonicalize(path).unwrap()
}

fn test_store(root: &Path) -> Arc<Store> {
    Arc::new(Store::new(root.join("profile")).unwrap())
}

fn fixture_archive() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")
}

fn register_installed_archive(store: &Store, destination: &Path) -> crate::store::Book {
    let book = crate::imports::load_book(destination.to_str().unwrap(), false, false).unwrap();
    store
        .update(|state| {
            state.books.push(book.clone());
            state.reader_storage.insert(
                book.installation_id.clone(),
                serde_json::json!({"localStorage":{"saved":"archive A"},"cookies":[]}),
            );
            state
                .consumed_acquisitions
                .insert("installed-receipt".into());
            Ok(())
        })
        .unwrap();
    book
}

fn zim_bytes(fill: u8, length: usize) -> Vec<u8> {
    let mut bytes = vec![fill; length];
    bytes[..4].copy_from_slice(b"ZIM\x04");
    bytes
}

/// A row whose durable journal proves every byte is present, with the part
/// already written. This is the shape left by a worker that died at 100%.
fn durable_stored(root: &Path, id: &str, bytes: &[u8], status: &str) -> StoredJob {
    let total = bytes.len() as u64;
    assert!(total >= 80);
    let destination = root.join("fixture.zim");
    let part = root.join(format!(".kiwix-{id}.part"));
    fs::write(&part, bytes).unwrap();
    let ownership =
        crate::torrent::CleanupEntry::retain(&part, &File::open(&part).unwrap()).unwrap();
    StoredJob {
        job: DownloadJob {
            id: id.to_string(),
            url: "http://127.0.0.1:9/fixture.zim".into(),
            file_name: "fixture.zim".into(),
            destination: destination.to_str().unwrap().to_string(),
            status: status.into(),
            interrupted: false,
            downloaded: total,
            total: Some(total),
            error: None,
            history_warning: None,
            speed: 0,
            eta_seconds: None,
            created_at: now(),
            updated_at: now(),
            sha256: None,
        },
        transport: None,
        expected_size: Some(total),
        transfer_url: Some("http://127.0.0.1:9/fixture.zim".into()),
        etag: Some("\"v1\"".into()),
        last_modified: None,
        publication: None,
        history_dismissed: false,
        files_deleted: false,
        part_ownership: Some(ownership),
        published_ownership: None,
        segments: Some(SegmentPlan {
            total,
            validator: "\"v1\"".into(),
            ranges: vec![
                SegmentRange {
                    start: 0,
                    end: total - 2,
                    done: total - 1,
                },
                SegmentRange {
                    start: total - 1,
                    end: total - 1,
                    done: 1,
                },
            ],
        }),
        segmented: true,
        verification: None,
        native: None,
        space_acknowledgement: None,
    }
}

#[test]
fn failed_final_history_write_preserves_publication_and_recovers_on_restart() {
    let root = test_directory();
    let store = test_store(&root);
    let folder = root.join("state");
    let manager = DownloadManager::new(folder.clone(), store.clone(), Default::default()).unwrap();
    let id = Uuid::new_v4().to_string();
    let bytes = zim_bytes(b'a', 1024);
    let stored = durable_stored(&root, &id, &bytes, "verifying");
    let (destination, part) = paths(&stored.job).unwrap();
    manager.inner.state().unwrap().jobs.insert(id.clone(), stored);
    // The first write records the recovery intent. Only the post-rename write fails.
    FAIL_JOURNAL_GENERATION.set(Some(manager.inner.journal_generation.load(Ordering::Relaxed) + 2));
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    assert_eq!(
        finalize::publish_verified_download(&manager.inner, &id, File::open(&part).unwrap(),
            bytes.len() as u64, &digest, FinalizeMode::Startup).unwrap(),
        Outcome::Complete,
    );
    let job = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(job.status, "complete");
    assert!(job.error.is_none());
    assert!(job.history_warning.as_deref().unwrap().contains("download history"));
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    drop(manager);
    let restored = DownloadManager::new(folder, store, Default::default()).unwrap();
    let job = restored.inner.snapshot(&id).unwrap().job;
    assert_eq!(job.status, "complete");
    assert!(job.error.is_none() && job.history_warning.is_none());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    drop(restored);
    fs::remove_dir_all(root).unwrap();
}


fn write_state(data_dir: &Path, stored: StoredJob) {
    fs::create_dir_all(data_dir).unwrap();
    let mut jobs = BTreeMap::new();
    jobs.insert(stored.job.id.clone(), stored);
    fs::write(
        data_dir.join("downloads.json"),
        serde_json::to_vec(&jobs).unwrap(),
    )
    .unwrap();
}

fn seed_partial(
    manager: &DownloadManager,
    root: &Path,
    url: String,
    bytes: &[u8],
    total: u64,
) -> String {
    let id = Uuid::new_v4().to_string();
    let job = DownloadJob {
        id: id.clone(),
        url: url.clone(),
        file_name: "fixture.zim".into(),
        destination: root.join("fixture.zim").to_str().unwrap().to_string(),
        status: "paused".into(),
        interrupted: false,
        downloaded: bytes.len() as u64,
        total: Some(total),
        error: None,
        history_warning: None,
        speed: 0,
        eta_seconds: None,
        created_at: now(),
        updated_at: now(),
        sha256: None,
    };
    let (_, part) = paths(&job).unwrap();
    fs::write(&part, bytes).unwrap();
    let mut state = manager.inner.state().unwrap();
    state.jobs.insert(
        id.clone(),
        StoredJob {
            job,
            transport: None,
            expected_size: Some(total),
            transfer_url: Some(url),
            etag: Some("\"stable\"".into()),
            last_modified: None,
            publication: None,
            history_dismissed: false,
            files_deleted: false,
            part_ownership: Some(crate::torrent::CleanupEntry::retain(&part, &File::open(&part).unwrap()).unwrap()),
            published_ownership: None,
            segments: None,
            segmented: false,
            verification: None,
            native: None,
            space_acknowledgement: None,
        },
    );
    manager.inner.save(&state).unwrap();
    id
}

async fn single_response(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0 && request.len() + count <= 16384);
            request.extend_from_slice(&buffer[..count]);
        }
        socket.write_all(&response).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8(request).unwrap()
    });
    (format!("http://{address}/fixture.zim"), server)
}

async fn finished_job(manager: &DownloadManager, id: &str) -> DownloadJob {
    let running = manager.inner.state().unwrap().running.get(id).cloned();
    if let Some(mut running) = running {
        // A generous deadline, not a performance assertion: the budget
        // test itself is what verifies throttling, and a loaded CI host
        // must not turn scheduler delay into a spurious failure.
        tokio::time::timeout(Duration::from_secs(60), async {
            while !*running.finished.borrow() {
                running.finished.changed().await.unwrap();
            }
        })
        .await
        .unwrap_or_else(|_| panic!("download {id} did not finish within 60s"));
    }
    manager.inner.snapshot(id).unwrap().job
}

#[tokio::test]
async fn failed_resume_checkpoint_keeps_a_retryable_row_and_owned_part() {
    for missing_part in [false, true] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let bytes = zim_bytes(b'r', 128);
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
        response.extend_from_slice(&bytes);
        let (url, server) = single_response(response).await;
        let id = seed_partial(&manager, &root, url, &bytes[..32], 128);
        manager.inner.update(&id, |stored| {
            stored.job.error = Some("Previous resumable failure".into());
        }).unwrap();
        let before = manager.inner.snapshot(&id).unwrap();
        let (destination, part) = paths(&before.job).unwrap();
        if missing_part { fs::remove_file(&part).unwrap(); }

        // A full/unwritable profile can reject the journal independently of
        // the archive volume. Fail its atomic rename after queue preparation.
        let state_path = root.join("state/downloads.json");
        fs::remove_file(&state_path).unwrap();
        fs::create_dir(&state_path).unwrap();
        assert!(manager.resume(&id, None).await.unwrap_err().contains("Publishing download state"));
        let failed = manager.inner.snapshot(&id).unwrap();
        assert_eq!(failed.job, before.job);
        assert!(!manager.inner.state().unwrap().running.contains_key(&id));
        assert!(failed.part_ownership.as_ref().unwrap().validate_existing(&part).unwrap().is_some());
        assert_eq!(fs::read(&part).unwrap(), if missing_part { Vec::new() } else { bytes[..32].to_vec() });
        assert!(!destination.exists());

        // Restoring storage and pressing Resume must use the same row and
        // actually finish a transfer without a restart or a manual pause first.
        fs::remove_dir(&state_path).unwrap();
        manager.resume(&id, None).await.unwrap();
        let completed = finished_job(&manager, &id).await;
        assert_eq!(completed.status, "complete", "{:?}", completed.error);
        let digest = hex::encode(Sha256::digest(&bytes));
        let assert_receipt = |published: Option<crate::acquisition::PublicationIdentity>| {
            assert_eq!(
                published.as_ref().map(|value| (value.size, value.sha256.as_str())),
                Some((bytes.len() as u64, digest.as_str())),
                "a completed job must expose the verified publication identity it imported"
            );
        };
        let receipt = |manager: &DownloadManager| manager.list_snapshots().unwrap().into_iter()
            .find(|snapshot| snapshot.id == id).unwrap().publication;
        assert_receipt(receipt(&manager));
        assert_eq!(completed.sha256.as_deref(), Some(digest.as_str()));
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert!(!part.exists());
        assert_eq!(manager.list().unwrap().len(), 1);
        // The receipt belongs to the durable row's own publication intent; a
        // second persisted key would serialize twice and be lost on reload.
        let persisted = fs::read_to_string(&state_path).unwrap();
        assert_eq!(persisted.matches("\"publication\":").count(), 1);
        drop(manager);
        let reopened = DownloadManager::new(
            root.join("state"),
            store.clone(),
            crate::update_admission::AdmissionGate::default(),
        )
        .unwrap();
        assert_receipt(receipt(&reopened));
        drop(reopened);
        server.await.unwrap();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn cancelled_part_capture_preserves_a_replacement_after_ownership_validation() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, "https://example.org/fixture.zim".into(), b"owned partial", 128);
    manager.inner.update(&id, |stored| stored.job.status = "cancelled".into()).unwrap();
    let stored = manager.inner.snapshot(&id).unwrap();
    let (_, part) = paths(&stored.job).unwrap();
    let original = root.join("original-part");
    let error = remove_owned_part(&part, stored.part_ownership.as_ref(), |entry| {
        manager.inner.update(&id, |stored| stored.part_ownership = Some(entry.clone()))?;
        // The public name changes after its preliminary validation and before
        // capture. Cleanup must inspect the captured entry, restore it, and fail.
        fs::rename(&part, &original).unwrap();
        fs::write(&part, b"unrelated replacement").unwrap();
        Ok(())
    }).unwrap_err();
    assert!(error.contains("unrelated entry preserved"), "{error}");
    assert_eq!(fs::read(&part).unwrap(), b"unrelated replacement");
    assert_eq!(fs::read(&original).unwrap(), b"owned partial");
    assert!(manager.inner.snapshot(&id).unwrap().part_ownership.unwrap().cleanup_started());
    fs::remove_file(&part).unwrap();
    fs::rename(&original, &part).unwrap();
    manager.cancel(&id).await.unwrap();
    assert!(!part.exists(), "Repeated cancel must finish a retained cleanup capture");
    assert!(manager.inner.snapshot(&id).unwrap().job.error.is_none());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelled_part_cleanup_recovers_its_durable_private_capture_after_restart() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, "https://example.org/fixture.zim".into(), b"owned partial", 128);
    manager.inner.update(&id, |stored| stored.job.status = "cancelled".into()).unwrap();
    let stored = manager.inner.snapshot(&id).unwrap();
    let (_, part) = paths(&stored.job).unwrap();
    let mut capture_path = None;
    // Model process death immediately after the durable capture move. No mutex
    // is held at the injected unwind, so the new manager reads the real journal.
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = remove_owned_part(&part, stored.part_ownership.as_ref(), |entry| {
            manager.inner.update(&id, |stored| stored.part_ownership = Some(entry.clone()))?;
            let value = serde_json::to_value(entry).unwrap();
            let capture = root.join(value["capture"]["captureName"].as_str().unwrap());
            fs::rename(&part, capture.join("payload")).unwrap();
            sync_directory(&capture).unwrap();
            sync_directory(&root).unwrap();
            capture_path = Some(capture);
            panic!("simulated process death after capture");
        });
    }));
    assert!(stopped.is_err());
    let capture = capture_path.unwrap();
    assert_eq!(fs::read(capture.join("payload")).unwrap(), b"owned partial");
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    assert!(!part.exists());
    assert!(!capture.exists());
    let row = restored.inner.snapshot(&id).unwrap();
    assert_eq!(row.job.status, "cancelled");
    assert!(row.job.error.is_none());
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancellation_remains_terminal_when_capture_and_worker_checkpoint_fail() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = Uuid::new_v4().to_string();
    let bytes = zim_bytes(b'c', 128);
    let stored = durable_stored(&root, &id, &bytes, "downloading");
    let (_, part) = paths(&stored.job).unwrap();
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    let (entered, entering) = tokio::sync::oneshot::channel();
    let (release, releasing) = std::sync::mpsc::sync_channel(1);
    *manager.inner.finalize_gate.lock().unwrap() = Some(FinalizeGate { entered, release: releasing });
    let recovery = manager.clone();
    let sweep = tokio::spawn(async move { recovery.recover_durable_local().await });
    tokio::time::timeout(Duration::from_secs(10), entering).await.unwrap().unwrap();
    let state_path = root.join("state/downloads.json");
    {
        let stopping = manager.cancel(&id);
        tokio::pin!(stopping);
        assert!(futures_util::poll!(&mut stopping).is_pending());
        assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "cancelled");
        // Cancellation intent committed; now its cleanup and worker-exit journal
        // checkpoints fail independently. Neither may convert Cancel to Retry.
        fs::remove_file(&state_path).unwrap();
        fs::create_dir(&state_path).unwrap();
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), &mut stopping).await.unwrap().unwrap();
    }
    sweep.await.unwrap().unwrap();
    let cancelled = manager.inner.snapshot(&id).unwrap();
    assert_eq!(cancelled.job.status, "cancelled");
    assert!(cancelled.job.error.unwrap().contains("Persisting download result"));
    assert_eq!(fs::read(&part).unwrap(), bytes);
    assert!(manager.resume(&id, None).await.unwrap_err().contains("Only paused or failed"));
    fs::remove_dir(&state_path).unwrap();
    manager.cancel(&id).await.unwrap();
    assert!(!part.exists());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn missing_installed_archive_rejects_fresh_http_acquisition_without_state_changes() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let destination = root.join("fixture.zim");
    fs::copy(fixture_archive(), &destination).unwrap();
    let installed = register_installed_archive(&store, &destination);
    fs::remove_file(&destination).unwrap();
    let before = serde_json::to_value(store.read().unwrap()).unwrap();
    let durable = fs::read(root.join("profile/state.json")).unwrap();

    let error = manager.start("http://127.0.0.1:1/fixture.zim".into(), root.clone(), None, Some(installed.metadata.size), None)
        .await
        .unwrap_err();
    assert!(error.contains("library"), "{error}");
    assert!(!destination.exists());
    assert!(manager.list().unwrap().is_empty());
    assert_eq!(serde_json::to_value(store.read().unwrap()).unwrap(), before);
    assert_eq!(fs::read(root.join("profile/state.json")).unwrap(), durable);
    assert_eq!(
        store
            .book(&installed.metadata.id)
            .unwrap()
            .unwrap()
            .installation_id,
        installed.installation_id
    );
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn library_reservation_added_during_http_transfer_blocks_verified_publication() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = fs::read(fixture_archive()).unwrap();
    let length = bytes.len() as u64;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture.zim", listener.local_addr().unwrap());
    let (requested_tx, requested_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
        }
        requested_tx.send(()).unwrap();
        release_rx.await.unwrap();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.write_all(&bytes).await.unwrap();
    });
    let job = manager.start(url, root.clone(), None, Some(length), None)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), requested_rx)
        .await
        .unwrap()
        .unwrap();
    let destination = PathBuf::from(&job.destination);
    fs::copy(fixture_archive(), &destination).unwrap();
    let installed = register_installed_archive(&store, &destination);
    fs::remove_file(&destination).unwrap();
    let before = serde_json::to_value(store.read().unwrap()).unwrap();
    let durable = fs::read(root.join("profile/state.json")).unwrap();
    release_tx.send(()).unwrap();

    let failed = finished_job(&manager, &job.id).await;
    assert_eq!(failed.status, "failed", "{:?}", failed.error);
    assert!(failed.error.as_deref().unwrap().contains("library"));
    assert!(!destination.exists());
    assert!(!store.acquisition_consumed(&job.id).unwrap());
    assert_eq!(serde_json::to_value(store.read().unwrap()).unwrap(), before);
    assert_eq!(fs::read(root.join("profile/state.json")).unwrap(), durable);
    assert_eq!(
        store
            .book(&installed.metadata.id)
            .unwrap()
            .unwrap()
            .installation_id,
        installed.installation_id
    );
    assert!(manager
        .inner
        .snapshot(&job.id)
        .unwrap()
        .publication
        .is_none());
    let (_, part) = paths(&failed).unwrap();
    assert_eq!(
        fs::read(part).unwrap(),
        fs::read(fixture_archive()).unwrap()
    );
    server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn publication_intent_recovery_never_republishes_into_missing_installed_path() {
    for (missing, missing_part) in [(false, false), (true, false), (true, true)] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let bytes = fs::read(fixture_archive()).unwrap();
        let length = bytes.len() as u64;
        let id = seed_partial(
            &manager,
            &root,
            "http://127.0.0.1:1/fixture.zim".into(),
            &bytes,
            length,
        );
        let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
        if missing {
            fs::copy(fixture_archive(), &destination).unwrap();
        } else {
            crate::publication::publish_noreplace(&part, &destination).unwrap();
        }
        register_installed_archive(&store, &destination);
        if missing {
            fs::remove_file(&destination).unwrap();
            if missing_part {
                fs::remove_file(&part).unwrap();
            }
        }
        if !missing {
            // A legitimate consumed receipt may already own this published path.
            store
                .update(|state| {
                    state.consumed_acquisitions.insert(id.clone());
                    Ok(())
                })
                .unwrap();
        }
        manager
            .inner
            .update(&id, |stored| {
                stored.job.status = "downloading".into();
                stored.publication = Some(PublicationIntent {
                    size: length,
                    sha256: hex::encode(Sha256::digest(&bytes)),
                });
            })
            .unwrap();
        let before = serde_json::to_value(store.read().unwrap()).unwrap();
        let durable = fs::read(root.join("profile/state.json")).unwrap();
        drop(manager);
        let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let recovered = restored.inner.snapshot(&id).unwrap().job;
        if missing {
            assert_eq!(recovered.status, "paused");
            let error = restored.resume(&id, None).await.unwrap_err();
            assert!(error.contains("library"), "{error}");
            assert!(!destination.exists());
            if missing_part {
                assert!(!part.exists(), "resume must not recreate a reserved part");
            } else {
                assert_eq!(fs::read(&part).unwrap(), bytes);
            }
        } else {
            assert_eq!(recovered.status, "complete");
            assert_eq!(fs::read(&destination).unwrap(), bytes);
        }
        assert_eq!(store.acquisition_consumed(&id).unwrap(), !missing);
        assert_eq!(serde_json::to_value(store.read().unwrap()).unwrap(), before);
        assert_eq!(fs::read(root.join("profile/state.json")).unwrap(), durable);
        drop(restored);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn completed_history_allows_redownload_after_archive_removal_even_after_restart() {
    for restart in [false, true] {
        let body = zim_bytes(b'n', 128);
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
        response.extend_from_slice(&body);
        let (url, server) = single_response(response).await;
        let root = test_directory();
        let store = test_store(&root);
        let mut manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let id = seed_partial(&manager, &root, url.clone(), &zim_bytes(b'o', 128), 128);
        let old = manager.inner.snapshot(&id).unwrap().job;
        let (destination, part) = paths(&old).unwrap();
        crate::publication::publish_noreplace(&part, &destination).unwrap();
        manager
            .inner
            .update(&id, |stored| stored.job.status = "complete".into())
            .unwrap();

        assert!(manager
            .start(url.clone(), root.clone(), None, Some(128), None)
            .await
            .is_err());
        assert_eq!(fs::read(&destination).unwrap(), zim_bytes(b'o', 128));
        fs::remove_file(&destination).unwrap();
        if restart {
            drop(manager);
            manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        }
        assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "complete");
        assert_eq!(manager.list_snapshots().unwrap().iter().find(|row| row.id == id).unwrap().transport, DownloadTransport::Http);
        let replacement = manager.start(url, root.clone(), None, Some(128), None)
            .await
            .unwrap();
        let replacement = finished_job(&manager, &replacement.id).await;
        assert_eq!(replacement.status, "complete", "{:?}", replacement.error);
        assert_eq!(manager.list_snapshots().unwrap().iter().find(|row| row.id == replacement.id).unwrap().transport, DownloadTransport::Http);
        assert_eq!(manager.inner.snapshot(&replacement.id).unwrap().transport, Some(DownloadTransport::Http));
        assert_ne!(replacement.id, id);
        assert_eq!(fs::read(&destination).unwrap(), body);
        server.await.unwrap();

        manager.remove_terminal(&id).unwrap();
        manager.remove_terminal(&replacement.id).unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert_eq!(fs::read(&destination).unwrap(), body);
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn a_late_cancel_cannot_erase_a_completed_archive() {
    // Upstream kiwix-desktop #183: a delayed cancel after completion erased the
    // finished archive. A terminal job must refuse the command and keep bytes.
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'k', 128);
    let id = seed_partial(&manager, &root, "https://example.org/fixture.zim".into(), &bytes, 128);
    let old = manager.inner.snapshot(&id).unwrap().job;
    let (destination, part) = paths(&old).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    manager
        .inner
        .update(&id, |stored| stored.job.status = "complete".into())
        .unwrap();

    let error = manager.cancel(&id).await.unwrap_err();
    assert!(error.contains("Completed downloads cannot be paused or cancelled"), "{error}");
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "complete");
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn active_and_resumable_jobs_keep_ownership_and_cannot_be_retired() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let url = "https://example.org/fixture.zim".to_string();
    let id = seed_partial(&manager, &root, url.clone(), &zim_bytes(b'p', 32), 128);
    let part = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap().1;
    for status in ["queued", "downloading", "paused"] {
        manager
            .inner
            .update(&id, |stored| stored.job.status = status.into())
            .unwrap();
        assert!(manager.remove_terminal(&id).is_err(), "{status}");
        assert!(manager
            .start(
                "https://example.org/other.zim".into(),
                root.clone(),
                Some("fixture.zim".into()),
                None,
                None
            )
            .await
            .is_err());
        assert!(manager
            .start(url.clone(), root.clone(), Some("other.zim".into()), None, None)
            .await
            .is_err());
        assert!(part.exists());
    }
    for status in ["complete", "cancelled", "failed"] {
        manager
            .inner
            .update(&id, |stored| stored.job.status = status.into())
            .unwrap();
        let (command, _) = watch::channel(Signal::Run);
        let (_, finished) = watch::channel(false);
        manager
            .inner
            .state()
            .unwrap()
            .running
            .insert(id.clone(), Running { command, finished });
        assert!(manager.remove_terminal(&id).is_err(), "{status}");
        assert!(manager
            .start(
                "https://example.org/other.zim".into(),
                root.clone(),
                Some("fixture.zim".into()),
                None,
                None
            )
            .await
            .is_err());
        assert!(manager
            .start(url.clone(), root.clone(), Some("other.zim".into()), None, None)
            .await
            .is_err());
        manager.inner.state().unwrap().running.remove(&id);
    }
    // Failed jobs remain resumable reservations until explicitly retired.
    assert!(manager
        .start(url, root.clone(), Some("other.zim".into()), None, None)
        .await
        .is_err());
    manager.remove_terminal(&id).unwrap();
    assert!(part.exists());
    assert!(manager.inner.snapshot(&id).unwrap().history_dismissed);
    assert!(manager.resume(&id, None).await.is_err());
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    assert!(restored.list().unwrap().is_empty());
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn terminal_dismissal_preserves_files_and_cleanup_authority_after_restart() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'p', 32);
    let id = seed_partial(&manager, &root, "https://example.org/history.zim".into(), &bytes, 128);
    manager.inner.update(&id, |stored| stored.job.status = "failed".into()).unwrap();
    let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
    manager.remove_terminal(&id).unwrap();
    assert!(manager.list().unwrap().is_empty());
    assert_eq!(manager.retained_snapshots().unwrap().len(), 1);
    assert_eq!(fs::read(&part).unwrap(), bytes);
    assert!(manager.monitored_paths().unwrap().claims_monitored_path(&destination).unwrap());
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    assert!(restored.list().unwrap().is_empty());
    assert_eq!(restored.retained_snapshots().unwrap().len(), 1);
    assert!(restored.inner.snapshot(&id).unwrap().part_ownership.is_some());
    assert_eq!(fs::read(&part).unwrap(), bytes);
    restored.delete_terminal(&id).await.unwrap();
    assert!(!part.exists());
    restored.remove_terminal(&id).unwrap();
    assert!(restored.retained_snapshots().unwrap().is_empty());
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retirement_keeps_history_when_persistence_fails_and_never_touches_destination() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/fixture.zim".into(),
        &zim_bytes(b'p', 32),
        128,
    );
    manager
        .inner
        .update(&id, |stored| stored.job.status = "failed".into())
        .unwrap();
    let destination = manager.inner.snapshot(&id).unwrap().job.destination;
    let published = b"unrelated destination must survive failed retirement";
    fs::write(&destination, published).unwrap();
    let state_path = root.join("state/downloads.json");
    fs::remove_file(&state_path).unwrap();
    fs::create_dir(&state_path).unwrap();
    assert!(manager.remove_terminal(&id).is_err());
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "failed");
    assert_eq!(fs::read(&destination).unwrap(), published);
    fs::remove_dir(&state_path).unwrap();
    manager.remove_terminal(&id).unwrap();
    assert!(manager.list().unwrap().is_empty());
    assert_eq!(fs::read(&destination).unwrap(), published);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_checksum_bounds_a_growing_source_to_admitted_length_plus_one() {
    struct Growing {
        bytes: std::io::Cursor<Vec<u8>>,
        consumed: usize,
    }
    impl Read for Growing {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = Read::read(&mut self.bytes, buffer)?;
            if count == 0 {
                return Err(std::io::Error::other("cleanup kept reading a growing source"));
            }
            self.consumed += count;
            Ok(count)
        }
    }
    let admitted = zim_bytes(b'd', 128 * 1024);
    let mut extended = admitted.clone();
    extended.push(b'x');
    let mut source = Growing { bytes: std::io::Cursor::new(extended), consumed: 0 };
    assert!(read_publication_digest(&mut source, admitted.len() as u64).unwrap().is_none());
    assert_eq!(source.consumed, admitted.len() + 1);
    assert_eq!(
        read_publication_digest(&mut admitted.as_slice(), admitted.len() as u64).unwrap(),
        Some(hex::encode(Sha256::digest(&admitted))),
    );
    assert!(read_publication_digest(&mut &admitted[..admitted.len() - 1], admitted.len() as u64).unwrap().is_none());
}

#[tokio::test]
async fn explicit_http_cleanup_removes_owned_publication_only_after_worker_finishes() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let body = zim_bytes(b'd', 128);
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let job = manager.start(url, root.clone(), Some("owned.zim".into()), Some(128), None).await.unwrap();
    let job = finished_job(&manager, &job.id).await;
    server.await.unwrap();
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    let proof = manager.terminal_owned_publications(&job.id).await.unwrap();
    assert_eq!(proof.len(), 1);
    assert!(proof[0].1.matches_retained(Path::new(&job.destination), &File::open(&job.destination).unwrap()).unwrap());
    manager.delete_terminal(&job.id).await.unwrap();
    assert!(!Path::new(&job.destination).exists());
    assert_eq!(manager.list().unwrap().len(), 1, "disk deletion must not hide history");
    manager.remove_terminal(&job.id).unwrap();
    assert!(manager.list().unwrap().is_empty());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn explicit_http_cleanup_rechecks_captured_bytes_and_keeps_failed_cleanup_retryable() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'd', 128);
    let id = seed_partial(&manager, &root, "https://example.org/archive.zim".into(), &bytes, bytes.len() as u64);
    manager.inner.update(&id, |stored| {
        stored.job.status = "complete".into();
        stored.publication = Some(PublicationIntent {
            size: bytes.len() as u64, sha256: hex::encode(Sha256::digest(&bytes)),
        });
        stored.published_ownership = stored.part_ownership.as_ref().map(|entry| entry.renamed(&stored.job.file_name));
    }).unwrap();
    let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    let mut changed = bytes.clone();
    changed[64] ^= 1;
    fs::write(&destination, &changed).unwrap();
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&destination).unwrap(), changed);
    assert_eq!(manager.list().unwrap().len(), 1);
    assert!(!manager.inner.snapshot(&id).unwrap().files_deleted);
    fs::write(&destination, &bytes).unwrap();
    manager.delete_terminal(&id).await.unwrap();
    assert!(!destination.exists());
    manager.remove_terminal(&id).unwrap();
    assert!(manager.list().unwrap().is_empty());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn explicit_http_cleanup_rejects_replaced_and_legacy_part_files() {
    for legacy in [false, true] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let id = seed_partial(&manager, &root, "https://example.org/archive.zim".into(), b"owned partial", 128);
        manager.inner.update(&id, |stored| {
            stored.job.status = "failed".into();
            if legacy { stored.part_ownership = None; }
        }).unwrap();
        let part = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap().1;
        if !legacy {
            fs::rename(&part, root.join("retained-original")).unwrap();
            fs::write(&part, b"foreign replacement").unwrap();
        }
        let before = fs::read(&part).unwrap();
        assert!(manager.terminal_owned_publications(&id).await.is_err());
        assert!(manager.delete_terminal(&id).await.is_err());
        assert_eq!(fs::read(&part).unwrap(), before);
        assert_eq!(manager.list().unwrap().len(), 1);
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_http_cleanup_preserves_symlink_target() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, "https://example.org/archive.zim".into(), b"partial", 128);
    manager.inner.update(&id, |stored| stored.job.status = "failed".into()).unwrap();
    let part = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap().1;
    let target = root.join("user-file");
    fs::write(&target, b"user bytes").unwrap();
    fs::remove_file(&part).unwrap();
    std::os::unix::fs::symlink(&target, &part).unwrap();
    assert!(manager.delete_terminal(&id).await.is_err());
    assert!(fs::symlink_metadata(&part).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"user bytes");
    assert_eq!(manager.list().unwrap().len(), 1);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn terminal_deletion_cannot_spend_an_edition_publication_exception() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = fs::read(fixture_archive()).unwrap();
    let url = "https://example.org/archive.zim";
    let id = seed_partial(&manager, &root, url.into(), &bytes, bytes.len() as u64);
    manager.inner.update(&id, |stored| {
        stored.job.status = "complete".into();
        stored.publication = Some(PublicationIntent { size: bytes.len() as u64, sha256: hex::encode(Sha256::digest(&bytes)) });
        stored.published_ownership = stored.part_ownership.as_ref().map(|entry| entry.renamed(&stored.job.file_name));
    }).unwrap();
    let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    let original = crate::imports::load_book(fixture_archive().to_str().unwrap(), false, true).unwrap();
    let verified = crate::imports::load_book(destination.to_str().unwrap(), true, true).unwrap();
    let metadata = &verified.metadata;
    let selection = crate::edition_updates::EditionSelection {
        source_id: crate::catalog::source_id("https://example.org/catalog").unwrap(),
        source_url: "https://example.org/catalog".into(), entry_id: "opaque-edition".into(), catalog_generation: 1,
        entry: crate::catalog::CatalogEntry {
            id: "opaque-edition".into(), title: metadata.title.clone(), description: metadata.description.clone(),
            language: metadata.language.clone(), creator: metadata.creator.clone(), publisher: metadata.publisher.clone(),
            date: metadata.date.clone(), name: metadata.name.clone(), tags: metadata.tags.clone(),
            size: metadata.size, article_count: metadata.article_count, media_count: metadata.media_count,
            url: url.into(), thumbnail_url: None, torrent_url: None, torrent_resolver_url: None,
        },
    };
    let intent = crate::edition_updates::EditionIntent {
        id: crate::store::new_id(), selection,
        lineage_associated: true,
        original_identity: crate::library_files::archive_content_identity(&original).unwrap(), target: original,
        destination: destination.to_string_lossy().into_owned(),
        download_directory: root.to_string_lossy().into_owned(), download_id: Some(id.clone()),
        receipt: Some(id.clone()), status: "ready".into(),
        verified_identity: Some(crate::library_files::archive_content_identity(&verified).unwrap()),
        verified: Some(verified), transaction_id: None,
    };
    store.update(|state| { state.edition_intents.push(intent); Ok(()) }).unwrap();
    assert!(store.with_http_acquisition_destinations(&[&destination], url, || Ok(())).is_ok());
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert!(!manager.inner.snapshot(&id).unwrap().files_deleted);
    store.update(|state| { state.edition_intents[0].status = "cancelled".into(); Ok(()) }).unwrap();
    assert!(store.with_http_acquisition_destinations(&[&destination], url, || Ok(())).is_err());
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn explicit_http_cleanup_preserves_installed_archive_and_missing_reservation() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = fs::read(fixture_archive()).unwrap();
    let id = seed_partial(&manager, &root, "https://example.org/archive.zim".into(), &bytes, bytes.len() as u64);
    manager.inner.update(&id, |stored| {
        stored.job.status = "complete".into();
        stored.publication = Some(PublicationIntent { size: bytes.len() as u64, sha256: hex::encode(Sha256::digest(&bytes)) });
        stored.published_ownership = stored.part_ownership.as_ref().map(|entry| entry.renamed(&stored.job.file_name));
    }).unwrap();
    let (destination, part) = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    register_installed_archive(&store, &destination);
    assert_eq!(manager.terminal_owned_publications(&id).await.unwrap().len(), 1, "preflight ignores installed reservation but requires original bytes");
    let retained_original = root.join("original-publication");
    fs::rename(&destination, &retained_original).unwrap();
    fs::write(&destination, &bytes).unwrap();
    assert!(manager.terminal_owned_publications(&id).await.is_err(), "same UUID and bytes at a reused path are not original ownership");
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    fs::remove_file(&destination).unwrap();
    fs::rename(&retained_original, &destination).unwrap();
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    fs::remove_file(&destination).unwrap();
    assert!(manager.terminal_owned_publications(&id).await.unwrap().is_empty(), "missing outputs authorize no library deletion");
    assert!(manager.delete_terminal(&id).await.is_err(), "missing installed paths stay reserved");
    assert_eq!(manager.list().unwrap().len(), 1);
    store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
    manager.delete_terminal(&id).await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn explicit_http_cleanup_waits_for_terminal_worker_and_preserves_checkpoint_failures() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, "https://example.org/archive.zim".into(), b"retained", 128);
    manager.inner.update(&id, |stored| stored.job.status = "failed".into()).unwrap();
    let part = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap().1;
    let (command, _) = watch::channel(Signal::Run);
    let (finished, receiver) = watch::channel(false);
    manager.inner.state().unwrap().running.insert(id.clone(), Running { command, finished: receiver });
    let owner = manager.clone();
    let job_id = id.clone();
    let deletion = tokio::spawn(async move { owner.delete_terminal(&job_id).await });
    tokio::task::yield_now().await;
    assert!(part.exists());
    let state_path = root.join("state/downloads.json");
    fs::remove_file(&state_path).unwrap();
    fs::create_dir(&state_path).unwrap();
    manager.inner.state().unwrap().running.remove(&id);
    finished.send_replace(true);
    assert!(deletion.await.unwrap().is_err());
    assert_eq!(fs::read(&part).unwrap(), b"retained");
    assert_eq!(manager.list().unwrap().len(), 1);
    fs::remove_dir(&state_path).unwrap();
    manager.delete_terminal(&id).await.unwrap();
    assert!(!part.exists());
    assert_eq!(manager.list().unwrap().len(), 1);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn range_ignored_with_200_restarts_instead_of_corrupt_append() {
    let body = zim_bytes(b'n', 128);
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nETag: \"new\"\r\nConnection: close\r\n\r\n"
            .to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, url, &zim_bytes(b'o', 32), 128);
    manager.resume(&id, None).await.unwrap();
    let job = finished_job(&manager, &id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    let request = server.await.unwrap().to_ascii_lowercase();
    assert!(request.contains("range: bytes=32-"));
    assert!(request.contains("if-range: \"stable\""));
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unvalidated_partial_is_retained_until_full_response_size_is_accepted() {
    for accepted in [false, true] {
        let body = zim_bytes(b'n', 128);
        let length = if accepted { 128 } else { 4_294_967_296u64 };
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
                .into_bytes();
        if accepted {
            response.extend_from_slice(&body);
        }
        let (url, server) = single_response(response).await;
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let old = zim_bytes(b'o', 96);
        let id = seed_partial(&manager, &root, url, &old, 128);
        manager
            .inner
            .update(&id, |stored| stored.etag = None)
            .unwrap();
        manager.resume(&id, None).await.unwrap();
        let job = finished_job(&manager, &id).await;
        if accepted {
            assert_eq!(job.status, "complete", "{:?}", job.error);
            assert_eq!(fs::read(&job.destination).unwrap(), body);
        } else {
            assert_eq!(job.status, "failed");
            assert!(job
                .error
                .as_deref()
                .unwrap()
                .contains("Download size differs"));
            assert_eq!(fs::read(paths(&job).unwrap().1).unwrap(), old);
            assert!(!Path::new(&job.destination).exists());
        }
        let request = server.await.unwrap().to_ascii_lowercase();
        assert!(!request.contains("range:"));
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn wrong_206_start_never_appends_any_response_bytes() {
    let mut response = b"HTTP/1.1 206 Partial Content\r\nContent-Length: 97\r\nContent-Range: bytes 31-127/128\r\nETag: \"stable\"\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&[b'x'; 97]);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let old = zim_bytes(b'o', 32);
    let id = seed_partial(&manager, &root, url, &old, 128);
    manager.resume(&id, None).await.unwrap();
    let job = finished_job(&manager, &id).await;
    assert_eq!(job.status, "failed");
    assert!(job.error.as_deref().unwrap().contains("206 range start"));
    assert_eq!(fs::read(paths(&job).unwrap().1).unwrap(), old);
    assert!(!Path::new(&job.destination).exists());
    server.await.unwrap();
    manager.cancel(&id).await.unwrap();
    assert!(!paths(&job).unwrap().1.exists());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn validated_416_publishes_already_complete_partial_file() {
    let (url, server) = single_response(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nContent-Range: bytes */128\r\nETag: \"stable\"\r\nConnection: close\r\n\r\n".to_vec()).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'a', 128);
    let id = seed_partial(&manager, &root, url, &bytes, 128);
    manager.resume(&id, None).await.unwrap();
    let job = finished_job(&manager, &id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), bytes);
    server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn restart_restores_paused_and_existing_destinations_survive_resume_and_cancel() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/fixture.zim".into(),
        &zim_bytes(b'a', 96),
        128,
    );
    manager
        .inner
        .update(&id, |stored| {
            stored.job.status = "downloading".into();
            stored.job.downloaded = 16;
        })
        .unwrap();
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = restored.list().unwrap().pop().unwrap();
    assert_eq!(job.status, "paused");
    assert!(job.interrupted);
    assert_eq!(job.downloaded, 96);
    fs::write(&job.destination, b"existing archive must survive").unwrap();
    assert!(restored
        .resume(&id, None)
        .await
        .unwrap_err()
        .contains("already exists"));
    restored.cancel(&id).await.unwrap();
    assert!(!paths(&job).unwrap().1.exists());
    assert_eq!(
        fs::read(&job.destination).unwrap(),
        b"existing archive must survive"
    );
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_bandwidth_budget_cannot_grant_each_worker_a_separate_limit() {
    let start = Instant::now();
    let mut budget = BandwidthBudget {
        updated: start,
        credit: 0.0,
        generation: 0,
    };
    let ready = start + Duration::from_millis(100);
    assert_eq!(budget.take(1000, ready), Ok(100));
    assert!(
        budget.take(1000, ready).is_err(),
        "a second worker must share the same exhausted quota"
    );
    assert_eq!(
        budget.take(1000, ready + Duration::from_millis(100)),
        Ok(100)
    );
    let mut slow = BandwidthBudget {
        updated: start,
        credit: 0.0,
        generation: 0,
    };
    assert!(slow.take(1, start + Duration::from_millis(999)).is_err());
    assert_eq!(slow.take(1, start + Duration::from_secs(1)), Ok(1));
}

#[test]
fn publication_recovery_rejects_byte_identical_replacement_and_preserves_both_files() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'p', 128);
    let id = seed_partial(&manager, &root, "https://example.org/fixture.zim".into(), &bytes, 128);
    manager.inner.update(&id, |stored| {
        stored.job.status = "verifying".into();
        stored.publication = Some(PublicationIntent { size: 128, sha256: hex::encode(Sha256::digest(&bytes)) });
        stored.published_ownership = stored.part_ownership.as_ref().map(|entry| entry.renamed(&stored.job.file_name));
    }).unwrap();
    let stored = manager.inner.snapshot(&id).unwrap();
    let (destination, part) = paths(&stored.job).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    let original = root.join("original-publication");
    fs::rename(&destination, &original).unwrap();
    fs::write(&destination, &bytes).unwrap();
    assert!(recover_publication(&stored, &destination, &part).unwrap_err().contains("replaced"));
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    assert_eq!(restored.inner.snapshot(&id).unwrap().job.status, "failed");
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert_eq!(fs::read(&original).unwrap(), bytes);
    fs::remove_file(&destination).unwrap();
    fs::rename(&original, &destination).unwrap();
    assert!(recover_publication(&stored, &destination, &part).unwrap());
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_intent_recovers_a_moved_part_but_not_an_unrelated_destination() {
    let root = test_directory();
    let store = test_store(&root);
    let state_directory = store.data_dir().join("downloads");
    let manager = DownloadManager::new(state_directory.clone(), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'p', 128);
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/fixture.zim".into(),
        &bytes,
        128,
    );
    manager
        .inner
        .update(&id, |stored| {
            stored.job.status = "downloading".into();
            stored.publication = Some(PublicationIntent {
                size: 128,
                sha256: hex::encode(Sha256::digest(&bytes)),
            });
        })
        .unwrap();
    let stored = manager.inner.snapshot(&id).unwrap();
    let (destination, part) = paths(&stored.job).unwrap();
    assert!(manager.monitored_paths().unwrap().claims_monitored_path(&destination).unwrap());
    assert!(!manager.monitored_paths().unwrap().claims_monitored_path(&root.join("external.zim")).unwrap());
    fs::write(&destination, &bytes).unwrap();
    assert!(
        !recover_publication(&stored, &destination, &part).unwrap(),
        "persisting an intent must not claim a pre-existing file while its part remains"
    );
    fs::remove_file(&destination).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    assert!(!part.exists());
    assert!(manager.monitored_paths().unwrap().claims_monitored_path(&destination).unwrap());
    assert!(recover_publication(&stored, &destination, &part).unwrap());
    let mut legacy = serde_json::to_value(&stored).unwrap();
    legacy.as_object_mut().unwrap().remove("publication");
    legacy.as_object_mut().unwrap().remove("interrupted");
    let legacy: StoredJob = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.job.interrupted);
    assert!(!recover_publication(&legacy, &destination, &part).unwrap());
    fs::write(&destination, zim_bytes(b'x', 128)).unwrap();
    assert!(!recover_publication(&stored, &destination, &part).unwrap());
    fs::write(&destination, &bytes).unwrap();
    drop(manager);
    // Lazy history checks must work before a manager performs recovery.
    let claims = DownloadManager::saved_monitored_paths(&store).unwrap();
    assert!(claims.claims_monitored_path(&destination).unwrap());
    assert!(!claims.claims_monitored_path(&root.join("external.zim")).unwrap());
    let restored = DownloadManager::new(state_directory, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = restored.inner.snapshot(&id).unwrap().job;
    assert_eq!(job.status, "complete");
    assert!(!job.interrupted);
    assert!(restored.monitored_paths().unwrap().claims_monitored_path(&destination).unwrap());
    restored.remove_terminal(&id).unwrap();
    assert!(restored.monitored_paths().unwrap().claims_monitored_path(&destination).unwrap());
    assert!(DownloadManager::saved_monitored_paths(&store).unwrap().claims_monitored_path(&destination).unwrap());
    assert_eq!(fs::read(destination).unwrap(), bytes);
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_recovery_retries_backup_policy_before_acknowledgement() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'p', 128);
    let id = seed_partial(
        &manager, &root, "https://example.org/fixture.zim".into(), &bytes, 128,
    );
    manager.inner.update(&id, |stored| {
        stored.job.status = "failed".into();
        stored.publication = Some(PublicationIntent {
            size: 128, sha256: hex::encode(Sha256::digest(&bytes)),
        });
    }).unwrap();
    let stored = manager.inner.snapshot(&id).unwrap();
    let (destination, part) = paths(&stored.job).unwrap();
    crate::publication::publish_noreplace(&part, &destination).unwrap();
    let attempts = std::cell::Cell::new(0);
    let result = recover_publication_with_policy(&stored, &destination, &part, |file, path| {
        attempts.set(attempts.get() + 1);
        assert_eq!(path, destination);
        assert_eq!(file.metadata().unwrap().len(), 128);
        Err("backup policy rejected".into())
    });
    assert_eq!(result.unwrap_err(), "backup policy rejected");
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "failed");
    assert!(destination.is_file());
    assert!(manager.monitored_paths().unwrap().claims_monitored_path(&destination).unwrap());
    assert!(!manager.monitored_paths().unwrap().claims_monitored_path(&root.join("external.zim")).unwrap());
    assert!(recover_publication_with_policy(&stored, &destination, &part, |file, path| {
        attempts.set(attempts.get() + 1);
        assert_eq!(path, destination);
        assert_eq!(file.metadata().unwrap().len(), 128);
        Ok(())
    }).unwrap());
    assert_eq!(attempts.get(), 2);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn shutdown_preserves_queued_resume_intent_and_never_resumes_manual_pauses() {
    let body = zim_bytes(b'r', 128);
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let manual_dir = root.join("manual");
    let active_dir = root.join("active");
    fs::create_dir(&manual_dir).unwrap();
    fs::create_dir(&active_dir).unwrap();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let manual = seed_partial(
        &manager,
        &manual_dir,
        "https://example.org/manual.zim".into(),
        &zim_bytes(b'm', 32),
        128,
    );
    let active = seed_partial(&manager, &active_dir, url, &zim_bytes(b'a', 32), 128);
    let slots = manager
        .inner
        .slots
        .clone()
        .acquire_many_owned(3)
        .await
        .unwrap();
    manager.resume(&active, None).await.unwrap();
    manager.shutdown().await.unwrap();
    assert!(manager.inner.snapshot(&active).unwrap().job.interrupted);
    assert!(!manager.inner.snapshot(&manual).unwrap().job.interrupted);
    drop(slots);
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    restored.resume_interrupted().await.unwrap();
    assert_eq!(finished_job(&restored, &active).await.status, "complete");
    let paused = restored.inner.snapshot(&manual).unwrap().job;
    assert_eq!(paused.status, "paused");
    assert!(!paused.interrupted);
    assert!(!restored
        .inner
        .state()
        .unwrap()
        .running
        .contains_key(&manual));
    server.await.unwrap();
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn reconnect_resume_restores_network_interrupted_jobs_only() {
    use crate::transfer_policy::{
        reconnect_resume, InternetAdmission, ResumeAdmission,
    };
    let body = zim_bytes(b'r', 128);
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let manual_dir = root.join("manual");
    let active_dir = root.join("active");
    fs::create_dir(&manual_dir).unwrap();
    fs::create_dir(&active_dir).unwrap();
    let store = test_store(&root);
    store.update(|state| {
        state.settings.auto_resume_downloads = true;
        Ok(())
    }).unwrap();
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let manual = seed_partial(
        &manager,
        &manual_dir,
        "https://example.org/manual.zim".into(),
        &zim_bytes(b'm', 32),
        128,
    );
    let interrupted = seed_partial(&manager, &active_dir, url, &zim_bytes(b'a', 32), 128);
    // The reconciler marks a policy retirement; a manual pause stays unmarked.
    manager
        .inner
        .update(&interrupted, |stored| stored.job.interrupted = true)
        .unwrap();
    let admission = reconnect_resume(
        &store.settings().unwrap(),
        InternetAdmission::Admitted,
        true,
        7,
        6,
    );
    assert_eq!(admission, ResumeAdmission::Attempt);
    assert_eq!(
        manager.resume_interrupted_after_reconnect(admission).await.unwrap(),
        None
    );
    assert_eq!(finished_job(&manager, &interrupted).await.status, "complete");
    let paused = manager.inner.snapshot(&manual).unwrap().job;
    assert_eq!(paused.status, "paused");
    assert!(!paused.interrupted);
    assert!(!manager.inner.state().unwrap().running.contains_key(&manual));
    server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn reconnect_resume_skips_metered_denial_and_read_only() {
    use crate::transfer_policy::{
        reconnect_resume, InternetAdmission, ResumeAdmission, ResumeSkip,
    };
    let root = test_directory();
    let store = test_store(&root);
    store.update(|state| {
        state.settings.auto_resume_downloads = true;
        Ok(())
    }).unwrap();
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/denied.zim".into(),
        &zim_bytes(b'd', 32),
        128,
    );
    manager
        .inner
        .update(&id, |stored| stored.job.interrupted = true)
        .unwrap();

    let metered = reconnect_resume(
        &store.settings().unwrap(),
        InternetAdmission::MeteredDenied,
        true,
        3,
        2,
    );
    assert_eq!(metered, ResumeAdmission::Skip(ResumeSkip::MeteredDenied));
    assert_eq!(
        manager.resume_interrupted_after_reconnect(metered).await.unwrap(),
        Some(ResumeSkip::MeteredDenied)
    );
    let unchanged = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(unchanged.status, "paused");
    assert!(unchanged.interrupted);
    assert!(!manager.inner.state().unwrap().running.contains_key(&id));

    store.update(|state| {
        state.settings.read_only = true;
        Ok(())
    }).unwrap();
    let read_only = reconnect_resume(
        &store.settings().unwrap(),
        InternetAdmission::Admitted,
        true,
        4,
        2,
    );
    assert_eq!(read_only, ResumeAdmission::Skip(ResumeSkip::ReadOnly));
    assert_eq!(
        manager.resume_interrupted_after_reconnect(read_only).await.unwrap(),
        Some(ResumeSkip::ReadOnly)
    );
    let unchanged = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(unchanged.status, "paused");
    assert!(unchanged.interrupted);
    assert!(!manager.inner.state().unwrap().running.contains_key(&id));
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn bandwidth_wait_is_manually_pausable_and_limit_updates_affect_the_stream() {
    let body = zim_bytes(b'b', 128);
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    manager.set_download_limit(1).unwrap();
    let id = seed_partial(&manager, &root, url, &[], 128);
    manager.resume(&id, None).await.unwrap();
    // Wait for the stream to reach its first admitted byte; the pause then
    // lands on a live budget wait, not on an idle or finished worker.
    let deadline = Instant::now() + Duration::from_secs(30);
    while manager.inner.snapshot(&id).unwrap().job.downloaded == 0 {
        assert!(
            Instant::now() < deadline,
            "the 1 B/s budget never admitted its first byte"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // 128 bytes need 128s at that rate, so a generous deadline still proves
    // the wakeup happened during the wait without racing a loaded host.
    tokio::time::timeout(Duration::from_secs(30), manager.pause(&id))
        .await
        .expect("pause did not wake the bandwidth wait within 30s")
        .unwrap();
    let paused = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(paused.status, "paused");
    assert!(!paused.interrupted);
    assert!(paused.downloaded < 128, "pause waited for the whole body");
    manager.cancel(&id).await.unwrap();
    server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();

    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    manager.set_download_limit(1).unwrap();
    let id = seed_partial(&manager, &root, url, &[], 128);
    manager.resume(&id, None).await.unwrap();
    // Wait until the 1 B/s budget has actually admitted a byte, so the
    // update below lands on a live wait instead of a finished body.
    let deadline = Instant::now() + Duration::from_secs(30);
    while manager.inner.snapshot(&id).unwrap().job.downloaded == 0 {
        assert!(
            Instant::now() < deadline,
            "the 1 B/s budget never admitted its first byte"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let start = Instant::now();
    manager.set_download_limit(128).unwrap();
    let job = finished_job(&manager, &id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert!(
        start.elapsed() >= Duration::from_millis(900),
        "body reads bypassed the configured byte budget"
    );
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}


#[tokio::test]
async fn metalink_exact_size_overrides_the_rounded_catalog_size() {
    let body = zim_bytes(b'e', 128);
    let hash = hex::encode(Sha256::digest(&body));
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (archive_url, archive_server) = single_response(response).await;
    let xml = format!(
        r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="fixture.zim"><size>128</size><hash type="sha-256">{hash}</hash><url>{archive_url}</url></file></metalink>"#
    );
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        xml.len()
    )
    .into_bytes();
    response.extend_from_slice(xml.as_bytes());
    let (url, metadata_server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager.start(url.replace("fixture.zim", "fixture.zim.meta4"), root.clone(), None, Some(100), None)
        .await
        .unwrap();
    let job = finished_job(&manager, &job.id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(job.total, Some(128));
    assert_eq!(job.sha256.as_deref(), Some(hash.as_str()));
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    metadata_server.await.unwrap();
    archive_server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unreadable_download_records_stay_failed_and_can_be_forgotten() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'p', 32);
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/readable.zim".into(),
        &bytes,
        128,
    );
    drop(manager);
    let path = root.join("state/downloads.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["broken"] = serde_json::json!({"job": {"id": "broken"}});
    let mut invalid = json[id.as_str()].clone();
    invalid["id"] = serde_json::json!("invalid-download");
    invalid["url"] = serde_json::json!("file:///not-a-download");
    json["invalid-download"] = invalid;
    // A row whose map key and job ID disagree still has an identity the
    // list can show; removal must find it by that ID.
    let mismatched_id = Uuid::new_v4().to_string();
    let mut mismatched = json[id.as_str()].clone();
    mismatched["id"] = serde_json::json!(mismatched_id);
    json["mismatched-key"] = mismatched;
    fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    let kept = root.join("fixture.zim");
    fs::write(&kept, b"a file the user keeps").unwrap();

    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let rows = restored.list().unwrap();
    // The unreadable row (not even deserializable) is still skipped; the
    // validated-but-invalid row stays visible as failed so its file and
    // ownership claims are never silently dropped.
    assert_eq!(rows.len(), 3, "{rows:?}");
    for failed_id in ["invalid-download", mismatched_id.as_str()] {
        let failed = rows.iter().find(|row| row.id == failed_id).unwrap();
        assert_eq!(failed.status, "failed");
        assert!(!failed.interrupted);
        assert!(failed
            .error
            .as_deref()
            .unwrap()
            .contains("could not restore this saved download"));
    }
    assert!(restored
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("could not be restored")));

    restored.forget_terminal("invalid-download").unwrap();
    restored.forget_terminal(&mismatched_id).unwrap();
    let remaining = restored.list().unwrap();
    assert!(remaining.iter().all(|row| row.id != "invalid-download"));
    assert!(remaining.iter().all(|row| row.id != mismatched_id));
    assert_eq!(fs::read(&kept).unwrap(), b"a file the user keeps");
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn returning_archive_clears_a_stale_missing_error() {
    let body = zim_bytes(b'q', 128);
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(url, root.join("received"), None, Some(128), None)
        .await
        .unwrap();
    let job = finished_job(&manager, &job.id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    server.await.unwrap();
    drop(manager);

    fs::remove_file(&job.destination).unwrap();
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let row = restored.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
    assert!(
        row.error.as_deref().unwrap().contains("missing or has changed"),
        "{:?}",
        row.error
    );
    drop(restored);

    // The archive comes back unchanged: the saved failure is stale and must
    // not keep the row painted Failed forever.
    fs::write(&job.destination, &body).unwrap();
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let row = restored.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
    assert_eq!(row.error, None, "a returned archive must clear the stale error");
    assert_eq!(row.status, "complete");
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transfer_journal_reads_are_buffered_and_enforce_the_stream_limit() {
    use std::io::{Cursor, Read};
    struct Counted {
        input: Cursor<Vec<u8>>,
        reads: usize,
    }
    impl Read for Counted {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            Read::read(&mut self.input, bytes)
        }
    }
    let mut bytes = b"{}".to_vec();
    bytes.resize(128 * 1024, b' ');
    let limit = bytes.len() as u64;
    let mut reader = Counted { input: Cursor::new(bytes), reads: 0 };
    assert_eq!(read_history_json(&mut reader, limit).unwrap(), Some(serde_json::json!({})));
    assert!(reader.reads < 100);
    reader.input.get_mut().push(b' ');
    reader.input.set_position(0);
    assert!(read_history_json(reader, limit).unwrap().is_none());
}

#[test]
fn transfer_journal_recovery_refuses_a_replaced_source() {
    let root = test_directory();
    let path = root.join("downloads.json");
    fs::write(&path, b"broken").unwrap();
    let first = open_history(&path).unwrap().unwrap();
    fs::rename(&path, root.join("old")).unwrap();
    fs::write(&path, b"edited").unwrap();
    assert!(preserve_history(&path, first).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"edited");
    assert_eq!(fs::read(root.join("old")).unwrap(), b"broken");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn interruption_survives_a_stopping_worker_and_an_unwound_worker_can_be_cancelled() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store, crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, "http://127.0.0.1:9/fixture.zim".into(), b"partial", 128);
    let (command, _receiver) = watch::channel(Signal::Interrupt);
    let (_done, finished) = watch::channel(false);
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.get_mut(&id).unwrap().job.interrupted = true;
        state.running.insert(id.clone(), Running { command, finished });
    }
    manager.resume_interrupted().await.unwrap();
    assert!(manager.inner.snapshot(&id).unwrap().job.interrupted);
    let mut finished = {
        let mut state = manager.inner.state().unwrap();
        state.running.remove(&id);
        spawn_download_worker(&manager.inner, &mut state, &id, WorkerMode::Panic);
        state.running[&id].finished.clone()
    };
    finished.wait_for(|done| *done).await.unwrap();
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "failed");
    assert!(!manager.inner.state().unwrap().running.contains_key(&id));
    manager.cancel(&id).await.unwrap();
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "cancelled");
    drop(manager);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn malformed_downloads_root_is_preserved_and_new_downloads_still_persist() {
    for original in [b"{not json".as_slice(), b"[]".as_slice()] {
        let root = test_directory();
        let store = test_store(&root);
        let state_dir = root.join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let live = state_dir.join("downloads.json");
        fs::write(&live, original).unwrap();

        let manager = DownloadManager::new(state_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        assert!(manager.list().unwrap().is_empty());
        let preserved = manager
            .history_notice()
            .expect("a quarantined record must expose its preserved path");
        assert_eq!(fs::read(&preserved).unwrap(), original);
        assert_ne!(preserved, live);
        let recovered: serde_json::Value =
            serde_json::from_slice(&fs::read(&live).unwrap()).unwrap();
        assert_eq!(recovered, serde_json::json!({}));

        let body = zim_bytes(b'r', 128);
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n".to_vec();
        response.extend_from_slice(&body);
        let (url, server) = single_response(response).await;
        let job = manager
            .start(url, root.join("received"), None, Some(128), None)
            .await
            .unwrap();
        let job = finished_job(&manager, &job.id).await;
        assert_eq!(job.status, "complete", "{:?}", job.error);
        assert_eq!(fs::read(&job.destination).unwrap(), body);
        server.await.unwrap();
        drop(manager);

        // A recovered history is ordinary durable state: the next launch
        // reads the job without a second notice or quarantine.
        let restored = DownloadManager::new(state_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        assert_eq!(restored.list().unwrap().len(), 1);
        assert!(restored.history_notice().is_none());
        drop(restored);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn monitored_path_key_matches_the_store_comparison() {
    // The claim index must be exactly the lexical comparison the rest of the
    // app uses. A divergence would silently adopt a file that a download owns
    // (or hide an unrelated one), so every pair is checked against the
    // authoritative comparison rather than a hand-written expectation.
    let corpus = [
        "/data/books/a.zim",
        "/data/books/A.ZIM",
        "/data/books/./a.zim",
        "/data/books/other/../a.zim",
        "/data/books/a.zim/",
        "/data//books/a.zim",
        "/data/books/a.zim.",
        "/data/books/a.zim ",
        "/data/books/a.zim..",
        "/data/books/a.zim/..",
        "/data/books/..",
        "C:\\data\\books\\a.zim",
        "c:/data/books/a.zim",
        "C:\\data\\books\\A.ZIM",
        "C:\\data\\books\\other\\..\\a.zim",
        "\\\\?\\C:\\data\\books\\a.zim",
        "\\\\?\\C:\\data\\books\\a.zim.",
        "\\\\server\\share\\books\\a.zim",
        "//server/share/books/a.zim",
        "\\\\server\\share\\books\\other\\..\\a.zim",
        "relative/books/a.zim",
        "relative\\books\\a.zim",
        "other/relative/books/a.zim",
        "/data/books/Straße.zim",
        "/data/books/STRASSE.zim",
        "/data/books/strasse.zim",
        "/data/books/Stra\u{00DF}e.zim",
        "/data/books/Stra\u{0073}\u{0073}e.zim",
        "/data/books/\u{212A}elvin.zim",
        "/data/books/Kelvin.zim",
        "/data/books/caf\u{00E9}.zim",
        "/data/books/cafe\u{0301}.zim",
        "/data/books/CAF\u{00C9}.zim",
        "/data/books/\u{1DF95}.zim",
        "/data/books/ss.zim",
    ];
    for left in corpus {
        for right in corpus {
            assert_eq!(
                monitored_path_key(left) == monitored_path_key(right),
                crate::store::acquisition_paths_equal(left, right, ""),
                "{left:?} vs {right:?}"
            );
        }
    }
}

#[test]
fn unreadable_downloads_root_claims_nothing_instead_of_blocking_monitors() {
    let root = test_directory();
    let store = test_store(&root);
    let directory = store.data_dir().join("downloads");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("downloads.json");
    fs::write(&path, b"{not json").unwrap();

    let claims = DownloadManager::saved_monitored_paths(&store).unwrap();
    assert!(!claims
        .claims_monitored_path(&root.join("external.zim"))
        .unwrap());
    // Reading claims never moves or rewrites the original record.
    assert_eq!(fs::read(&path).unwrap(), b"{not json");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transient_history_failure_is_not_an_empty_claim_list() {
    let root = test_directory();
    let store = test_store(&root);
    // A file where the downloads directory belongs is an unavailable
    // profile, not a missing or invalid record: the read must fail so a
    // monitored scan cannot adopt an unverified download destination.
    fs::write(store.data_dir().join("downloads"), b"not a directory").unwrap();
    let Err(error) = DownloadManager::saved_monitored_paths(&store) else {
        panic!("an unreadable download history must not claim an empty set");
    };
    assert!(!error.is_empty());
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn failed_auto_resume_keeps_the_job_visible_and_never_retries_it() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/fixture.zim".into(),
        &zim_bytes(b'a', 32),
        128,
    );
    manager
        .inner
        .update(&id, |stored| stored.job.interrupted = true)
        .unwrap();
    fs::write(root.join("fixture.zim"), b"user file").unwrap();

    let error = manager.resume_interrupted().await.unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    let job = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(job.status, "paused");
    assert!(!job.interrupted);
    assert!(job.error.as_deref().unwrap().contains("already exists"));
    assert!(!manager.inner.state().unwrap().running.contains_key(&id));

    // A later launch sees an ordinary manual pause: no retry, no rewrite.
    manager.resume_interrupted().await.unwrap();
    let again = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(again.error, job.error);
    assert_eq!(fs::read(root.join("fixture.zim")).unwrap(), b"user file");
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn short_chunked_200_on_resume_is_never_published_as_complete() {
    let archive = fs::read(fixture_archive()).unwrap();
    let total = archive.len() as u64;
    let retained = archive[..total as usize / 2].to_vec();
    let short = &archive[..16 * 1024];
    let mut response = b"HTTP/1.1 200 OK\r\nETag: \"stable\"\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(format!("{:x}\r\n", short.len()).as_bytes());
    response.extend_from_slice(short);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    let (url, server) = single_response(response).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(&manager, &root, url, &retained, total);
    // Only a previously learned total remains: the catalog never named a size.
    manager
        .inner
        .update(&id, |stored| stored.expected_size = None)
        .unwrap();
    manager.resume(&id, None).await.unwrap();
    let job = finished_job(&manager, &id).await;
    assert_eq!(job.status, "failed", "{:?}", job.error);
    assert!(
        job.error
            .as_deref()
            .unwrap()
            .contains("before the expected object size"),
        "{:?}",
        job.error
    );
    assert!(!Path::new(&job.destination).exists());
    let part = paths(&job).unwrap().1;
    assert!(part.exists(), "a rejected truncation must stay resumable");
    assert_eq!(fs::read(&part).unwrap(), short);
    server.await.unwrap();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancel_preserves_a_part_file_it_cannot_prove_it_owns() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = seed_partial(
        &manager,
        &root,
        "https://example.org/archive.zim".into(),
        b"owned partial",
        128,
    );
    let part = paths(&manager.inner.snapshot(&id).unwrap().job).unwrap().1;
    fs::rename(&part, root.join("original")).unwrap();
    fs::write(&part, b"foreign replacement").unwrap();

    manager.cancel(&id).await.unwrap();
    let job = manager.inner.snapshot(&id).unwrap().job;
    assert_eq!(job.status, "cancelled");
    assert!(
        job.error.as_deref().unwrap().contains("cannot prove it created it"),
        "{:?}",
        job.error
    );
    assert_eq!(fs::read(&part).unwrap(), b"foreign replacement");

    // The load-time unlink applies the same ownership gate.
    drop(manager);
    let restored = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = restored.inner.snapshot(&id).unwrap().job;
    assert_eq!(job.status, "cancelled");
    assert!(
        job.error.as_deref().unwrap().contains("cannot prove it created it"),
        "{:?}",
        job.error
    );
    assert_eq!(fs::read(&part).unwrap(), b"foreign replacement");
    drop(restored);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

struct TestRangeState {
    body: Arc<Vec<u8>>,
    etag: String,
    ignore_ranges: bool,
    advertise_ranges: bool,
    chunk_delay: Duration,
    requests: Vec<String>,
    metalink: bool,
    /// While `Some(false)`, every range response stops after a durable
    /// checkpoint until `TestRangeServer::release` (see `hold_mid_range`).
    hold: Option<watch::Sender<bool>>,
}

struct TestRangeServer {
    archive_url: String,
    metalink_url: String,
    state: Arc<Mutex<TestRangeState>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestRangeServer {
    fn set_body(&self, body: Vec<u8>, etag: &str) {
        let mut state = self.state.lock().unwrap();
        state.body = Arc::new(body);
        state.etag = etag.to_string();
    }

    /// Makes mid-transfer state deterministic instead of a race against a
    /// throttled body. Each range response sends one chunk, waits past the
    /// segment sync interval and sends a second chunk, so the worker publishes
    /// durable progress at that read. It then holds until `release`, so the
    /// transfer cannot complete before the test acts.
    fn hold_mid_range(&self) {
        self.state.lock().unwrap().hold = Some(watch::channel(false).0);
    }

    fn release(&self) {
        if let Some(hold) = &self.state.lock().unwrap().hold {
            hold.send_replace(true);
        }
    }

    fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    fn range_requests(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|request| request.contains("\r\nrange:"))
            .collect()
    }
}

fn test_range_bounds(request: &str) -> Option<(u64, u64)> {
    request.lines().find_map(|line| {
        let value = line.strip_prefix("range: bytes=")?;
        let (start, end) = value.split_once('-')?;
        Some((start.parse().ok()?, end.parse().ok()?))
    })
}

async fn handle_test_connection(
    mut socket: tokio::net::TcpStream,
    state: Arc<Mutex<TestRangeState>>,
    base: &str,
) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let Ok(count) = socket.read(&mut buffer).await else { return };
        if count == 0 || request.len() + count > 16 * 1024 {
            return;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let text = String::from_utf8_lossy(&request).to_string();
    let lower = text.to_ascii_lowercase();
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let (body, etag, ignore, advertise, delay, metalink, mut hold) = {
        let mut state = state.lock().unwrap();
        state.requests.push(lower.clone());
        (
            Arc::clone(&state.body),
            state.etag.clone(),
            state.ignore_ranges,
            state.advertise_ranges,
            state.chunk_delay,
            state.metalink,
            state.hold.as_ref().map(watch::Sender::subscribe),
        )
    };
    if metalink && path.ends_with(".meta4") {
        let xml = format!(
            "<metalink xmlns=\"urn:ietf:params:xml:ns:metalink\"><file name=\"fixture.zim\"><size>{}</size><hash type=\"sha-256\">{}</hash><url>{base}/fixture.zim</url></file></metalink>",
            body.len(),
            hex::encode(Sha256::digest(body.as_slice())),
        );
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/metalink4+xml\r\nConnection: close\r\n\r\n",
            xml.len()
        );
        let _ = socket.write_all(header.as_bytes()).await;
        let _ = socket.write_all(xml.as_bytes()).await;
        let _ = socket.shutdown().await;
        return;
    }
    let requested = test_range_bounds(&lower);
    let if_range = lower
        .lines()
        .find_map(|line| line.strip_prefix("if-range:"))
        .map(str::trim)
        .map(str::to_string);
    let changed = if_range.as_deref().is_some_and(|value| value != etag);
    let (status, start, end) = match requested {
        Some((start, end)) if !ignore && !changed => {
            if start >= body.len() as u64 {
                let header = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nContent-Range: bytes */{}\r\nETag: {etag}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.shutdown().await;
                return;
            }
            (206u16, start, end.min(body.len() as u64 - 1))
        }
        _ => (200, 0, body.len() as u64 - 1),
    };
    let length = end - start + 1;
    let mut header = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {length}\r\nETag: {etag}\r\nConnection: close\r\n",
        status,
        if status == 206 { "Partial Content" } else { "OK" },
    );
    if status == 206 {
        header.push_str(&format!(
            "Content-Range: bytes {start}-{end}/{}\r\n",
            body.len()
        ));
    }
    if advertise {
        header.push_str("Accept-Ranges: bytes\r\n");
    }
    header.push_str("\r\n");
    if socket.write_all(header.as_bytes()).await.is_err() {
        return;
    }
    if status != 206 || hold.as_ref().is_some_and(|released| *released.borrow()) {
        hold = None;
    }
    for (index, chunk) in body[start as usize..=end as usize].chunks(16 * 1024).enumerate() {
        if let Some(released) = hold.as_mut() {
            match index {
                // The worker syncs on the first read after SYNC_INTERVAL.
                1 => tokio::time::sleep(SYNC_INTERVAL + Duration::from_millis(100)).await,
                2 => {
                    let _ = released.wait_for(|released| *released).await;
                }
                _ => {}
            }
        }
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if socket.write_all(chunk).await.is_err() {
            return;
        }
    }
    let _ = socket.shutdown().await;
}

async fn test_range_server(
    body: Vec<u8>,
    etag: &str,
    chunk_delay: Duration,
) -> TestRangeServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(TestRangeState {
        body: Arc::new(body),
        etag: etag.to_string(),
        ignore_ranges: false,
        advertise_ranges: true,
        chunk_delay,
        requests: Vec::new(),
        metalink: false,
        hold: None,
    }));
    let server_state = Arc::clone(&state);
    let base = format!("http://{address}");
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else { break };
            let state = Arc::clone(&server_state);
            let base = base.clone();
            tokio::spawn(async move {
                handle_test_connection(socket, state, &base).await;
            });
        }
    });
    TestRangeServer {
        archive_url: format!("http://{address}/fixture.zim"),
        metalink_url: format!("http://{address}/fixture.zim.meta4"),
        state,
        task,
    }
}

async fn wait_for_segment_progress(manager: &DownloadManager, id: &str) -> StoredJob {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let stored = manager.inner.snapshot(id).unwrap();
        if stored
            .segments
            .as_ref()
            .is_some_and(|plan| plan.ranges.iter().any(|range| range.done > 0))
        {
            return stored;
        }
        assert!(
            Instant::now() < deadline,
            "the segmented transfer never recorded durable progress"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn segmented_download_is_byte_exact_and_verifies_integrity() {
    let body = zim_bytes(b's', 4 * 1024 * 1024 + 12_345);
    let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
    server.state.lock().unwrap().metalink = true;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(
            server.metalink_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            None,
            None,
        )
        .await
        .unwrap();
    let job = finished_job(&manager, &job.id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    assert_eq!(
        job.sha256.as_deref(),
        Some(hex::encode(Sha256::digest(&body)).as_str())
    );
    let stored = manager.inner.snapshot(&job.id).unwrap();
    assert!(
        stored.segments.is_none() && !stored.segmented,
        "the completed row kept a transfer layout"
    );
    assert!(
        server.range_requests().len() >= SEGMENT_COUNT,
        "expected one range request per segment"
    );
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn resume_mid_segment_continues_at_durable_offsets() {
    let body = zim_bytes(b'r', 4 * 1024 * 1024);
    let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
    // Hold every range after its durable checkpoint: a fast runner must not
    // finish the transfer between the observed progress and the pause.
    server.hold_mid_range();
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(body.len() as u64),
            None,
        )
        .await
        .unwrap();
    let progressed = wait_for_segment_progress(&manager, &job.id).await;
    assert!(progressed.segments.is_some());
    manager.pause(&job.id).await.unwrap();
    let paused = manager.inner.snapshot(&job.id).unwrap();
    assert_eq!(paused.job.status, "paused", "{:?}", paused.job.error);
    let plan = paused.segments.clone().expect("plan disappeared on pause");
    assert!(!plan.complete(), "pause happened after the whole transfer");
    let resumed_offsets: Vec<u64> = plan
        .ranges
        .iter()
        .filter(|range| range.done > 0)
        .map(|range| range.start + range.done)
        .collect();
    assert!(!resumed_offsets.is_empty());
    let before = server.range_requests().len();
    // Recreate the manager: the preallocated part must be read through the
    // durable plan, never through its file length.
    drop(manager);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let loaded = manager.inner.snapshot(&job.id).unwrap();
    assert_eq!(loaded.job.status, "paused");
    assert!(loaded.segmented);
    assert_eq!(loaded.job.downloaded, plan.durable());
    let (_, part) = paths(&loaded.job).unwrap();
    assert_eq!(fs::metadata(&part).unwrap().len(), plan.total);
    server.release();
    manager.resume(&job.id, None).await.unwrap();
    let job = finished_job(&manager, &job.id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    let requests = server.range_requests();
    for offset in resumed_offsets {
        assert!(
            requests[before..]
                .iter()
                .any(|request| test_range_bounds(request).is_some_and(|(start, _)| start == offset)),
            "no resumed request started at durable offset {offset}"
        );
    }
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn changed_validator_restarts_a_segmented_transfer() {
    let first = zim_bytes(b'1', 4 * 1024 * 1024);
    let second = zim_bytes(b'2', first.len());
    let server = test_range_server(first.clone(), "\"v1\"", Duration::ZERO).await;
    server.hold_mid_range();
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(first.len() as u64),
            None,
        )
        .await
        .unwrap();
    let progressed = wait_for_segment_progress(&manager, &job.id).await;
    assert!(progressed.segments.is_some());
    manager.pause(&job.id).await.unwrap();
    let paused = manager.inner.snapshot(&job.id).unwrap();
    assert!(paused.segments.as_ref().is_some_and(|plan| !plan.complete()));
    server.set_body(second.clone(), "\"v2\"");
    server.release();
    manager.resume(&job.id, None).await.unwrap();
    let job = finished_job(&manager, &job.id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), second);
    let requests = server.requests();
    assert!(requests.iter().any(|request| request.contains("if-range: \"v1\"")));
    assert!(
        requests
            .iter()
            .any(|request| request.contains("if-range: \"v2\"")),
        "the restarted transfer never used the new validator"
    );
    let stored = manager.inner.snapshot(&job.id).unwrap();
    assert!(stored.segments.is_none() && !stored.segmented);
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancel_during_segments_removes_only_the_owned_part() {
    let body = zim_bytes(b'c', 4 * 1024 * 1024);
    let server = test_range_server(body, "\"v1\"", Duration::ZERO).await;
    server.hold_mid_range();
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(4 * 1024 * 1024),
            None,
        )
        .await
        .unwrap();
    wait_for_segment_progress(&manager, &job.id).await;
    let sibling = root.join("keep.bin");
    fs::write(&sibling, b"unrelated").unwrap();
    manager.cancel(&job.id).await.unwrap();
    let job = manager.inner.snapshot(&job.id).unwrap().job;
    assert_eq!(job.status, "cancelled");
    assert_eq!(job.downloaded, 0);
    let (destination, part) = paths(&job).unwrap();
    assert!(!part.exists(), "the owned part survived cancellation");
    assert!(!destination.exists());
    assert_eq!(fs::read(&sibling).unwrap(), b"unrelated");
    server.release();
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn shared_limit_caps_the_aggregate_of_all_segments() {
    let body = zim_bytes(b'l', 1024 * 1024);
    let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    manager.set_download_limit(256 * 1024).unwrap();
    let started = Instant::now();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(body.len() as u64),
            None,
        )
        .await
        .unwrap();
    let job = finished_job(&manager, &job.id).await;
    let elapsed = started.elapsed();
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    assert!(
        elapsed >= Duration::from_millis(3500),
        "four segments consumed the 256 KiB/s budget independently in {elapsed:?}"
    );
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn server_without_range_support_falls_back_to_one_stream() {
    for advertise in [false, true] {
        let body = zim_bytes(b'f', 2 * 1024 * 1024);
        let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
        {
            let mut state = server.state.lock().unwrap();
            state.ignore_ranges = true;
            state.advertise_ranges = advertise;
        }
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let job = manager
            .start(
                server.archive_url.clone(),
                root.clone(),
                Some("fixture.zim".into()),
                Some(body.len() as u64),
                None,
            )
            .await
            .unwrap();
        let job = finished_job(&manager, &job.id).await;
        assert_eq!(job.status, "complete", "{:?}", job.error);
        assert_eq!(fs::read(&job.destination).unwrap(), body);
        if advertise {
            assert!(
                !server.range_requests().is_empty(),
                "the advertised range support was never exercised"
            );
        } else {
            assert!(server.range_requests().is_empty());
        }
        server.task.abort();
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn a_cancel_persisted_at_the_end_of_transfer_is_never_overwritten_by_verifying() {
    // Cancel lands after the last byte but before the worker's `verifying`
    // checkpoint. A crash right after that checkpoint must not restart with a
    // durable `verifying` row that startup finalize would publish.
    for action in [Signal::Cancel, Signal::Pause] {
        let body = zim_bytes(b'v', 256 * 1024);
        let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
        server.state.lock().unwrap().ignore_ranges = true;
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let (before_entered, before_entering) = tokio::sync::oneshot::channel();
        let (before_release, before_releasing) = tokio::sync::oneshot::channel();
        let (after_entered, mut after_entering) = tokio::sync::oneshot::channel();
        let (after_release, after_releasing) = tokio::sync::oneshot::channel();
        *manager.inner.verify_gates.lock().unwrap() = [
            Some(AsyncGate { entered: before_entered, release: before_releasing }),
            Some(AsyncGate { entered: after_entered, release: after_releasing }),
        ];
        let job = manager
            .start(server.archive_url.clone(), root.clone(), Some("fixture.zim".into()), Some(body.len() as u64), None)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(20), before_entering).await.unwrap().unwrap();
        let stopper = manager.clone();
        let stop_id = job.id.clone();
        let stopping = tokio::spawn(async move { stopper.stop(&stop_id, action).await });
        let expected = if action == Signal::Cancel { "cancelled" } else { "paused" };
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.inner.snapshot(&job.id).unwrap().job.status != expected {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        before_release.send(()).unwrap();
        let journal = root.join("state/downloads.json");
        let persisted = |journal: &Path, id: &str| -> String {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(journal).unwrap()).unwrap();
            let text = value.to_string();
            assert!(text.contains(id));
            text
        };
        tokio::select! {
            _ = &mut after_entering => {
                // Simulated crash point: what would a restart load?
                let text = persisted(&journal, &job.id);
                assert!(!text.contains("\"verifying\""), "{action:?}: a persisted stop was overwritten: {text}");
                after_release.send(()).unwrap();
            }
            result = tokio::time::timeout(Duration::from_secs(20), stopping) => {
                result.unwrap().unwrap().unwrap();
                let finished = finished_job(&manager, &job.id).await;
                assert_eq!(finished.status, expected);
                assert!(!persisted(&journal, &job.id).contains("\"verifying\""));
                server.task.abort();
                drop(manager);
                drop(store);
                fs::remove_dir_all(root).unwrap();
                continue;
            }
        }
        panic!("{action:?}: the worker reached its post-verifying checkpoint after a persisted stop");
    }
}

// Reproducible throughput harness: `cargo test --release --lib \
// --features localswarm-profiling bench_loopback_throughput -- --ignored --nocapture`.
#[cfg(unix)] // getrusage(2) is Unix-only; the profiling harness is too.
fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) }, 0);
    let usage = unsafe { usage.assume_init() };
    usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1e6
        + usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1e6
}

#[cfg(unix)] // Uses cpu_seconds (getrusage).
#[tokio::test]
#[ignore]
async fn bench_loopback_throughput() {
    let size = 256usize * 1024 * 1024;
    let server = test_range_server(zim_bytes(b'b', size), "\"stable\"", Duration::ZERO).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let cpu_start = cpu_seconds();
    let started = Instant::now();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("bench.zim".into()),
            Some(size as u64),
            None,
        )
        .await
        .unwrap();
    let job = loop {
        let job = manager.inner.snapshot(&job.id).unwrap().job;
        if !manager.inner.state().unwrap().running.contains_key(&job.id) { break job; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let elapsed = started.elapsed().as_secs_f64();
    let cpu = cpu_seconds() - cpu_start;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(root.join("bench.zim")).unwrap().len(), size);
    println!("BENCH loopback: {:.1} MiB/s, {:.2} s, cpu {:.2} s, {:.2} cpu-s/GiB, {} range requests",
        size as f64 / 1048576.0 / elapsed, elapsed, cpu,
        cpu / (size as f64 / 1073741824.0), server.range_requests().len());
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore]
async fn bench_remote_throughput() {
    let url = "https://download.kiwix.org/zim/wikipedia/wikipedia_af_all_nopic_2026-10.zim".to_string();
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let started = Instant::now();
    let job = manager
        .start(url, root.clone(), Some("remote.zim".into()), None, None)
        .await
        .unwrap();
    let mut last = 0u64;
    let mut last_at = started;
    let mut samples = Vec::new();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let snapshot = manager.inner.snapshot(&job.id).unwrap().job;
        if snapshot.downloaded > last {
            let now = Instant::now();
            let rate = (snapshot.downloaded - last) as f64
                / now.duration_since(last_at).as_secs_f64();
            samples.push(rate);
            last = snapshot.downloaded;
            last_at = now;
        }
        if started.elapsed() > Duration::from_secs(20) { break; }
    }
    let downloaded = manager.inner.snapshot(&job.id).unwrap().job.downloaded;
    manager.cancel(&job.id).await.unwrap();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("BENCH remote: {:.2} MiB/s average, {:.2} MiB/s median, {} bytes in {:.1} s",
        downloaded as f64 / 1048576.0 / started.elapsed().as_secs_f64(),
        samples.get(samples.len() / 2).copied().unwrap_or(0.0) / 1048576.0,
        downloaded, started.elapsed().as_secs_f64());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn overflowing_segment_end_is_rejected_when_restoring_history() {
    let root = test_directory();
    let store = test_store(&root);
    let bytes = fs::read(fixture_archive()).unwrap();
    let id = Uuid::new_v4().to_string();
    let mut stored = durable_stored(&root, &id, &bytes, "paused");
    stored.segments.as_mut().unwrap().ranges = vec![SegmentRange {
        start: 0,
        end: u64::MAX,
        done: 0,
    }];
    let data_dir = root.join("state");
    write_state(&data_dir, stored);

    let manager = DownloadManager::new(data_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let restored = manager.inner.snapshot(&id).unwrap();
    assert!(!restored.segmented);
    assert!(restored.segments.is_none());
    assert_eq!(restored.job.downloaded, 0);
    assert!(restored.etag.is_none());
    assert_eq!(fs::read(root.join(format!(".kiwix-{id}.part"))).unwrap(), bytes);
    assert!(!root.join("fixture.zim").exists());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_fully_durable_segmented_row_is_finalized_on_load() {
    let root = test_directory();
    let store = test_store(&root);
    let data_dir = root.join("state");
    fs::create_dir_all(&data_dir).unwrap();
    let bytes = fs::read(fixture_archive()).unwrap();
    let total = bytes.len() as u64;
    let id = Uuid::new_v4().to_string();
    let destination = root.join("fixture.zim");
    let part = root.join(format!(".kiwix-{id}.part"));
    fs::write(&part, &bytes).unwrap();
    let ownership =
        crate::torrent::CleanupEntry::retain(&part, &File::open(&part).unwrap()).unwrap();
    // Exactly the shape reported from a wedged device: status stays
    // "downloading", both ranges are fully durable, and the part is whole.
    let plan = SegmentPlan {
        total,
        validator: "\"6aad173b-a3aa5b63\"".into(),
        ranges: vec![
            SegmentRange {
                start: 0,
                end: total - 2,
                done: total - 1,
            },
            SegmentRange {
                start: total - 1,
                end: total - 1,
                done: 1,
            },
        ],
    };
    assert_eq!(plan.durable(), plan.total);
    let stored = StoredJob {
        job: DownloadJob {
            id: id.clone(),
            url: "http://127.0.0.1:9/fixture.zim".into(),
            file_name: "fixture.zim".into(),
            destination: destination.to_str().unwrap().to_string(),
            status: "downloading".into(),
            interrupted: false,
            downloaded: total,
            total: Some(total),
            error: None,
            history_warning: None,
            speed: 0,
            eta_seconds: None,
            created_at: now(),
            updated_at: now(),
            sha256: None,
        },
        transport: None,
        expected_size: Some(total),
        transfer_url: Some("http://127.0.0.1:9/fixture.zim".into()),
        etag: Some("\"6aad173b-a3aa5b63\"".into()),
        last_modified: None,
        publication: None,
        history_dismissed: false,
        files_deleted: false,
        part_ownership: Some(ownership),
        published_ownership: None,
        segments: Some(plan),
        segmented: true,
        verification: None,
        native: None,
        space_acknowledgement: None,
    };
    let mut jobs = BTreeMap::new();
    jobs.insert(id.clone(), stored);
    fs::write(
        data_dir.join("downloads.json"),
        serde_json::to_vec(&jobs).unwrap(),
    )
    .unwrap();

    let manager = DownloadManager::new(data_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    assert!(loaded.segments.is_none() && !loaded.segmented);
    assert_eq!(loaded.job.downloaded, total);
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert!(!part.exists(), "publication must consume the verified part");
    assert_eq!(manager.list().unwrap().len(), 1);
    assert_eq!(
        manager.inner.hash_passes.load(Ordering::SeqCst),
        1,
        "the device row must finalize with exactly one checksum pass"
    );
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
fn native_stored(root: &Path, id: &str, bytes: &[u8]) -> StoredJob {
    let mut stored = durable_stored(root, id, bytes, "verifying");
    stored.segments = None;
    stored.segmented = false;
    stored.job.downloaded = 0;
    stored.job.sha256 = Some(hex::encode(Sha256::digest(bytes)));
    stored.native = Some(NativeAcquisition {
        generation: Uuid::new_v4().to_string(), copied: 0, total: None,
        stage_identity: None, retirement_error: None,
    });
    let (_, part) = paths(&stored.job).unwrap();
    let file = writable_options().open(&part).unwrap();
    file.set_len(0).unwrap();
    file.sync_all().unwrap();
    stored
}

/// Receipt identity of a part, read from an open handle as production does
/// (Windows path metadata carries no volume/file index).
fn part_receipt_identity(part: &Path) -> Option<PartIdentity> {
    file_identity(&File::open(part).unwrap()).unwrap()
}

#[tokio::test]
async fn the_download_manager_owns_its_data_directory_exclusively() {
    let gate = || crate::update_admission::AdmissionGate::default();
    let root = test_directory();
    let store = test_store(&root);
    let state_dir = root.join("state");
    let owner = DownloadManager::new(state_dir.clone(), store.clone(), gate()).unwrap();
    let refused = DownloadManager::new(state_dir.clone(), store.clone(), gate()).err().unwrap();
    assert!(refused.contains("Another download manager owns this data directory"), "{refused}");
    drop(owner);
    // A whole-file holder (an older build, or fs_ext) also excludes a new manager.
    let holder = writable_options_shared().open(state_dir.join("downloads.lock")).unwrap();
    crate::fs_ext::FileExt::try_flock_exclusive(&holder).unwrap();
    let refused = DownloadManager::new(state_dir.clone(), store.clone(), gate()).err().unwrap();
    assert!(refused.contains("Another download manager owns this data directory"), "{refused}");
    drop(holder);
    drop(DownloadManager::new(state_dir, store, gate()).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_part_ownership_lock_excludes_other_owners_but_never_blocks_payload_io() {
    let root = test_directory();
    let part = root.join(".kiwix-lock.part");
    fs::write(&part, vec![7u8; 4096]).unwrap();
    let owner = open_part(&part).unwrap();
    // A second owner (sentinel or whole-file lock) is refused while held.
    assert!(open_part(&part).unwrap_err().contains("already in use"));
    let contender = writable_options_shared().open(&part).unwrap();
    assert!(try_lock_exclusive(&contender).is_err());
    assert!(crate::fs_ext::FileExt::try_flock_shared(&contender).is_err());
    // Segment workers write through their own handles and recovery readers
    // read, exactly as with Unix advisory flock. A whole-file Windows lock
    // failed both with ERROR_LOCK_VIOLATION (os error 33).
    let identity = owner.metadata().unwrap();
    let mut segment =
        open_segment_part(&part, &identity, file_identity(&owner).unwrap().as_ref()).unwrap();
    segment.seek(std::io::SeekFrom::Start(1024)).unwrap();
    segment.write_all(&[9u8; 16]).unwrap();
    segment.sync_data().unwrap();
    assert_eq!(&fs::read(&part).unwrap()[1024..1040], &[9u8; 16]);
    drop((segment, contender, owner));
    // The lock is released with its handle.
    drop(open_part(&part).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_same_length_replacement_cannot_receive_segment_writes() {
    let root = test_directory();
    let part = root.join(".kiwix-segment.part");
    fs::write(&part, vec![1u8; 4096]).unwrap();
    let owner = open_part(&part).unwrap();
    let identity = owner.metadata().unwrap();
    let handle = file_identity(&owner).unwrap();
    assert!(handle.is_some(), "this platform must prove part identity from the handle");
    // Swap in a same-length file under the coordinator. Windows path
    // metadata alone (length) cannot tell these apart.
    fs::rename(&part, root.join("moved.part")).unwrap();
    fs::write(&part, vec![2u8; 4096]).unwrap();
    assert!(open_segment_part(&part, &identity, handle.as_ref())
        .unwrap_err()
        .contains("changed identity"));
    assert_eq!(fs::read(&part).unwrap(), vec![2u8; 4096]);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn handle_identity_is_stable_across_rename_and_distinct_for_another_file() {
    let root = test_directory();
    let bytes = zim_bytes(b'h', 1024);
    let stored = durable_stored(&root, &Uuid::new_v4().to_string(), &bytes, "paused");
    let (_, first) = paths(&stored.job).unwrap();
    let second = root.join("second.part");
    fs::write(&second, &bytes).unwrap();
    let before = part_receipt_identity(&first).unwrap();
    assert_eq!((before.links, before.size), (1, bytes.len() as u64));
    let renamed = root.join("renamed.part");
    fs::rename(&first, &renamed).unwrap();
    assert_eq!(part_receipt_identity(&renamed).unwrap(), before);
    let other = part_receipt_identity(&second).unwrap();
    assert_ne!((other.device, other.inode), (before.device, before.inode));
    // A receipt for one file never matches another, even with equal bytes.
    let mut stored = stored;
    stored.verification = verification_receipt(
        Some(before.clone()), &"0".repeat(64), VerificationOutcome::Passed, None,
    );
    assert!(matching_receipt(&stored, Some(&before)).is_some());
    assert!(matching_receipt(&stored, Some(&other)).is_none());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
fn native_stage(root: &Path, bytes: &[u8]) -> NativeStage {
    let path = root.join("native-stage.zim");
    fs::write(&path, bytes).unwrap();
    let identity = part_identity(&fs::metadata(&path).unwrap()).unwrap();
    NativeStage { path, total: bytes.len() as u64, device: identity.device, inode: identity.inode }
}
#[cfg(all(unix, not(target_os = "ios")))]
fn native_transport_fixture(root: &Path, stored: &StoredJob, stage: Option<&NativeStage>) -> native_transport::Transport {
    native_transport::Transport {
        snapshot: Mutex::new(crate::ios_background::NativeSnapshot {
            state: if stage.is_some() { "complete" } else { "missing" }.into(),
            staging_directory: root.join("not-yet-created-native-root").to_str().unwrap().into(),
            copied: stage.map_or(0, |stage| stage.total),
            total: stage.map(|stage| stage.total),
            path: stage.map(|stage| stage.path.to_str().unwrap().into()),
            error: None,
            response_status: stage.map(|_| 200),
            source_url: stored.transfer_url.clone(),
            final_url: Some("https://download.example/fixture.zim".into()),
            allows_cellular: false,
            device: stage.map(|stage| stage.device),
            inode: stage.map(|stage| stage.inode),
        }),
        starts: AtomicUsize::new(0),
        acknowledgements: AtomicUsize::new(0),
        stops: Mutex::new(Vec::new()),
        stop_entered: [tokio::sync::Semaphore::new(0), tokio::sync::Semaphore::new(0)],
        stop_release: [None, None],
        reject_cancel: false,
    }
}

#[cfg(all(unix, not(target_os = "ios")))]
async fn await_native_worker(manager: &DownloadManager, id: &str) {
    let running = manager.inner.state().unwrap().running.get(id).cloned();
    if let Some(mut running) = running {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !*running.finished.borrow() {
                running.finished.changed().await.unwrap();
            }
        }).await.unwrap();
    }
}

#[cfg(all(unix, not(target_os = "ios")))]
#[tokio::test]
async fn fresh_direct_native_start_requires_only_one_combined_confirmation() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    manager.inner.warning_available.store(1_400_000_000, Ordering::SeqCst);
    // Both a single 500 MB copy and the combined 1 GB projection are below
    // the advisory threshold. The initial request must not prompt for the
    // weaker, single-copy admission before the worker's combined warning.
    let job = manager.start_transport(
        "https://download.example/direct.zim".into(), root.clone(), Some("direct.zim".into()),
        Some(500_000_000), None,
        AcquisitionSource::Internet(Some(NativeAcquisition {
            generation: Uuid::new_v4().to_string(), copied: 0, total: None,
            stage_identity: None, retirement_error: None,
        })),
    ).await.expect("native start must defer advisory consent to combined admission");
    let stored = manager.inner.snapshot(&job.id).unwrap();
    let transport = native_transport::register(&job.id, native_transport_fixture(&root, &stored, None));
    await_native_worker(&manager, &job.id).await;
    let failed = manager.inner.snapshot(&job.id).unwrap();
    let warning: crate::disk_warning::SpaceWarning = serde_json::from_str(
        failed.job.error.as_ref().unwrap().strip_prefix(crate::disk_warning::WARNING_PREFIX).unwrap(),
    ).unwrap();
    assert_eq!(warning.remaining_bytes, 1_000_000_000);
    assert_eq!(transport.1.starts.load(Ordering::SeqCst), 0);
    manager.resume(&job.id, Some(warning.acknowledgement)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while transport.1.starts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    manager.cancel(&job.id).await.unwrap();
    assert_eq!(transport.1.starts.load(Ordering::SeqCst), 1);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, not(target_os = "ios")))]
#[tokio::test]
async fn resolved_native_admission_reserves_stage_and_copy_before_transport_and_accepts_retry_consent() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = Uuid::new_v4().to_string();
    let mut stored = native_stored(&root, &id, &zim_bytes(b'a', 128));
    // Model the exact authoritative Metalink result already committed by
    // resolution. No huge payload is created or allocated by this test.
    let available = crate::fs_ext::available_space(&root).unwrap();
    let total = (available - crate::disk_warning::THRESHOLD_BYTES / 2) / 2;
    stored.expected_size = Some(total);
    stored.job.total = Some(total);
    stored.job.url = "https://download.example/fixture.zim.meta4".into();
    stored.transfer_url = Some("https://download.example/fixture.zim".into());
    stored.job.status = "queued".into();
    let (_, part) = paths(&stored.job).unwrap();
    let transport = native_transport::register(&id, native_transport_fixture(&root, &stored, None));
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
        spawn_worker(&manager.inner, &mut state, &id);
    }
    await_native_worker(&manager, &id).await;
    assert_eq!(transport.1.starts.load(Ordering::SeqCst), 0, "native payload must not start before consent");
    assert_eq!(fs::metadata(&part).unwrap().len(), 0);
    let failed = manager.inner.snapshot(&id).unwrap();
    let warning: crate::disk_warning::SpaceWarning = serde_json::from_str(
        failed.job.error.as_ref().unwrap().strip_prefix(crate::disk_warning::WARNING_PREFIX).unwrap(),
    ).unwrap();
    assert_eq!(warning.remaining_bytes, 2 * total, "stage and canonical part coexist");
    assert!(warning.projected_free_bytes < warning.threshold_bytes);
    manager.resume(&id, Some(warning.acknowledgement)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while transport.1.starts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    manager.cancel(&id).await.unwrap();
    assert_eq!(transport.1.starts.load(Ordering::SeqCst), 1, "native consent must not be replaced by single-copy consent on retry");
    assert!(!part.exists());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, not(target_os = "ios")))]
#[tokio::test]
async fn native_transport_rejects_capacity_that_fits_only_one_copy() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let id = Uuid::new_v4().to_string();
    let mut stored = native_stored(&root, &id, &zim_bytes(b'h', 128));
    let available = crate::fs_ext::available_space(&root).unwrap();
    let total = available / 4 * 3;
    stored.expected_size = Some(total);
    stored.job.total = Some(total);
    stored.job.status = "queued".into();
    let (_, part) = paths(&stored.job).unwrap();
    let transport = native_transport::register(&id, native_transport_fixture(&root, &stored, None));
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
        spawn_worker(&manager.inner, &mut state, &id);
    }
    await_native_worker(&manager, &id).await;
    let failed = manager.inner.snapshot(&id).unwrap();
    assert_eq!(failed.job.status, "failed");
    assert!(failed.job.error.unwrap().contains("Not enough disk space"));
    assert_eq!(transport.1.starts.load(Ordering::SeqCst), 0);
    assert_eq!(fs::metadata(&part).unwrap().len(), 0);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, not(target_os = "ios")))]
#[tokio::test]
async fn service_resume_adopts_retained_native_completion_without_transport_restart() {
    let root = test_directory();
    let store = test_store(&root);
    let bytes = zim_bytes(b'o', WRITE_BUDGET * 2 + 4096);
    let id = Uuid::new_v4().to_string();
    let mut stored = native_stored(&root, &id, &bytes);
    stored.job.status = "paused".into();
    stored.job.url = "https://offline.invalid/fixture.zim".into();
    stored.transfer_url = Some(stored.job.url.clone());
    let (destination, part) = paths(&stored.job).unwrap();
    let original = part_identity(&fs::metadata(&part).unwrap()).unwrap();
    let stage = native_stage(&root, &bytes);
    let transport = native_transport::register(&id, native_transport_fixture(&root, &stored, Some(&stage)));
    write_state(&store.data_dir().join("downloads"), stored);
    drop(store);
    let service = crate::service::Service::new(root.join("profile"), false, false).unwrap();
    service.clone().dispatch("resume_download", serde_json::json!({"id":id})).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !destination.exists() {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    let published = part_identity(&fs::metadata(&destination).unwrap()).unwrap();
    assert_eq!((published.device, published.inode), (original.device, original.inode));
    assert_eq!(transport.1.starts.load(Ordering::SeqCst), 0);
    assert_eq!(transport.1.acknowledgements.load(Ordering::SeqCst), 1);
    assert!(!stage.path.exists());
    assert!(!part.exists());
    service.shutdown().await.unwrap();
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, not(target_os = "ios")))]
#[tokio::test]
async fn service_native_local_resume_retains_read_only_and_private_policy() {
    for (private, read_only) in [(true, false), (false, true)] {
        let root = test_directory();
        let store = test_store(&root);
        let bytes = zim_bytes(b'p', 256);
        let id = Uuid::new_v4().to_string();
        let mut stored = native_stored(&root, &id, &bytes);
        stored.job.status = "paused".into();
        let (destination, part) = paths(&stored.job).unwrap();
        let stage = native_stage(&root, &bytes);
        let transport = native_transport::register(&id, native_transport_fixture(&root, &stored, Some(&stage)));
        write_state(&store.data_dir().join("downloads"), stored);
        drop(store);
        let service = crate::service::Service::new(root.join("profile"), private, read_only).unwrap();
        let result = service.clone().dispatch("resume_download", serde_json::json!({"id":id})).await;
        if read_only {
            assert!(result.is_err(), "local receipt must not bypass read-only dispatch");
        }
        service.shutdown().await.unwrap();
        assert!(!destination.exists());
        assert_eq!(fs::metadata(&part).unwrap().len(), 0);
        assert_eq!(fs::read(&stage.path).unwrap(), bytes);
        assert_eq!(transport.1.starts.load(Ordering::SeqCst), 0);
        assert_eq!(transport.1.acknowledgements.load(Ordering::SeqCst), 0);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(unix, not(target_os = "ios")))]
#[tokio::test]
async fn late_native_cancel_waits_for_escalated_retirement_and_reports_rejection() {
    for reject_cancel in [false, true] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
        let bytes = zim_bytes(b'c', 256);
        let id = Uuid::new_v4().to_string();
        let mut stored = native_stored(&root, &id, &bytes);
        stored.job.status = "queued".into();
        let (_, part) = paths(&stored.job).unwrap();
        let stage = native_stage(&root, &bytes);
        let mut fake = native_transport_fixture(&root, &stored, Some(&stage));
        // A real worker adoption rejection starts the original stop(false).
        fake.snapshot.get_mut().unwrap().response_status = Some(500);
        let release_pause = Arc::new(tokio::sync::Semaphore::new(0));
        let release_cancel = Arc::new(tokio::sync::Semaphore::new(0));
        fake.stop_release = [Some(release_pause.clone()), Some(release_cancel.clone())];
        fake.reject_cancel = reject_cancel;
        let transport = native_transport::register(&id, fake);
        {
            let mut state = manager.inner.state().unwrap();
            state.jobs.insert(id.clone(), stored);
            manager.inner.save(&state).unwrap();
            spawn_download_worker(&manager.inner, &mut state, &id, WorkerMode::NativeAttach);
        }
        tokio::time::timeout(Duration::from_secs(10), transport.1.stop_entered[0].acquire()).await.unwrap().unwrap().forget();
        let cancel_manager = manager.clone();
        let cancel_id = id.clone();
        let cancellation = tokio::spawn(async move { cancel_manager.cancel(&cancel_id).await });
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.inner.snapshot(&id).unwrap().job.status != "cancelled" {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        release_pause.add_permits(1);
        tokio::time::timeout(Duration::from_secs(10), transport.1.stop_entered[1].acquire()).await.unwrap().unwrap().forget();
        assert!(!cancellation.is_finished(), "Cancel cannot report success before stop(true) acknowledgement");
        assert!(stage.path.exists());
        release_cancel.add_permits(1);
        let result = tokio::time::timeout(Duration::from_secs(10), cancellation).await.unwrap().unwrap();
        if reject_cancel {
            assert!(result.unwrap_err().contains("did not acknowledge"));
            assert_eq!(fs::read(&stage.path).unwrap(), bytes, "failed native retirement retains its recoverable receipt");
            assert!(manager.inner.snapshot(&id).unwrap().job.error.unwrap().contains("did not acknowledge"));
        } else {
            result.unwrap();
            assert!(!stage.path.exists());
        }
        assert_eq!(*transport.1.stops.lock().unwrap(), vec![false, true]);
        assert!(!part.exists());
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn native_progress_samples_reset_on_attach_regression_and_stall() {
    let root = test_directory();
    let id = Uuid::new_v4().to_string();
    let mut job = native_stored(&root, &id, &zim_bytes(b's', 128)).job;
    let start = Instant::now();
    let mut sample = NativeProgressSample::new(start, 400);
    sample.update(&mut job, 600, Some(1000), start + Duration::from_secs(2));
    assert_eq!((job.speed, job.eta_seconds), (100, Some(4)));
    sample.update(&mut job, 600, None, start + Duration::from_secs(3));
    assert_eq!((job.speed, job.eta_seconds), (0, None), "stalled native bytes must not keep a stale positive rate");
    sample.update(&mut job, 10, Some(500), start + Duration::from_secs(4));
    assert_eq!((job.speed, job.eta_seconds), (0, None), "counter regression resets rather than underflowing");
    sample.update(&mut job, 110, None, start + Duration::from_secs(6));
    assert_eq!((job.speed, job.eta_seconds), (50, Some(8)));
    sample = NativeProgressSample::new(start + Duration::from_secs(20), 450);
    sample.update(&mut job, 450, None, start + Duration::from_secs(21));
    assert_eq!((job.speed, job.eta_seconds), (0, None), "reattachment must not count detached daemon progress");
    sample.update(&mut job, 460, None, start + Duration::from_secs(21));
    assert_eq!((job.speed, job.eta_seconds), (0, None), "zero monotonic elapsed time cannot produce a burst");
    sample.update(&mut job, 470, None, start + Duration::from_secs(22));
    assert_eq!((job.speed, job.eta_seconds), (10, Some(3)));
    sample.update(&mut job, 481, None, start + Duration::from_secs(23));
    assert_eq!((job.speed, job.eta_seconds), (11, Some(2)), "fractional remaining time rounds up");
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[tokio::test]
async fn native_adoption_pause_and_restart_reuse_the_synced_owned_prefix() {
    let root = test_directory();
    let store = test_store(&root);
    let data_dir = root.join("state");
    let manager = DownloadManager::new(
        data_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default(),
    ).unwrap();
    let bytes = zim_bytes(b'n', WRITE_BUDGET * 2 + 4096);
    let id = Uuid::new_v4().to_string();
    let stored = native_stored(&root, &id, &bytes);
    let (destination, part) = paths(&stored.job).unwrap();
    let original_part = part_identity(&fs::metadata(&part).unwrap()).unwrap();
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    let stage = native_stage(&root, &bytes);
    let (entered, entering) = tokio::sync::oneshot::channel();
    let (release, releasing) = std::sync::mpsc::sync_channel(1);
    *manager.inner.native_copy_gate.lock().unwrap() = Some(FinalizeGate { entered, release: releasing });
    let owner = Arc::clone(&manager.inner);
    let copy_id = id.clone();
    let copying = tokio::task::spawn_blocking(move || {
        let result = copy_native_completion(&owner, &copy_id, &stage);
        (stage, result)
    });
    tokio::time::timeout(Duration::from_secs(10), entering).await.unwrap().unwrap();
    manager.inner.update(&id, |stored| {
        stored.job.status = "paused".into();
        stored.job.interrupted = false;
    }).unwrap();
    release.send(()).unwrap();
    let (stage, result) = copying.await.unwrap();
    assert_eq!(result.unwrap(), Some(Signal::Pause));
    assert_eq!(manager.inner.native_copy_bytes.load(Ordering::SeqCst), WRITE_BUDGET as u64);
    assert_eq!(manager.inner.snapshot(&id).unwrap().native.unwrap().copied, WRITE_BUDGET as u64);
    assert_eq!(fs::metadata(&part).unwrap().len(), WRITE_BUDGET as u64);
    assert!(!destination.exists());
    drop(manager);
    let manager = DownloadManager::new(
        data_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default(),
    ).unwrap();
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "paused");
    manager.inner.update(&id, |stored| stored.job.status = "verifying".into()).unwrap();
    assert_eq!(copy_native_completion(&manager.inner, &id, &stage).unwrap(), None);
    assert_eq!(manager.inner.native_copy_bytes.load(Ordering::SeqCst), bytes.len() as u64 - WRITE_BUDGET as u64);
    let adopted_part = part_identity(&fs::metadata(&part).unwrap()).unwrap();
    assert_eq!(adopted_part.device, original_part.device);
    assert_eq!(adopted_part.inode, original_part.inode);
    assert_eq!(durable_total(&manager.inner.snapshot(&id).unwrap()), Some(bytes.len() as u64));
    assert_eq!(fs::read(&stage.path).unwrap(), bytes, "transport receipt survives until native acknowledgement");
    manager.inner.update(&id, |stored| {
        stored.job.status = "paused".into();
        stored.job.interrupted = false;
    }).unwrap();
    drop(manager);
    let manager = DownloadManager::new(
        data_dir, store.clone(), crate::update_admission::AdmissionGate::default(),
    ).unwrap();
    assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "paused");
    assert!(!destination.exists(), "durable native adoption cannot erase a manual pause at restart");
    manager.inner.update(&id, |stored| stored.job.status = "verifying".into()).unwrap();
    assert_eq!(finalize_durable_download(&manager.inner, &id, FinalizeMode::Startup).unwrap(), Outcome::Complete);
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert!(!part.exists());
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn native_adoption_rejects_a_replaced_staging_inode_without_touching_the_part() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(
        root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default(),
    ).unwrap();
    let bytes = zim_bytes(b'r', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let stored = native_stored(&root, &id, &bytes);
    let (_, part) = paths(&stored.job).unwrap();
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    let stage = native_stage(&root, &bytes);
    let retained = root.join("retained-stage.zim");
    fs::rename(&stage.path, &retained).unwrap();
    let foreign = zim_bytes(b'x', bytes.len());
    fs::write(&stage.path, &foreign).unwrap();
    assert!(copy_native_completion(&manager.inner, &id, &stage).is_err());
    assert_eq!(fs::metadata(&part).unwrap().len(), 0);
    assert_eq!(manager.inner.native_copy_bytes.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(&stage.path).unwrap(), foreign);
    assert_eq!(fs::read(&retained).unwrap(), bytes);
    assert_eq!(durable_total(&manager.inner.snapshot(&id).unwrap()), None);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_persisted_verifying_row_finishes_on_load() {
    let root = test_directory();
    let store = test_store(&root);
    let data_dir = root.join("state");
    let bytes = zim_bytes(b'v', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let stored = durable_stored(&root, &id, &bytes, "verifying");
    write_state(&data_dir, stored);
    let manager = DownloadManager::new(data_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    assert_eq!(fs::read(&loaded.job.destination).unwrap(), bytes);
    assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 1);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn manually_paused_durable_downloads_stay_paused_until_explicit_resume() {
    for with_receipt in [false, true] {
        let root = test_directory();
        let store = test_store(&root);
        let data_dir = root.join("state");
        let bytes = zim_bytes(b'p', 256 * 1024);
        let id = Uuid::new_v4().to_string();
        let mut stored = durable_stored(&root, &id, &bytes, "paused");
        let (destination, part) = paths(&stored.job).unwrap();
        if with_receipt {
            let digest = hex::encode(Sha256::digest(&bytes));
            stored.verification = verification_receipt(
                part_receipt_identity(&part), &digest, VerificationOutcome::Passed, None,
            );
            stored.job.sha256 = Some(digest);
        }
        write_state(&data_dir, stored);
        let manager = DownloadManager::new(
            data_dir.clone(), store.clone(), crate::update_admission::AdmissionGate::default(),
        ).unwrap();
        manager.recover_durable_local().await.unwrap();
        manager.resume_interrupted().await.unwrap();
        let paused = manager.inner.snapshot(&id).unwrap().job;
        assert_eq!(paused.status, "paused");
        assert!(!paused.interrupted);
        assert!(!destination.exists());
        assert_eq!(fs::read(&part).unwrap(), bytes);
        assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 0);
        drop(manager);
        let manager = DownloadManager::new(
            data_dir, store.clone(), crate::update_admission::AdmissionGate::default(),
        ).unwrap();
        assert_eq!(manager.inner.snapshot(&id).unwrap().job.status, "paused");
        manager.resume(&id, None).await.unwrap();
        let complete = finished_job(&manager, &id).await;
        assert_eq!(complete.status, "complete", "{:?}", complete.error);
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert_eq!(
            manager.inner.hash_passes.load(Ordering::SeqCst),
            if with_receipt { 0 } else { 1 },
        );
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn a_matching_verification_receipt_skips_the_hash_pass_on_load() {
    let root = test_directory();
    let store = test_store(&root);
    let data_dir = root.join("state");
    let bytes = zim_bytes(b'r', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let mut stored = durable_stored(&root, &id, &bytes, "paused");
    stored.job.interrupted = true;
    let digest = hex::encode(Sha256::digest(&bytes));
    let (_, part) = paths(&stored.job).unwrap();
    stored.verification =
        verification_receipt(part_receipt_identity(&part), &digest, VerificationOutcome::Passed, None);
    assert!(stored.verification.is_some());
    stored.job.sha256 = Some(digest);
    write_state(&data_dir, stored);
    let manager = DownloadManager::new(data_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    assert_eq!(fs::read(&loaded.job.destination).unwrap(), bytes);
    assert_eq!(
        manager.inner.hash_passes.load(Ordering::SeqCst),
        0,
        "a receipt for the same inode must skip rehashing"
    );
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_changed_part_identity_forces_a_fresh_hash_pass() {
    let root = test_directory();
    let store = test_store(&root);
    let data_dir = root.join("state");
    let bytes = zim_bytes(b'i', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let mut stored = durable_stored(&root, &id, &bytes, "paused");
    stored.job.interrupted = true;
    let digest = hex::encode(Sha256::digest(&bytes));
    let (_, part) = paths(&stored.job).unwrap();
    let mut receipt =
        verification_receipt(part_receipt_identity(&part), &digest, VerificationOutcome::Passed, None).unwrap();
    receipt.identity.inode = receipt.identity.inode.wrapping_add(1);
    stored.verification = Some(receipt);
    stored.job.sha256 = Some(digest);
    write_state(&data_dir, stored);
    let manager = DownloadManager::new(data_dir, store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    assert_eq!(
        manager.inner.hash_passes.load(Ordering::SeqCst),
        1,
        "a changed identity must never reuse the receipt"
    );
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_matching_receipt_skips_hashing_on_resume() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'q', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let mut stored = durable_stored(&root, &id, &bytes, "paused");
    let digest = hex::encode(Sha256::digest(&bytes));
    let (_, part) = paths(&stored.job).unwrap();
    stored.verification =
        verification_receipt(part_receipt_identity(&part), &digest, VerificationOutcome::Passed, None);
    stored.job.sha256 = Some(digest);
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    manager.resume(&id, None).await.unwrap();
    let job = finished_job(&manager, &id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), bytes);
    assert_eq!(
        manager.inner.hash_passes.load(Ordering::SeqCst),
        0,
        "resume must reuse the receipt for the same inode"
    );
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_live_sweep_finishes_a_stuck_downloading_row() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'w', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let stored = durable_stored(&root, &id, &bytes, "downloading");
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    manager.recover_durable_local().await.unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    assert_eq!(fs::read(&loaded.job.destination).unwrap(), bytes);
    assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 1);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn the_recovery_sweep_leaves_durable_rows_alone_while_finalizing_is_forbidden() {
    // Entering private/read-only mode interrupts a hashing download; the
    // manager and its 5 s sweeper stay alive. The sweep must not claim the row
    // just to have the finalizer bounce it back, rewriting the journal forever.
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'p', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let mut stored = durable_stored(&root, &id, &bytes, "paused");
    stored.job.interrupted = true;
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    let journal = |root: &Path| -> Vec<(PathBuf, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(root.join("state")).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| { let bytes = fs::read(&path).unwrap(); (path, bytes) })
            .collect();
        files.sort();
        files
    };
    for private in [true, false] {
        store.update(|state| {
            state.settings.private_mode = private;
            state.settings.read_only = !private;
            Ok(())
        }).unwrap();
        let before = journal(&root);
        for _ in 0..3 {
            manager.recover_durable_local().await.unwrap();
        }
        let row = manager.inner.snapshot(&id).unwrap().job;
        assert_eq!((row.status.as_str(), row.interrupted), ("paused", true), "private={private}");
        assert_eq!(journal(&root), before, "private={private}: the sweep rewrote the journal");
        assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 0);
        assert!(!manager.inner.state().unwrap().running.contains_key(&id));
    }
    // Leaving the restricted mode lets the same sweep finish the archive.
    store.update(|state| {
        state.settings.private_mode = false;
        state.settings.read_only = false;
        Ok(())
    }).unwrap();
    manager.recover_durable_local().await.unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_sweep_finishes_a_row_whose_worker_vanished() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'z', 256 * 1024);
    let id = Uuid::new_v4().to_string();
    let stored = durable_stored(&root, &id, &bytes, "downloading");
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        let (command, _) = watch::channel(Signal::Run);
        let (finished, receiver) = watch::channel(false);
        // The task dropped without unwinding: the finished sender is gone,
        // but its running entry still exists.
        drop(finished);
        state.running.insert(
            id.clone(),
            Running {
                command,
                finished: receiver,
            },
        );
        manager.inner.save(&state).unwrap();
    }
    manager.recover_durable_local().await.unwrap();
    let loaded = manager.inner.snapshot(&id).unwrap();
    assert_eq!(loaded.job.status, "complete", "{:?}", loaded.job.error);
    assert_eq!(fs::read(&loaded.job.destination).unwrap(), bytes);
    assert!(manager.inner.state().unwrap().running.is_empty());
    assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 1);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sweep_stop_joins_file_ownership_even_after_the_sweep_future_is_dropped() {
    for action in [Signal::Pause, Signal::Cancel] {
        let root = test_directory();
        let store = test_store(&root);
        let manager = DownloadManager::new(
            root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default(),
        ).unwrap();
        let bytes = zim_bytes(b's', 256 * 1024);
        let id = Uuid::new_v4().to_string();
        let stored = durable_stored(&root, &id, &bytes, "downloading");
        let (destination, part) = paths(&stored.job).unwrap();
        {
            let mut state = manager.inner.state().unwrap();
            state.jobs.insert(id.clone(), stored);
            manager.inner.save(&state).unwrap();
        }
        let (entered, entering) = tokio::sync::oneshot::channel();
        let (release, releasing) = std::sync::mpsc::sync_channel(1);
        *manager.inner.finalize_gate.lock().unwrap() = Some(FinalizeGate {
            entered,
            release: releasing,
        });
        let recovery = manager.clone();
        let sweep = tokio::spawn(async move { recovery.recover_durable_local().await });
        tokio::time::timeout(Duration::from_secs(10), entering).await.unwrap().unwrap();
        // The worker owns an open locked part, but its initiating sweep
        // need not remain alive to settle its result or acknowledge stop.
        sweep.abort();
        assert!(sweep.await.unwrap_err().is_cancelled());
        {
            let stopping = manager.stop(&id, action);
            tokio::pin!(stopping);
            assert!(futures_util::poll!(&mut stopping).is_pending());
            assert!(manager.inner.state().unwrap().running.contains_key(&id));
            assert!(manager.has_running_work().unwrap());
            assert!(manager.resume(&id, None).await.is_err());
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(10), &mut stopping).await.unwrap().unwrap();
        }
        assert!(!manager.inner.state().unwrap().running.contains_key(&id));
        assert!(!manager.has_running_work().unwrap());
        let stopped = manager.inner.snapshot(&id).unwrap().job;
        assert!(!stopped.interrupted);
        assert!(stopped.error.is_none(), "{:?}", stopped.error);
        assert!(!destination.exists());
        if action == Signal::Pause {
            assert_eq!(stopped.status, "paused");
            // This lock succeeds only after the old finalizer released
            // its retained part, not merely after its status changed.
            drop(open_part(&part).unwrap());
            manager.resume(&id, None).await.unwrap();
            let complete = finished_job(&manager, &id).await;
            assert_eq!(complete.status, "complete", "{:?}", complete.error);
            assert_eq!(fs::read(&destination).unwrap(), bytes);
            assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 1);
        } else {
            assert_eq!(stopped.status, "cancelled");
            assert!(!part.exists());
            assert_eq!(stopped.downloaded, 0);
        }
        drop(manager);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn a_foreground_cycle_during_verification_keeps_one_hash_pass() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let bytes = zim_bytes(b'f', 2 * 1024 * 1024);
    let id = Uuid::new_v4().to_string();
    let stored = durable_stored(&root, &id, &bytes, "paused");
    {
        let mut state = manager.inner.state().unwrap();
        state.jobs.insert(id.clone(), stored);
        manager.inner.save(&state).unwrap();
    }
    // Slow the pass so the background transition lands inside it.
    manager.inner.hash_delay.store(20, Ordering::SeqCst);
    manager.resume(&id, None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let job = manager.inner.snapshot(&id).unwrap().job;
        if job.status == "verifying" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker never entered verification: {:?}",
            job.error
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    // Background suspension: this must not abort the local pass.
    manager.shutdown().await.unwrap();
    let job = manager.inner.snapshot(&id).unwrap();
    assert_eq!(job.job.status, "complete", "{:?}", job.job.error);
    assert_eq!(fs::read(&job.job.destination).unwrap(), bytes);
    assert_eq!(
        manager.inner.hash_passes.load(Ordering::SeqCst),
        1,
        "a foreground/background cycle must not restart hashing"
    );
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_background_transition_mid_finalize_finishes_a_real_segmented_download() {
    let body = zim_bytes(b'b', 4 * 1024 * 1024 + 4096);
    let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    manager.inner.hash_delay.store(30, Ordering::SeqCst);
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(body.len() as u64),
            None,
        )
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = manager.inner.snapshot(&job.id).unwrap();
        if snapshot.job.status == "verifying" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the segmented transfer never reached verification: {:?}",
            snapshot.job.error
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    // This is the mobile background transition: an interrupt arrives while
    // the local hash/publish pass owns the durable bytes.
    manager.shutdown().await.unwrap();
    let finished = manager.inner.snapshot(&job.id).unwrap();
    assert_eq!(finished.job.status, "complete", "{:?}", finished.job.error);
    assert_eq!(fs::read(&finished.job.destination).unwrap(), body);
    assert_eq!(
        manager.inner.hash_passes.load(Ordering::SeqCst),
        1,
        "backgrounding mid-finalize must not restart the hash"
    );
    // Foreground reconciliation must not start a second pass either.
    manager.resume_interrupted().await.unwrap();
    assert_eq!(manager.inner.hash_passes.load(Ordering::SeqCst), 1);
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn full_response_restart_does_not_delay_the_incremental_space_check() {
    let discarded = 96 * 1024 * 1024;
    // A checkpoint left behind by the discarded partial would hide the
    // first 8 MiB of the replacement body.
    assert!(!space_check_due(SPACE_CHECK_INTERVAL - 1, discarded));
    assert!(!space_check_due(SPACE_CHECK_INTERVAL, discarded));
    // The truncation branch resets the checkpoint with the byte counter.
    let restarted = 0;
    assert!(!space_check_due(SPACE_CHECK_INTERVAL - 1, restarted));
    assert!(space_check_due(SPACE_CHECK_INTERVAL, restarted));
}

#[test]
fn preallocated_bytes_are_not_demanded_twice_by_incremental_checks() {
    let total = 100;
    // The admission check before `set_len` already demanded the whole
    // object; on NTFS the reservation charges it to the transfer.
    assert_eq!(space_remaining(total, 0, total), 0);
    assert_eq!(space_remaining(total, 40, total), 0);
    // A filesystem that leaves the tail sparse still reserved the length,
    // so growth past the written prefix is already paid for.
    assert_eq!(space_remaining(total, 40, 40), total - 40);
    // An unstarted part must still find the whole object.
    assert_eq!(space_remaining(total, 0, 0), total);
    // A part allocated beyond the durable count still covers that growth.
    assert_eq!(space_remaining(total, 30, 70), 30);
    // A part shorter than its durable count needs the remaining tail.
    assert_eq!(space_remaining(total, 70, 0), 30);
    // Never underflow on a stale or oversized row.
    assert_eq!(space_remaining(total, total + 5, total + 5), 0);
}

#[test]
fn outstanding_space_sums_only_active_same_volume_jobs() {
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(
        root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default(),
    ).unwrap();
    let make = |name: &str, status: &str, total: Option<u64>, downloaded: u64, part_len: usize| {
        let id = Uuid::new_v4().to_string();
        let mut stored = durable_stored(&root, &id, &[b'x'; 100], status);
        stored.job.file_name = name.into();
        stored.job.destination = root.join(name).to_str().unwrap().into();
        stored.job.total = total;
        stored.job.downloaded = downloaded;
        stored.expected_size = total;
        let (_, part) = paths(&stored.job).unwrap();
        fs::write(&part, vec![0u8; part_len]).unwrap();
        (id, stored)
    };
    let (first_id, first) = make("first.zim", "downloading", Some(1_000), 400, 300);
    let (second_id, second) = make("second.zim", "queued", Some(1_000), 100, 200);
    let (paused_id, paused) = make("paused.zim", "paused", Some(1_000), 0, 0);
    let (unknown_id, unknown) = make("unknown.zim", "downloading", None, 0, 0);
    let mut state = manager.inner.state().unwrap();
    state.jobs.insert(first_id.clone(), first);
    state.jobs.insert(second_id.clone(), second);
    state.jobs.insert(paused_id, paused);
    state.jobs.insert(unknown_id, unknown);
    assert_eq!(outstanding_space_from_state(&state, &root, &first_id).unwrap(), 800);
    assert_eq!(outstanding_space_from_state(&state, &root, &second_id).unwrap(), 600);
    assert_eq!(outstanding_space_from_state(&state, &root, "new-job").unwrap(), 1_400);
    drop(state);
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unstarted_ranges_do_not_require_the_preallocation_to_exist() {
    let total = 4 * 1024 * 1024;
    let plan = SegmentPlan::build(total, "\"v1\"".into(), 0).unwrap();
    // A crash between the journal write and `set_len` leaves an empty part
    // while every range still has `done == 0`.
    assert!(plan_fits(&plan, 0));
    assert!(plan_fits(&plan, 512 * 1024));
    assert!(plan_fits(&plan, total));
    // The object itself may never be shorter than the plan.
    assert!(!plan_fits(&plan, total + 1));
    // A started range must still have all of its synced bytes present.
    let mut started = plan.clone();
    started.ranges[0].done = 1024;
    assert!(plan_fits(&started, 1024));
    assert!(!plan_fits(&started, 1023));
}

struct DenialProbe {
    polls: Arc<AtomicUsize>,
}

impl Future for DenialProbe {
    type Output = ();
    fn poll(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<()> {
        let poll = self.polls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            poll, 0,
            "the completed denial future was polled again, as a mobile async fn would panic"
        );
        Poll::Ready(())
    }
}

#[tokio::test]
async fn a_fired_denial_is_never_polled_again_while_segments_drain() {
    let body = zim_bytes(b'd', 4 * 1024 * 1024);
    let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
    server.hold_mid_range();
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(body.len() as u64),
            None,
        )
        .await
        .unwrap();
    wait_for_segment_progress(&manager, &job.id).await;
    manager.pause(&job.id).await.unwrap();
    let paused = manager.inner.snapshot(&job.id).unwrap();
    let plan = paused.segments.clone().expect("plan disappeared on pause");
    assert!(!plan.complete(), "pause happened after the whole transfer");
    let (_, part) = paths(&paused.job).unwrap();
    let identity = fs::metadata(&part).unwrap();
    let handle_identity = part_receipt_identity(&part);
    let transfer = Url::parse(paused.transfer_url.as_deref().unwrap()).unwrap();
    let (_tx, mut signal) = watch::channel(Signal::Run);
    let polls = Arc::new(AtomicUsize::new(0));
    let result = run_segments(
        Arc::new(SegmentTransfer::new(
            Arc::clone(&manager.inner), job.id.clone(), transfer, part, identity, handle_identity, plan,
        )),
        &mut signal,
        DenialProbe {
            polls: Arc::clone(&polls),
        },
    )
    .await;
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "the denial future must be polled exactly once"
    );
    assert!(
        matches!(result, Ok(SegmentRun::Stopped(Outcome::Interrupted))),
        "a fired denial must stop the transfer instead of wedging it: {result:?}"
    );
    server.release();
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_crash_before_preallocation_resumes_its_journaled_ranges() {
    let body = zim_bytes(b'p', 4 * 1024 * 1024);
    let server = test_range_server(body.clone(), "\"v1\"", Duration::ZERO).await;
    server.hold_mid_range();
    let root = test_directory();
    let store = test_store(&root);
    let manager = DownloadManager::new(root.join("state"), store.clone(), crate::update_admission::AdmissionGate::default()).unwrap();
    let job = manager
        .start(
            server.archive_url.clone(),
            root.clone(),
            Some("fixture.zim".into()),
            Some(body.len() as u64),
            None,
        )
        .await
        .unwrap();
    wait_for_segment_progress(&manager, &job.id).await;
    manager.pause(&job.id).await.unwrap();
    let paused = manager.inner.snapshot(&job.id).unwrap();
    assert!(paused.segments.is_some());
    let (_, part) = paths(&paused.job).unwrap();
    // Model a crash between the journal write and `set_len`: the layout is
    // durable, but the part holds no bytes and was never preallocated.
    fs::write(&part, b"").unwrap();
    manager
        .inner
        .update(&job.id, |stored| {
            for range in &mut stored.segments.as_mut().unwrap().ranges {
                range.done = 0;
            }
            stored.job.downloaded = 0;
        })
        .unwrap();
    let before = server.requests().len();
    server.release();
    manager.resume(&job.id, None).await.unwrap();
    let job = finished_job(&manager, &job.id).await;
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert_eq!(fs::read(&job.destination).unwrap(), body);
    let requests = server.requests();
    assert!(
        requests[before..]
            .iter()
            .all(|request| test_range_bounds(request).is_some()),
        "the journaled layout was discarded and probed with a full response: {:?}",
        &requests[before..]
    );
    server.task.abort();
    drop(manager);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
