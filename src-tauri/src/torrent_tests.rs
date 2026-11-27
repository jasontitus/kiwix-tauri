// SPDX-License-Identifier: GPL-3.0-or-later
//! Unit tests for `torrent`, kept beside it rather than inline.
#![forbid(unsafe_code)]
use super::*;

struct Profile(PathBuf, Arc<Store>);

impl Profile {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("kiwix-torrent-policy-{}", Uuid::new_v4()));
        crate::fs_ext::create_private_dir_all(path.join("torrents")).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let store = Arc::new(Store::new(path.clone()).unwrap());
        Self(path, store)
    }

    fn record(&self) -> Record {
        let suffix = Uuid::new_v4().to_string();
        Record {
            job: TorrentJob {
                id: format!("torrent:{suffix}"),
                url: "https://example.invalid/archive.torrent".into(),
                file_name: "archive.zim".into(),
                destination: self.0.to_string_lossy().into_owned(),
                status: "downloading".into(),
                downloaded: 1,
                total: Some(2),
                error: None,
                speed: 1.0,
                eta_seconds: Some(1),
                import_error: None,
                transport: "torrent".into(),
                seeding: false,
                upload_speed: 1.0,
                uploaded: 0,
                completed_paths: vec![],
                shared: false,
                magnet: None,
                torrent_path: None,
                created_at: None,
            },
            staging: self.0.join(format!(".kiwix-torrent-{suffix}")),
            directory: self.0.clone(),
            metadata: self.0.join("torrents").join(format!("{suffix}.torrent")),
            files: vec![],
            paused: false,
            interrupted: false,
            verified: false,
            publication_intents: vec![],
            publication_temporaries: vec![],
            space_files: vec![],
            space_acknowledgements: vec![],
            history_dismissed: false,
            files_deleted: false,
            staging_directories: vec![],
            staging_cleanup: vec![],
            metadata_cleanup: None,
            metadata_temporary: None,
            published_cleanup: vec![],
            share: None,
            initial_peers: vec![],
        }
    }

    fn save(&self, jobs: Vec<Record>) {
        save_state(
            &self.0.join("torrents"),
            &State {
                version: STATE_VERSION,
                download_limit: 0,
                upload_limit: 0,
                jobs,
            },
        )
        .unwrap();
    }
}

const TEST_METADATA: &[u8] = b"d4:infod6:lengthi4e4:name11:archive.zim12:piece lengthi4e6:pieces20:12345678901234567890ee";

#[cfg(unix)]
#[test]
fn aliased_profile_allows_owned_metadata_but_not_descendant_symlinks() {
    let profile = Profile::new();
    let real = profile.0.join("real-profile");
    let alias = profile.0.join("profile-alias");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let store = Store::new(alias).unwrap();
    let metadata = store.data_dir().join("torrents");
    crate::fs_ext::create_private_dir_all(&metadata).unwrap();
    let published = metadata.join("archive.torrent");
    publish_metadata(&published, TEST_METADATA, |_, _| Ok(())).unwrap();
    assert_eq!(fs::read(published).unwrap(), TEST_METADATA);

    let outside = profile.0.join("outside-profile");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, store.data_dir().join("redirect")).unwrap();
    let escaped = store.data_dir().join("redirect").join("archive.torrent");
    assert!(publish_metadata(&escaped, TEST_METADATA, |_, _| Ok(())).is_err());
    assert!(!outside.join("archive.torrent").exists());
}

fn metadata_record(profile: &Profile, shared: bool) -> Record {
    let mut record = if shared {
        share_record(profile, &profile.0).0
    } else {
        profile.record()
    };
    record.job.status = "paused".into();
    record.paused = true;
    record
}

#[test]
fn metadata_final_name_exposes_only_complete_synced_owned_bytes() {
    let profile = Profile::new();
    let record = profile.record();
    let mut checkpoints = 0;
    publish_metadata(&record.metadata, TEST_METADATA, |temporary, ownership| {
        checkpoints += 1;
        match checkpoints {
            1 => {
                assert!(!record.metadata.exists());
                assert!(ownership.is_none());
                let temporary = temporary.unwrap();
                assert_eq!(fs::metadata(record.metadata.with_file_name(temporary.name)).unwrap().len(), 0);
            }
            2 => {
                assert!(!record.metadata.exists());
                let temporary = temporary.unwrap();
                let ownership = ownership.unwrap();
                let path = record.metadata.with_file_name(&temporary.name);
                let file = ownership.renamed(&temporary.name).validate_existing(&path).unwrap().unwrap();
                assert!(ownership.matches_contents(&file).unwrap());
                assert_eq!(fs::read(path).unwrap(), TEST_METADATA);
            }
            3 => {
                assert!(temporary.is_none());
                let ownership = ownership.unwrap();
                let file = ownership.validate_existing(&record.metadata).unwrap().unwrap();
                assert!(ownership.matches_contents(&file).unwrap());
                assert_eq!(fs::read(&record.metadata).unwrap(), TEST_METADATA);
            }
            _ => panic!("Unexpected metadata checkpoint"),
        }
        Ok(())
    }).unwrap();
    assert_eq!(checkpoints, 3);
}

#[test]
fn metadata_checkpoint_failures_recover_acquisitions_and_shares_without_an_engine() {
    for shared in [false, true] {
        for failure in 1..=3 {
            for committed in [false, true] {
                let profile = Profile::new();
                let record = metadata_record(&profile, shared);
                let id = record.job.id.clone();
                profile.save(vec![record.clone()]);
                let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
                let mut checkpoints = 0;
                let result = publish_metadata(&record.metadata, TEST_METADATA, |temporary, ownership| {
                    checkpoints += 1;
                    if checkpoints == failure && !committed {
                        return Err("injected metadata checkpoint failure".into());
                    }
                    manager.update_record(&id, |record| {
                        record.metadata_temporary = temporary;
                        record.metadata_cleanup = ownership;
                    })?;
                    if checkpoints == failure {
                        return Err("injected committed metadata checkpoint failure".into());
                    }
                    Ok(())
                });
                assert!(result.is_err());
                assert_eq!(checkpoints, failure);
                drop(manager);
                let recovered = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
                let saved = recovered.record(&id).unwrap();
                assert!(saved.metadata_temporary.is_none());
                assert!(saved.paused);
                assert_eq!(saved.job.status, "paused");
                assert_eq!(record.metadata.exists(), failure == 3 || failure == 2 && committed);
                if !record.metadata.exists() {
                    recovered.publish_metadata(&id, TEST_METADATA).unwrap();
                }
                assert_eq!(read_torrent_metadata(&recovered.record(&id).unwrap()).unwrap(), TEST_METADATA);
                assert!(recovered.engine_cell().unwrap().get().is_none());
            }
        }
    }
}

#[test]
fn interrupted_partial_metadata_is_reclaimed_by_inode_and_can_be_retried() {
    for shared in [false, true] {
        let profile = Profile::new();
        let mut record = metadata_record(&profile, shared);
        let parent = record.metadata.parent().unwrap();
        let directory = open_publication_directory(parent).unwrap();
        let (temporary, mut output) = create_publication_temporary(
            parent, &directory, format!(".kiwix-publish-{}", Uuid::new_v4()),
        ).unwrap();
        record.metadata_temporary = Some(temporary.clone());
        profile.save(vec![record.clone()]);
        output.write_all(&TEST_METADATA[..17]).unwrap();
        output.sync_all().unwrap();
        drop(output);
        assert!(!record.metadata.exists());
        let id = record.job.id.clone();
        let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        assert!(!parent.join(&temporary.name).exists());
        assert!(!parent.join(&temporary.capture_name).exists());
        assert!(manager.record(&id).unwrap().metadata_temporary.is_none());
        manager.publish_metadata(&id, TEST_METADATA).unwrap();
        assert_eq!(read_torrent_metadata(&manager.record(&id).unwrap()).unwrap(), TEST_METADATA);
    }
}

#[test]
fn metadata_recovery_preserves_identical_unowned_final_and_complete_temporary() {
    let profile = Profile::new();
    let record = metadata_record(&profile, false);
    let id = record.job.id.clone();
    profile.save(vec![record.clone()]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let result = publish_metadata(&record.metadata, TEST_METADATA, |temporary, ownership| {
        manager.update_record(&id, |record| {
            record.metadata_temporary = temporary;
            record.metadata_cleanup = ownership.clone();
        })?;
        if ownership.is_some() {
            fs::write(&record.metadata, TEST_METADATA).unwrap();
        }
        Ok(())
    });
    assert!(result.is_err());
    let saved = manager.record(&id).unwrap();
    let temporary = saved.metadata_temporary.unwrap();
    let parent = record.metadata.parent().unwrap();
    let original = saved.metadata_cleanup.unwrap();
    drop(manager);
    let recovered = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert!(recovered.record(&id).unwrap().metadata_temporary.is_some());
    assert!(recovered.record(&id).unwrap().job.error.is_some());
    assert_eq!(fs::read(&record.metadata).unwrap(), TEST_METADATA);
    let temporary_path = parent.join(&temporary.name);
    assert!(original.renamed(&temporary.name).validate_existing(&temporary_path).unwrap().is_some());
    assert_eq!(fs::read(temporary_path).unwrap(), TEST_METADATA);
    assert!(read_torrent_metadata(&recovered.record(&id).unwrap()).is_err());
}

#[test]
fn metadata_recovery_refuses_corrupt_complete_temporary_and_foreign_partial() {
    for complete in [false, true] {
        let profile = Profile::new();
        let record = metadata_record(&profile, false);
        let id = record.job.id.clone();
        profile.save(vec![record.clone()]);
        let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        let result = publish_metadata(&record.metadata, TEST_METADATA, |temporary, ownership| {
            manager.update_record(&id, |record| {
                record.metadata_temporary = temporary;
                record.metadata_cleanup = ownership.clone();
            })?;
            if ownership.is_some() {
                return Err("interrupted before rename".into());
            }
            Ok(())
        });
        assert!(result.is_err());
        let saved = manager.record(&id).unwrap();
        let temporary = saved.metadata_temporary.unwrap();
        let path = record.metadata.with_file_name(&temporary.name);
        let retained = record.metadata.with_extension("retained");
        if complete {
            fs::write(&path, b"corrupt complete metadata").unwrap();
        } else {
            manager.update_record(&id, |record| record.metadata_cleanup = None).unwrap();
            fs::rename(&path, &retained).unwrap();
            fs::write(&path, b"foreign partial replacement").unwrap();
        }
        drop(manager);
        let recovered = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        assert!(!record.metadata.exists());
        assert!(recovered.record(&id).unwrap().metadata_temporary.is_some());
        assert!(recovered.record(&id).unwrap().job.error.is_some());
        if complete {
            assert_eq!(fs::read(&path).unwrap(), b"corrupt complete metadata");
        } else {
            assert_eq!(fs::read(&path).unwrap(), b"foreign partial replacement");
            assert_eq!(fs::read(retained).unwrap(), TEST_METADATA);
        }
    }
}

fn share_record(profile: &Profile, stale_root: &Path) -> (Record, PathBuf) {
    let archives = profile.0.join("archives");
    fs::create_dir_all(&archives).unwrap();
    let archive = archives.join("archive.zim");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
        &archive,
    )
    .unwrap();
    let mut book = crate::imports::load_book(archive.to_str().unwrap(), false, false).unwrap();
    book.path = archive.to_string_lossy().into_owned();
    book.available = true;
    profile
        .1
        .update(|state| {
            state.books.push(book.clone());
            Ok(())
        })
        .unwrap();
    let mut record = profile.record();
    let suffix = record.job.id.strip_prefix("torrent:").unwrap().to_owned();
    let stale = stale_root.join("archives").join("archive.zim");
    let size = fs::metadata(&archive).unwrap().len();
    record.job.shared = true;
    record.job.seeding = true;
    record.job.status = "seeding".into();
    record.job.file_name = "archive.zim".into();
    record.job.destination = stale.to_string_lossy().into_owned();
    record.job.downloaded = size;
    record.job.total = Some(size);
    record.job.torrent_path =
        Some(stale_root.join("torrents").join(format!("{suffix}.torrent")).to_string_lossy().into_owned());
    record.directory = stale.parent().unwrap().to_path_buf();
    record.staging = record.directory.join(format!(".kiwix-torrent-{suffix}"));
    record.metadata = stale_root.join("torrents").join(format!("{suffix}.torrent"));
    record.verified = true;
    record.share = Some(ShareSource {
        book_id: book.metadata.id.clone(),
        installation_id: book.installation_id.clone(),
        path: stale,
        identity: ShareIdentity::of(
            &cap_std::fs::Metadata::from_file(&File::open(&archive).unwrap()).unwrap(),
        )
        .unwrap(),
        info_hash: String::new(),
        piece_length: SHARE_PIECE_LENGTH,
    });
    (record, archive)
}

