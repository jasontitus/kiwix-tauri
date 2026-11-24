// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
use std::{fs, io::Write, time::Duration};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("kiwix-localswarm-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn manifest(names: &[&str]) -> Manifest {
    let mut files = Vec::new(); let mut hashes = Vec::new();
    for name in names {
        let start = hashes.len() as u32;
        hashes.extend([hex::encode(Sha256::digest(b"abcd")), hex::encode(Sha256::digest(b"efgh"))]);
        files.push(SwarmFile { path: (*name).into(), size_bytes: 8, start_chunk_index: start, end_chunk_index: start + 1 });
    }
    let mut manifest = Manifest { zim: None, protocol_version: 1, swarm_id: String::new(), name: "Boundary fixture".into(), total_bytes: files.len() as u64 * 8, chunk_size_bytes: 4, chunk_hash_algo: "sha256".into(), files, chunk_hashes: hashes };
    manifest.swarm_id = manifest.content_address(); manifest.validate().unwrap(); manifest
}
#[test]
fn accepts_only_one_exact_archive_set_before_output_creation() {
    assert!(supported(&manifest(&["book.zim"])).is_ok());
    assert!(supported(&manifest(&["book.zimaa", "book.zimab"])).is_ok());
    for names in [vec!["book.zim", "other.zim"], vec!["book.zimaa", "book.zimac"], vec!["sub/book.zim"], vec!["book.zimab"], vec!["notes.txt"]] {
        assert!(supported(&manifest(&names)).is_err());
    }
}
#[test]
fn resume_rehash_repairs_corrupt_claims_without_discarding_valid_chunks() {
    let root = Temp::new(); let manifest = manifest(&["book.zim"]);
    let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    open.write_chunk(&manifest, 0, b"abcd").unwrap(); open.write_chunk(&manifest, 1, b"efgh").unwrap(); drop(open);
    let path = root.0.join(&owned.staging).join("book.zim");
    let mut changed = fs::OpenOptions::new().write(true).open(&path).unwrap(); changed.write_all(b"xxxx").unwrap(); changed.sync_all().unwrap(); drop(changed);
    let mut open = owned.open(&manifest, false).unwrap(); let mut bits = vec![true, true];
    rehash_verified(&mut open, &manifest, &owned, &mut bits, &CancellationToken::new()).unwrap();
    assert_eq!(bits, vec![false, true]); assert_eq!(verified_bytes(&manifest, &bits).unwrap(), 4);
    open.write_chunk(&manifest, 0, b"abcd").unwrap(); bits[0] = true;
    rehash_verified(&mut open, &manifest, &owned, &mut bits, &CancellationToken::new()).unwrap();
    assert_eq!(bits, vec![true, true]); assert_eq!(open.read_chunk(&manifest, 1).unwrap(), b"efgh");
}
#[test]
fn in_place_changes_after_admission_do_not_become_verified_writes() {
    let root = Temp::new(); let manifest = manifest(&["book.zim"]);
    let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    open.write_chunk(&manifest, 0, b"abcd").unwrap();
    open.files[0].set_len(7).unwrap();
    assert!(open.unchanged(&manifest, &owned).is_err());
}
#[test]
fn restart_rejects_replaced_parent_or_staged_file_without_touching_it() {
    for replace_parent in [false, true] {
        let root = Temp::new(); let parent = root.0.join("downloads"); fs::create_dir(&parent).unwrap();
        let manifest = manifest(&["book.zim"]);
        let (owned, open) = OwnedSet::create(Directory::open(&parent).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap(); drop(open);
        if replace_parent {
            fs::rename(&parent, root.0.join("retained-downloads")).unwrap(); fs::create_dir(&parent).unwrap();
            fs::write(parent.join("sentinel"), b"foreign parent").unwrap();
        } else {
            let file = parent.join(&owned.staging).join("book.zim"); fs::rename(&file, parent.join("retained-part")).unwrap(); fs::write(&file, b"sentinel").unwrap();
        }
        assert!(owned.open(&manifest, true).is_err());
        if replace_parent { assert_eq!(fs::read(parent.join("sentinel")).unwrap(), b"foreign parent"); }
        else { assert_eq!(fs::read(parent.join(&owned.staging).join("book.zim")).unwrap(), b"sentinel"); }
    }
}
#[test]
fn recovery_requires_publication_intent_and_retained_directory_identity() {
    let root = Temp::new(); let manifest = manifest(&["book.zimaa", "book.zimab"]);
    let (owned, open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap(); drop(open);
    fs::rename(root.0.join(&owned.staging), root.0.join(&owned.destination)).unwrap();
    assert!(owned.open(&manifest, false).is_err());
    assert!(owned.open(&manifest, true).unwrap().published);
    fs::rename(root.0.join(&owned.destination), root.0.join("retained-set")).unwrap();
    fs::create_dir(root.0.join(&owned.destination)).unwrap();
    assert!(owned.open(&manifest, true).is_err());
}
#[test]
fn observing_unopened_profile_never_opts_in() {
    let root = Temp::new(); let store = Store::new(root.0.clone()).unwrap();
    assert!(LocalSwarmManager::saved_jobs(&store).unwrap().is_empty());
    assert!(!root.0.join("localswarm").exists());
}
#[test]
fn source_handoff_rejects_replaced_primary_or_parent_after_stock_verification() {
    for replace_parent in [false, true] {
        let root = Temp::new(); let profile = root.0.join("profile"); let parent = root.0.join("books"); fs::create_dir(&parent).unwrap();
        let archive = parent.join("selected.zim");
        fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &archive).unwrap();
        let book = crate::imports::load_book(archive.to_str().unwrap(), false, false).unwrap();
        let store = Arc::new(Store::new(profile).unwrap()); store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
        let pinned = crate::library_files::pin_archive_files(&book).unwrap();
        if replace_parent {
            // Windows: the pin's own directory capability (which never shares
            // delete) makes the parent impossible to displace at all, a
            // stronger guarantee than the POSIX rejection asserted below.
            if cfg!(windows) {
                assert_eq!(fs::rename(&parent, root.0.join("retained-books")).unwrap_err().raw_os_error(), Some(32));
                assert!(fs::read(&archive).unwrap().starts_with(b"ZIM"));
                continue;
            }
            fs::rename(&parent, root.0.join("retained-books")).unwrap(); fs::create_dir(&parent).unwrap();
            fs::write(&archive, b"foreign file").unwrap();
        } else {
            fs::rename(&archive, parent.join("retained.zim")).unwrap(); fs::write(&archive, b"foreign file").unwrap();
        }
        assert!(SourceSet::from_pinned(store, book, CancellationToken::new(), pinned).is_err());
        assert_eq!(fs::read(&archive).unwrap(), b"foreign file");
    }
}
#[tokio::test]
async fn split_header_offer_and_receiver_preserve_every_part() {
    let root = Temp::new();
    let directory = root.0.join("books");
    fs::create_dir(&directory).unwrap();
    let bytes = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    for (name, part) in [
        ("selected.zimaa", &bytes[..7]),
        ("selected.zimab", &bytes[7..80]),
        ("selected.zimac", &bytes[80..]),
    ] {
        fs::write(directory.join(name), part).unwrap();
    }
    let primary = directory.join("selected.zimaa");
    let book = crate::imports::load_book(primary.to_str().unwrap(), false, false).unwrap();
    let store = Arc::new(Store::new(root.0.join("profile")).unwrap());
    store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    let (manifest, source) = SourceSet::prepare(store, book, CancellationToken::new()).await.unwrap();
    let mut transferred = Vec::new();
    for index in 0..manifest.chunk_hashes.len() {
        transferred.extend(source.chunk(index as u32).unwrap());
    }
    assert_eq!(transferred, bytes);
    let files = manifest.files.iter().map(|entry|
        (entry.path.clone(), File::open(directory.join(&entry.path)).unwrap())).collect::<Vec<_>>();
    let cancel = CancellationToken::new();
    validate_zim(&directory, &files, &manifest, &cancel).unwrap();
    let last = directory.join("selected.zimac");
    if cfg!(windows) {
        // The pinned offer denies write sharing, so no in-place writer can
        // even open a member on Windows; retire the offer to model a later edit.
        assert_eq!(fs::OpenOptions::new().write(true).open(&last).unwrap_err().raw_os_error(), Some(32));
        drop(source);
    } else {
        fs::OpenOptions::new().write(true).open(&last).unwrap()
            .set_len((bytes.len() - 81) as u64).unwrap();
        assert!(source.chunk((manifest.chunk_hashes.len() - 1) as u32).is_err());
    }
    fs::OpenOptions::new().write(true).open(&last).unwrap()
        .set_len((bytes.len() - 81) as u64).unwrap();
    assert!(validate_zim(&directory, &files, &manifest, &cancel).is_err());
}
#[tokio::test]
async fn retired_native_checksum_does_not_block_stop_or_publish_late_result() {
    let cancel = CancellationToken::new(); let token = cancel.clone();
    let (entered, started) = tokio::sync::oneshot::channel(); let (release, gated) = std::sync::mpsc::channel();
    let (exited, done) = tokio::sync::oneshot::channel();
    let operation = tokio::spawn(async move {
        readonly_verification(&token, move || { let _ = entered.send(()); gated.recv().map_err(err)?; let _ = exited.send(()); Ok(42u32) }).await
    });
    started.await.unwrap(); cancel.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(1), operation).await.unwrap().unwrap().is_err());
    release.send(()).unwrap(); tokio::time::timeout(Duration::from_secs(1), done).await.unwrap().unwrap();
}

#[tokio::test]
async fn protected_transfer_restart_retains_data_but_never_credentials_or_automatic_activity() {
    let root = Temp::new();
    let source = root.0.join("source"); fs::create_dir(&source).unwrap();
    let path = source.join("selected.zim");
    fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &path).unwrap();
    let book = crate::imports::load_book(path.to_str().unwrap(), false, false).unwrap();
    let source_store = Arc::new(Store::new(root.0.join("source-profile")).unwrap());
    source_store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    let (manifest, pinned) = SourceSet::prepare(source_store, book, CancellationToken::new()).await.unwrap();
    let sid = manifest.swarm_id.clone();
    let reader: network::ChunkReader = Arc::new(move |index| pinned.chunk(index));
    let secret = "638274-private-test-pin";
    let server = network::seed(manifest, Uuid::new_v4().to_string(), Some(secret.into()), reader, Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let profile = root.0.join("receiver-profile");
    let store = Arc::new(Store::new(profile.clone()).unwrap());
    store.update(|state| { state.settings.download_limit = 1; Ok(()) }).unwrap();
    let manager = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
    let job = manager.start(ConnectionTarget::manual("127.0.0.1".into(), server.port), sid.clone(), Some(secret.into()), root.0.join("received"), None).await.unwrap();
    manager.pause(&job.id, false).await.unwrap();
    drop(manager);
    let bytes = fs::read(profile.join("localswarm/jobs.json")).unwrap();
    let persisted = String::from_utf8(bytes).unwrap();
    assert!(!persisted.contains(secret));
    assert!(!persisted.contains(&hex::encode(Sha256::digest(format!("{sid}:{secret}").as_bytes()))));
    let restored = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let jobs = restored.list().unwrap();
    assert_eq!(jobs[0].status, "paused");
    assert!(jobs[0].requires_pin);
    assert!(restored.inner.state.lock().unwrap().running.is_empty());
    assert!(restored.resume(&job.id, None, None).await.is_err());
    assert!(!restored.status().await.unwrap()["discovery"].as_bool().unwrap());
    assert!(restored.status().await.unwrap()["seeds"].as_array().unwrap().is_empty());
    restored.stop().await.unwrap(); server.shutdown().await;
}

#[test]
fn retained_store_lock_rejects_a_replacement_profile_instead_of_leaking_state() {
    let root = Temp::new(); let profile = root.0.join("profile");
    let store = Store::new(profile.clone()).unwrap();
    let (_, retained) = store.acquisition_profile_directory().unwrap();
    if cfg!(windows) {
        // The retained profile capability never shares delete on Windows, so
        // the profile cannot be displaced while the store holds it at all.
        assert_eq!(fs::rename(&profile, root.0.join("retained-profile")).unwrap_err().raw_os_error(), Some(32));
        assert!(LocalSwarmManager::saved_jobs(&store).unwrap().is_empty());
        assert!(!profile.join("localswarm").exists());
        return;
    }
    fs::rename(&profile, root.0.join("retained-profile")).unwrap();
    fs::create_dir(&profile).unwrap(); fs::write(profile.join("store.lock"), b"foreign lock").unwrap();
    assert!(LocalSwarmManager::saved_jobs(&store).is_err());
    assert!(!profile.join("localswarm").exists());
    assert_eq!(fs::read(profile.join("store.lock")).unwrap(), b"foreign lock");
    drop(retained);
}

#[test]
fn strict_read_only_history_observation_does_not_create_or_repair_state() {
    let root = Temp::new(); let profile = root.0.join("profile");
    let store = Store::new(profile.clone()).unwrap(); drop(store);
    let before = fs::read(profile.join("state.json")).unwrap();
    let store = Store::new_with_mode(profile.clone(), true).unwrap();
    assert!(LocalSwarmManager::saved_jobs(&store).unwrap().is_empty());
    assert!(!profile.join("localswarm").exists());
    assert_eq!(fs::read(profile.join("state.json")).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn resume_never_writes_through_hardlinks_or_symlink_substitutions() {
    for symlink in [false, true] {
        let root = Temp::new(); let manifest = manifest(&["book.zim"]);
        let (owned, open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap(); drop(open);
        let part = root.0.join(&owned.staging).join("book.zim");
        let foreign = root.0.join("foreign");
        if symlink {
            fs::rename(&part, root.0.join("retained-part")).unwrap();
            fs::write(&foreign, b"sentinel").unwrap();
            std::os::unix::fs::symlink(&foreign, &part).unwrap();
        } else { fs::hard_link(&part, &foreign).unwrap(); }
        let before = fs::read(&foreign).unwrap();
        assert!(owned.open(&manifest, false).is_err());
        assert_eq!(fs::read(&foreign).unwrap(), before);
    }
}

fn content_manifest(parts: &[(&str, &[u8])]) -> Manifest {
    let mut files = Vec::new(); let mut hashes = Vec::new();
    for (name, bytes) in parts {
        let start = hashes.len() as u32;
        hashes.extend(bytes.chunks(1024).map(|chunk| hex::encode(Sha256::digest(chunk))));
        files.push(SwarmFile { path: (*name).into(), size_bytes: bytes.len() as u64, start_chunk_index: start, end_chunk_index: hashes.len() as u32 - 1 });
    }
    let mut manifest = Manifest { zim: None, protocol_version: 1, swarm_id: String::new(), name: "Identity fixture".into(), total_bytes: parts.iter().map(|(_, bytes)| bytes.len() as u64).sum(), chunk_size_bytes: 1024, chunk_hash_algo: "sha256".into(), files, chunk_hashes: hashes };
    manifest.swarm_id = manifest.content_address(); manifest.validate().unwrap(); manifest
}

#[test]
fn native_verification_cannot_validate_a_good_path_replacement_for_bad_retained_bytes() {
    let root = Temp::new();
    let good = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    let mut bad = good.clone(); *bad.last_mut().unwrap() ^= 0xff;
    let manifest = content_manifest(&[("book.zim", &bad)]);
    let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    for (index, chunk) in bad.chunks(1024).enumerate() { open.write_chunk(&manifest, index as u32, chunk).unwrap(); }
    open.seal(&manifest, &owned).unwrap();
    let files = open.verification_files(&manifest, &owned).unwrap();
    let original_name = open.directory.path.clone();
    if cfg!(windows) {
        // Windows refuses to rename a directory while the set or any member
        // is open, as production holds them throughout native verification,
        // so substitute the member itself (its handles share delete).
        assert!(fs::rename(&original_name, root.0.join("retained-bad-set")).is_err());
        fs::rename(original_name.join("book.zim"), original_name.join("retained-bad.zim")).unwrap();
    } else {
        fs::rename(&original_name, root.0.join("retained-bad-set")).unwrap();
        fs::create_dir(&original_name).unwrap();
    }
    fs::write(original_name.join("book.zim"), &good).unwrap();
    // The replacement is a genuinely good ZIM of the exact same size. POSIX
    // verifies the bad retained descriptors; Windows rejects its foreign IDs.
    assert!(validate_zim(&original_name, &files, &manifest, &CancellationToken::new()).is_err());
    assert_eq!(fs::read(original_name.join("book.zim")).unwrap(), good);
}

#[test]
fn stock_native_descriptor_verification_covers_the_complete_split_set() {
    let bytes = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    // Also split the stored 16-byte checksum itself across the retained files.
    for split in [bytes.len() / 2, bytes.len() - 7] {
        let root = Temp::new();
        let manifest = content_manifest(&[("book.zimaa", &bytes[..split]), ("book.zimab", &bytes[split..])]);
        let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
        for entry in &manifest.files {
            let part = if entry.path.ends_with("aa") { &bytes[..split] } else { &bytes[split..] };
            for (index, chunk) in part.chunks(1024).enumerate() { open.write_chunk(&manifest, entry.start_chunk_index + index as u32, chunk).unwrap(); }
        }
        open.seal(&manifest, &owned).unwrap();
        let first = open.read_chunk(&manifest, 0).unwrap();
        assert!(open.write_chunk(&manifest, 0, &first).is_err());
        let files = open.verification_files(&manifest, &owned).unwrap();
        let cancel = CancellationToken::new();
        validate_zim(&open.directory.path, &files, &manifest, &cancel).unwrap();
        cancel.cancel();
        assert!(validate_zim(&open.directory.path, &files, &manifest, &cancel).unwrap_err().contains("stopped"));
    }
}

#[test]
fn retired_native_checksum_worker_allows_process_exit() {
    const CHILD: &str = "KIWIX_TEST_RETIRED_LOCALSWARM_CHECKSUM";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "localswarm::tests::retired_native_checksum_worker_allows_process_exit", "--test-threads=1"])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() { assert!(status.success()); break; }
            if Instant::now() >= deadline {
                let _ = child.kill(); let _ = child.wait();
                panic!("Headless process waited for an uncancellable retired checksum");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    runtime.block_on(async {
        let cancel = CancellationToken::new(); let token = cancel.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let operation = tokio::spawn(async move {
            readonly_verification(&token, move || -> Result<(), String> {
                let _ = entered.send(());
                // Simulate a stock native checksum that cannot be interrupted.
                loop { std::thread::park(); }
            }).await
        });
        started.await.unwrap(); cancel.cancel();
        assert!(operation.await.unwrap().is_err());
    });
    // Actual Runtime destruction, not just Service::shutdown returning.
    // This child must exit while its read-only native worker remains blocked.
    drop(runtime);
}

#[test]
fn late_discovery_replenishes_sources_without_retrying_failed_or_cross_swarm_endpoints() {
    let sid = "a".repeat(64);
    let peer = |host: &str, swarm_id: String| Peer {
        id: host.into(), peer_id: Uuid::new_v4().to_string(), swarm_id,
        name: "Actual swarm selection boundary".into(), total_bytes: 8,
        publisher_name: None,
        chunk_count: 2, host: host.into(), port: 19481, locked: false,
        transport: "quic".into(), addresses: Vec::new(), library_session_id: None,
        library_generation: None,
        library_count: None,
    };
    let mut attempted = HashSet::new();
    let mut queue = VecDeque::new();
    sources::enqueue_discovered_sources(&sid, vec![peer("192.0.2.1", sid.clone())], &attempted, &HashSet::new(), &mut queue, false);
    let first = queue.pop_front().unwrap();
    attempted.insert((first.hosts()[0].clone(), first.port()));
    assert!(queue.is_empty());
    sources::enqueue_discovered_sources(&sid, vec![
        peer("192.0.2.1", sid.clone()),
        peer("192.0.2.2", "b".repeat(64)),
        peer("192.0.2.3", sid.clone()),
        peer("192.0.2.3", sid.clone()),
    ], &attempted, &HashSet::new(), &mut queue, false);
    let replacement = queue.pop_front().unwrap();
    assert_eq!(replacement.hosts(), &["192.0.2.3"]);
    assert_eq!(replacement.port(), 19481);
    assert!(queue.is_empty());
}

#[tokio::test]
async fn source_metadata_monitor_revokes_replacements_without_a_new_chunk_request() {
    let root = Temp::new(); let archive = root.0.join("selected.zim");
    fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &archive).unwrap();
    let book = crate::imports::load_book(archive.to_str().unwrap(), false, false).unwrap();
    let store = Arc::new(Store::new(root.0.join("profile")).unwrap());
    store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    let cancel = CancellationToken::new();
    let (_, source) = SourceSet::prepare(store, book, cancel.clone()).await.unwrap();
    let reason = Arc::new(Mutex::new(None));
    let monitor = SourceMonitor::start(&source, cancel.clone(), reason.clone()).unwrap();
    fs::rename(&archive, root.0.join("retained.zim")).unwrap();
    fs::write(&archive, b"replacement archive").unwrap();
    tokio::time::timeout(Duration::from_secs(2), cancel.cancelled()).await.unwrap();
    assert!(reason.lock().unwrap().is_some());
    drop(monitor);
    assert_eq!(fs::read(&archive).unwrap(), b"replacement archive");
}

#[tokio::test]
async fn source_revocation_stops_buffered_throttled_chunks_and_retires_the_entire_offer() {
    for kind in ["remove", "relink", "file-replacement", "principal-replacement"] {
        let root = Temp::new(); let archive = root.0.join("selected.zim");
        fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &archive).unwrap();
        let book = crate::imports::load_book(archive.to_str().unwrap(), false, false).unwrap();
        let old_principal = book.installation_id.clone();
        let service = crate::service::Service::new(root.0.join("profile"), false, false).unwrap();
        service.store.update(|state| {
            state.books.push(book.clone());
            state.settings.upload_limit = 16;
            Ok(())
        }).unwrap();
        let offer = service.clone().dispatch("localswarm_seed", json!({"bookId":book.metadata.id})).await.unwrap();
        let sid = offer["swarmId"].as_str().unwrap().to_string();
        let port = offer["port"].as_u64().unwrap() as u16;
        let client_cancel = CancellationToken::new();
        let mut client = network::connect("127.0.0.1", port, &sid, None,
            &Uuid::new_v4().to_string(), client_cancel.clone(), Arc::new(RateBudget::new(0))).await.unwrap();
        let mut transfer = tokio::spawn(async move {
            let result = client.get_chunk(0).await;
            client.shutdown().await;
            result
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = service.clone().dispatch("localswarm_status", json!({})).await.unwrap();
            if status["seeds"][0]["uploaded"].as_u64().unwrap_or(0) > 0 { break; }
            assert!(Instant::now() < deadline, "No real throttled payload was sent");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        match kind {
            "remove" => {
                service.clone().dispatch("remove_book", json!({"id":book.metadata.id,"installationId":book.installation_id,"deleteFile":false,"privacyGeneration":service.launch_generation()})).await.unwrap();
                assert!(service.store.book(&book.metadata.id).unwrap().is_none());
            },
            "relink" => {
                let replacement = root.0.join("relocated.zim");
                fs::copy(&archive, &replacement).unwrap();
                service.clone().dispatch("relink_book", json!({"id":book.metadata.id,"path":replacement})).await.unwrap();
                assert_ne!(service.store.book(&book.metadata.id).unwrap().unwrap().installation_id, old_principal);
            },
            "file-replacement" => {
                fs::rename(&archive, root.0.join("retained.zim")).unwrap();
                fs::write(&archive, b"replacement archive").unwrap();
            },
            _ => {
                service.store.update(|state| {
                    state.books[0].installation_id = Uuid::new_v4().to_string();
                    Ok(())
                }).unwrap();
            },
        }
        let result = tokio::time::timeout(Duration::from_secs(5), &mut transfer).await;
        if result.is_err() {
            client_cancel.cancel(); let _ = transfer.await; service.shutdown().await.unwrap();
            panic!("Buffered source traffic continued after {kind}");
        }
        assert!(result.unwrap().unwrap().is_err(), "Revoked source completed its throttled chunk");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = service.clone().dispatch("localswarm_status", json!({})).await.unwrap();
            if status["seeds"].as_array().unwrap().is_empty() { break; }
            assert!(Instant::now() < deadline, "Source listener/advertisement retirement did not finish");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        service.shutdown().await.unwrap();
    }
}

#[test]
fn saved_history_budget_stops_serialization_before_consuming_an_oversized_manifest() {
    use serde::ser::SerializeSeq;
    struct Entries<'a> { visited: &'a std::cell::Cell<usize>, value: &'a str }
    impl Serialize for Entries<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(10_000))?;
            for _ in 0..10_000 {
                self.visited.set(self.visited.get() + 1);
                sequence.serialize_element(self.value)?;
            }
            sequence.end()
        }
    }
    let visited = std::cell::Cell::new(0);
    let value = "x".repeat(128);
    let error = bounded_json(&Entries { visited: &visited, value: &value }, 1024).unwrap_err();
    assert!(error.contains("state limit"));
    assert!(visited.get() < 10, "The serializer consumed the oversized input before enforcing its byte budget");
}