#[tokio::test(flavor = "current_thread")]
async fn a_stopped_share_cannot_restart_beside_a_live_share_of_the_same_archive() {
    let profile = Profile::new();
    let root = profile.0.clone();
    let (mut live, archive) = share_record(&profile, &root);
    live.share.as_mut().unwrap().path = archive.clone();
    let mut stopped = live.clone();
    let suffix = Uuid::new_v4().to_string();
    stopped.job.id = format!("torrent:{suffix}");
    stopped.job.seeding = false;
    stopped.job.status = "completed".into();
    stopped.paused = true;
    let (live_id, stopped_id) = (live.job.id.clone(), stopped.job.id.clone());
    profile.save(vec![live, stopped]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    // Loading without auto-resume retires shares; mark one live again without
    // contacting the engine (only intent is checked here).
    manager.update_record(&live_id, |r| {
        r.job.seeding = true;
        r.job.status = "seeding".into();
        r.paused = false;
    }).unwrap();
    let error = manager.seeding(&stopped_id, true).await.unwrap_err();
    assert!(error.contains("already being shared"), "{error}");
    let row = manager.record(&stopped_id).unwrap();
    assert!(!row.job.seeding);
    assert_eq!(row.job.status, "completed");
    assert!(!manager.inner.runtimes.lock().unwrap().contains_key(&stopped_id));
    assert!(manager.record(&live_id).unwrap().job.seeding);
}

#[tokio::test(flavor = "current_thread")]
async fn stale_share_path_is_reanchored_to_the_library_instead_of_bricking() {
    let profile = Profile::new();
    let stale_root = Path::new("/old/ios/container/Documents");
    let (record, archive) = share_record(&profile, stale_root);
    let id = record.job.id.clone();
    let suffix = id.strip_prefix("torrent:").unwrap().to_owned();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert!(manager.history_notice().is_none());
    assert!(profile.0.join("torrents/jobs.json").exists());
    let saved = manager.record(&id).unwrap();
    assert_eq!(saved.share.as_ref().unwrap().path, archive);
    assert_eq!(saved.job.destination, archive.to_string_lossy());
    assert_eq!(saved.job.file_name, "archive.zim");
    assert_eq!(saved.directory, archive.parent().unwrap());
    assert_eq!(saved.staging, archive.parent().unwrap().join(format!(".kiwix-torrent-{suffix}")));
    assert_eq!(saved.metadata, profile.0.join("torrents").join(format!("{suffix}.torrent")));
    assert_eq!(manager.list().await.len(), 1);
    assert!(manager.list().await[0].shared);
}

#[tokio::test(flavor = "current_thread")]
async fn duplicate_transfer_record_is_skipped_and_profile_still_opens() {
    let profile = Profile::new();
    let record = profile.record();
    let duplicate = record.clone();
    profile.save(vec![record, duplicate]);
    let path = profile.0.join("torrents/jobs.json");
    let original = fs::read(&path).unwrap();
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert_eq!(manager.list().await.len(), 1);
    assert!(manager.history_notice().is_none());
    assert!(manager
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("saved transfer")));
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(manager);
    let restarted = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert!(restarted.history_notice().is_none());
    assert_eq!(restarted.list().await.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_job_envelopes_preserve_the_original_history() {
    for document in [r#"{"version":4294967297,"jobs":[]}"#, r#"{"version":1}"#, r#"{"version":1,"jobs":{}}"#] {
        let profile = Profile::new();
        let path = profile.0.join("torrents/jobs.json");
        fs::write(&path, document).unwrap();
        let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        let kept = manager.history_notice().expect("unreadable history has a recovery file");
        assert_eq!(fs::read_to_string(kept).unwrap(), document);
        assert!(manager.list().await.is_empty());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unreadable_transfer_record_is_skipped_without_quarantining_history() {
    let profile = Profile::new();
    let record = profile.record();
    profile.save(vec![record.clone()]);
    let path = profile.0.join("torrents/jobs.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["jobs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"job": {"id": "torrent:not-a-uuid"}}));
    fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert_eq!(manager.list().await.len(), 1);
    assert!(manager.history_notice().is_none());
    assert!(manager
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("saved transfer")));
    assert!(path.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn terminal_history_removal_joins_retirement_and_preserves_payload() {
    let profile = Profile::new();
    let mut record = profile.record();
    record.job.status = "error".into();
    let id = record.job.id.clone();
    crate::fs_ext::create_private_dir_all(&record.staging).unwrap();
    let partial = record.staging.join("archive.zim");
    let archive = profile.0.join("archive.zim");
    fs::write(&partial, b"retained staging").unwrap();
    fs::write(&archive, b"published archive").unwrap();
    fs::write(&record.metadata, b"retained metadata").unwrap();
    let metadata = record.metadata.clone();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let token = manager.inner.stop.child_token();
    let (retirement, retired) = tokio::sync::oneshot::channel();
    manager.inner.runtimes.lock().unwrap().insert(id.clone(), Runtime {
        generation: Uuid::new_v4(), token: token.clone(), retired: Some(retired), handle: None,
    });
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    let owner = manager.clone();
    let worker_id = id.clone();
    let worker = tokio::spawn(async move {
        token.cancelled().await;
        // A terminal record cannot disappear while its worker still owns it.
        assert!(owner.record(&worker_id).is_ok());
        let _gate = owner.inner.gate.lock().await;
        owner.inner.runtimes.lock().unwrap().remove(&worker_id);
        worker_finished.store(true, Ordering::SeqCst);
        retirement.send(Ok(())).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), manager.remove_terminal(&id)).await.unwrap().unwrap();
    assert!(finished.load(Ordering::SeqCst));
    worker.await.unwrap();
    assert!(manager.list().await.is_empty());
    assert_eq!(fs::read(&partial).unwrap(), b"retained staging");
    assert_eq!(fs::read(&archive).unwrap(), b"published archive");
    assert_eq!(fs::read(&metadata).unwrap(), b"retained metadata");
    assert!(manager.engine_cell().unwrap().get().is_none());
    drop(manager);
    let restored = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert!(restored.list().await.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn an_errored_seed_can_still_be_removed_or_forgotten() {
    // `apply_worker_error` keeps the seed intent of a failed share/seed so
    // Retry can restart it; that flag must not make the row unremovable.
    for (verified, forget) in [(true, false), (false, false), (true, true)] {
        let profile = Profile::new();
        let mut record = profile.record();
        record.job.status = "error".into();
        record.job.error = Some("The shared file changed".into());
        record.job.seeding = true;
        record.verified = verified;
        let id = record.job.id.clone();
        profile.save(vec![record]);
        let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        // A refused removal (its worker is still stopping) must not clear the
        // seed intent: Retry of a verified row with no intent is a no-op.
        let token = manager.inner.stop.child_token();
        manager.inner.runtimes.lock().unwrap().insert(id.clone(), Runtime {
            generation: Uuid::new_v4(), token, retired: None, handle: None,
        });
        assert!(manager.remove_terminal(&id).await.is_err());
        assert!(manager.record(&id).unwrap().job.seeding, "a refused removal cleared the seed intent");
        manager.inner.runtimes.lock().unwrap().remove(&id);
        if forget {
            manager.forget_terminal(&id).await.unwrap();
        } else {
            manager.remove_terminal(&id).await.unwrap();
        }
        assert!(manager.list().await.is_empty(), "verified={verified} forget={forget}");
        drop(manager);
        let restored = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        assert!(restored.list().await.is_empty());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn terminal_actions_reject_incomplete_torrents_without_cancelling() {
    for status in ["queued", "downloading", "paused"] {
        let profile = Profile::new();
        let mut record = profile.record();
        record.job.status = status.into();
        record.paused = true;
        record.job.seeding = status == "seeding";
        record.verified = status == "seeding";
        let id = record.job.id.clone();
        profile.save(vec![record]);
        let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        let token = manager.inner.stop.child_token();
        let (_retirement, retired) = tokio::sync::oneshot::channel();
        manager.inner.runtimes.lock().unwrap().insert(id.clone(), Runtime {
            generation: Uuid::new_v4(), token: token.clone(), retired: Some(retired), handle: None,
        });
        assert!(manager.remove_terminal(&id).await.is_err(), "{status}");
        assert!(manager.delete_terminal(&id).await.is_err(), "{status}");
        assert!(!token.is_cancelled(), "{status}");
        assert!(manager.record(&id).is_ok());
        assert!(manager.engine_cell().unwrap().get().is_none());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn failed_history_checkpoint_retains_torrent_record_and_files() {
    let profile = Profile::new();
    let mut record = profile.record();
    record.job.status = "cancelled".into();
    let id = record.job.id.clone();
    crate::fs_ext::create_private_dir_all(&record.staging).unwrap();
    let payload = record.staging.join("archive.zim");
    fs::write(&payload, b"retained staging").unwrap();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let state = profile.0.join("torrents/jobs.json");
    fs::rename(&state, state.with_extension("retained")).unwrap();
    fs::create_dir(&state).unwrap();
    assert!(manager.remove_terminal(&id).await.is_err());
    assert_eq!(manager.record(&id).unwrap().job.status, "cancelled");
    assert_eq!(fs::read(payload).unwrap(), b"retained staging");
}

fn owned_terminal_fixture() -> (Profile, Arc<TorrentManager>, String) {
    let profile = Profile::new();
    let mut record = profile.record();
    record.job.status = "error".into();
    record.files = vec![PathBuf::from("archive.zim")];
    crate::fs_ext::create_private_dir_all(&record.staging).unwrap();
    let root = open_staging(&record.staging).unwrap();
    record.staging_directories.push((PathBuf::new(), PublicationIdentity::of(&root.dir_metadata().unwrap())));
    let partial = record.staging.join("archive.zim");
    fs::write(&partial, TEST_ARCHIVE).unwrap();
    record.staging_cleanup.push((PathBuf::from("archive.zim"),
        CleanupEntry::retain(&partial, &File::open(&partial).unwrap()).unwrap()));
    fs::write(&record.metadata, b"owned torrent metadata").unwrap();
    record.metadata_cleanup = Some(CleanupEntry::retain(&record.metadata, &File::open(&record.metadata).unwrap()).unwrap());
    let id = record.job.id.clone();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    (profile, manager, id)
}

fn retained_torrent_import(path: &Path) -> crate::archive_source::RetainedSource {
    let parent = path.parent().unwrap();
    let file = File::open(path).unwrap();
    let stamp = cap_std::fs::Metadata::from_file(&file).unwrap();
    crate::archive_source::RetainedSource::Filesystem(crate::library_files::PinnedArchive {
        path: parent.to_path_buf(), directory: crate::storage::open_directory_nofollow(parent).unwrap(),
        files: vec![(path.file_name().unwrap().to_str().unwrap().to_owned(), file)], stamps: vec![stamp],
    })
}

#[test]
fn import_authority_retains_each_completed_torrent_publication_without_engine_work() {
    let (profile, manager, id) = owned_terminal_fixture();
    publish_fixture(&manager, &id, false, false).unwrap();
    let path = profile.0.join("archive.zim");
    let destination = path.to_str().unwrap();
    let proof = manager.import_publication(&id, destination).unwrap();
    let source = retained_torrent_import(&path);
    proof.verify(&source, || {
        assert!(manager.inner.state.try_lock().is_ok(), "payload hashing must not hold the manager lock");
        false
    }).unwrap();
    manager.with_import_publication(&id, destination, &source, &proof, || {
        assert!(manager.inner.state.try_lock().is_err(), "ownership must be fenced through registration");
        Ok(())
    }).unwrap();
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(!manager.record(&id).unwrap().verified, "a completed member does not need the entire torrent to finish");
}

#[test]
fn torrent_import_rejects_identical_replacement_and_same_inode_changed_bytes() {
    for replacement in [true, false] {
        let (profile, manager, id) = owned_terminal_fixture();
        publish_fixture(&manager, &id, false, false).unwrap();
        let path = profile.0.join("archive.zim");
        let proof = manager.import_publication(&id, path.to_str().unwrap()).unwrap();
        let bytes = if replacement {
            fs::rename(&path, profile.0.join("retained-original.zim")).unwrap();
            TEST_ARCHIVE.to_vec()
        } else {
            let mut bytes = TEST_ARCHIVE.to_vec();
            *bytes.last_mut().unwrap() ^= 1;
            bytes
        };
        fs::write(&path, &bytes).unwrap();
        let source = retained_torrent_import(&path);
        assert!(proof.verify(&source, || false).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes, "foreign bytes must not be modified");
    }
}

#[test]
fn torrent_import_refuses_missing_or_retired_publication_authority() {
    let (profile, manager, id) = owned_terminal_fixture();
    publish_fixture(&manager, &id, false, false).unwrap();
    let path = profile.0.join("archive.zim");
    let destination = path.to_str().unwrap();
    let proof = manager.import_publication(&id, destination).unwrap();
    let source = retained_torrent_import(&path);
    proof.verify(&source, || false).unwrap();
    manager.update_record(&id, |record| record.history_dismissed = true).unwrap();
    let committed = std::cell::Cell::new(false);
    assert!(manager.with_import_publication(&id, destination, &source, &proof, || {
        committed.set(true); Ok(())
    }).is_err());
    assert!(!committed.get());
    manager.update_record(&id, |record| { record.history_dismissed = false; record.publication_intents.clear(); }).unwrap();
    assert!(manager.import_publication(&id, destination).is_err());
    assert_eq!(fs::read(&path).unwrap(), TEST_ARCHIVE);
}

#[test]
fn publication_digest_verification_observes_cancellation_between_reads() {
    let profile = Profile::new();
    let path = profile.0.join("owned.zim");
    let bytes = vec![7u8; 512 * 1024];
    fs::write(&path, &bytes).unwrap();
    let file = File::open(&path).unwrap();
    let proof = CleanupEntry::retain(&path, &file).unwrap()
        .with_publication(bytes.len() as u64, Some(Sha256::digest(&bytes).into()));
    let checks = std::cell::Cell::new(0);
    assert!(proof.matches_retained_controlled(&path, &file, || {
        checks.set(checks.get() + 1); checks.get() >= 4
    }).is_err());
    assert_eq!(checks.get(), 4);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_torrent_cleanup_removes_owned_partial_metadata_and_published_archive() {
    let (profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    publish_fixture(&manager, &id, false, false).unwrap();
    let archive = profile.0.join("archive.zim");
    assert_eq!(fs::read(&archive).unwrap(), TEST_ARCHIVE);
    assert_eq!(manager.terminal_owned_publications(&id).await.unwrap().len(), 1);
    manager.update_record(&id, |record| {
        record.verified = true;
        record.job.status = "completed".into();
        record.job.seeding = true;
    }).unwrap();
    manager.delete_terminal(&id).await.unwrap();
    assert!(!archive.exists());
    assert!(!record.staging.exists());
    assert!(!record.metadata.exists());
    assert_eq!(manager.list().await.len(), 1);
    manager.remove_terminal(&id).await.unwrap();
    assert!(manager.record(&id).is_err());
    assert!(manager.list().await.is_empty());
    assert!(manager.engine_cell().unwrap().get().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn dismissed_torrent_keeps_durable_ownership_and_can_be_cleaned_after_restart() {
    let (profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    manager.remove_terminal(&id).await.unwrap();
    assert!(manager.list().await.is_empty());
    assert_eq!(manager.retained_jobs().await.len(), 1);
    assert!(manager.claims_monitored_path(&record.directory.join("archive.zim")).unwrap());
    assert_eq!(fs::read(record.staging.join("archive.zim")).unwrap(), TEST_ARCHIVE);
    drop(manager);
    let restored = TorrentManager::new(profile.0.clone(), true, profile.1.clone()).unwrap();
    assert!(restored.list().await.is_empty());
    assert_eq!(restored.retained_jobs().await.len(), 1);
    assert!(!can_restore_record(&restored.record(&id).unwrap()));
    assert!(restored.engine_cell().unwrap().get().is_none());
    restored.delete_terminal(&id).await.unwrap();
    assert!(!record.staging.exists());
    assert!(!record.metadata.exists());
    restored.remove_terminal(&id).await.unwrap();
    assert!(restored.retained_jobs().await.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn keeping_completed_seed_files_retires_seed_and_keeps_hidden_ownership() {
    let (_profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    manager.update_record(&id, |record| {
        record.verified = true;
        record.job.status = "seeding".into();
        record.job.seeding = true;
    }).unwrap();
    manager.remove_terminal(&id).await.unwrap();
    assert!(!manager.record(&id).unwrap().job.seeding);
    assert_eq!(manager.retained_jobs().await.len(), 1);
    assert_eq!(fs::read(record.staging.join("archive.zim")).unwrap(), TEST_ARCHIVE);
    assert!(record.metadata.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn torrent_factory_journals_new_nested_payload_before_cleanup() {
    let (_profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    let relative = PathBuf::from("nested/second.bin");
    let sibling = PathBuf::from("nested/third.bin");
    manager.update_record(&id, |record| { record.files.push(relative.clone()); record.files.push(sibling.clone()); }).unwrap();
    let factory = ConfinedFactory {
        root: Arc::new(open_staging(&record.staging).unwrap()),
        path: record.staging.clone(),
        expected: Arc::new(HashMap::from([(relative.clone(), 5), (sibling.clone(), 5)])),
        sources: Arc::new(Mutex::new(HashMap::new())),
        existing: true,
        stop: CancellationToken::new(),
        gate: manager.storage_gate(&id).unwrap(),
        dirs: Arc::new(Mutex::new(HashSet::new())),
        owner: Some(((*manager).clone(), id.clone())),
    };
    let file = factory.open_file(&relative, 5).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(record.staging.join("nested")).unwrap().permissions().mode() & 0o777, 0o700);
    }
    let mut writer = file.as_ref();
    writer.write_all(b"bytes").unwrap();
    file.sync_all().unwrap();
    // Fixture journal: the staging root plus the new `nested` directory.
    let journal = manager.record(&id).unwrap();
    assert_eq!(journal.staging_cleanup.len(), 2);
    assert_eq!(journal.staging_directories.len(), 2);
    assert!(journal.staging_directories.iter().any(|(path, _)| path.as_path() == Path::new("nested")));
    // The second file reuses the directory the first one created and
    // journaled: no second directory entry, one more file entry.
    let second = factory.open_file(&sibling, 5).unwrap();
    let mut writer = second.as_ref();
    writer.write_all(b"other").unwrap();
    second.sync_all().unwrap();
    let journal = manager.record(&id).unwrap();
    assert_eq!(journal.staging_cleanup.len(), 3);
    assert_eq!(journal.staging_directories.len(), 2);
    drop(second);
    drop(file);
    drop(factory);
    manager.delete_terminal(&id).await.unwrap();
    assert!(!record.staging.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn an_unjournaled_new_staging_entry_is_removed_so_cleanup_stays_possible() {
    // Creating a payload file/directory and journaling it are two steps. When
    // the journal write fails (full disk), the just-created empty entry must
    // not survive: the next attempt would adopt it without a ledger entry and
    // deletion would refuse it as foreign forever.
    let (profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    let nested = PathBuf::from("nested/second.bin");
    let flat = PathBuf::from("third.bin");
    manager.update_record(&id, |record| { record.files.push(nested.clone()); record.files.push(flat.clone()); }).unwrap();
    let factory = ConfinedFactory {
        root: Arc::new(open_staging(&record.staging).unwrap()),
        path: record.staging.clone(),
        expected: Arc::new(HashMap::from([(nested.clone(), 5), (flat.clone(), 5)])),
        sources: Arc::new(Mutex::new(HashMap::new())),
        existing: true,
        stop: CancellationToken::new(),
        gate: manager.storage_gate(&id).unwrap(),
        dirs: Arc::new(Mutex::new(HashSet::new())),
        owner: Some(((*manager).clone(), id.clone())),
    };
    let state = profile.0.join("torrents/jobs.json");
    let saved = state.with_extension("retained");
    fs::rename(&state, &saved).unwrap();
    fs::create_dir(&state).unwrap();
    assert!(factory.open_file(&nested, 5).is_err());
    assert!(factory.open_file(&flat, 5).is_err());
    assert!(!record.staging.join("nested").exists(), "unjournaled directory survived");
    assert!(!record.staging.join(&flat).exists(), "unjournaled file survived");
    fs::remove_dir(&state).unwrap();
    fs::rename(&saved, &state).unwrap();
    drop(factory);
    manager.delete_terminal(&id).await.unwrap();
    assert!(!record.staging.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn torrent_cleanup_preserves_replaced_partial_and_unrecorded_staging_files() {
    for replacement in [true, false] {
        let (_profile, manager, id) = owned_terminal_fixture();
        let record = manager.record(&id).unwrap();
        let partial = record.staging.join("archive.zim");
        let sentinel = record.staging.join("user-file");
        if replacement {
            fs::rename(&partial, &sentinel).unwrap();
            fs::write(&partial, b"replacement inode").unwrap();
        } else {
            fs::write(&sentinel, b"unrecorded user bytes").unwrap();
        }
        let before = fs::read(&sentinel).unwrap();
        assert!(manager.delete_terminal(&id).await.is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), before);
        if replacement { assert_eq!(fs::read(&partial).unwrap(), b"replacement inode"); }
        assert!(record.metadata.exists());
        assert_eq!(manager.list().await.len(), 1);
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn torrent_cleanup_never_follows_replaced_partial_symlink() {
    let (profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    let partial = record.staging.join("archive.zim");
    let target = profile.0.join("user-file");
    fs::write(&target, b"user bytes").unwrap();
    fs::remove_file(&partial).unwrap();
    std::os::unix::fs::symlink(&target, &partial).unwrap();
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"user bytes");
    assert!(fs::symlink_metadata(&partial).unwrap().file_type().is_symlink());
    assert_eq!(manager.list().await.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn torrent_cleanup_requires_original_publication_inode_and_library_release() {
    for replace in [true, false] {
        let (profile, manager, id) = owned_terminal_fixture();
        publish_fixture(&manager, &id, false, false).unwrap();
        let archive = profile.0.join("archive.zim");
        if replace {
            fs::rename(&archive, profile.0.join("original-publication")).unwrap();
            fs::write(&archive, TEST_ARCHIVE).unwrap();
        } else {
            let book = destination_book(&archive);
            profile.1.update(|state| { state.books.push(book); Ok(()) }).unwrap();
        }
        if replace { assert!(manager.terminal_owned_publications(&id).await.is_err()); }
        else { assert_eq!(manager.terminal_owned_publications(&id).await.unwrap().len(), 1); }
        assert!(manager.delete_terminal(&id).await.is_err());
        assert_eq!(fs::read(&archive).unwrap(), TEST_ARCHIVE);
        assert_eq!(manager.list().await.len(), 1);
        if !replace {
            fs::remove_file(&archive).unwrap();
            assert!(manager.delete_terminal(&id).await.is_err());
            profile.1.update(|state| { state.books.clear(); Ok(()) }).unwrap();
            manager.delete_terminal(&id).await.unwrap();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn torrent_cleanup_checkpoint_and_retirement_failures_preserve_payload_and_history() {
    let (profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    let partial = record.staging.join("archive.zim");
    let state = profile.0.join("torrents/jobs.json");
    fs::rename(&state, state.with_extension("retained")).unwrap();
    fs::create_dir(&state).unwrap();
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&partial).unwrap(), TEST_ARCHIVE);
    assert_eq!(manager.list().await.len(), 1);
    fs::remove_dir(&state).unwrap();
    manager.inner.retirement_failures.lock().unwrap().insert(id.clone(), "fixture engine retirement failure".into());
    assert!(manager.delete_terminal(&id).await.is_err());
    assert_eq!(fs::read(&partial).unwrap(), TEST_ARCHIVE);
    assert_eq!(manager.list().await.len(), 1);
    manager.inner.retirement_failures.lock().unwrap().clear();
    manager.delete_terminal(&id).await.unwrap();
    assert!(!record.staging.exists());
}

#[test]
fn owned_file_cleanup_retries_failed_capture_checkpoint_before_mutating_payload() {
    let profile = Profile::new();
    let path = profile.0.join("owned-part");
    fs::write(&path, b"owned bytes").unwrap();
    let mut entry = CleanupEntry::retain(&path, &File::open(&path).unwrap()).unwrap();
    for _ in 0..2 {
        assert!(delete_owned_file(&path, &mut entry, |_| Err("fixture checkpoint failure".into())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"owned bytes");
        assert!(!entry.cleanup_started());
        assert!(!fs::read_dir(&profile.0).unwrap().any(|entry| {
            entry.unwrap().file_name().to_string_lossy().starts_with(".kiwix-publish-cleanup-")
        }));
    }
    let mut persisted = false;
    delete_owned_file(&path, &mut entry, |entry| {
        assert!(entry.capture.is_some());
        persisted = true;
        Ok(())
    }).unwrap();
    assert!(persisted);
    assert!(!path.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn legacy_staging_or_metadata_blocks_before_any_published_archive_is_removed() {
    for legacy_staging in [true, false] {
        let (profile, manager, id) = owned_terminal_fixture();
        publish_fixture(&manager, &id, false, false).unwrap();
        let before = manager.record(&id).unwrap();
        manager.update_record(&id, |record| {
            if legacy_staging { record.staging_directories.clear(); }
            else { record.metadata_cleanup = None; }
        }).unwrap();
        assert!(manager.terminal_owned_publications(&id).await.is_err());
        assert!(manager.delete_terminal(&id).await.is_err());
        assert_eq!(fs::read(profile.0.join("archive.zim")).unwrap(), TEST_ARCHIVE);
        assert_eq!(fs::read(before.staging.join("archive.zim")).unwrap(), TEST_ARCHIVE);
        assert!(before.metadata.exists());
        assert_eq!(manager.list().await.len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn completed_seed_preflight_is_readonly_and_does_not_cancel_its_worker() {
    let (_profile, manager, id) = owned_terminal_fixture();
    publish_fixture(&manager, &id, false, false).unwrap();
    manager.update_record(&id, |record| {
        record.verified = true;
        record.job.seeding = true;
        record.job.status = "seeding".into();
    }).unwrap();
    let token = manager.inner.stop.child_token();
    let (_retirement, retired) = tokio::sync::oneshot::channel();
    manager.inner.runtimes.lock().unwrap().insert(id.clone(), Runtime {
        generation: Uuid::new_v4(), token: token.clone(), retired: Some(retired), handle: None,
    });
    let before = fs::read(manager.inner.folder.join("jobs.json")).unwrap();
    assert_eq!(manager.terminal_owned_publications(&id).await.unwrap().len(), 1);
    assert!(!token.is_cancelled());
    assert!(manager.record(&id).unwrap().job.seeding);
    assert!(manager.inner.runtimes.lock().unwrap().contains_key(&id));
    assert_eq!(fs::read(manager.inner.folder.join("jobs.json")).unwrap(), before);
}

#[test]
fn proof_snapshot_identity_ignores_telemetry_but_rejects_changed_ownership() {
    let (_profile, manager, id) = owned_terminal_fixture();
    publish_fixture(&manager, &id, false, false).unwrap();
    let snapshot = manager.record(&id).unwrap();
    let mut current = snapshot.clone();
    current.job.uploaded += 1024;
    current.job.upload_speed = 42.0;
    current.job.speed = 17.0;
    assert!(snapshot.same_cleanup_identity(&current));
    current.job.seeding = !snapshot.job.seeding;
    assert!(!snapshot.same_cleanup_identity(&current));
    current = snapshot.clone();
    current.publication_intents[0].sha256[0] ^= 1;
    assert!(!snapshot.same_cleanup_identity(&current));
    current = snapshot.clone();
    current.metadata_cleanup = None;
    assert!(!snapshot.same_cleanup_identity(&current));
    current = snapshot.clone();
    current.staging_cleanup.clear();
    assert!(!snapshot.same_cleanup_identity(&current));
}

#[test]
fn staged_publication_proof_checks_original_inode_and_same_inode_content_changes() {
    let profile = Profile::new();
    let original = profile.0.join("owned.zim");
    fs::write(&original, TEST_ARCHIVE).unwrap();
    let proof = CleanupEntry::retain(&original, &File::open(&original).unwrap()).unwrap()
        .with_publication(TEST_ARCHIVE.len() as u64, Some(Sha256::digest(TEST_ARCHIVE).into()));
    let staged = profile.0.join("private-staged-file");
    fs::rename(&original, &staged).unwrap();
    let retained = File::open(&staged).unwrap();
    assert!(proof.matches_retained(&original, &retained).unwrap(), "original child is deliberately absent");
    fs::write(&original, TEST_ARCHIVE).unwrap();
    assert!(!proof.matches_retained(&original, &File::open(&original).unwrap()).unwrap());
    assert!(proof.matches_retained(&original, &retained).unwrap());
    fs::write(&staged, vec![b'x'; TEST_ARCHIVE.len()]).unwrap();
    assert!(!proof.matches_retained(&original, &retained).unwrap(), "same inode and length are insufficient");
}

#[tokio::test(flavor = "current_thread")]
async fn torrent_cleanup_joins_completed_seed_worker_before_payload_mutation() {
    let (_profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    manager.update_record(&id, |record| {
        record.verified = true;
        record.job.status = "seeding".into();
        record.job.seeding = true;
    }).unwrap();
    let token = manager.inner.stop.child_token();
    let (retirement, retired) = tokio::sync::oneshot::channel();
    manager.inner.runtimes.lock().unwrap().insert(id.clone(), Runtime {
        generation: Uuid::new_v4(), token: token.clone(), retired: Some(retired), handle: None,
    });
    let owner = manager.clone();
    let worker_id = id.clone();
    let partial = record.staging.join("archive.zim");
    let worker = tokio::spawn(async move {
        token.cancelled().await;
        assert_eq!(fs::read(&partial).unwrap(), TEST_ARCHIVE);
        assert!(owner.record(&worker_id).unwrap().job.seeding,
            "the durable seed intent remains until retirement joins");
        let _gate = owner.inner.gate.lock().await;
        owner.inner.runtimes.lock().unwrap().remove(&worker_id);
        retirement.send(Ok(())).unwrap();
    });
    manager.delete_terminal(&id).await.unwrap();
    worker.await.unwrap();
    assert!(!manager.record(&id).unwrap().job.seeding);
    assert!(!record.staging.exists());
    assert_eq!(manager.list().await.len(), 1);
}

#[test]
fn capacity_counts_retained_staging_and_publication_copies_together() {
    let profile = Profile::new();
    let mut record = profile.record();
    record.files = vec![PathBuf::from("archive.zim"), PathBuf::from("notes.txt")];
    record.space_files = vec![
        SpaceFile { path: record.files[0].clone(), size: 100 },
        SpaceFile { path: record.files[1].clone(), size: 20 },
    ];
    record.job.total = Some(120);
    record.job.downloaded = 30;
    assert_eq!(
        space_requirements(&record).unwrap(),
        vec![(record.directory.clone(), 190, "staging-and-publication")]
    );
    record.job.downloaded = 120;
    assert_eq!(space_requirements(&record).unwrap()[0].1, 100);
}

#[test]
fn unknown_capacity_is_not_a_zero_byte_estimate() {
    let profile = Profile::new();
    let mut record = profile.record();
    record.job.total = None;
    assert!(space_requirements(&record).unwrap().is_empty());
}

#[test]
fn automatic_restoration_discards_prior_space_confirmation() {
    let profile = Profile::new();
    let mut record = profile.record();
    record.space_acknowledgements.push("previous-confirmation".into());
    assert!(restore_record(&mut record, true));
    assert!(record.space_acknowledgements.is_empty());
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn manual_stops_while_suspended_are_durable_without_reopening_admission() {
    let profile = Profile::new();
    let paused = profile.record();
    let cancelled = profile.record();
    let paused_id = paused.job.id.clone();
    let cancelled_id = cancelled.job.id.clone();
    profile.save(vec![paused, cancelled]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    manager.suspend().await.unwrap();
    manager.pause(&paused_id).await.unwrap();
    manager.cancel(&cancelled_id).await.unwrap();
    assert!(manager.inner.suspended.load(Ordering::SeqCst));
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(!manager.has_interrupted().unwrap());
    assert!(manager.resume(&paused_id, None).await.is_err());
    let state: State = serde_json::from_slice(
        &fs::read(profile.0.join("torrents/jobs.json")).unwrap()
    ).unwrap();
    assert_eq!(state.jobs[0].job.status, "paused");
    assert!(state.jobs[0].paused);
    assert!(!state.jobs[0].interrupted);
    assert_eq!(state.jobs[1].job.status, "cancelled");
    assert!(state.jobs[1].paused);
    assert!(!state.jobs[1].interrupted);
}

#[test]
fn inner_policy_rejection_is_recoverable_and_never_overrides_manual_stop() {
    let profile = Profile::new();
    let mut record = profile.record();
    apply_worker_error(&mut record, "MOBILE_NETWORK_BLOCKED".into(), false);
    assert_eq!(record.job.status, "paused");
    assert!(record.interrupted);
    assert!(!record.paused);
    assert!(record.job.error.is_none());
    assert!(restore_record(&mut record, true));
    for status in ["paused", "cancelled"] {
        let mut manual = profile.record();
        manual.paused = true;
        manual.job.status = status.into();
        apply_worker_error(&mut manual, "MOBILE_NETWORK_BLOCKED".into(), true);
        assert_eq!(manual.job.status, status);
        assert!(!manual.interrupted);
        assert!(!can_restore_record(&manual));
    }
    let mut failure = profile.record();
    apply_worker_error(&mut failure, "Invalid torrent".into(), false);
    assert_eq!(failure.job.status, "error");
    assert!(!can_restore_record(&failure));
}

#[test]
fn disabled_restoration_needs_no_runtime_and_does_not_rewrite_state() {
    let profile = Profile::new();
    profile.save(vec![profile.record()]);
    let path = profile.0.join("torrents/jobs.json");
    let before = fs::read(&path).unwrap();
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let state = manager.inner.state.lock().unwrap();
    let jobs = &state.jobs;
    assert_eq!(jobs[0].job.status, "paused");
    assert_eq!(jobs[0].job.speed, 0.0);
    assert_eq!(jobs[0].job.upload_speed, 0.0);
    assert_eq!(jobs[0].job.eta_seconds, None);
    assert!(!jobs[0].paused);
    assert!(jobs[0].interrupted);
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(manager.inner.runtimes.lock().unwrap().is_empty());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn policy_hold_survives_serialization_without_becoming_manual_pause() {
    let profile = Profile::new();
    let mut interrupted = profile.record();
    assert!(!restore_record(&mut interrupted, false));
    let bytes = serde_json::to_vec(&interrupted).unwrap();
    let mut interrupted: Record = serde_json::from_slice(&bytes).unwrap();
    assert!(restore_record(&mut interrupted, true));
    assert_eq!(interrupted.job.status, "queued");
    assert!(!interrupted.interrupted);
    let mut manual = profile.record();
    manual.paused = true;
    manual.job.status = "paused".into();
    assert!(!restore_record(&mut manual, true));
    assert_eq!(manual.job.status, "paused");
    let mut legacy = serde_json::to_value(profile.record()).unwrap();
    legacy.as_object_mut().unwrap().remove("interrupted");
    let legacy: Record = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.interrupted);
}

#[test]
fn terminal_jobs_do_not_restart_and_manual_resume_clears_both_pause_reasons() {
    let profile = Profile::new();
    for status in ["cancelled", "error"] {
        let mut record = profile.record();
        record.job.status = status.into();
        assert!(!restore_record(&mut record, true));
        assert_eq!(record.job.status, status);
    }
    let mut record = profile.record();
    record.verified = true;
    record.job.status = "completed".into();
    assert!(!restore_record(&mut record, true));
    record.job.seeding = true;
    assert!(!restore_record(&mut record, false));
    assert!(record.interrupted);
    assert!(restore_record(&mut record, true));
    record.paused = true;
    record.interrupted = true;
    resume_record(&mut record);
    assert!(!record.paused);
    assert!(!record.interrupted);
    assert_eq!(record.job.status, "seeding");
}

#[test]
fn interrupted_query_is_pure_and_matches_restore_eligibility() {
    for (paused, verified, seeding, status, expected) in [
        (false, false, false, "downloading", true),
        (true, false, false, "paused", false),
        (false, false, false, "error", false),
        (false, false, false, "cancelled", false),
        (false, true, false, "completed", false),
        (false, true, true, "completed", true),
        (true, true, true, "completed", false),
    ] {
        let profile = Profile::new();
        let mut record = profile.record();
        record.paused = paused;
        record.verified = verified;
        record.job.seeding = seeding;
        record.job.status = status.into();
        profile.save(vec![record]);
        let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
        let before = fs::read(profile.0.join("torrents/jobs.json")).unwrap();
        assert_eq!(manager.has_interrupted().unwrap(), expected, "{status}");
        assert!(manager.engine_cell().unwrap().get().is_none());
        assert!(manager.inner.tasks.lock().unwrap().is_empty());
        assert_eq!(fs::read(profile.0.join("torrents/jobs.json")).unwrap(), before);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn lifecycle_checkpoint_is_reversible_and_preserves_pause_reasons() {
    let profile = Profile::new();
    let active = profile.record();
    let mut manual = profile.record();
    manual.paused = true;
    manual.job.status = "paused".into();
    let mut error = profile.record();
    error.job.status = "error".into();
    error.job.error = Some("retained failure".into());
    let mut cancelled = profile.record();
    cancelled.job.status = "cancelled".into();
    let mut completed = profile.record();
    completed.verified = true;
    completed.job.status = "completed".into();
    let mut seeding = completed.clone();
    let suffix = Uuid::new_v4().to_string();
    seeding.job.id = format!("torrent:{suffix}");
    seeding.metadata = profile.0.join("torrents").join(format!("{suffix}.torrent"));
    seeding.staging = profile.0.join(format!(".kiwix-torrent-{suffix}"));
    seeding.job.seeding = true;
    let active_id = active.job.id.clone();
    let seed_id = seeding.job.id.clone();
    profile.save(vec![active, manual, error, cancelled, completed, seeding]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    manager.resume(&active_id, None).await.unwrap();
    manager.suspend().await.unwrap();
    assert!(manager.inner.runtimes.lock().unwrap().is_empty());
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.resume(&active_id, None).await.is_err());
    assert!(manager.start(
        "https://example.invalid/held.torrent".into(), profile.0.clone(), None
    ).await.is_err());
    assert_eq!(manager.list().await.len(), 6);
    let held = manager.inner.state.lock().unwrap().jobs.clone();
    assert!(held[0].interrupted);
    assert!(!held[0].paused);
    assert!(held[1].paused);
    assert!(!held[1].interrupted);
    assert_eq!(held[2].job.error.as_deref(), Some("retained failure"));
    assert_eq!(held[3].job.status, "cancelled");
    assert_eq!(held[4].job.status, "completed");
    assert!(held[5].job.seeding);
    assert!(held[5].interrupted);
    assert!(manager.has_interrupted().unwrap());
    manager.resume_interrupted().await.unwrap();
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(!manager.inner.suspended.load(Ordering::SeqCst));
    profile.1.update(|state| {
        state.settings.auto_resume_downloads = true;
        Ok(())
    }).unwrap();
    manager.suspend().await.unwrap();
    manager.resume_admission().await.unwrap();
    assert!(!manager.inner.suspended.load(Ordering::SeqCst));
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.record(&active_id).unwrap().interrupted);
    manager.resume_interrupted().await.unwrap();
    assert_eq!(manager.record(&active_id).unwrap().job.status, "queued");
    assert_eq!(manager.record(&seed_id).unwrap().job.status, "queued");
    assert!(!manager.has_interrupted().unwrap());
    let restored = manager.inner.state.lock().unwrap().jobs.clone();
    assert!(restored[1].paused);
    assert_eq!(restored[2].job.status, "error");
    assert_eq!(restored[3].job.status, "cancelled");
    assert_eq!(restored[4].job.status, "completed");
    // Cancel before this current-thread runtime yields to queued workers:
    // exercise lifecycle admission without introducing a network fixture.
    manager.suspend().await.unwrap();
    manager.shutdown().await.unwrap();
    assert!(manager.resume_interrupted().await.is_err());
    assert!(manager.resume_admission().await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn reconnect_resume_skips_denials_and_only_restores_interrupted_jobs() {
    use crate::transfer_policy::{
        reconnect_resume, InternetAdmission, ResumeAdmission, ResumeSkip,
    };
    let profile = Profile::new();
    let mut active = profile.record();
    active.paused = false;
    active.interrupted = true;
    active.job.status = "paused".into();
    let active_id = active.job.id.clone();
    let mut manual = profile.record();
    manual.paused = true;
    manual.interrupted = false;
    manual.job.status = "paused".into();
    let manual_id = manual.job.id.clone();
    profile.save(vec![active, manual]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();

    // Metered/cellular denial and read-only mode must not open an engine.
    for (network, reason) in [
        (InternetAdmission::MeteredDenied, ResumeSkip::MeteredDenied),
        (InternetAdmission::Offline, ResumeSkip::Offline),
    ] {
        profile.1.update(|state| {
            state.settings.auto_resume_downloads = true;
            Ok(())
        }).unwrap();
        let decision = reconnect_resume(&profile.1.settings().unwrap(), network, true, 3, 2);
        assert_eq!(decision, ResumeAdmission::Skip(reason));
        assert_eq!(
            manager.resume_interrupted_after_reconnect(decision).await.unwrap(),
            Some(reason)
        );
        assert!(manager.inner.tasks.lock().unwrap().is_empty());
        assert!(manager.record(&active_id).unwrap().interrupted);
        assert_eq!(manager.record(&active_id).unwrap().job.status, "paused");
    }
    profile.1.update(|state| {
        state.settings.read_only = true;
        Ok(())
    }).unwrap();
    let read_only = reconnect_resume(
        &profile.1.settings().unwrap(),
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
    assert!(manager.record(&active_id).unwrap().interrupted);

    // Policy re-admission restores only the interrupted job; the manual
    // pause keeps its state and is never spawned.
    profile.1.update(|state| {
        state.settings.read_only = false;
        Ok(())
    }).unwrap();
    let admitted = reconnect_resume(
        &profile.1.settings().unwrap(),
        InternetAdmission::Admitted,
        true,
        5,
        4,
    );
    assert_eq!(admitted, ResumeAdmission::Attempt);
    assert_eq!(
        manager.resume_interrupted_after_reconnect(admitted).await.unwrap(),
        None
    );
    assert_eq!(manager.record(&active_id).unwrap().job.status, "queued");
    let manual = manager.record(&manual_id).unwrap();
    assert_eq!(manual.job.status, "paused");
    assert!(manual.paused);
    assert!(!manual.interrupted);
    manager.suspend().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn suspended_limits_persist_without_reopening_transfer_admission() {
    let profile = Profile::new();
    let mut record = profile.record();
    record.paused = true;
    record.job.status = "paused".into();
    let id = record.job.id.clone();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    manager.suspend().await.unwrap();
    let jobs_before = serde_json::to_value(&manager.inner.state.lock().unwrap().jobs).unwrap();

    manager.limits(32_768, 16_384).await.unwrap();
    let saved: State = serde_json::from_slice(
        &fs::read(profile.0.join("torrents/jobs.json")).unwrap(),
    ).unwrap();
    assert_eq!((saved.download_limit, saved.upload_limit), (32_768, 16_384));
    assert_eq!(serde_json::to_value(&saved.jobs).unwrap(), jobs_before);
    assert!(manager.inner.suspended.load(Ordering::SeqCst));
    assert!(manager.inner.runtimes.lock().unwrap().is_empty());
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.resume(&id, None).await.unwrap_err().contains("suspended"));
    let before = fs::read(profile.0.join("torrents/jobs.json")).unwrap();
    assert!(manager.limits(1, 16_384).await.is_err());
    assert!(manager.limits(32_768, u32::MAX as u64 + 1).await.is_err());
    assert_eq!(fs::read(profile.0.join("torrents/jobs.json")).unwrap(), before);

    manager.shutdown().await.unwrap();
    let before = fs::read(profile.0.join("torrents/jobs.json")).unwrap();
    assert!(manager.limits(0, 0).await.unwrap_err().contains("shut down"));
    assert_eq!(fs::read(profile.0.join("torrents/jobs.json")).unwrap(), before);
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_resume_works_with_policy_disabled_and_shutdown_joins_workers() {
    let profile = Profile::new();
    let record = profile.record();
    let id = record.job.id.clone();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    manager.resume(&id, None).await.unwrap();
    let jobs = manager.list().await;
    assert_eq!(jobs[0].status, "queued");
    // No await above yields on this single-thread runtime: stop before the
    // newly queued worker can create an engine or contact any network.
    manager.shutdown().await.unwrap();
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.inner.tasks.lock().unwrap().is_empty());
    assert!(manager.inner.runtimes.lock().unwrap().is_empty());
    let before = fs::read(profile.0.join("torrents/jobs.json")).unwrap();
    assert!(manager.resume(&id, None).await.is_err());
    assert!(manager.pause(&id).await.is_err());
    assert!(manager.cancel(&id).await.is_err());
    assert!(manager.limits(0, 0).await.is_err());
    assert!(manager.seeding(&id, true).await.is_err());
    assert_eq!(
        fs::read(profile.0.join("torrents/jobs.json")).unwrap(),
        before
    );
}

#[tokio::test(flavor = "current_thread")]
async fn queued_worker_retirement_preserves_manual_pause_and_cancellation() {
    let profile = Profile::new();
    let paused = profile.record();
    let cancelled = profile.record();
    let paused_id = paused.job.id.clone();
    let cancelled_id = cancelled.job.id.clone();
    profile.save(vec![paused, cancelled]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    manager.resume(&paused_id, None).await.unwrap();
    manager.pause(&paused_id).await.unwrap();
    assert!(!manager.inner.runtimes.lock().unwrap().contains_key(&paused_id));
    assert!(manager.engine_cell().unwrap().get().is_none());
    manager.resume(&cancelled_id, None).await.unwrap();
    manager.cancel(&cancelled_id).await.unwrap();
    manager.suspend().await.unwrap();
    manager.resume_interrupted().await.unwrap();
    assert_eq!(manager.record(&paused_id).unwrap().job.status, "paused");
    assert_eq!(manager.record(&cancelled_id).unwrap().job.status, "cancelled");
    assert!(!manager.has_interrupted().unwrap());
    assert!(manager.resume(&cancelled_id, None).await.is_err());
    assert!(manager.engine_cell().unwrap().get().is_none());
    manager.shutdown().await.unwrap();
    drop(manager);
    let restored = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert_eq!(restored.record(&paused_id).unwrap().job.status, "paused");
    assert_eq!(restored.record(&cancelled_id).unwrap().job.status, "cancelled");
    assert!(restored.inner.tasks.lock().unwrap().is_empty());
}

const TEST_ARCHIVE: &[u8] = b"ZIM\x04test archive bytes";

fn publication_factory(record: &Record) -> ConfinedFactory {
    ConfinedFactory {
        root: Arc::new(open_staging(&record.staging).unwrap()),
        path: record.staging.clone(),
        expected: Arc::new(
            record
                .files
                .iter()
                .cloned()
                .map(|path| (path, TEST_ARCHIVE.len() as u64))
                .collect(),
        ),
        sources: Arc::new(Mutex::new(HashMap::new())),
        existing: true,
        stop: CancellationToken::new(),
        gate: Arc::new(std::sync::RwLock::new(())),
        dirs: Arc::new(Mutex::new(HashSet::new())),
        owner: None,
    }
}

fn publication_fixture() -> (Profile, Arc<TorrentManager>, String) {
    let profile = Profile::new();
    let mut record = profile.record();
    record.files.push(PathBuf::from("archive.zim"));
    crate::fs_ext::create_private_dir_all(&record.staging).unwrap();
    fs::write(
        record.staging.join("archive.zim"),
        b"ZIM\x04test archive bytes",
    )
    .unwrap();
    let id = record.job.id.clone();
    profile.save(vec![record]);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    (profile, manager, id)
}

fn publish_fixture(
    manager: &TorrentManager,
    id: &str,
    before_rename: bool,
    after_rename: bool,
) -> Result<Vec<String>, String> {
    let record = manager.record(id)?;
    let mut sources = HashMap::new();
    if record.staging.exists() {
        let factory = publication_factory(&record);
        for path in &record.files {
            if record.staging.join(path).exists() {
                factory
                    .open_file(path, TEST_ARCHIVE.len() as u64)
                    .map_err(display)?;
            }
        }
        sources = factory.sources.lock().map_err(display)?.clone();
    }
    publish_fixture_with_sources(manager, id, &sources, before_rename, after_rename)
}

fn publish_fixture_with_sources(
    manager: &TorrentManager,
    id: &str,
    sources: &HashMap<PathBuf, EngineSource>,
    before_rename: bool,
    after_rename: bool,
) -> Result<Vec<String>, String> {
    let record = manager.record(id)?;
    publish_archives(
        &record,
        &manager.inner.store,
        sources,
        &CancellationToken::new(),
        |temporary, remove| {
            manager.update_record(id, |r| {
                r.publication_temporaries
                    .retain(|prior| prior.name != temporary.name);
                if !remove {
                    r.publication_temporaries.push(temporary);
                }
            })
        },
        |intent| {
            let intended_path = record.directory.join(&intent.file_name);
            manager.update_record(id, |r| {
                r.publication_intents
                    .retain(|prior| prior.file_name != intent.file_name);
                r.publication_intents.push(intent);
            })?;
            assert!(manager.claims_monitored_path(&intended_path)?);
            assert!(!manager.claims_monitored_path(&record.directory.join("external.zim"))?);
            if before_rename {
                Err("Interrupted after durable intent, before rename".into())
            } else {
                Ok(())
            }
        },
        |path| {
            assert!(manager.claims_monitored_path(Path::new(&path))?);
            if after_rename {
                return Err("Interrupted after rename, before acknowledgement".into());
            }
            manager.update_record(id, |r| {
                if !r.job.completed_paths.contains(&path) {
                    r.job.completed_paths.push(path);
                }
            })
        },
    )
}

fn destination_book(path: &Path) -> crate::store::Book {
    let mut book = crate::imports::load_book(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/small.zim")
            .to_str()
            .unwrap(),
        false,
        false,
    )
    .unwrap();
    book.path = path.to_string_lossy().into_owned();
    book.available = false;
    book
}

#[test]
fn fresh_publication_cannot_fill_missing_installed_path_even_with_copied_uuid() {
    for reserve_before_copy in [true, false] {
        let (profile, manager, id) = publication_fixture();
        let record = manager.record(&id).unwrap();
        let target = profile.0.join("archive.zim");
        let book = destination_book(&target);
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let bytes = fs::read(&fixture).unwrap();
        fs::write(record.staging.join("archive.zim"), &bytes).unwrap();
        let mut factory = publication_factory(&record);
        factory.expected = Arc::new(HashMap::from([(
            PathBuf::from("archive.zim"),
            bytes.len() as u64,
        )]));
        factory
            .open_file(Path::new("archive.zim"), bytes.len() as u64)
            .unwrap();
        let sources = factory.sources.lock().unwrap().clone();
        let reserve = || {
            profile.1.update(|state| {
                state.books.push(book.clone());
                Ok(())
            })
        };
        if reserve_before_copy {
            reserve().unwrap();
        }
        let result = publish_archives(
            &record,
            &manager.inner.store,
            &sources,
            &CancellationToken::new(),
            |temporary, remove| {
                manager.update_record(&id, |r| {
                    r.publication_temporaries
                        .retain(|prior| prior.name != temporary.name);
                    if !remove {
                        r.publication_temporaries.push(temporary);
                    }
                })
            },
            |intent| {
                manager.update_record(&id, |r| r.publication_intents.push(intent))?;
                // Reserve after copying to prove the commit itself is guarded,
                // rather than relying on an early destination preflight.
                if !reserve_before_copy {
                    reserve()?;
                }
                Ok(())
            },
            |_| panic!("A reserved library destination must never be committed"),
        );
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read(record.staging.join("archive.zim")).unwrap(), bytes);
        let retained = profile.1.book(&book.metadata.id).unwrap().unwrap();
        assert_eq!(retained.installation_id, book.installation_id);
        assert_eq!(retained.path, book.path);
        assert_eq!(retained.metadata.id, book.metadata.id);
        assert!(!retained.available);
        let record = manager.record(&id).unwrap();
        assert!(record.job.completed_paths.is_empty());
        assert!(record.publication_temporaries.is_empty());
        assert_eq!(record.publication_intents.len(), 1);
    }
}

#[cfg(unix)]
#[test]
fn publication_directory_policy_requires_owned_mutation_not_private_reads() {
    let uid = crate::fs_ext::effective_uid();
    for mode in [0o700, 0o750, 0o755] {
        assert!(publication_directory_mutation_is_owned(uid, mode));
        assert!(!publication_directory_mutation_is_owned(uid ^ 1, mode));
    }
    for mode in [0o720, 0o702, 0o775, 0o757, 0o777, 0o1777] {
        assert!(!publication_directory_mutation_is_owned(uid, mode));
    }
}

#[cfg(unix)]
#[test]
fn publication_succeeds_with_readable_0755_parent_and_capture() {
    use std::os::unix::fs::PermissionsExt;
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    fs::set_permissions(&record.directory, fs::Permissions::from_mode(0o755)).unwrap();
    let factory = publication_factory(&record);
    factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    let mut capture_path = None;
    let paths = publish_archives(
        &record,
        &manager.inner.store,
        &sources,
        &CancellationToken::new(),
        |temporary, remove| {
            if !remove {
                // Model mount-derived modes without claiming a real FAT run.
                let path = record.directory.join(&temporary.capture_name);
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                let destination = open_publication_directory(&record.directory)?;
                open_publication_capture(&destination, &temporary.capture_name)?;
                capture_path = Some(path);
            }
            manager.update_record(&id, |r| {
                r.publication_temporaries
                    .retain(|prior| prior.name != temporary.name);
                if !remove {
                    r.publication_temporaries.push(temporary);
                }
            })
        },
        |intent| manager.update_record(&id, |r| r.publication_intents.push(intent)),
        |path| manager.update_record(&id, |r| r.job.completed_paths.push(path)),
    )
    .unwrap();
    assert_eq!(paths, manager.record(&id).unwrap().job.completed_paths);
    assert_eq!(
        fs::read(profile.0.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
    assert!(!capture_path.unwrap().exists());
    assert!(manager
        .record(&id)
        .unwrap()
        .publication_temporaries
        .is_empty());
}

#[cfg(unix)]
#[test]
fn publication_rejects_a_temporary_rewritten_after_its_digest() {
    use std::io::{Seek, SeekFrom, Write};
    let (_profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    let mut substituted = TEST_ARCHIVE.to_vec();
    substituted[4..8].copy_from_slice(b"EVIL");
    let result = publish_archives(
        &record,
        &manager.inner.store,
        &sources,
        &CancellationToken::new(),
        |temporary, remove| {
            manager.update_record(&id, |r| {
                r.publication_temporaries
                    .retain(|prior| prior.name != temporary.name);
                if !remove {
                    r.publication_temporaries.push(temporary);
                }
            })
        },
        |intent| {
            manager.update_record(&id, |r| r.publication_intents.push(intent))?;
            // Rewrite the exact temporary in place, keeping its length, and put
            // mtime back. The rename must still refuse because ctime moved.
            let temporary = fs::read_dir(&record.directory)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    let name = path.file_name().unwrap().to_string_lossy();
                    name.starts_with(".kiwix-publish-")
                        && !name.starts_with(".kiwix-publish-cleanup-")
                })
                .unwrap();
            let modified = fs::metadata(&temporary).unwrap().modified().unwrap();
            let mut file = fs::OpenOptions::new().write(true).open(&temporary).unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&substituted).unwrap();
            file.sync_all().unwrap();
            drop(file);
            fs::OpenOptions::new().write(true).open(&temporary).unwrap()
                .set_modified(modified).unwrap();
            Ok(())
        },
        |_| panic!("a rewritten temporary must never be committed"),
    );
    let error = result.unwrap_err();
    assert!(error.contains("verified identity"), "the digest identity must bind the rename: {error}");
    assert!(!record.directory.join("archive.zim").exists());
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
}

#[cfg(unix)]
#[test]
fn publication_rejects_mutable_parent_before_creating_temporaries() {
    use std::os::unix::fs::PermissionsExt;
    let (profile, manager, id) = publication_fixture();
    for mode in [0o775, 0o757, 0o777] {
        fs::set_permissions(&profile.0, fs::Permissions::from_mode(mode)).unwrap();
        let error = publish_fixture(&manager, &id, false, false).unwrap_err();
        assert!(error.contains("Choose a private download folder"), "{error}");
        assert_eq!(fs::metadata(&profile.0).unwrap().permissions().mode() & 0o777, mode);
        let record = manager.record(&id).unwrap();
        assert!(record.publication_temporaries.is_empty());
        assert!(record.publication_intents.is_empty());
        assert!(!profile.0.join("archive.zim").exists());
        assert_eq!(
            fs::read(record.staging.join("archive.zim")).unwrap(),
            TEST_ARCHIVE
        );
    }
}

#[cfg(unix)]
#[test]
fn cleanup_preserves_payload_until_capture_denies_other_users_writes() {
    use std::os::unix::fs::PermissionsExt;
    let (_profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let destination = open_publication_directory(&record.directory).unwrap();
    let (temporary, mut output) = create_publication_temporary(
        &record.directory,
        &destination,
        format!(".kiwix-publish-{}", Uuid::new_v4()),
    )
    .unwrap();
    output.write_all(b"interrupted payload").unwrap();
    drop(output);
    manager
        .update_record(&id, |r| r.publication_temporaries.push(temporary.clone()))
        .unwrap();
    let capture = record.directory.join(&temporary.capture_name);
    for mode in [0o775, 0o757, 0o777] {
        fs::set_permissions(&capture, fs::Permissions::from_mode(mode)).unwrap();
        assert!(manager
            .reclaim_temporaries(&manager.record(&id).unwrap())
            .is_err());
        assert_eq!(
            fs::read(record.directory.join(&temporary.name)).unwrap(),
            b"interrupted payload"
        );
        assert_eq!(
            manager.record(&id).unwrap().publication_temporaries.len(),
            1
        );
    }
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    manager
        .reclaim_temporaries(&manager.record(&id).unwrap())
        .unwrap();
    assert!(!record.directory.join(&temporary.name).exists());
    assert!(!capture.exists());
    assert!(manager
        .record(&id)
        .unwrap()
        .publication_temporaries
        .is_empty());
}

#[cfg(unix)] // Uses std::os::unix::fs::MetadataExt (dev/ino/nlink); Windows identity is volume serial + file index.
#[test]
fn publication_copies_engine_opened_inode_after_valid_source_replacement() {
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    // This is the exact opening path used by ConfinedStorage::init. Model
    // the engine's final verified read before replacing its directory entry.
    let engine_file = factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    let mut verified_bytes = vec![0; TEST_ARCHIVE.len()];
    engine_file.pread_exact(0, &mut verified_bytes).unwrap();
    assert_eq!(verified_bytes, TEST_ARCHIVE);
    fs::rename(
        record.staging.join("archive.zim"),
        record.staging.join("engine-opened.zim"),
    )
    .unwrap();
    let foreign = b"ZIM\x04evil archive bytes";
    fs::write(record.staging.join("archive.zim"), foreign).unwrap();
    let foreign_metadata = File::open(record.staging.join("archive.zim"))
        .unwrap()
        .metadata()
        .unwrap();
    assert!(foreign_metadata.is_file());
    assert_eq!(foreign_metadata.nlink(), 1);
    assert_eq!(foreign_metadata.len(), TEST_ARCHIVE.len() as u64);
    let sources = factory.sources.lock().unwrap().clone();
    assert!(Arc::ptr_eq(
        &engine_file,
        &sources[Path::new("archive.zim")].file
    ));
    let paths = publish_fixture_with_sources(&manager, &id, &sources, false, false).unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        fs::read(profile.0.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        foreign
    );
}

#[test]
fn publication_refuses_source_rewritten_after_engine_verification() {
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    let mut substituted = TEST_ARCHIVE.to_vec();
    substituted[4..8].copy_from_slice(b"EVIL");
    fs::write(record.staging.join("archive.zim"), &substituted).unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    let error = publish_fixture_with_sources(&manager, &id, &sources, false, false).unwrap_err();
    assert!(error.contains("changed after its pieces were verified"), "{error}");
    assert!(!profile.0.join("archive.zim").exists());
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        substituted
    );
}

#[test]
fn publication_requires_exact_advertised_length_of_engine_opened_source() {
    for delta in [-1i64, 1] {
        let (profile, manager, id) = publication_fixture();
        let record = manager.record(&id).unwrap();
        let factory = publication_factory(&record);
        let engine_file = factory
            .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
            .unwrap();
        engine_file
            .set_len((TEST_ARCHIVE.len() as i64 + delta) as u64)
            .unwrap();
        let sources = factory.sources.lock().unwrap().clone();
        assert!(publish_fixture_with_sources(&manager, &id, &sources, false, false).is_err());
        assert!(!profile.0.join("archive.zim").exists());
        assert!(manager.record(&id).unwrap().publication_intents.is_empty());
    }
}

#[test]
fn publication_rejects_engine_opened_source_with_new_hardlink() {
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    fs::hard_link(
        record.staging.join("archive.zim"),
        profile.0.join("source-alias.zim"),
    )
    .unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    assert!(publish_fixture_with_sources(&manager, &id, &sources, false, false).is_err());
    assert!(!profile.0.join("archive.zim").exists());
    assert!(manager.record(&id).unwrap().publication_intents.is_empty());
}

#[cfg(unix)]
#[test]
fn publication_rejects_unlinked_engine_source_after_foreign_replacement() {
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    let engine_file = factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    let replacement = record.staging.join("replacement.zim");
    fs::write(&replacement, b"ZIM\x04evil archive bytes").unwrap();
    fs::rename(&replacement, record.staging.join("archive.zim")).unwrap();
    assert_eq!(engine_file.metadata().unwrap().nlink(), 0);
    let sources = factory.sources.lock().unwrap().clone();
    assert!(publish_fixture_with_sources(&manager, &id, &sources, false, false).is_err());
    assert!(!profile.0.join("archive.zim").exists());
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        b"ZIM\x04evil archive bytes"
    );
}

#[cfg(unix)] // Uses std::os::unix::fs::MetadataExt (dev/ino/nlink); Windows identity is volume serial + file index.
#[test]
fn publication_recovers_interruption_before_rename() {
    let (profile, manager, id) = publication_fixture();
    assert!(publish_fixture(&manager, &id, true, false).is_err());
    assert!(!profile.0.join("archive.zim").exists());
    drop(manager);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert_eq!(manager.record(&id).unwrap().publication_intents.len(), 1);
    let paths = publish_fixture(&manager, &id, false, false).unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(manager.record(&id).unwrap().job.completed_paths, paths);
    let record = manager.record(&id).unwrap();
    let staged = File::open(record.staging.join("archive.zim"))
        .unwrap()
        .metadata()
        .unwrap();
    let published = File::open(profile.0.join("archive.zim"))
        .unwrap()
        .metadata()
        .unwrap();
    assert_ne!(
        (staged.dev(), staged.ino()),
        (published.dev(), published.ino())
    );
    assert_eq!(published.nlink(), 1);
}

#[test]
fn publication_recovery_replays_backup_policy_before_committing() {
    let (profile, manager, id) = publication_fixture();
    assert!(publish_fixture(&manager, &id, false, true).is_err());
    let record = manager.record(&id).unwrap();
    let directory = open_publication_directory(&record.directory).unwrap();
    let stop = CancellationToken::new();
    let committed = std::cell::Cell::new(0);
    let attempts = std::cell::Cell::new(0);
    let result = recover_archives_with_policy(&record, &directory, &stop, |_| {
        committed.set(committed.get() + 1);
        Ok(())
    }, |file, path| {
        attempts.set(attempts.get() + 1);
        assert_eq!(path, profile.0.join("archive.zim"));
        assert_eq!(file.metadata().unwrap().len(), TEST_ARCHIVE.len() as u64);
        Err("backup policy rejected".into())
    });
    assert_eq!(result.unwrap_err(), "backup policy rejected");
    assert_eq!(committed.get(), 0);
    assert_eq!(record.publication_intents.len(), 1);
    let recovered = recover_archives_with_policy(&record, &directory, &stop, |_| {
        committed.set(committed.get() + 1);
        Ok(())
    }, |file, _| {
        attempts.set(attempts.get() + 1);
        assert_eq!(file.metadata().unwrap().len(), TEST_ARCHIVE.len() as u64);
        Ok(())
    }).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(attempts.get(), 2);
    assert_eq!(committed.get(), 1);
}

#[test]
fn legacy_completed_publication_is_recovered_only_when_its_destination_is_present() {
    let (profile, manager, id) = publication_fixture();
    publish_fixture(&manager, &id, false, false).unwrap();
    // Rewrite the saved record as one acknowledged before publication intents.
    manager
        .update_record(&id, |record| {
            record.publication_intents.clear();
        })
        .unwrap();
    let record = manager.record(&id).unwrap();
    let destination = profile.0.join("archive.zim");
    assert!(record
        .job
        .completed_paths
        .contains(&destination.to_string_lossy().into_owned()));
    let directory = open_publication_directory(&record.directory).unwrap();
    let stop = CancellationToken::new();
    let policy = std::cell::Cell::new(0);
    let recovered = recover_archives_with_policy(
        &record,
        &directory,
        &stop,
        |_| Ok(()),
        |file, path| {
            policy.set(policy.get() + 1);
            assert_eq!(path, destination.as_path());
            assert_eq!(file.metadata().unwrap().len(), TEST_ARCHIVE.len() as u64);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(recovered, vec![destination.to_string_lossy().into_owned()]);
    assert_eq!(policy.get(), 1);
    // An absent destination is left unrecovered, so publication runs again
    // instead of reporting a file that is not there.
    fs::remove_file(&destination).unwrap();
    let recovered = recover_archives_with_policy(
        &record,
        &directory,
        &stop,
        |_| Err("unexpected commit".into()),
        |_, _| Err("unexpected policy".into()),
    )
    .unwrap();
    assert!(recovered.is_empty());
}

#[cfg(unix)] // Uses std::os::unix::fs::MetadataExt (dev/ino/nlink); Windows identity is volume serial + file index.
#[tokio::test(flavor = "current_thread")]
async fn worker_recovers_publication_without_staging_metadata_engine_or_peers() {
    let (profile, manager, id) = publication_fixture();
    assert!(publish_fixture(&manager, &id, false, true).is_err());
    let destination = profile.0.join("archive.zim");
    let before = File::open(&destination).unwrap().metadata().unwrap();
    assert!(manager.record(&id).unwrap().job.completed_paths.is_empty());
    assert!(manager.claims_monitored_path(&destination).unwrap());
    assert!(!manager.claims_monitored_path(&profile.0.join("external.zim")).unwrap());
    manager
        .update_record(&id, |r| r.job.seeding = true)
        .unwrap();
    let book = destination_book(&destination);
    profile
        .1
        .update(|state| {
            state.books.push(book.clone());
            Ok(())
        })
        .unwrap();
    drop(manager);
    let claims = TorrentManager::saved_monitored_paths(&profile.1).unwrap();
    assert!(claims.claims_monitored_path(&destination).unwrap());
    assert!(!claims.claims_monitored_path(&profile.0.join("external.zim")).unwrap());
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert!(manager.claims_monitored_path(&destination).unwrap());
    // Exercise the worker entry point, not just the low-level publication
    // helper: recovery must precede open_staging and engine initialization.
    let staging = manager.record(&id).unwrap().staging;
    fs::remove_dir_all(&staging).unwrap();
    assert!(!manager.record(&id).unwrap().metadata.exists());
    manager.run(&id, &CancellationToken::new()).await.unwrap();
    let record = manager.record(&id).unwrap();
    assert!(record.verified);
    assert_eq!(record.job.status, "completed");
    assert_eq!(
        record.job.completed_paths,
        vec![destination.to_string_lossy().into_owned()]
    );
    assert!(!record.job.seeding);
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(manager.claims_monitored_path(&destination).unwrap());
    assert!(!staging.exists());
    let after = File::open(destination).unwrap().metadata().unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!(
        fs::read(profile.0.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
    let retained = profile.1.book(&book.metadata.id).unwrap().unwrap();
    assert_eq!(retained.installation_id, book.installation_id);
    assert_eq!(retained.path, book.path);
}

#[tokio::test(flavor = "current_thread")]
async fn worker_recovers_committed_subset_before_missing_staging_failure() {
    let (profile, manager, id) = publication_fixture();
    assert!(publish_fixture(&manager, &id, false, true).is_err());
    manager
        .update_record(&id, |r| r.files.push(PathBuf::from("remaining.zim")))
        .unwrap();
    let staging = manager.record(&id).unwrap().staging;
    fs::remove_dir_all(&staging).unwrap();
    drop(manager);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    assert!(manager.run(&id, &CancellationToken::new()).await.is_err());
    let record = manager.record(&id).unwrap();
    assert!(!record.verified);
    assert_eq!(
        record.job.completed_paths,
        vec![profile.0.join("archive.zim").to_string_lossy().into_owned()]
    );
    assert!(manager.engine_cell().unwrap().get().is_none());
    assert!(!staging.exists());
}

#[test]
fn publication_preserves_unrelated_existing_files_even_with_identical_bytes() {
    let (profile, manager, id) = publication_fixture();
    let destination = profile.0.join("archive.zim");
    fs::write(&destination, b"ZIM\x04test archive bytes").unwrap();
    assert!(publish_fixture(&manager, &id, false, false).is_err());
    assert!(manager.record(&id).unwrap().publication_intents.is_empty());
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"ZIM\x04test archive bytes"
    );
    fs::remove_file(&destination).unwrap();
    assert!(publish_fixture(&manager, &id, false, true).is_err());
    // Keep the published inode alive so inode reuse cannot weaken the test.
    fs::rename(&destination, profile.0.join("original.zim")).unwrap();
    fs::write(&destination, b"ZIM\x04test archive bytes").unwrap();
    assert!(publish_fixture(&manager, &id, false, false).is_err());
    assert!(manager.record(&id).unwrap().job.completed_paths.is_empty());
    assert_eq!(fs::read(destination).unwrap(), b"ZIM\x04test archive bytes");
}

#[test]
fn publication_recovery_rejects_hash_mismatch_and_hardlinks() {
    let (profile, manager, id) = publication_fixture();
    assert!(publish_fixture(&manager, &id, false, true).is_err());
    let destination = profile.0.join("archive.zim");
    fs::write(&destination, b"ZIM\x04evil archive bytes").unwrap();
    assert!(publish_fixture(&manager, &id, false, false).is_err());
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"ZIM\x04evil archive bytes"
    );
    fs::write(&destination, b"ZIM\x04test archive bytes").unwrap();
    fs::hard_link(&destination, profile.0.join("alias.zim")).unwrap();
    assert!(publish_fixture(&manager, &id, false, false).is_err());
    assert!(manager.record(&id).unwrap().job.completed_paths.is_empty());
}

#[cfg(unix)]
#[test]
fn publication_recovery_rejects_symlink_and_fifo_destinations() {
    let (profile, manager, id) = publication_fixture();
    assert!(publish_fixture(&manager, &id, false, true).is_err());
    let destination = profile.0.join("archive.zim");
    fs::rename(&destination, profile.0.join("original.zim")).unwrap();
    std::os::unix::fs::symlink(profile.0.join("original.zim"), &destination).unwrap();
    assert!(publish_fixture(&manager, &id, false, false).is_err());
    fs::remove_file(&destination).unwrap();
    assert!(nix::unistd::mkfifo(&destination, nix::sys::stat::Mode::from_bits_truncate(0o600)).is_ok());
    assert!(publish_fixture(&manager, &id, false, false).is_err());
    assert!(manager.record(&id).unwrap().job.completed_paths.is_empty());
}

#[test]
fn publication_cancellation_reclaims_the_durably_journaled_copy() {
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    factory
        .open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64)
        .unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    let stop = CancellationToken::new();
    let mut temporary_path = None;
    let result = publish_archives(
        &record,
        &manager.inner.store,
        &sources,
        &stop,
        |temporary, remove| {
            if !remove {
                assert_eq!(
                    fs::metadata(record.directory.join(&temporary.name))
                        .unwrap()
                        .len(),
                    0
                );
                temporary_path = Some(record.directory.join(&temporary.name));
            }
            manager.update_record(&id, |r| {
                r.publication_temporaries
                    .retain(|prior| prior.name != temporary.name);
                if !remove {
                    r.publication_temporaries.push(temporary);
                }
            })
        },
        |intent| {
            assert_eq!(intent.size, TEST_ARCHIVE.len() as u64);
            assert_eq!(
                manager.record(&id).unwrap().publication_temporaries.len(),
                1
            );
            stop.cancel();
            Ok(())
        },
        |_| panic!("Cancelled copy must not be committed"),
    );
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(!temporary_path.unwrap().exists());
    assert!(!profile.0.join("archive.zim").exists());
    assert!(manager
        .record(&id)
        .unwrap()
        .publication_temporaries
        .is_empty());
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
}

#[tokio::test(flavor = "current_thread")]
async fn manual_pause_joins_publication_before_ack_and_can_resume() {
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    factory.open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64).unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    let stop = manager.inner.stop.child_token();
    let _cancel_on_failure = stop.clone().drop_guard();
    let (retirement, retired) = tokio::sync::oneshot::channel();
    let (copied, ready) = tokio::sync::oneshot::channel();
    manager.inner.runtimes.lock().unwrap().insert(id.clone(), Runtime {
        generation: Uuid::new_v4(),
        token: stop.clone(),
        retired: Some(retired),
        handle: None,
    });
    let owner = manager.clone();
    let worker_id = id.clone();
    manager.inner.tasks.lock().unwrap().spawn(async move {
        let publisher = owner.clone();
        let copy_id = worker_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut copied = Some(copied);
            publish_archives(
                &record,
                &publisher.inner.store,
                &sources,
                &stop,
                |temporary, remove| publisher.update_record(&copy_id, |r| {
                    r.publication_temporaries.retain(|prior| prior.name != temporary.name);
                    if !remove { r.publication_temporaries.push(temporary); }
                }),
                |intent| {
                    publisher.update_record(&copy_id, |r| r.publication_intents.push(intent))?;
                    let _ = copied.take().unwrap().send(());
                    // Hold the real copied output before rename until Pause
                    // cancels this publication's actual worker token.
                    while !stop.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Ok(())
                },
                |_| panic!("A manually paused publication must not acknowledge an import path"),
            )
        }).await.unwrap();
        assert!(result.unwrap_err().contains("cancelled"));
        let _gate = owner.inner.gate.lock().await;
        owner.inner.runtimes.lock().unwrap().remove(&worker_id);
        let _ = retirement.send(Ok(()));
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        ready.await.unwrap();
        manager.pause(&id).await.unwrap();
    }).await.unwrap();
    let paused = manager.record(&id).unwrap();
    assert!(paused.paused);
    assert_eq!(paused.job.status, "paused");
    assert!(!paused.verified);
    assert!(paused.job.completed_paths.is_empty());
    assert!(paused.publication_temporaries.is_empty());
    assert!(!profile.0.join("archive.zim").exists());
    assert!(!manager.inner.runtimes.lock().unwrap().contains_key(&id));
    assert_eq!(fs::read(paused.staging.join("archive.zim")).unwrap(), TEST_ARCHIVE);
    manager.resume(&id, None).await.unwrap();
    assert_eq!(manager.record(&id).unwrap().job.status, "queued");
    manager.suspend().await.unwrap();
}

#[test]
fn publication_write_failure_reclaims_partial_payload_and_keeps_staging() {
    struct DiskFull<'a> {
        file: &'a mut cap_std::fs::File,
        remaining: usize,
    }
    impl Write for DiskFull<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(std::io::Error::from_raw_os_error(28));
            }
            let count = bytes.len().min(self.remaining);
            let written = self.file.write(&bytes[..count])?;
            self.remaining -= written;
            Ok(written)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    let (_profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let destination = open_publication_directory(&record.directory).unwrap();
    let (temporary, mut output) = create_publication_temporary(
        &record.directory,
        &destination,
        format!(".kiwix-publish-{}", Uuid::new_v4()),
    )
    .unwrap();
    manager
        .update_record(&id, |r| r.publication_temporaries.push(temporary.clone()))
        .unwrap();
    let failure = publication_digest(
        &mut &TEST_ARCHIVE[..],
        &mut DiskFull {
            file: &mut output,
            remaining: 7,
        },
        TEST_ARCHIVE.len() as u64,
        &CancellationToken::new(),
    );
    assert!(failure.is_err());
    assert_eq!(output.metadata().unwrap().len(), 7);
    drop(output);
    manager
        .reclaim_temporaries(&manager.record(&id).unwrap())
        .unwrap();
    assert!(!record.directory.join(&temporary.name).exists());
    assert!(!record.directory.join(&temporary.capture_name).exists());
    assert!(manager
        .record(&id)
        .unwrap()
        .publication_temporaries
        .is_empty());
    assert_eq!(
        fs::read(record.staging.join("archive.zim")).unwrap(),
        TEST_ARCHIVE
    );
}

#[test]
fn startup_reclaims_partial_copy_for_cancelled_paused_and_completed_jobs() {
    for status in ["cancelled", "paused", "completed"] {
        for captured in [false, true] {
            let (profile, manager, id) = publication_fixture();
            let record = manager.record(&id).unwrap();
            let destination = open_publication_directory(&record.directory).unwrap();
            let (temporary, mut output) = create_publication_temporary(
                &record.directory,
                &destination,
                format!(".kiwix-publish-{}", Uuid::new_v4()),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for path in [
                    record.directory.clone(),
                    record.directory.join(&temporary.capture_name),
                ] {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
                }
            }
            manager
                .update_record(&id, |r| {
                    r.job.status = status.into();
                    r.paused = true;
                    r.verified = status == "completed";
                    r.publication_temporaries.push(temporary.clone());
                })
                .unwrap();
            output.write_all(b"interrupted payload").unwrap();
            output.sync_all().unwrap();
            drop(output);
            if captured {
                let capture =
                    open_publication_capture(&destination, &temporary.capture_name).unwrap();
                move_publication_entry(&destination, &temporary.name, &capture, "payload")
                    .unwrap();
                sync_publication_directory(&capture).unwrap();
                sync_publication_directory(&destination).unwrap();
            }
            let sentinel = profile.0.join(format!(".kiwix-publish-{}", Uuid::new_v4()));
            fs::write(&sentinel, b"untracked user sentinel").unwrap();
            fs::remove_dir_all(&record.staging).unwrap();
            drop(manager);
            let reopened =
                TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
            assert!(reopened
                .record(&id)
                .unwrap()
                .publication_temporaries
                .is_empty());
            assert!(!record.directory.join(&temporary.name).exists());
            assert!(!record.directory.join(&temporary.capture_name).exists());
            assert_eq!(fs::read(sentinel).unwrap(), b"untracked user sentinel");
            assert!(reopened.engine_cell().unwrap().get().is_none());
        }
    }
}

#[test]
fn temporary_cleanup_captures_before_identity_check_and_preserves_replacement() {
    for collision in [false, true] {
        let (profile, manager, id) = publication_fixture();
        let record = manager.record(&id).unwrap();
        let destination = open_publication_directory(&record.directory).unwrap();
        let (temporary, mut output) = create_publication_temporary(
            &record.directory,
            &destination,
            format!(".kiwix-publish-{}", Uuid::new_v4()),
        )
        .unwrap();
        manager
            .update_record(&id, |r| r.publication_temporaries.push(temporary.clone()))
            .unwrap();
        output.write_all(b"owned partial").unwrap();
        drop(output);
        let original = profile.0.join("retained-original");
        let public = profile.0.join(&temporary.name);
        let result = reclaim_publication_temporary_with_hook(
            &record.directory,
            &destination,
            &temporary,
            || {
                fs::rename(&public, &original).unwrap();
                fs::write(&public, b"foreign replacement").unwrap();
                if collision {
                    let capture =
                        open_publication_capture(&destination, &temporary.capture_name)
                            .unwrap();
                    move_publication_entry(&destination, &temporary.name, &capture, "payload")
                        .unwrap();
                    fs::write(&public, b"new public sentinel").unwrap();
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&original).unwrap(), b"owned partial");
        if collision {
            assert_eq!(fs::read(&public).unwrap(), b"new public sentinel");
            assert_eq!(
                fs::read(profile.0.join(&temporary.capture_name).join("payload")).unwrap(),
                b"foreign replacement"
            );
        } else {
            assert_eq!(fs::read(&public).unwrap(), b"foreign replacement");
        }
        assert_eq!(
            manager.record(&id).unwrap().publication_temporaries.len(),
            1
        );
    }
}

#[test]
fn temporary_cleanup_preserves_a_replaced_destination_directory() {
    let profile = Profile::new();
    let path = profile.0.join("destination");
    crate::fs_ext::create_private_dir_all(&path).unwrap();
    let destination = open_publication_directory(&path).unwrap();
    let (temporary, mut output) = create_publication_temporary(
        &path,
        &destination,
        format!(".kiwix-publish-{}", Uuid::new_v4()),
    )
    .unwrap();
    output.write_all(b"owned partial").unwrap();
    drop(output);
    let retained = profile.0.join("retained");
    // Windows itself denies renaming a directory while a capability for it is
    // open, so there a replacement can only precede the cleanup's capability:
    // cleanup then opens the replacement by path and must refuse it by identity.
    #[cfg(windows)]
    {
        assert!(fs::rename(&path, &retained).is_err(), "an open capability pins its directory");
        drop(destination);
    }
    fs::rename(&path, &retained).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join(&temporary.name),
        b"replacement directory sentinel",
    )
    .unwrap();
    #[cfg(windows)]
    let destination = open_publication_directory(&path).unwrap();
    assert!(reclaim_publication_temporary(&path, &destination, &temporary).is_err());
    assert_eq!(
        fs::read(path.join(&temporary.name)).unwrap(),
        b"replacement directory sentinel"
    );
    assert_eq!(
        fs::read(retained.join(&temporary.name)).unwrap(),
        b"owned partial"
    );
}

#[test]
fn failed_first_publication_checkpoint_reclaims_the_open_empty_temporary() {
    // The temporary's writer is still open when its ownership checkpoint fails.
    // Windows cannot capture (rename) a file while that writer is open, so
    // cleanup must release it first instead of leaving the temporary behind.
    let (profile, manager, id) = publication_fixture();
    let record = manager.record(&id).unwrap();
    let factory = publication_factory(&record);
    factory.open_file(Path::new("archive.zim"), TEST_ARCHIVE.len() as u64).unwrap();
    let sources = factory.sources.lock().unwrap().clone();
    let mut journaled = None;
    let result = publish_archives(
        &record,
        &manager.inner.store,
        &sources,
        &CancellationToken::new(),
        |temporary, remove| {
            assert!(!remove, "a failed checkpoint is never acknowledged as removed");
            journaled = Some(temporary);
            Err("injected temporary checkpoint failure".into())
        },
        |_| panic!("No payload may be copied before the temporary is journaled"),
        |_| panic!("A failed checkpoint must not commit"),
    );
    assert_eq!(result.unwrap_err(), "injected temporary checkpoint failure");
    let temporary = journaled.unwrap();
    assert!(!record.directory.join(&temporary.name).exists());
    assert!(!record.directory.join(&temporary.capture_name).exists());
    assert!(!profile.0.join("archive.zim").exists());
    assert_eq!(fs::read(record.staging.join("archive.zim")).unwrap(), TEST_ARCHIVE);
}

#[test]
fn owned_empty_directory_removal_is_empty_only_and_identity_bound() {
    let profile = Profile::new();
    let parent = open_publication_directory(&profile.0).unwrap();
    let path = profile.0.join("owned-empty");
    fs::create_dir(&path).unwrap();
    let identity = PublicationIdentity::of(&parent.open_dir_nofollow("owned-empty").unwrap().dir_metadata().unwrap());
    fs::write(path.join("unknown"), b"unknown entry").unwrap();
    assert!(remove_owned_empty_directory(&parent, "owned-empty", &identity).is_err());
    assert_eq!(fs::read(path.join("unknown")).unwrap(), b"unknown entry");
    fs::remove_file(path.join("unknown")).unwrap();
    // Unix keeps name-based rmdir; Windows also re-binds to the identity.
    #[cfg(windows)]
    {
        let foreign = profile.0.join("foreign-empty");
        fs::create_dir(&foreign).unwrap();
        assert!(remove_owned_empty_directory(&parent, "foreign-empty", &identity).is_err());
        assert!(foreign.is_dir(), "a directory with another identity is preserved");
    }
    remove_owned_empty_directory(&parent, "owned-empty", &identity).unwrap();
    assert!(!path.exists());
    assert!(remove_owned_empty_directory(&parent, "owned-empty", &identity).is_err());
}

#[test]
fn publication_digest_rejects_growth_truncation_and_cancellation() {
    let stop = CancellationToken::new();
    assert!(publication_digest(&mut &b"too long"[..], &mut std::io::sink(), 3, &stop).is_err());
    assert!(publication_digest(&mut &b"short"[..], &mut std::io::sink(), 10, &stop).is_err());
    stop.cancel();
    assert!(publication_digest(&mut &b"data"[..], &mut std::io::sink(), 4, &stop).is_err());
}

fn share_fixture(profile: &Profile) -> (PathBuf, crate::store::Book) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
    let path = profile.0.join("shared.zim");
    fs::copy(&fixture, &path).unwrap();
    let book = crate::imports::load_book(path.to_str().unwrap(), true, true).unwrap();
    profile
        .1
        .update(|state| {
            state.books.push(book.clone());
            Ok(())
        })
        .unwrap();
    (path, book)
}

async fn wait_for_share(manager: &TorrentManager, id: &str) -> TorrentJob {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let job = manager.job(id).await.unwrap();
        if job.magnet.is_some() && matches!(job.status.as_str(), "seeding" | "error") {
            return job;
        }
        assert!(Instant::now() < deadline, "share did not start: {job:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_download(manager: &TorrentManager, id: &str) -> TorrentJob {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = manager.job(id).await.unwrap();
        if job.status == "error"
            || (job.status == "completed" && !job.completed_paths.is_empty())
        {
            return job;
        }
        assert!(Instant::now() < deadline, "download did not finish: {job:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_archive_metadata_is_stable_and_preserves_archive() {
    let profile = Profile::new();
    let (path, book) = share_fixture(&profile);
    let before = fs::read(&path).unwrap();
    let before_hash = Sha256::digest(&before);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let first = manager.share(book.clone()).await.unwrap();
    let first = wait_for_share(&manager, &first.id).await;
    let magnet = first.magnet.clone().expect("share magnet");
    assert!(magnet.starts_with("magnet:?xt=urn:btih:"));
    assert!(first.shared && first.seeding);
    assert_eq!(first.total, Some(before.len() as u64));

    manager.pause(&first.id).await.unwrap();
    manager.seeding(&first.id, false).await.unwrap();
    manager.delete_terminal(&first.id).await.unwrap();

    let second = manager.share(book.clone()).await.unwrap();
    let second = wait_for_share(&manager, &second.id).await;
    assert_eq!(second.magnet.as_deref(), Some(magnet.as_str()));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(Sha256::digest(fs::read(&path).unwrap()), before_hash);
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        before.len() as u64
    );
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_library_archive_downloads_from_magnet_in_second_profile() {
    let seeder_profile = Profile::new();
    let (path, book) = share_fixture(&seeder_profile);
    let original = fs::read(&path).unwrap();
    let original_hash = Sha256::digest(&original);
    let seeder =
        TorrentManager::new(seeder_profile.0.clone(), false, seeder_profile.1.clone()).unwrap();
    let shared = seeder.share(book.clone()).await.unwrap();
    let shared = wait_for_share(&seeder, &shared.id).await;
    assert_eq!(shared.status, "seeding");
    let magnet = shared.magnet.clone().expect("share magnet");
    let mut address = seeder
        .engine()
        .await
        .unwrap()
        .listen_addr()
        .expect("seeder listener address");
    // The engine listens on the unspecified address ([::], or 0.0.0.0 on a
    // host without IPv6). Unix connects that to loopback, but Windows refuses
    // to connect to it (WSAEADDRNOTAVAIL), so dial the same port on the
    // loopback address of the same family explicitly.
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }

    let downloader_profile = Profile::new();
    let destination = downloader_profile.0.join("downloads");
    crate::fs_ext::create_private_dir_all(&destination).unwrap();
    let downloader = TorrentManager::new(
        downloader_profile.0.clone(),
        false,
        downloader_profile.1.clone(),
    )
    .unwrap();
    let download = downloader
        .start_with_peers(
            magnet,
            destination,
            None,
            vec![address],
        )
        .await
        .unwrap();
    let download = wait_for_download(&downloader, &download.id).await;
    assert_eq!(download.status, "completed", "{download:?}");
    let published = PathBuf::from(
        download
            .completed_paths
            .first()
            .expect("published archive path"),
    );
    let downloaded = fs::read(&published).unwrap();
    assert_eq!(downloaded.len(), original.len());
    assert_eq!(Sha256::digest(&downloaded), original_hash);
    let imported = crate::imports::load_book(published.to_str().unwrap(), true, true).unwrap();
    assert_eq!(imported.metadata.id, book.metadata.id);
    assert_eq!(Sha256::digest(fs::read(&path).unwrap()), original_hash);

    // Removing the files of a real engine transfer must first release every
    // engine handle: Windows cannot capture or remove open staging entries.
    let record = downloader.record(&download.id).unwrap();
    downloader.delete_terminal(&download.id).await.unwrap();
    assert!(!published.exists());
    assert!(!record.staging.exists());
    assert!(!record.metadata.exists());
    // Ending a share never deletes the user's library archive.
    seeder.delete_terminal(&shared.id).await.unwrap();
    assert_eq!(Sha256::digest(fs::read(&path).unwrap()), original_hash);

    downloader.shutdown().await.unwrap();
    seeder.shutdown().await.unwrap();
}

/// Pins librqbit's SHA-1 backend (librqbit-sha1-wrapper) against an
/// info hash computed independently with Python's hashlib over the same bytes:
/// sha1(bencode({length, name, piece length, pieces})).
#[tokio::test]
async fn created_torrent_info_hash_matches_independent_sha1() {
    let dir = std::env::temp_dir().join(format!("kiwix-sha1-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fixture.bin");
    let data: Vec<u8> = (0..40_000u32).map(|i| ((i * 7 + 3) % 251) as u8).collect();
    fs::write(&path, &data).unwrap();
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(1);
    let created = librqbit::create_torrent(
        &path,
        librqbit::CreateTorrentOptions { name: Some("fixture.bin"), trackers: Vec::new(), piece_length: Some(16_384) },
        &spawner,
    )
    .await
    .unwrap();
    let info_hash = created.info_hash().as_string();
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(info_hash, "7c1724a25d534622de217800bb600bded02bb294");
}

// --- Dedicated torrent runtime -------------------------------------------
//
// librqbit's HTTPS tracker client verifies certificates with blocking platform
// calls (JNI on Android, possibly fetching a CRL). Those must block only the
// torrent runtime, and no pause, cancel, suspend or privacy cutover may wait
// for one of its threads.

#[tokio::test(flavor = "current_thread")]
async fn engine_work_runs_on_the_dedicated_torrent_runtime() {
    let name = on_torrent_runtime(async { std::thread::current().name().map(str::to_owned) })
        .await
        .unwrap();
    assert_eq!(name.as_deref(), Some("kiwix-torrent"));
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_the_caller_aborts_engine_work() {
    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let work = on_torrent_runtime(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        flag.store(true, Ordering::SeqCst);
    });
    tokio::select! {
        _ = work => panic!("work should still be sleeping"),
        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!finished.load(Ordering::SeqCst), "cancelled engine work kept running");
}

#[tokio::test(flavor = "current_thread")]
async fn engine_work_panics_reach_the_caller() {
    let caller = tokio::spawn(on_torrent_runtime(async { panic!("engine bug") }));
    let error = caller.await.expect_err("the panic must propagate");
    assert!(error.is_panic());
}

// Helpers for the torrent-thread tests below, which are Unix-only.
/// The torrent runtime is shared by the whole test process. Tests that block
/// its threads hold this so they do not stall each other's measurements.
#[cfg(unix)]
static TORRENT_THREAD_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A synchronous gate that a routed call blocks on, standing in for a slow
/// certificate verification inside JNI.
#[cfg(unix)]
#[derive(Clone)]
struct Verification(Arc<(Mutex<bool>, std::sync::Condvar)>);

#[cfg(unix)]
impl Verification {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(false), std::sync::Condvar::new())))
    }

    /// Blocks the calling thread until released (at most 30 s).
    fn block(&self) {
        let (released, wake) = &*self.0;
        let guard = released.lock().unwrap();
        let _ = wake.wait_timeout_while(guard, Duration::from_secs(30), |released| !*released).unwrap();
    }

    fn release(&self) {
        let (released, wake) = &*self.0;
        *released.lock().unwrap() = true;
        wake.notify_all();
    }
}

/// Fills every torrent-runtime worker that is not already blocked with a
/// blocking call, then proves saturation: a probe task must not run.
#[cfg(unix)]
async fn saturate_torrent_threads(verification: &Verification) -> Vec<tokio::task::JoinHandle<()>> {
    let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let blockers: Vec<_> = (0..2)
        .map(|_| {
            let (verification, entered) = (verification.clone(), Arc::clone(&entered));
            torrent_runtime().unwrap().spawn(async move {
                entered.fetch_add(1, Ordering::SeqCst);
                verification.block();
            })
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    while entered.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "no torrent thread picked up a blocker");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // A thread already blocked inside a routed call can leave one blocker
    // queued; either way every worker must now be busy.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let probe = Arc::new(AtomicBool::new(false));
    let ran = Arc::clone(&probe);
    let probe_task = torrent_runtime().unwrap().spawn(async move { ran.store(true, Ordering::SeqCst) });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!probe.load(Ordering::SeqCst), "the torrent runtime still had a free thread");
    probe_task.abort();
    blockers
}

#[cfg(unix)]
fn install_engine_hook(manager: &TorrentManager, hook: impl Fn(EngineCall) + Send + Sync + 'static) {
    *manager.inner.engine_hook.lock().unwrap() = Some(Arc::new(hook));
}

#[cfg(unix)]
fn clear_engine_hook(manager: &TorrentManager) {
    *manager.inner.engine_hook.lock().unwrap() = None;
}

#[cfg(unix)]
fn session_torrents(engine: &Session) -> usize {
    engine.with_torrents(|torrents| torrents.count())
}

#[cfg(unix)]
async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Starts a share whose `add_torrent` registers in the session and then blocks
/// its torrent thread, and fills the other thread. Returns once the worker is
/// waiting on the routed add with every torrent thread blocked.
#[cfg(unix)]
async fn share_blocked_after_registration(
    manager: &Arc<TorrentManager>,
    book: crate::store::Book,
    verification: &Verification,
) -> (String, Vec<tokio::task::JoinHandle<()>>) {
    let added = Arc::new(AtomicBool::new(false));
    let (seen, blocked) = (Arc::clone(&added), verification.clone());
    install_engine_hook(manager, move |call| {
        if call == EngineCall::Added(true) {
            seen.store(true, Ordering::SeqCst);
            blocked.block();
        }
    });
    let id = manager.share(book).await.unwrap().id;
    wait_until("the routed add registered the share", || added.load(Ordering::SeqCst)).await;
    let blockers = saturate_torrent_threads(verification).await;
    (id, blockers)
}

/// `resume_record` reports "seeding" before the new worker adds the torrent,
/// so wait for the worker's recorded handle and the session entry instead.
#[cfg(unix)]
async fn wait_for_readded(manager: &TorrentManager, engine: &Session, id: &str) {
    wait_until("the new worker re-added the torrent", || {
        manager.handle(id).is_some() && session_torrents(engine) == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let job = manager.job(id).await.unwrap();
    assert_eq!((job.status.as_str(), job.error.as_deref()), ("seeding", None), "{job:?}");
    assert!(manager.handle(id).is_some(), "the worker retired after re-adding: {job:?}");
    assert_eq!(session_torrents(engine), 1);
}

#[cfg(unix)]
async fn release(verification: Verification, blockers: Vec<tokio::task::JoinHandle<()>>) {
    verification.release();
    for blocker in blockers {
        blocker.await.unwrap();
    }
}

// Regression for the reverted attempts: pause waited for a free torrent thread
// (the routed add ran under the manager's gate), and an abandoned add that had
// already registered stayed in the session, so the next resume failed with
// "already managed".
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_does_not_wait_for_blocked_torrent_threads_and_reaps_the_abandoned_add() {
    let _serial = TORRENT_THREAD_TESTS.lock().await;
    let profile = Profile::new();
    let (_, book) = share_fixture(&profile);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let verification = Verification::new();
    let (id, blockers) = share_blocked_after_registration(&manager, book, &verification).await;
    let engine = manager.engine_cell().unwrap().get().cloned().expect("engine");

    let started = Instant::now();
    manager.pause(&id).await.unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(1), "pause waited {elapsed:?} for a torrent thread");
    // The worker abandoned an add that librqbit had already registered.
    assert_eq!(session_torrents(&engine), 1);
    assert!(manager.handle(&id).is_none());
    assert_eq!(manager.job(&id).await.unwrap().status, "completed");

    clear_engine_hook(&manager);
    release(verification, blockers).await;
    wait_until("the abandoned add was reaped", || {
        session_torrents(&engine) == 0 && manager.pending_reaps(&id).unwrap().is_empty()
    })
    .await;

    // Sharing again adds the same torrent; it must not be "already managed".
    manager.seeding(&id, true).await.unwrap();
    wait_for_readded(&manager, &engine, &id).await;
    manager.shutdown().await.unwrap();
}

// The worker loop's unpause starts tracker tasks, so it runs on the torrent
// runtime outside the gate. Pausing while it is blocked must not wait for it,
// and the late unpause must not revive the deleted torrent.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_does_not_wait_for_a_blocked_worker_unpause() {
    let _serial = TORRENT_THREAD_TESTS.lock().await;
    let profile = Profile::new();
    let (_, book) = share_fixture(&profile);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let id = manager.share(book).await.unwrap().id;
    let job = wait_for_share(&manager, &id).await;
    assert_eq!(job.status, "seeding", "{job:?}");
    let engine = manager.engine_cell().unwrap().get().cloned().expect("engine");

    let verification = Verification::new();
    let unpausing = Arc::new(AtomicBool::new(false));
    let (seen, blocked) = (Arc::clone(&unpausing), verification.clone());
    install_engine_hook(&manager, move |call| {
        if call == EngineCall::Unpause {
            seen.store(true, Ordering::SeqCst);
            blocked.block();
        }
    });
    // The engine pauses the live torrent behind the worker's back; the worker
    // loop then unpauses it, as it does for a resume during a hash check.
    engine.pause(&manager.handle(&id).expect("live handle")).await.unwrap();
    wait_until("the worker started its unpause", || unpausing.load(Ordering::SeqCst)).await;
    let blockers = saturate_torrent_threads(&verification).await;

    let started = Instant::now();
    manager.pause(&id).await.unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(1), "pause waited {elapsed:?} for a torrent thread");
    assert_eq!(session_torrents(&engine), 0, "retirement deletes the handle");

    clear_engine_hook(&manager);
    release(verification, blockers).await;
    // The released unpause runs against a handle the session no longer holds;
    // nothing is registered again.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(session_torrents(&engine), 0);

    manager.seeding(&id, true).await.unwrap();
    wait_for_readded(&manager, &engine, &id).await;
    manager.shutdown().await.unwrap();
}

// The Android privacy/read-only and notification cutovers cancel or pause each
// job, then suspend the manager. Neither may wait for a torrent thread.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_and_suspend_do_not_wait_for_blocked_torrent_threads() {
    let _serial = TORRENT_THREAD_TESTS.lock().await;
    let profile = Profile::new();
    let (_, book) = share_fixture(&profile);
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let verification = Verification::new();
    let (id, blockers) = share_blocked_after_registration(&manager, book, &verification).await;
    let engine = manager.engine_cell().unwrap().get().cloned().expect("engine");

    let started = Instant::now();
    manager.cancel(&id).await.unwrap();
    let cancelled = started.elapsed();
    assert!(cancelled < Duration::from_secs(1), "cancel waited {cancelled:?}");
    let started = Instant::now();
    manager.suspend().await.unwrap();
    let suspended = started.elapsed();
    // librqbit's Session::stop always sleeps 1 s after cancelling its tasks.
    assert!(suspended < Duration::from_millis(2500), "suspend waited {suspended:?}");
    assert!(manager.engine_cell().unwrap().get().is_none(), "suspend retired the engine");
    assert_eq!(manager.job(&id).await.unwrap().status, "cancelled");

    clear_engine_hook(&manager);
    release(verification, blockers).await;
    wait_until("the abandoned add was reaped from the retired engine", || {
        session_torrents(&engine) == 0 && manager.pending_reaps(&id).unwrap().is_empty()
    })
    .await;
    manager.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn first_run_prefill_copies_only_donor_verified_ranges_into_journaled_staging() {
    let (profile, manager, id) = owned_terminal_fixture();
    let record = manager.record(&id).unwrap();
    let relative = PathBuf::from("fresh.zim");
    let size = 3 * 1024 * 1024 + 17;
    let donor_bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8 + 1).collect();
    let donor_path = profile.0.join("localswarm-partial.zim");
    fs::write(&donor_path, &donor_bytes).unwrap();
    let mib = 1024 * 1024u64;
    let ranges = vec![(0, mib), (2 * mib, size as u64 - 2 * mib)];
    let donors = vec![
        // Wrong size, absent range map and missing file are all ignored.
        crate::partial_donor::Donor { path: donor_path.clone(), size: size as u64 + 1, ranges: Some(ranges.clone()) },
        crate::partial_donor::Donor { path: donor_path.clone(), size: size as u64, ranges: None },
        crate::partial_donor::Donor { path: profile.0.join("missing.zim"), size: size as u64, ranges: Some(ranges.clone()) },
        crate::partial_donor::Donor { path: donor_path.clone(), size: size as u64, ranges: Some(ranges) },
    ];
    let factory = ConfinedFactory {
        root: Arc::new(open_staging(&record.staging).unwrap()),
        path: record.staging.clone(),
        expected: Arc::new(HashMap::from([(relative.clone(), size as u64), (PathBuf::from("other.bin"), 4)])),
        sources: Arc::new(Mutex::new(HashMap::new())),
        existing: false,
        stop: CancellationToken::new(),
        gate: manager.storage_gate(&id).unwrap(),
        dirs: Arc::new(Mutex::new(HashSet::new())),
        owner: Some(((*manager).clone(), id.clone())),
    };
    let files = vec![
        SpaceFile { path: relative.clone(), size: size as u64 },
        SpaceFile { path: PathBuf::from("other.bin"), size: 4 },
    ];
    assert!(prefill_from_donors(&factory, &donors, &files).unwrap());
    let staged = fs::read(record.staging.join(&relative)).unwrap();
    assert_eq!(staged.len(), size);
    assert_eq!(staged[..mib as usize], donor_bytes[..mib as usize]);
    assert!(staged[mib as usize..2 * mib as usize].iter().all(|byte| *byte == 0));
    assert_eq!(staged[2 * mib as usize..], donor_bytes[2 * mib as usize..]);
    // Files without a donor are left for the engine to create.
    assert!(!record.staging.join("other.bin").exists());
    // Ownership is journaled as for an engine-created file, so the engine's
    // own open of the existing file passes the replacement check.
    let journal = manager.record(&id).unwrap();
    assert!(journal.staging_cleanup.iter().any(|(path, _)| path == &relative));
    let engine = ConfinedFactory { existing: true, ..factory.clone() };
    engine.open_file(&relative, size as u64).unwrap();
    // The donor is only read.
    assert_eq!(fs::read(&donor_path).unwrap(), donor_bytes);
    // No usable donor: nothing is created.
    let none = ConfinedFactory { expected: Arc::new(HashMap::from([(PathBuf::from("none.zim"), 9)])), ..factory };
    assert!(!prefill_from_donors(&none, &donors, &[SpaceFile { path: PathBuf::from("none.zim"), size: 9 }]).unwrap());
    assert!(!record.staging.join("none.zim").exists());
}

#[tokio::test]
async fn web_seed_only_torrent_completes_through_the_engine() {
    use sha1::{Digest, Sha1};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let body = TEST_ARCHIVE.to_vec();
    let piece_length = 5usize;
    let pieces: Vec<u8> = body
        .chunks(piece_length)
        .flat_map(|chunk| Sha1::digest(chunk).to_vec())
        .collect();
    let bstr = |value: &[u8]| {
        let mut out = format!("{}:", value.len()).into_bytes();
        out.extend_from_slice(value);
        out
    };
    let info = {
        let mut out = b"d".to_vec();
        out.extend(bstr(b"length"));
        out.extend(format!("i{}e", body.len()).into_bytes());
        out.extend(bstr(b"name"));
        out.extend(bstr(b"map.zim"));
        out.extend(bstr(b"piece length"));
        out.extend(format!("i{piece_length}e").into_bytes());
        out.extend(bstr(b"pieces"));
        out.extend(bstr(&pieces));
        out.push(b'e');
        out
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let metadata = {
        let mut out = b"d".to_vec();
        out.extend(bstr(b"info"));
        out.extend(info);
        out.extend(bstr(b"url-list"));
        out.extend(bstr(format!("http://127.0.0.1:{port}/map.zim").as_bytes()));
        out.push(b'e');
        out
    };
    let served_body = body.clone();
    std::thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else { return; };
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let (status, payload) = if request.starts_with("GET /map.torrent ") {
                ("200 OK", metadata.clone())
            } else if request.starts_with("GET /map.zim ") {
                ("200 OK", served_body.clone())
            } else {
                ("404 Not Found", Vec::new())
            };
            let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len());
            let _ = stream.write_all(&payload);
        }
    });

    let profile = Profile::new();
    let manager = TorrentManager::new(profile.0.clone(), false, profile.1.clone()).unwrap();
    let job = manager
        .start(format!("http://127.0.0.1:{port}/map.torrent"), profile.0.clone(), None)
        .await
        .unwrap();
    let id = job.id.clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    let completed = loop {
        let jobs = manager.list().await;
        let current = jobs.iter().find(|job| job.id == id).unwrap();
        if current.status == "completed" {
            break current.clone();
        }
        if current.status == "error" {
            panic!("web-seed torrent failed: {:?}", current.error);
        }
        assert!(Instant::now() < deadline, "web-seed torrent did not complete");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(std::fs::read(&completed.completed_paths[0]).unwrap(), body);
    manager.suspend().await.unwrap();
}