#[tokio::test]
async fn failed_initial_history_commit_does_not_admit_a_job_or_replace_foreign_entries() {
    let root = Temp::new();
    let (server, manifest, _) = endpoint_fixture(&root).await;
    let profile = root.0.join("receiver-profile");
    let store = Arc::new(Store::new(profile.clone()).unwrap());
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    fs::create_dir(profile.join("localswarm/jobs.json")).unwrap();
    let sentinel = profile.join("localswarm/jobs.json/sentinel");
    fs::write(&sentinel, b"unrelated entry").unwrap();
    let error = manager.start(ConnectionTarget::manual("127.0.0.1".into(), server.port),
        manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap_err();
    let staged: Vec<_> = fs::read_dir(root.0.join("received")).unwrap()
        .map(|entry| entry.unwrap().path()).collect();
    assert_eq!(staged.len(), 1, "History failure must retain exactly its owned archive set: {error}");
    assert_eq!(fs::metadata(staged[0].join(&manifest.files[0].path)).unwrap().len(), manifest.total_bytes,
        "The initial allocation must survive without an admitted writer: {error}");
    assert!(manager.list().unwrap().is_empty());
    assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated entry");
    assert_eq!(manager.status().await.unwrap()["seeds"], json!([]));
    manager.stop().await.unwrap();
    server.shutdown().await;
}

#[test]
fn discovered_selection_binds_latest_record_and_keeps_all_safe_addresses() {
    let sid = "c".repeat(64);
    let peer = Peer {
        id: "chosen-record".into(), peer_id: Uuid::new_v4().to_string(), swarm_id: sid.clone(),
        name: "Multihomed source".into(), total_bytes: 8, chunk_count: 2,
        publisher_name: None,
        host: "192.0.2.10".into(), port: 19482, locked: false, transport: "quic".into(),
        addresses: vec!["192.0.2.10".into(), "198.51.100.20".into(), "2001:db8::20".into(),
            "127.0.0.1".into(), "fe80::1".into(), "not-an-address".into()],
        library_session_id: None,
        library_generation: None,
        library_count: None,
    };
    let selected = sources::selected_peer_target(std::slice::from_ref(&peer), &peer.id, &sid, peer.port, false).unwrap();
    assert_eq!(selected.hosts(), &["192.0.2.10", "198.51.100.20", "2001:db8::20"]);
    assert!(sources::selected_peer_target(std::slice::from_ref(&peer), "unknown", &sid, peer.port, false).is_err());
    assert!(sources::selected_peer_target(std::slice::from_ref(&peer), &peer.id, &"d".repeat(64), peer.port, false).is_err());
    assert!(sources::selected_peer_target(std::slice::from_ref(&peer), &peer.id, &sid, peer.port + 1, false).is_err());
    let candidates = sources::source_candidates("198.51.100.20".into(), peer.port, &sid, vec![peer.clone()], false);
    assert_eq!(candidates.len(), 1, "Addresses of one discovered peer are one source, not extra blocking sources");
    assert_eq!(candidates[0].hosts()[0], "198.51.100.20");
    assert_eq!(candidates[0].hosts().len(), 3);
    assert!(!candidates[0].direct(), "The default route preference must stay infrastructure-first");
    let direct = sources::source_candidates("198.51.100.20".into(), peer.port, &sid, vec![peer.clone()], true);
    assert!(direct[0].direct(), "Direct mode must reach every dialed source");
    assert!(sources::selected_peer_target(std::slice::from_ref(&peer), &peer.id, &sid, peer.port, true).unwrap().direct());
    assert!(!ConnectionTarget::manual("192.0.2.1".into(), peer.port).direct());
    let mut updated = peer;
    updated.host = "203.0.113.40".into(); updated.addresses = vec![updated.host.clone()];
    let selected = sources::selected_peer_target(&[updated.clone()], &updated.id, &sid, updated.port, false).unwrap();
    assert_eq!(selected.hosts(), &["203.0.113.40"]);
}

async fn endpoint_fixture(root: &Temp) -> (SeedServer, Arc<Manifest>, Vec<u8>) {
    let path = root.0.join("endpoint-fixture.zim");
    fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &path).unwrap();
    let bytes = fs::read(&path).unwrap();
    let book = crate::imports::load_book(path.to_str().unwrap(), false, false).unwrap();
    let store = Arc::new(Store::new(root.0.join("sender-profile")).unwrap());
    store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    let (manifest, source) = SourceSet::prepare(store, book, CancellationToken::new()).await.unwrap();
    let reader: network::ChunkReader = Arc::new(move |index| source.chunk(index));
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    (server, manifest, bytes)
}

#[tokio::test]
async fn multihomed_inspect_and_download_use_a_working_alternate_and_verify_real_bytes() {
    let root = Temp::new();
    let (server, manifest, bytes) = endpoint_fixture(&root).await;
    // TEST-NET is not a local interface. The second address is a real QUIC
    // seeder using the reference pinned identity and a stock-valid ZIM.
    let target = ConnectionTarget::test_discovered(vec!["192.0.2.1".into(), "127.0.0.1".into()], server.port);
    let started = Instant::now();
    let inspection = LocalSwarmManager::inspect(target.clone(), &manifest.swarm_id, None,
        CancellationToken::new(), 0, true).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5), "Unreachable primary serialized the working alternate");
    assert_eq!(inspection.connected_host, "127.0.0.1");
    assert!(inspection.supported);
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let job = manager.start(target, manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap();
    assert_eq!(manager.inner.state.lock().unwrap().jobs[&job.id].host, "127.0.0.1");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let current = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        if current.status == "complete" {
            assert_eq!(Sha256::digest(fs::read(&current.destination).unwrap()), Sha256::digest(&bytes));
            break;
        }
        assert_ne!(current.status, "failed", "{:?}", current.error);
        assert!(Instant::now() < deadline, "Alternate-address download did not complete");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    manager.stop().await.unwrap(); server.shutdown().await;
}

#[tokio::test]
async fn installed_archive_preflight_creates_no_transfer_or_destination() {
    let root = Temp::new();
    let (server, manifest, original) = endpoint_fixture(&root).await;
    let installed_path = root.0.join("endpoint-fixture.zim");
    let book = crate::imports::load_book(installed_path.to_str().unwrap(), false, false).unwrap();
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    let manager = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
    let target = ConnectionTarget::manual("127.0.0.1".into(), server.port);
    let inspection = LocalSwarmManager::inspect(target.clone(), &manifest.swarm_id, None,
        CancellationToken::new(), 0, true).await.unwrap();
    assert_eq!(inspection.archive_id.as_deref(), Some(book.metadata.id.as_str()));
    let destination = root.0.join("never-created");
    for available in [true, false] {
        store.update(|state| { state.books[0].available = available; Ok(()) }).unwrap();
        assert!(manager.start(target.clone(), manifest.swarm_id.clone(), None,
            destination.clone(), None).await.is_err());
        assert!(manager.list().unwrap().is_empty());
        assert!(!destination.exists());
        let retained = store.book(&book.metadata.id).unwrap().unwrap();
        assert_eq!(retained.installation_id, book.installation_id);
        assert_eq!(retained.path, book.path);
        assert_eq!(retained.available, available);
        assert_eq!(fs::read(&installed_path).unwrap(), original);
    }
    manager.stop().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn throttled_identity_preflight_inspects_and_admits_into_a_nonempty_library() {
    let root = Temp::new();
    let (initial_server, manifest, original) = endpoint_fixture(&root).await;
    initial_server.shutdown().await;
    // Keep a genuinely distinct, checksum-valid installation in the receiver.
    let mut existing_bytes = original.clone();
    existing_bytes[8..24].copy_from_slice(Uuid::new_v4().as_bytes());
    let checksum = u64::from_le_bytes(existing_bytes[72..80].try_into().unwrap()) as usize;
    let digest = md5::compute(&existing_bytes[..checksum]).0;
    existing_bytes[checksum..checksum + 16].copy_from_slice(&digest);
    let existing_path = root.0.join("existing.zim");
    fs::write(&existing_path, &existing_bytes).unwrap();
    let existing = crate::imports::load_book(existing_path.to_str().unwrap(), false, false).unwrap();
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    store.update(|state| { state.books.push(existing.clone()); Ok(()) }).unwrap();
    let manager = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
    // One whole verified chunk takes >30s at this supported limit. The stock
    // fixture is smaller than a production 1MiB chunk, so scale the limit down
    // rather than altering the archive or replacing the transport with a mock.
    assert_eq!(manifest.chunk_hashes.len(), 1);
    let limit = (original.len() as u64 / 35).max(1);
    let data = Arc::new(original.clone());
    let reader: network::ChunkReader = Arc::new(move |index| {
        if index != 0 { return Err("Unexpected fixture chunk".into()); }
        Ok((*data).clone())
    });
    let upload = Arc::new(RateBudget::new(0));
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        upload.clone(), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let target = ConnectionTarget::manual("127.0.0.1".into(), server.port);
    let started = Instant::now();
    let inspection = tokio::time::timeout(Duration::from_secs(90),
        LocalSwarmManager::inspect(target.clone(), &manifest.swarm_id, None,
            CancellationToken::new(), limit, true)).await.unwrap().unwrap();
    assert!(started.elapsed() > Duration::from_secs(30), "Inspection bypassed the throttled verified chunk");
    assert_eq!(inspection.archive_id, Some(Uuid::from_slice(&original[8..24]).unwrap().to_string()));
    assert!(manager.list().unwrap().is_empty());
    upload.set_limit(limit);
    let started = Instant::now();
    let job = tokio::time::timeout(Duration::from_secs(90), manager.start(target,
        manifest.swarm_id.clone(), None, root.0.join("received"), None)).await.unwrap().unwrap();
    assert!(started.elapsed() > Duration::from_secs(30), "Nonempty-library admission bypassed identity verification");
    upload.set_limit(0);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        if row.status == "complete" {
            assert_eq!(fs::read(&row.destination).unwrap(), original);
            break;
        }
        assert_ne!(row.status, "failed", "{:?}", row.error);
        assert!(Instant::now() < deadline, "Admitted throttled transfer did not finish");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let retained = store.book(&existing.metadata.id).unwrap().unwrap();
    assert_eq!(retained.installation_id, existing.installation_id);
    assert_eq!(fs::read(&existing_path).unwrap(), existing_bytes);
    manager.stop().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn stopping_cancels_identity_preflight_without_admitting_or_staging() {
    let root = Temp::new();
    let (server, manifest, _) = endpoint_fixture(&root).await;
    let existing = crate::imports::load_book(root.0.join("endpoint-fixture.zim").to_str().unwrap(), false, false).unwrap();
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    store.update(|state| { state.books.push(existing); state.settings.download_limit = 1; Ok(()) }).unwrap();
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let destination = root.0.join("never-staged");
    let receiver = manager.clone(); let target = ConnectionTarget::manual("127.0.0.1".into(), server.port);
    let directory = destination.clone();
    let admission = tokio::spawn(async move { receiver.start(target, manifest.swarm_id.clone(), None, directory, None).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!admission.is_finished(), "The identity check did not wait for its download budget");
    tokio::time::timeout(Duration::from_secs(3), manager.stop()).await.unwrap().unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), admission).await.unwrap().unwrap().is_err());
    assert!(manager.list().unwrap().is_empty());
    assert!(!destination.exists());
    server.shutdown().await;
}

#[tokio::test]
async fn duplicate_preflight_reads_identity_across_split_part_boundaries() {
    let original = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    let manifest = Arc::new(content_manifest(&[
        ("book.zimaa", &original[..7]), ("book.zimab", &original[7..17]),
        ("book.zimac", &original[17..]),
    ]));
    let data = Arc::new(vec![original[..7].to_vec(), original[7..17].to_vec(), original[17..].to_vec()]);
    let chunks = manifest.clone();
    let reader: network::ChunkReader = Arc::new(move |index| {
        let (file, offset) = storage::chunk_position(&chunks, index)?;
        let offset = offset as usize;
        Ok(data[file][offset..data[file].len().min(offset + 1024)].to_vec())
    });
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let inspection = LocalSwarmManager::inspect(ConnectionTarget::manual("127.0.0.1".into(), server.port),
        &manifest.swarm_id, None, CancellationToken::new(), 0, true).await.unwrap();
    let expected = Archive::open(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap().metadata().unwrap().id;
    assert_eq!(inspection.archive_id.as_deref(), Some(expected.as_str()));
    server.shutdown().await;
}

#[tokio::test]
async fn optional_silent_source_does_not_delay_payload_and_is_joined_on_retirement() {
    let root = Temp::new();
    let (server, manifest, _) = endpoint_fixture(&root).await;
    // A genuinely bound silent UDP endpoint: there is no ICMP refusal to make
    // a serial QUIC connect fail quickly. Its normal connect deadline is 8s.
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let candidates = VecDeque::from([
        ConnectionTarget::manual("127.0.0.1".into(), server.port),
        ConnectionTarget::manual("127.0.0.1".into(), silent.local_addr().unwrap().port()),
    ]);
    let activity = Arc::new(network::ReceiveActivity::default());
    let mut dialer = SourceDialer::new(candidates, manifest.swarm_id.clone(), None,
        Uuid::new_v4().to_string(), CancellationToken::new(), Arc::new(RateBudget::new(0)), activity.clone());
    let started = Instant::now();
    dialer.fill(0);
    let mut session = loop {
        if let Some(session) = dialer.next().await { break session; }
        assert!(dialer.has_pending(), "{}", dialer.error());
    };
    assert_eq!(activity.active_sessions(), 1);
    activity.set_accepting(false);
    let bytes = session.get_chunk(0).await.unwrap();
    assert_eq!(hex::encode(Sha256::digest(&bytes)), manifest.chunk_hashes[0]);
    dialer.stop().await;
    session.shutdown().await;
    drop(session);
    assert_eq!(activity.active_sessions(), 0);
    assert!(started.elapsed() < Duration::from_secs(5), "Optional source setup blocked verified payload or teardown");
    assert!(!dialer.has_pending());
    server.shutdown().await;
}

#[tokio::test]
async fn stopping_and_restarting_preserves_terminal_failures_and_cancellations() {
    let root = Temp::new();
    let mut bytes = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff; // Chunk-valid manifest, invalid ZIM MD5.
    let manifest = Arc::new(content_manifest(&[("book.zim", &bytes)]));
    let sid = manifest.swarm_id.clone();
    let data = Arc::new(bytes);
    let reader: network::ChunkReader = Arc::new(move |index| {
        Ok(data[index as usize * 1024..data.len().min((index as usize + 1) * 1024)].to_vec())
    });
    let server = network::seed(manifest, Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let store = Arc::new(Store::new(root.0.join("profile")).unwrap());
    let manager = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
    let target = ConnectionTarget::manual("127.0.0.1".into(), server.port);
    let failed = manager.start(target.clone(), sid.clone(), None, root.0.join("received"), None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let failure = loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == failed.id).unwrap();
        if row.status == "failed" { break row.error.unwrap(); }
        assert!(Instant::now() < deadline, "Invalid ZIM did not fail verification");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let cancelled = manager.start(target, sid, None, root.0.join("received"), None).await.unwrap();
    manager.pause(&cancelled.id, true).await.unwrap();
    manager.stop().await.unwrap();
    let rows = manager.list().unwrap();
    let retained = rows.iter().find(|row| row.id == failed.id).unwrap();
    assert_eq!(retained.status, "failed");
    assert_eq!(retained.error.as_deref(), Some(failure.as_str()));
    assert_eq!(rows.iter().find(|row| row.id == cancelled.id).unwrap().status, "cancelled");
    drop(manager);
    let restarted = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let rows = restarted.list().unwrap();
    assert_eq!(rows.iter().find(|row| row.id == failed.id).unwrap().error.as_deref(), Some(failure.as_str()));
    assert_eq!(rows.iter().find(|row| row.id == cancelled.id).unwrap().status, "cancelled");
    restarted.stop().await.unwrap();
    server.shutdown().await;
}

/// The pipeline writes every verified chunk immediately, but the archive fsync,
/// staged-identity check and job-state checkpoint are batched: the persisted job
/// advances only in whole checkpoint windows, while in-memory progress moves per
/// chunk, and a pause flushes exactly the chunks it claims.
#[tokio::test]
async fn checkpoint_window_batches_state_updates_and_pause_flushes_verified_chunks() {
    let root = Temp::new();
    let data = Arc::new((0..4 * CHECKPOINT_CHUNKS * 1024).map(|offset| (offset % 251) as u8).collect::<Vec<u8>>());
    let manifest = Arc::new(content_manifest(&[("book.zim", &data)]));
    assert_eq!(manifest.chunk_hashes.len(), 4 * CHECKPOINT_CHUNKS);
    let source = data.clone();
    let reader: network::ChunkReader = Arc::new(move |index| {
        let start = index as usize * 1024;
        Ok(source[start..(start + 1024).min(source.len())].to_vec())
    });
    // Throttled so the checkpoint cadence is observable while the transfer runs.
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(64 * 1024)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    let manager = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
    let target = ConnectionTarget::manual("127.0.0.1".into(), server.port);
    let job = manager.start(target, manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut observed = std::collections::BTreeSet::new();
    let mut saved_during_window = false;
    loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        if !saved_during_window && (1..(CHECKPOINT_CHUNKS * 1024) as u64).contains(&row.downloaded) {
            // An unrelated save while UI progress is ahead must not accidentally
            // journal the worker's unsynced bytes.
            manager.record_route(&job.id, "test route change");
            let jobs = load(&manager.inner.directory, &store).unwrap();
            assert_eq!(jobs[&job.id].job.downloaded, 0);
            assert!(jobs[&job.id].verified.iter().all(|bit| !bit));
            saved_during_window = true;
        }
        assert_ne!(row.status, "failed", "{:?}", row.error);
        // The on-disk job row is the durable record; it must only ever move in
        // whole windows even though the in-memory row moves per chunk.
        if let Some(saved) = load(&manager.inner.directory, &store).unwrap().get(&job.id) {
            let count = saved.verified.iter().filter(|bit| **bit).count();
            if count != 0 { observed.insert(count); }
            if count >= 2 * CHECKPOINT_CHUNKS { break; }
        }
        assert!(Instant::now() < deadline, "Checkpoint window did not advance");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(saved_during_window, "Did not exercise an unrelated save during the first window");
    assert!(observed.len() >= 2, "Only one checkpoint window was observed: {observed:?}");
    assert!(observed.iter().all(|count| count % CHECKPOINT_CHUNKS == 0), "Per-chunk durable checkpoints: {observed:?}");
    manager.pause(&job.id, false).await.unwrap();
    let saved = manager.inner.state.lock().unwrap().jobs.get(&job.id).cloned().unwrap();
    let verified = saved.verified.clone();
    assert!(verified.iter().any(|bit| *bit));
    assert!(verified.iter().any(|bit| !*bit), "The transfer must still be partial");
    assert_eq!(saved.job.downloaded, verified_bytes(&manifest, &verified).unwrap());
    // Pause flushed: every chunk the committed checkpoint claims is on disk.
    let mut open = saved.owned.open(&manifest, false).unwrap();
    for (index, bit) in verified.iter().enumerate() {
        if !*bit { continue; }
        let start = index * 1024;
        assert_eq!(open.read_chunk(&manifest, index as u32).unwrap(), data[start..start + 1024],
            "Checkpoint claimed an unflushed chunk");
    }
    manager.stop().await.unwrap();
    server.shutdown().await;
}

/// Unknown bytes after a crash are not claims, even if they match the manifest.
/// This covers both a surviving uncommitted write and a matching sparse zero.
#[test]
fn crash_between_checkpoints_keeps_only_the_committed_verified_prefix() {
    let root = Temp::new();
    let chunk = |index: u32| {
        if index == 19 { return vec![0; 4]; }
        (0..4).map(|offset| (index * 11 + offset) as u8).collect::<Vec<u8>>()
    };
    let mut manifest = Manifest { zim: None,
        protocol_version: 1, swarm_id: String::new(), name: "durability-fixture.zim".into(),
        total_bytes: 80, chunk_size_bytes: 4, chunk_hash_algo: "sha256".into(),
        files: vec![SwarmFile { path: "durability-fixture.zim".into(), size_bytes: 80, start_chunk_index: 0, end_chunk_index: 19 }],
        chunk_hashes: (0..20u32).map(|index| hex::encode(Sha256::digest(chunk(index)))).collect(),
    };
    manifest.swarm_id = manifest.content_address();
    manifest.validate().unwrap();
    let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    // A committed prefix of eight chunks; the production window is much larger,
    // but the crash contract is identical for any window boundary.
    let checkpoint = 8usize;
    for index in 0..checkpoint as u32 { open.write_chunk(&manifest, index, &chunk(index)).unwrap(); }
    open.commit_data(&manifest, &owned).unwrap();
    open.write_chunk(&manifest, checkpoint as u32, &chunk(checkpoint as u32)).unwrap();
    let mut verified = vec![false; 20];
    verified[..checkpoint].fill(true);
    drop(open);
    // An uncommitted matching write is still present; the rest of the staging
    // file, including a legitimate zero chunk in the manifest, was never written.
    let mut open = owned.open(&manifest, false).unwrap();
    rehash_verified(&mut open, &manifest, &owned, &mut verified, &CancellationToken::new()).unwrap();
    let mut expected = vec![false; 20]; expected[..checkpoint].fill(true);
    assert_eq!(verified, expected, "Recovery must not claim matching bytes outside the checkpoint");
    assert_eq!(verified_bytes(&manifest, &verified).unwrap(), (checkpoint * 4) as u64);
    // Resume re-downloads the lost window and the file becomes whole again.
    for index in checkpoint as u32..20 {
        open.write_chunk(&manifest, index, &chunk(index)).unwrap();
        verified[index as usize] = true;
    }
    open.sync_data().unwrap();
    rehash_verified(&mut open, &manifest, &owned, &mut verified, &CancellationToken::new()).unwrap();
    assert!(verified.iter().all(|bit| *bit));
}

/// Whole production receive path (per-source request window, transport SHA-256,
/// byte-capped prefetch, staged write, batched fsync, staged-identity check and
/// job-state checkpoint) over a real local interface. The synthetic archive is
/// not a valid ZIM, so the measured window ends after the final durability
/// flush and before native verification.
///
/// Knobs: KIWIX_LOCALSWARM_TEST_HOST, KIWIX_LOCALSWARM_BENCH_CHUNKS,
/// KIWIX_LOCALSWARM_BENCH_CHUNK_KIB, KIWIX_LOCALSWARM_BENCH_RATE (receiver
/// download budget in bytes/s, 0 = unlimited) and
/// KIWIX_LOCALSWARM_BENCH_SEED_APPLE=1 to serve through the Network.framework
/// bridge instead of Quinn (the shape a Mac actually offers).
/// KIWIX_LOCALSWARM_TEST_SYNC_DELAY_MS
/// adds simulated fsync latency, and KIWIX_LOCALSWARM_TEST_CHECKPOINT_CHUNKS /
/// _BYTES override the durability window, so the same build can measure a
/// phone-like commit cost at both a per-chunk and the production cadence.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Production receive-path measurement; requires a live local interface"]
async fn receive_path_throughput_measurement() {
    let host = std::env::var("KIWIX_LOCALSWARM_TEST_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let chunk_kib = std::env::var("KIWIX_LOCALSWARM_BENCH_CHUNK_KIB").ok().and_then(|value| value.parse::<u32>().ok()).unwrap_or(1024);
    let chunks = std::env::var("KIWIX_LOCALSWARM_BENCH_CHUNKS").ok().and_then(|value| value.parse::<u32>().ok()).unwrap_or(64);
    let rate = std::env::var("KIWIX_LOCALSWARM_BENCH_RATE").ok().and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
    assert!(chunks > 0 && chunk_kib > 0, "Benchmark needs at least one chunk");
    let chunk_size = chunk_kib * 1024;
    let root = Temp::new();
    let data = Arc::new((0..u64::from(chunks) * u64::from(chunk_size)).map(|offset| (offset % 251) as u8).collect::<Vec<u8>>());
    let mut manifest = Manifest { zim: None,
        protocol_version: 1, swarm_id: String::new(), name: "receive-benchmark.zim".into(),
        total_bytes: u64::from(chunks) * u64::from(chunk_size), chunk_size_bytes: chunk_size, chunk_hash_algo: "sha256".into(),
        files: vec![SwarmFile { path: "receive-benchmark.zim".into(), size_bytes: u64::from(chunks) * u64::from(chunk_size),
            start_chunk_index: 0, end_chunk_index: chunks - 1 }],
        chunk_hashes: data.chunks(chunk_size as usize).map(|chunk| hex::encode(Sha256::digest(chunk))).collect(),
    };
    manifest.swarm_id = manifest.content_address();
    manifest.validate().unwrap();
    let manifest = Arc::new(manifest);
    let source = data.clone();
    let reader: network::ChunkReader = Arc::new(move |index| {
        let start = index as usize * chunk_size as usize;
        Ok(source[start..(start + chunk_size as usize).min(source.len())].to_vec())
    });
    // `KIWIX_LOCALSWARM_BENCH_SEED_APPLE=1` swaps in the Network.framework
    // bridge as the seeder, which is the shape a Mac actually serves.
    let apple = std::env::var("KIWIX_LOCALSWARM_BENCH_SEED_APPLE").is_ok_and(|value| value != "0");
    let server = {
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if apple {
            let frame = Arc::new(protocol::Message::ManifestResponse(serde_json::to_vec(manifest.as_ref()).unwrap()).encode().unwrap());
            network::seed_apple(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
                Arc::new(RateBudget::new(0)), CancellationToken::new(), frame, None).await.unwrap()
        } else {
            network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
                Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap()
        }
        #[cfg(not(any(target_os = "ios", target_os = "macos")))]
        {
            assert!(!apple, "Only Apple platforms serve through the Network.framework bridge");
            network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
                Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap()
        }
    };
    server.stats.set_accepting(true);
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    manager.limits(rate, 0);
    let target = ConnectionTarget::manual(host.clone(), server.port);
    #[cfg(feature = "localswarm-profiling")]
    profile::reset();
    let started = Instant::now();
    let job = manager.start(target, manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(900);
    let mut download = None;
    let flushed = loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        if download.is_none() {
            assert_ne!(row.status, "failed", "{:?}", row.error);
            if row.downloaded >= manifest.total_bytes { download = Some(started.elapsed()); }
        } else if matches!(row.status.as_str(), "verifying" | "failed") {
            // The download loop has written every chunk and the final fsync +
            // checkpoint completed; native verification starts after this point.
            break started.elapsed();
        }
        assert!(Instant::now() < deadline, "Receive-path benchmark did not finish");
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let download = download.expect("Benchmark download completed");
    let (checkpoint_chunks, checkpoint_bytes) = checkpoint_window();
    println!("RECEIVE_PATH_MEASUREMENT host={host} seeder={} chunks={chunks} chunk_kib={chunk_kib} bytes={} download_ns={} download_bytes_per_second={:.0} flushed_ns={} flushed_bytes_per_second={:.0} pipeline_window={} pipeline_buffered={} prefetch_mib={} checkpoint_chunks={checkpoint_chunks} checkpoint_mib={} rate={rate}",
        if apple { "apple" } else { "quinn" },
        manifest.total_bytes, download.as_nanos(), manifest.total_bytes as f64 / download.as_secs_f64(),
        flushed.as_nanos(), manifest.total_bytes as f64 / flushed.as_secs_f64(),
        network::PIPELINE_WINDOW, network::PIPELINE_BUFFERED, network::PREFETCH_BYTES / (1024 * 1024),
        checkpoint_bytes / (1024 * 1024));
    #[cfg(feature = "localswarm-profiling")]
    {
        let snapshot = profile::snapshot();
        println!("RECEIVE_PATH_PROFILE {snapshot}");
        let wall = flushed.as_secs_f64().max(0.001);
        if let Some(phases) = snapshot.as_array() {
            for phase in phases {
                let name = phase["phase"].as_str().unwrap_or("?");
                let count = phase["count"].as_u64().unwrap_or(0);
                let bytes = phase["bytes"].as_u64().unwrap_or(0);
                let nanos: f64 = phase["elapsed_ns"].as_str().and_then(|value| value.parse().ok()).unwrap_or(0.0);
                println!("RECEIVE_PATH_PHASE {name} count={count} bytes={bytes} elapsed_ms={:.3} wall_percent={:.1} mb_per_second={:.1}",
                    nanos / 1e6, nanos / 1e9 / wall * 100.0,
                    if nanos == 0.0 { 0.0 } else { bytes as f64 / (nanos / 1e9) / 1e6 });
            }
        }
    }
    manager.stop().await.unwrap();
    server.shutdown().await;
}

/// One-file archive set with a caller-chosen chunk size, so a test can span
/// several durability windows without a larger ZIM fixture.
fn chunked_manifest(bytes: &[u8], chunk: u32) -> Manifest {
    let hashes: Vec<String> = bytes.chunks(chunk as usize).map(|part| hex::encode(Sha256::digest(part))).collect();
    let mut manifest = Manifest { zim: None,
        protocol_version: 1, swarm_id: String::new(), name: "Link loss fixture".into(),
        total_bytes: bytes.len() as u64, chunk_size_bytes: chunk, chunk_hash_algo: "sha256".into(),
        files: vec![SwarmFile { path: "book.zim".into(), size_bytes: bytes.len() as u64,
            start_chunk_index: 0, end_chunk_index: hashes.len() as u32 - 1 }],
        chunk_hashes: hashes,
    };
    manifest.swarm_id = manifest.content_address();
    manifest.validate().unwrap();
    manifest
}

/// Holds the seeder's final chunk read so a test can tear the link down at a
/// known point, then serve the redial normally.
struct ReadGate { released: Mutex<bool>, ready: std::sync::Condvar }
impl ReadGate {
    fn new() -> Self { Self { released: Mutex::new(false), ready: std::sync::Condvar::new() } }
    fn wait(&self) {
        let mut released = self.released.lock().unwrap_or_else(|error| error.into_inner());
        while !*released { released = self.ready.wait(released).unwrap_or_else(|error| error.into_inner()); }
    }
    fn release(&self) {
        *self.released.lock().unwrap_or_else(|error| error.into_inner()) = true;
        self.ready.notify_all();
    }
}

/// A mid-transfer link loss retires only that connection. The receiver redials
/// the same endpoint through the normal dial path — which revalidates the local
/// link and repeats the handshake, manifest and bitfield — and resumes from the
/// durable verified prefix instead of failing the whole job.
#[tokio::test]
async fn mid_transfer_link_loss_redials_the_peer_and_resumes_from_the_verified_prefix() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // 256-byte chunks put the 36 KiB fixture across more than one durability
    // window, so the loss lands after a checkpointed prefix exists.
    const CHUNK: u32 = 256;
    let root = Temp::new();
    let bytes = Arc::new(fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap());
    let manifest = Arc::new(chunked_manifest(&bytes, CHUNK));
    let chunk_count = manifest.chunk_hashes.len() as u32;
    assert!(chunk_count as usize > CHECKPOINT_CHUNKS, "Fixture must span a durability window");
    let requests = Arc::new(Mutex::new(vec![0u32; chunk_count as usize]));
    let blocked = Arc::new(AtomicBool::new(false));
    let gate = Arc::new(ReadGate::new());
    let reader: network::ChunkReader = {
        let bytes = bytes.clone(); let requests = requests.clone();
        let blocked = blocked.clone(); let gate = gate.clone();
        let last = chunk_count - 1;
        Arc::new(move |index| {
            requests.lock().unwrap()[index as usize] += 1;
            if index == last {
                // Hold the final response until the test has observed the
                // checkpointed prefix and torn the link down.
                blocked.store(true, Ordering::Release);
                gate.wait();
            }
            let start = index as usize * CHUNK as usize;
            Ok(bytes[start..bytes.len().min(start + CHUNK as usize)].to_vec())
        })
    };
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let job = manager.start(ConnectionTarget::manual("127.0.0.1".into(), server.port),
        manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let durable = loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        assert_ne!(row.status, "failed", "{:?}", row.error);
        let checkpointed = (row.downloaded / u64::from(CHUNK)) as usize;
        if checkpointed != 0 && blocked.load(Ordering::Acquire) { break checkpointed; }
        assert!(Instant::now() < deadline, "No durable prefix or final request before the loss");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert!(durable < chunk_count as usize, "The loss must land before the last chunk is written");
    // Force the link down exactly as a Wi-Fi roam or AWDL flap does: the
    // listener stays up, so only a redial can recover the transfer.
    server.stats.drop_connections();
    gate.release();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        if row.status == "complete" {
            assert_eq!(Sha256::digest(fs::read(&row.destination).unwrap()), Sha256::digest(bytes.as_slice()));
            break;
        }
        assert_ne!(row.status, "failed", "{:?}", row.error);
        assert!(Instant::now() < deadline, "The job did not complete after the redial");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(server.stats.connection_count() >= 1,
        "The lost source was not redialed; the listener saw no connection after the loss");
    {
        let requests = requests.lock().unwrap_or_else(|error| error.into_inner());
        assert!(requests[..durable].iter().all(|count| *count == 1),
            "The durable verified prefix was requested again after the redial: {:?}", &requests[..durable]);
        assert_eq!(requests[(chunk_count - 1) as usize], 2,
            "The chunk that was in flight when the link died was not re-requested");
    }
    manager.stop().await.unwrap();
    server.shutdown().await;
}

/// A peer that keeps dropping the stream is redialed a bounded number of times
/// and then reported truthfully, instead of looping forever or failing the job
/// on the first flap.
#[tokio::test]
async fn repeated_link_loss_surfaces_a_bounded_error_instead_of_looping() {
    let root = Temp::new();
    let bytes = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    let manifest = Arc::new(chunked_manifest(&bytes, 1024));
    // Every chunk read fails, so the seeder closes each connection right after
    // the handshake: a peer whose link never comes back.
    let reader: network::ChunkReader = Arc::new(|_| Err("LocalSwarm source read failed".into()));
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let store = Arc::new(Store::new(root.0.join("receiver-profile")).unwrap());
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let started = Instant::now();
    let job = manager.start(ConnectionTarget::manual("127.0.0.1".into(), server.port),
        manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let error = loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        if row.status == "failed" { break row.error.unwrap_or_default(); }
        assert!(Instant::now() < deadline, "Repeated link loss never surfaced a bounded error");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(error.contains("reconnect limit reached"), "Untruthful terminal error: {error}");
    assert!(error.contains("retained verified data can be resumed"), "Missing resume guidance: {error}");
    // One manifest connection plus the first session plus exactly the bounded
    // reconnect budget; never an unbounded redial loop.
    let attempts = usize::from(sources::MAX_SOURCE_RECONNECTS);
    let connections = server.stats.connection_count();
    assert!(connections > attempts, "Too few redials: {connections}");
    assert!(connections <= 2 + attempts, "The retry budget did not bound the attempts: {connections}");
    assert!(started.elapsed() < Duration::from_secs(25), "The retry policy did not stay bounded in time");
    manager.stop().await.unwrap();
    server.shutdown().await;
}

/// Real DNS-SD acceptance; kept opt-in so the ordinary suite never publishes
/// test libraries onto the user's network.
#[tokio::test]
#[ignore = "requires KIWIX_LOCALSWARM_TEST_HOST set to an actual local Wi-Fi IPv4 address"]
async fn library_restart_replaces_one_row_and_stale_withdrawal_preserves_the_new_session() {
    let address = std::env::var("KIWIX_LOCALSWARM_TEST_HOST").expect("Select the test Wi-Fi IPv4 address");
    let first_id = Uuid::new_v4().to_string();
    let second_id = Uuid::new_v4().to_string();
    let mut first = Discovery::new(first_id.clone()).unwrap();
    let mut second = Discovery::new(second_id.clone()).unwrap();
    let mut receiver = Discovery::new(Uuid::new_v4().to_string()).unwrap();
    async fn wait_session(receiver: &Discovery, publisher: &str, session: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let peers = receiver.peers().into_iter().filter(|peer| peer.peer_id == publisher
                && peer.transport == "quic-library").collect::<Vec<_>>();
            if peers.len() == 1 && peers[0].library_session_id.as_deref() == Some(session) {
                return peers[0].id.clone();
            }
            assert!(Instant::now() < deadline, "Expected one current library for {publisher}: {peers:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    receiver.start_browse().await.unwrap();
    let old_session = "a".repeat(32);
    let new_session = "b".repeat(32);
    let other_session = "c".repeat(32);
    let old = first.advertise_library(&old_session, 8090, 1, std::slice::from_ref(&address), "Same nickname", 1, false).await.unwrap();
    let old_row = wait_session(&receiver, &first_id, &old_session).await;
    first.unadvertise(&old).await.unwrap();
    let current = first.advertise_library(&new_session, 8091, 1, std::slice::from_ref(&address), "Renamed sender", 2, false).await.unwrap();
    let current_row = wait_session(&receiver, &first_id, &new_session).await;
    assert_eq!(receiver.peers().iter().find(|peer| peer.peer_id == first_id).unwrap().publisher_name.as_deref(), Some("Renamed sender"));
    assert_ne!(old_row, current_row);
    assert!(receiver.peers().iter().all(|peer| peer.id != old_row));
    assert_ne!(old, current);
    first.unadvertise(&old).await.unwrap();
    let other = second.advertise_library(&other_session, 8092, 1, &[address], "Renamed sender", 2, false).await.unwrap();
    let other_row = wait_session(&receiver, &second_id, &other_session).await;
    assert_ne!(current_row, other_row);
    assert_eq!(receiver.peers().iter().filter(|peer| peer.publisher_name.as_deref() == Some("Renamed sender")).count(), 2);
    receiver.stop_browse().await.unwrap();
    receiver.start_browse().await.unwrap();
    assert_eq!(wait_session(&receiver, &first_id, &new_session).await, current_row);
    assert_eq!(wait_session(&receiver, &second_id, &other_session).await, other_row);
    first.unadvertise(&current).await.unwrap();
    second.unadvertise(&other).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while receiver.peers().iter().any(|peer| peer.peer_id == first_id || peer.peer_id == second_id) {
        assert!(Instant::now() < deadline, "Stopped test library remained discoverable");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    receiver.shutdown().await.unwrap();
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}

#[test]
fn adopts_only_hash_verified_chunks_from_partial_downloads() {
    use crate::partial_donor::Donor;
    let root = Temp::new();
    let bytes: Vec<u8> = (0..40u8).map(|b| b.wrapping_mul(31).wrapping_add(1)).collect();
    let manifest = chunked_manifest(&bytes, 8);
    let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    // Chunk 0 correct, 1 unwritten (zeros), 2 corrupt, 3 and 4 correct.
    let mut partial = bytes.clone();
    partial[8..16].fill(0);
    partial[16] ^= 0xff;
    let donor_dir = root.0.join("torrent-staging");
    fs::create_dir(&donor_dir).unwrap();
    let donor = donor_dir.join("book.zim");
    fs::write(&donor, &partial).unwrap();
    let donors = vec![
        Donor { path: donor_dir.join("missing.zim"), size: 40, ranges: None },
        Donor { path: donor.clone(), size: 41, ranges: None },
        // The owner verified only the first 32 bytes, so chunk 4 is not offered.
        Donor { path: donor.clone(), size: 40, ranges: Some(vec![(0, 32)]) },
    ];
    let mut bits = vec![false; 5];
    let cancel = CancellationToken::new();
    assert_eq!(adopt_partial_chunks(&mut open, &manifest, &mut bits, &donors, &cancel).unwrap(), 16);
    assert_eq!(bits, [true, false, false, true, false]);
    open.commit_data(&manifest, &owned).unwrap();
    assert_eq!(open.read_chunk(&manifest, 0).unwrap(), bytes[0..8]);
    assert_eq!(open.read_chunk(&manifest, 3).unwrap(), bytes[24..32]);
    assert!(open.read_chunk(&manifest, 2).unwrap().iter().all(|b| *b == 0));

    // A donor without a range map offers every chunk; only verified bytes land.
    let unbounded = [Donor { path: donor.clone(), size: 40, ranges: None }];
    assert_eq!(adopt_partial_chunks(&mut open, &manifest, &mut bits, &unbounded, &cancel).unwrap(), 8);
    assert_eq!(bits, [true, false, false, true, true]);
    assert_eq!(fs::read(&donor).unwrap(), partial, "donor must only be read");

    // Cancellation stops before adopting anything further.
    let mut fresh = vec![false; 5];
    cancel.cancel();
    assert_eq!(adopt_partial_chunks(&mut open, &manifest, &mut fresh, &unbounded, &cancel).unwrap(), 0);
    assert!(fresh.iter().all(|bit| !bit));

    // A rehash of the adopted staging agrees with the recorded bits.
    let mut recheck = bits.clone();
    rehash_verified(&mut open, &manifest, &owned, &mut recheck, &CancellationToken::new()).unwrap();
    assert_eq!(recheck, bits);
}

#[test]
fn abandons_a_same_sized_donor_with_different_content() {
    use crate::partial_donor::{Donor, MISMATCH_LIMIT};
    let root = Temp::new();
    let chunks = MISMATCH_LIMIT + 2;
    let bytes: Vec<u8> = (0..chunks * 4).map(|i| (i % 200) as u8 + 1).collect();
    let manifest = chunked_manifest(&bytes, 4);
    let (_owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    // Every chunk differs except the last, which a full scan would adopt.
    let mut other: Vec<u8> = bytes.iter().map(|b| b.wrapping_add(1).max(1)).collect();
    let tail = other.len() - 4;
    other[tail..].copy_from_slice(&bytes[tail..]);
    let donor = root.0.join("other.zim");
    fs::write(&donor, &other).unwrap();
    let mut bits = vec![false; chunks];
    let donors = [Donor { path: donor, size: bytes.len() as u64, ranges: None }];
    assert_eq!(adopt_partial_chunks(&mut open, &manifest, &mut bits, &donors, &CancellationToken::new()).unwrap(), 0);
    assert!(bits.iter().all(|bit| !bit));
}

/// End to end: an unfinished BitTorrent acquisition of the same archive is
/// recorded in the receiver's profile. The LocalSwarm receive adopts its
/// hash-verified chunks and requests only the rest from the peer, then the
/// normal final ZIM verification and publication run on the combined bytes.
#[tokio::test]
async fn receive_reuses_an_unfinished_torrent_partial_and_fetches_only_missing_chunks() {
    let root = Temp::new();
    let data = Arc::new(fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap());
    let manifest = Arc::new(content_manifest(&[("book.zim", &data)]));
    let chunks = manifest.chunk_hashes.len();
    assert!(chunks > 8, "fixture must span several chunks");
    let half = chunks / 2;
    // The torrent partial holds the first half; the rest was never written.
    let profile = root.0.join("receiver-profile");
    let staging = profile.join(".kiwix-torrent-fixture");
    fs::create_dir_all(&staging).unwrap();
    fs::create_dir_all(profile.join("torrents")).unwrap();
    let mut partial = vec![0u8; data.len()];
    partial[..half * 1024].copy_from_slice(&data[..half * 1024]);
    fs::write(staging.join("wikipedia_test.zim"), &partial).unwrap();
    let record = serde_json::json!({
        "version": 1, "downloadLimit": 0, "uploadLimit": 0,
        "jobs": [{
            "job": {
                "id": "torrent:fixture", "url": "https://example.invalid/a.torrent",
                "fileName": "wikipedia_test.zim", "destination": profile.to_string_lossy(),
                "status": "paused", "downloaded": half * 1024, "total": data.len(), "error": null,
                "speed": 0.0, "etaSeconds": null, "importError": null, "transport": "torrent",
                "seeding": false, "uploadSpeed": 0.0, "uploaded": 0, "completedPaths": []
            },
            "staging": staging, "directory": profile, "metadata": profile.join("torrents/fixture.torrent"),
            "files": ["wikipedia_test.zim"], "paused": true, "verified": false,
            "spaceFiles": [{"path": "wikipedia_test.zim", "size": data.len()}]
        }]
    });
    let store = Arc::new(Store::new(profile.clone()).unwrap());
    fs::write(profile.join("torrents/jobs.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    let donors = crate::torrent::TorrentManager::saved_partial_files(&store).unwrap();
    assert_eq!(donors.len(), 1, "the saved torrent record must be read as a donor");

    let requested = Arc::new(Mutex::new(Vec::new()));
    let (source, log) = (data.clone(), requested.clone());
    let reader: network::ChunkReader = Arc::new(move |index| {
        log.lock().unwrap().push(index);
        let start = index as usize * 1024;
        Ok(source[start..(start + 1024).min(source.len())].to_vec())
    });
    let server = network::seed(manifest.clone(), Uuid::new_v4().to_string(), None, reader,
        Arc::new(RateBudget::new(0)), CancellationToken::new(), None).await.unwrap();
    server.stats.set_accepting(true);
    let manager = LocalSwarmManager::new(store, CancellationToken::new()).unwrap();
    let target = ConnectionTarget::manual("127.0.0.1".into(), server.port);
    let job = manager.start(target, manifest.swarm_id.clone(), None, root.0.join("received"), None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let row = manager.list().unwrap().into_iter().find(|row| row.id == job.id).unwrap();
        assert_ne!(row.status, "failed", "{:?}", row.error);
        if row.status == "complete" {
            assert_eq!(fs::read(&row.destination).unwrap(), *data);
            break;
        }
        assert!(Instant::now() < deadline, "Transfer with a reused partial did not complete");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut served = requested.lock().unwrap().clone();
    served.sort_unstable();
    served.dedup();
    // Unwritten (zero) regions of the partial still satisfy the archive's own
    // all-zero chunks, which are verified by hash like any other chunk.
    let missing: Vec<u32> = (half..chunks)
        .filter(|index| data[index * 1024..((index + 1) * 1024).min(data.len())].iter().any(|byte| *byte != 0))
        .map(|index| index as u32).collect();
    assert!(!missing.is_empty() && missing.len() < chunks - half);
    assert_eq!(served, missing, "only chunks missing from the partial may be fetched");
    assert_eq!(fs::read(staging.join("wikipedia_test.zim")).unwrap(), partial, "the torrent partial is only read");
    manager.stop().await.unwrap();
    server.shutdown().await;
}

#[test]
fn in_flight_pieces_do_not_abandon_a_donor() {
    use crate::partial_donor::{Donor, MISMATCH_LIMIT};
    let root = Temp::new();
    const CHUNK: usize = 32 * 1024;
    let chunks = MISMATCH_LIMIT + 2;
    let bytes: Vec<u8> = (0..chunks * CHUNK).map(|i| (i % 199) as u8 + 1).collect();
    let manifest = chunked_manifest(&bytes, CHUNK as u32);
    let (_owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    // Every chunk but the last is a half-written piece: wrong bytes followed
    // by an unwritten 16 KiB block. Only the final chunk is complete.
    let mut partial = vec![0u8; bytes.len()];
    for index in 0..chunks - 1 { partial[index * CHUNK..index * CHUNK + CHUNK / 2].fill(0xab); }
    let tail = (chunks - 1) * CHUNK;
    partial[tail..].copy_from_slice(&bytes[tail..]);
    let donor = root.0.join("in-flight.zim");
    fs::write(&donor, &partial).unwrap();
    let mut bits = vec![false; chunks];
    let donors = [Donor { path: donor, size: bytes.len() as u64, ranges: None }];
    assert_eq!(adopt_partial_chunks(&mut open, &manifest, &mut bits, &donors, &CancellationToken::new()).unwrap(), CHUNK as u64);
    assert!(bits[chunks - 1] && bits[..chunks - 1].iter().all(|bit| !bit));
}

#[test]
fn staged_set_publication_survives_a_refused_commit_and_follows_the_published_identity() {
    let root = Temp::new(); let manifest = manifest(&["book.zim"]);
    let (owned, mut open) = OwnedSet::create(Directory::open(&root.0).unwrap(), &manifest, &Uuid::new_v4().to_string()).unwrap();
    open.write_chunk(&manifest, 0, b"abcd").unwrap(); open.write_chunk(&manifest, 1, b"efgh").unwrap();
    open.seal(&manifest, &owned).unwrap();
    let verified = |open: &OpenSet| manifest.files.iter().zip(&open.files).map(|(entry, file)| (
        PathBuf::from(&entry.path),
        crate::publication::VerifiedFile::new(file.try_clone().unwrap(), entry.size_bytes).unwrap(),
    )).collect::<Vec<_>>();
    let destination = root.0.join(&owned.destination);
    fs::create_dir(&destination).unwrap(); fs::write(destination.join("sentinel"), b"user").unwrap();
    assert!(open.publish(&manifest, &owned, verified(&open)).is_err());
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"user");
    assert!(!open.published);
    // Still staged: Windows rebinds its surrendered handles by identity.
    open.unchanged(&manifest, &owned).unwrap();
    assert_eq!(open.read_chunk(&manifest, 0).unwrap(), b"abcd");
    fs::remove_file(destination.join("sentinel")).unwrap(); fs::remove_dir(&destination).unwrap();
    open.publish(&manifest, &owned, verified(&open)).unwrap();
    assert!(open.published && !root.0.join(&owned.staging).exists());
    // The set's handles now name the published directory and members.
    open.unchanged(&manifest, &owned).unwrap();
    assert_eq!(open.read_chunk(&manifest, 1).unwrap(), b"efgh");
    assert_eq!(fs::read(destination.join("book.zim")).unwrap(), b"abcdefgh");
    drop(open);
    assert!(owned.open(&manifest, true).unwrap().published);
}

#[cfg(windows)]
#[test]
fn windows_empty_directory_removal_is_bound_to_identity() {
    let root = Temp::new(); let parent = Directory::open(&root.0).unwrap();
    let owned = parent.child("owned", true).unwrap(); let identity = owned.identity.clone();
    // An open capability (no FILE_SHARE_DELETE) refuses the DELETE handle.
    assert!(parent.remove_empty_directory("owned", &identity).is_err());
    drop(owned);
    fs::rename(root.0.join("owned"), root.0.join("retained")).unwrap(); fs::create_dir(root.0.join("owned")).unwrap();
    assert!(parent.remove_empty_directory("owned", &identity).is_err(), "a substituted directory is foreign");
    assert!(root.0.join("owned").exists());
    fs::write(root.0.join("retained/member"), b"user").unwrap();
    assert!(parent.remove_empty_directory("retained", &identity).is_err(), "a non-empty directory is preserved");
    assert_eq!(fs::read(root.0.join("retained/member")).unwrap(), b"user");
    fs::remove_file(root.0.join("retained/member")).unwrap();
    parent.remove_empty_directory("retained", &identity).unwrap();
    assert!(!root.0.join("retained").exists());
}
