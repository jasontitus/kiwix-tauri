// SPDX-License-Identifier: GPL-3.0-or-later
//! Unit tests for `maps`, kept beside it rather than inline.
#![forbid(unsafe_code)]
use super::*;

struct Fixture {
    root: PathBuf,
    bytes: Vec<u8>,
    store: Arc<crate::store::Store>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("streetzim-composer-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        Self {
            store: Arc::new(crate::store::Store::new(root.join("profile")).unwrap()),
            root,
            bytes: include_bytes!("../tests/fixtures/small.zim").to_vec(),
        }
    }

    fn manager(&self) -> MapManager {
        self.manager_in(self.root.join("state"))
    }

    fn manager_in(&self, state_directory: PathBuf) -> MapManager {
        let manager = MapManager::new(state_directory, self.store.clone()).unwrap();
        let job = MapJob {
            id: "offline-fixture".into(),
            status: "composing".into(),
            created_at: None,
            region_id: "fixture".into(),
            language: "en".into(),
            written: 0,
            total: self.bytes.len() as u64,
            incoming: 0,
            incoming_total: 0,
            speed: 0,
            error: None,
            path: None,
            history_dismissed: false,
            owned_partial: None,
            recovery_records: Vec::new(),
            monitored_paths: Vec::new(),
            retained_publications: Vec::new(),
            blocks_total: None, blocks_ready: None, blocks_downloaded: None, decision: None,
            resumable: false, resume: None, published: None,
            can_remove_from_list: false,
            recovery_paths: Vec::new(),
        };
        persist(&manager.inner.state_path, &job).unwrap();
        {
            let mut state = manager.inner.state.lock().unwrap();
            state.job = Some(job);
            state.active = Some(Arc::new(Control::new()));
        }
        manager
    }

    fn interrupted_partial(&self, manager: &MapManager, destination: &Path) -> OwnedPartial {
        fs::create_dir_all(destination).unwrap();
        let directory = MapDirectory::open(destination).unwrap();
        let mut record = OwnedPartial {
            destination: destination.to_path_buf(),
            directory_identity: directory.identity,
            name: format!(".streetzim-{}.part", Uuid::new_v4()),
            identity: None,
            capture: None,
            publication: None,
        };
        manager
            .checkpoint_partial("offline-fixture", Some(record.clone()))
            .unwrap();
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = directory
            .directory
            .open_with(&record.name, &options)
            .unwrap();
        record.identity = Some(FileIdentity::of(&file.metadata().unwrap()));
        manager
            .checkpoint_partial("offline-fixture", Some(record.clone()))
            .unwrap();
        use std::io::Write;
        let mut file = file.into_std();
        file.write_all(b"unverified interrupted output").unwrap();
        file.sync_all().unwrap();
        record
    }

    fn publication_checkpoint(&self, manager: &MapManager, destination: &Path) -> OwnedPartial {
        let mut record = self.interrupted_partial(manager, destination);
        let path = destination.join(&record.name);
        fs::write(&path, &self.bytes).unwrap();
        // Windows FlushFileBuffers requires write access (a read-only handle
        // fails with ERROR_ACCESS_DENIED); fsync accepts either on Unix.
        fs::OpenOptions::new().write(true).open(&path).unwrap().sync_all().unwrap();
        record.publication = Some(PublicationRecovery {
            name: "publication-recovery.zim".into(),
            size: self.bytes.len() as u64,
            sha256: None,
        });
        manager
            .checkpoint_partial("offline-fixture", Some(record.clone()))
            .unwrap();
        record
    }

    fn recipe(&self) -> Recipe {
        let bytes = &self.bytes;
        assert!(bytes.len() > 80);
        let checksum_offset = bytes.len() - 16;
        let checksum = md5::compute(&bytes[..checksum_offset]).0;
        assert_eq!(
            &checksum[..],
            &bytes[checksum_offset..],
            "fixture must have a real ZIM checksum"
        );
        let uuid = Uuid::from_slice(&bytes[8..24]).unwrap().to_string();
        let mut masked = bytes.clone();
        masked[32..64].fill(0);
        let hashes: Vec<String> = masked
            .chunks(1024)
            .map(|chunk| hex::encode(Sha256::digest(chunk)))
            .collect();
        serde_json::from_value(serde_json::json!({
            "format": "streetzim-recipe/2", "uuid": uuid, "name": "osm_en_fixture",
            "size": bytes.len(), "md5": hex::encode(checksum),
            "file_md5": hex::encode(md5::compute(bytes).0),
            "base": {
                "file": "fixture-base.zim", "size": bytes.len(), "uuid": uuid,
                "chunk": 1024, "sha256": hashes, "mask": [[32,32]]
            },
            "packs": {},
            "segments": [
                {"src":"base", "off":0, "len":32},
                {"src":"inline", "len":32, "data":STANDARD.encode(&bytes[32..64]),
                 "sha256":hex::encode(Sha256::digest(&bytes[32..64]))},
                {"src":"base", "off":64, "len":checksum_offset-64},
                {"src":"md5"}
            ]
        }))
        .unwrap()
    }

    fn candidate(&self) -> PathBuf {
        let path = self.root.join("read-only-source.zim");
        fs::write(&path, &self.bytes).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

async fn paused_output(fixture: &Fixture) -> (MapManager, PathBuf, MapJob) {
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let mut control = Control::new();
    control.pause_after = Some(1024);
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &destination, vec![fixture.candidate()]).await;
    assert_eq!(result.as_ref().unwrap_err(), CANCELLED);
    manager.finish("offline-fixture", &control, result);
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "paused");
    assert!(job.resumable);
    assert!(job.written >= 1024 && job.written < job.total);
    let record = job.owned_partial.as_ref().unwrap();
    let partial = record.destination.join(&record.name);
    assert!(partial.is_file());
    (manager, partial, job)
}

#[tokio::test]
async fn durable_pause_restart_resumes_the_same_inode_and_exact_prefix_without_rebuilding() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let identity = paused.owned_partial.as_ref().unwrap().identity.clone().unwrap();
    let prefix = fs::read(&partial).unwrap();
    assert_eq!(prefix, fixture.bytes[..paused.written as usize]);
    // Simulate writes past the last durable journal checkpoint at a crash.
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(&partial).unwrap().write_all(b"uncommitted-tail").unwrap();
    drop(manager);
    let restored = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let retained = restored.status().unwrap().unwrap();
    assert_eq!(retained.status, "paused");
    assert_eq!(retained.written, paused.written);
    assert!(retained.resumable && partial.exists());
    let control = Arc::new(Control::new());
    restored.inner.state.lock().unwrap().active = Some(control.clone());
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = restored.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &fixture.root.join("output"), vec![fixture.candidate()]).await;
    let published = result.as_ref().unwrap().clone();
    restored.finish("offline-fixture", &control, result);
    assert_eq!(fs::read(&published).unwrap(), fixture.bytes);
    let directory = MapDirectory::open(published.parent().unwrap()).unwrap();
    assert_eq!(FileIdentity::of(&directory.directory.symlink_metadata(published.file_name().unwrap()).unwrap()), identity);
    assert!(!partial.exists());
    let complete = restored.status().unwrap().unwrap();
    assert_eq!(complete.status, "complete");
    assert!(!complete.resumable);
    assert_eq!(complete.blocks_ready, complete.blocks_total);
    assert_eq!(complete.blocks_downloaded, Some(0));
    assert!(complete.published.is_some());
}

#[tokio::test]
async fn resume_rejects_changed_prefix_inode_links_and_recipe_before_appending() {
    for change in ["prefix", "inode", "hardlink", "recipe"] {
        let fixture = Fixture::new();
        let (manager, partial, _) = paused_output(&fixture).await;
        let mut recipe = fixture.recipe();
        match change {
            "prefix" => {
                let mut bytes = fs::read(&partial).unwrap();
                bytes[0] ^= 1;
                fs::write(&partial, bytes).unwrap();
            }
            "inode" => {
                fs::rename(&partial, fixture.root.join("retained-original.part")).unwrap();
                fs::write(&partial, vec![b'x'; 2048]).unwrap();
            }
            "hardlink" => { fs::hard_link(&partial, fixture.root.join("external-link")).unwrap(); }
            "recipe" => { recipe.md5 = "0".repeat(32); }
            _ => unreachable!(),
        }
        let before = fs::read(&partial).unwrap();
        let control = Arc::new(Control::new());
        manager.inner.state.lock().unwrap().active = Some(control.clone());
        let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
        let result = manager.compose("offline-fixture", &control, recipe,
            url.as_str(), &url, &fixture.root.join("output"), Vec::new()).await;
        assert!(result.is_err(), "{change}");
        manager.finish("offline-fixture", &control, result);
        assert_eq!(fs::read(&partial).unwrap(), before, "{change}");
        assert!(manager.status().unwrap().unwrap().path.is_none());
        if change == "prefix" {
            let failed = manager.status().unwrap().unwrap();
            assert!(!failed.resumable);
            assert!(failed.resume.as_ref().unwrap().invalid);
            assert!(!failed.can_remove_from_list);
        }
    }
}

#[tokio::test]
async fn deleting_a_paused_map_requires_unreserved_owned_identity_and_preserves_shared_cache() {
    let fixture = Fixture::new();
    let (manager, partial, _) = paused_output(&fixture).await;
    let source = fixture.candidate();
    let mut reserved = crate::imports::load_book(source.to_str().unwrap(), false, false).unwrap();
    reserved.path = partial.to_string_lossy().into_owned();
    fixture.store.update(|state| { state.books.push(reserved); Ok(()) }).unwrap();
    let before = fs::read(&partial).unwrap();
    assert!(manager.delete_download("offline-fixture").is_err());
    assert_eq!(fs::read(&partial).unwrap(), before);
    // A library reservation is retryable and proves nothing about the file:
    // the owned record must stay reclaimable instead of becoming a warning.
    let refused = manager.status().unwrap().unwrap();
    assert!(refused.owned_partial.is_some() && refused.resume.is_some());
    assert!(refused.recovery_records.is_empty() && refused.recovery_paths.is_empty());
    fixture.store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
    let sentinel = manager.inner.cache.join("unrelated-cache-entry");
    fs::write(&sentinel, b"shared-cache-must-stay").unwrap();
    manager.delete_download("offline-fixture").unwrap();
    assert!(!partial.exists());
    assert!(manager.status().unwrap().is_none());
    assert_eq!(fs::read(sentinel).unwrap(), b"shared-cache-must-stay");
    assert_eq!(fs::read(source).unwrap(), fixture.bytes);
}

fn start_another_region(manager: &MapManager, fixture: &Fixture) -> Result<MapJob, String> {
    manager.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "another-region".into(), "fr".into(), fixture.root.join("next-output"), Vec::new(), None, None,
    )
}

/// Make the paused map's retained prefix impossible to prove or reach, the way
/// a moved folder or a rewriting sync tool does. Returns where the original
/// bytes now live and, if one took its place, the foreign file at the old path.
fn orphan_partial(fixture: &Fixture, partial: &Path, change: &str) -> (PathBuf, Option<PathBuf>) {
    let output = partial.parent().unwrap();
    let name = partial.file_name().unwrap();
    match change {
        "folder recreated" => {
            let moved = fixture.root.join("moved-output");
            fs::rename(output, &moved).unwrap();
            fs::create_dir(output).unwrap();
            (moved.join(name), None)
        }
        "folder missing" => {
            let moved = fixture.root.join("moved-output");
            fs::rename(output, &moved).unwrap();
            (moved.join(name), None)
        }
        "file replaced" => {
            let original = fixture.root.join("retained-original.part");
            fs::rename(partial, &original).unwrap();
            fs::write(partial, b"not written by this download").unwrap();
            (original, Some(partial.to_path_buf()))
        }
        _ => unreachable!(),
    }
}

/// Nothing but the map's own output directory entries, hidden ones included.
fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory).unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn deleting_a_map_whose_folder_is_unreachable_keeps_it_until_delete_can_finish() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let prefix = fs::read(&partial).unwrap();
    let output = partial.parent().unwrap().to_path_buf();
    let (moved, _) = orphan_partial(&fixture, &partial, "folder missing");
    // The retained checkpoint still vetoes another map until it is given up.
    assert!(start_another_region(&manager, &fixture).unwrap_err().contains("previous map download"));
    // Delete could not remove the file: it says so, where it is, and why.
    let error = manager.delete_download(&paused.id).unwrap_err();
    assert!(error.contains("was kept at") && error.contains(partial.to_str().unwrap()), "{error}");
    let kept = manager.status().unwrap().expect("a download with kept files stays listed");
    assert_eq!(kept.status, "cancelled");
    assert!(kept.resume.is_none() && kept.owned_partial.is_none() && !kept.resumable);
    assert_eq!(kept.recovery_paths, vec![partial.to_string_lossy().into_owned()]);
    assert!(kept.error.as_deref().is_some_and(|text|
        text.contains("could not be cleaned up") && text.contains(partial.to_str().unwrap())));
    assert!(!kept.can_remove_from_list);
    assert_eq!(fs::read(&moved).unwrap(), prefix);
    // Still unreachable: the retry fails the same way and nothing is forgotten.
    assert!(manager.delete_download(&paused.id).unwrap_err().contains(partial.to_str().unwrap()));
    assert_eq!(manager.status().unwrap().unwrap().recovery_records.len(), 1);
    // The kept record is durable. Once the folder is back, the same button
    // finishes the job: the proven partial is removed and no warning remains.
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert_eq!(manager.status().unwrap().unwrap().recovery_records.len(), 1);
    fs::rename(moved.parent().unwrap(), &output).unwrap();
    assert_eq!(fs::read(&partial).unwrap(), prefix);
    manager.delete_download(&paused.id).unwrap();
    assert!(entries(&output).is_empty(), "{:?}", entries(&output));
    assert!(manager.status().unwrap().is_none());
    start_another_region(&manager, &fixture).unwrap();
    manager.cancel().await.unwrap();
}

#[tokio::test]
async fn deleting_a_map_whose_folder_was_replaced_says_so_once_and_reports_nothing_that_is_not_there() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let prefix = fs::read(&partial).unwrap();
    let (moved, _) = orphan_partial(&fixture, &partial, "folder recreated");
    let error = manager.delete_download(&paused.id).unwrap_err();
    assert!(error.contains("moved or changed"), "{error}");
    let kept = manager.status().unwrap().unwrap();
    assert_eq!(kept.recovery_records.len(), 1);
    assert!(kept.error.as_deref().is_some_and(|text| text.contains("moved or changed")), "the reason stays on the job");
    // The new folder holds nothing this download named, so the next Delete
    // has nothing to report; the bytes the user moved away were never touched.
    manager.delete_download(&paused.id).unwrap();
    assert!(manager.status().unwrap().is_none());
    assert!(entries(partial.parent().unwrap()).is_empty());
    assert_eq!(fs::read(&moved).unwrap(), prefix);
}

#[tokio::test]
async fn a_failed_cleanup_capture_does_not_strand_the_partial() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let prefix = fs::read(&partial).unwrap();
    // For example a Windows indexer still holding the file that was just closed.
    manager.inner.fail_capture.store(true, std::sync::atomic::Ordering::SeqCst);
    let error = manager.delete_download(&paused.id).unwrap_err();
    assert!(error.contains("injected capture failure") && error.contains(partial.to_str().unwrap()), "{error}");
    assert_eq!(fs::read(&partial).unwrap(), prefix, "the file never left its place");
    let kept = manager.status().unwrap().unwrap();
    assert_eq!(kept.recovery_paths, vec![partial.to_string_lossy().into_owned()]);
    assert!(kept.error.as_deref().is_some_and(|text| text.contains("injected capture failure")));
    assert!(kept.recovery_records[0].capture.is_some(), "the journaled capability is what the retry resumes");
    manager.inner.fail_capture.store(false, std::sync::atomic::Ordering::SeqCst);
    manager.delete_download(&paused.id).unwrap();
    assert!(entries(partial.parent().unwrap()).is_empty(), "{:?}", entries(partial.parent().unwrap()));
    assert!(manager.status().unwrap().is_none());
}

#[tokio::test]
async fn a_partial_stuck_inside_its_cleanup_capability_is_reported_where_it_is() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let mut record = fixture.interrupted_partial(&manager, &destination);
    // A cleanup captured the entry, then could neither finish nor put it back.
    let capability = format!(".streetzim-cleanup-{}", Uuid::new_v4());
    fs::create_dir(destination.join(&capability)).unwrap();
    let directory = MapDirectory::open(&destination).unwrap();
    record.capture = Some(PartialCapture {
        name: capability.clone(),
        identity: FileIdentity::of(&directory.directory.open_dir_nofollow(&capability).unwrap().dir_metadata().unwrap()),
        size: fs::metadata(destination.join(&record.name)).unwrap().len(),
    });
    let journaled = destination.join(&record.name);
    let captured = destination.join(&capability).join(&record.name);
    fs::rename(&journaled, &captured).unwrap();
    {
        let mut state = manager.inner.state.lock().unwrap();
        state.active = None;
        let job = state.job.as_mut().unwrap();
        job.status = "failed".into();
        job.owned_partial = None;
        job.recovery_records.push(record.clone());
        job.append_recovery_warning();
        persist(&manager.inner.state_path, job).unwrap();
    }
    let status = manager.status().unwrap().unwrap();
    assert_eq!(status.recovery_paths, vec![captured.to_string_lossy().into_owned()]);
    assert!(status.error.as_deref().is_some_and(|text| text.contains(captured.to_str().unwrap())));
    // Either spelling still names the record for a dismissal.
    assert!(MapJob::recovery_path_matches(&record, &captured) && MapJob::recovery_path_matches(&record, &journaled));
    assert!(!MapJob::recovery_path_matches(&record, &destination.join("something-else.part")));
    // Delete resumes the journaled cleanup of the inode it can still prove.
    manager.delete_download("offline-fixture").unwrap();
    assert!(entries(&destination).is_empty(), "{:?}", entries(&destination));
    assert!(manager.status().unwrap().is_none());
}

#[tokio::test]
async fn delete_reclaims_a_proven_partial_that_an_automatic_cancel_had_to_keep() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let prefix = fs::read(&partial).unwrap();
    let output = partial.parent().unwrap().to_path_buf();
    let (moved, _) = orphan_partial(&fixture, &partial, "folder missing");
    // Network retirement and mode changes cancel unattended. A kept file is
    // recorded on the job, not reported as a cancellation that failed.
    manager.cancel().await.unwrap();
    let cancelled = manager.status().unwrap().unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.resume.is_none() && cancelled.owned_partial.is_none());
    assert_eq!(cancelled.recovery_paths, vec![partial.to_string_lossy().into_owned()]);
    assert!(cancelled.error.as_deref().is_some_and(|text| text.contains("could not be cleaned up")));
    manager.cancel().await.unwrap();
    assert_eq!(fs::read(&moved).unwrap(), prefix, "cancel never retries a kept record");
    assert_eq!(manager.status().unwrap().unwrap().recovery_records.len(), 1);
    // The volume returns; the user's Delete removes what cancel could not.
    fs::rename(moved.parent().unwrap(), &output).unwrap();
    manager.cancel().await.unwrap();
    assert_eq!(fs::read(&partial).unwrap(), prefix, "only explicit Delete reclaims a kept record");
    manager.delete_download(&paused.id).unwrap();
    assert!(entries(&output).is_empty(), "{:?}", entries(&output));
    assert!(manager.status().unwrap().is_none());
}

#[tokio::test]
async fn cancelling_a_map_whose_partial_cannot_be_reclaimed_demotes_it_instead_of_failing_forever() {
    for change in ["folder recreated", "file replaced"] {
        let fixture = Fixture::new();
        let (manager, partial, _) = paused_output(&fixture).await;
        let prefix = fs::read(&partial).unwrap();
        let (original, foreign) = orphan_partial(&fixture, &partial, change);
        manager.cancel().await.unwrap();
        let cancelled = manager.status().unwrap().unwrap();
        assert_eq!(cancelled.status, "cancelled", "{change}");
        assert!(cancelled.resume.is_none() && cancelled.owned_partial.is_none(), "{change}");
        assert_eq!(cancelled.recovery_paths, vec![partial.to_string_lossy().into_owned()], "{change}");
        assert!(cancelled.error.as_deref().is_some_and(|error| error.contains("could not be cleaned up")), "{change}");
        assert_eq!(fs::read(&original).unwrap(), prefix, "{change}");
        if let Some(foreign) = &foreign {
            assert_eq!(fs::read(foreign).unwrap(), b"not written by this download", "{change}");
        }
        let next = start_another_region(&manager, &fixture).unwrap();
        assert_eq!(next.recovery_records.len(), 1, "{change}");
        manager.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn a_deleted_map_with_kept_files_reports_them_until_each_warning_is_dismissed() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let (original, _) = orphan_partial(&fixture, &partial, "file replaced");
    let prefix = fs::read(&original).unwrap();
    // The call that has to keep something says so. The entry at that name is
    // not the inode this download wrote, so nothing there is Kiwix's to remove:
    // a repeated Delete has nothing left to do and only the warning remains.
    let error = manager.delete_download(&paused.id).unwrap_err();
    assert!(error.contains(partial.to_str().unwrap()) && error.contains("not the one this download wrote"), "{error}");
    manager.delete_download(&paused.id).unwrap();
    let repeated = manager.status().unwrap().unwrap();
    assert!(repeated.error.as_deref().is_some_and(|text|
        text.contains("could not be cleaned up") && !text.contains("not the one this download wrote")));
    // The IPC shape: exactly the strings `map_forget_recovery` takes as `path`,
    // and nothing of the kind is written to the journal.
    let status = serde_json::to_value(manager.status().unwrap().unwrap()).unwrap();
    assert_eq!(status["recoveryPaths"], serde_json::json!([partial.to_str().unwrap()]));
    assert_eq!(status["canRemoveFromList"], false);
    assert!(status.get("etaSeconds").is_none());
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(&manager.inner.state_path).unwrap()).unwrap();
    assert!(journal.get("recoveryPaths").is_none());
    assert!(manager.remove_terminal(&paused.id).is_err(), "hiding the row would hide the kept file");
    let reported = PathBuf::from(status["recoveryPaths"][0].as_str().unwrap());
    manager.forget_recovery(&reported).unwrap();
    assert!(manager.forget_recovery(&reported).is_err(), "a dismissed entry is gone");
    // Dismissal never touches the file it stops reporting.
    assert_eq!(fs::read(&partial).unwrap(), b"not written by this download");
    assert_eq!(fs::read(&original).unwrap(), prefix);
    let dismissed = manager.status().unwrap().unwrap();
    assert!(dismissed.recovery_paths.is_empty() && dismissed.error.is_none());
    assert!(dismissed.can_remove_from_list);
    manager.delete_download(&paused.id).unwrap();
    assert!(manager.status().unwrap().is_none());
    start_another_region(&manager, &fixture).unwrap();
    manager.cancel().await.unwrap();
}

#[test]
fn a_journal_written_by_the_previous_build_loads_unchanged() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let recovery = fixture.interrupted_partial(&manager, &fixture.root.join("older-output"));
    {
        let mut state = manager.inner.state.lock().unwrap();
        state.active = None;
        let job = state.job.as_mut().unwrap();
        job.status = "failed".into();
        job.owned_partial = None;
        job.recovery_records.push(recovery.clone());
        persist(&manager.inner.state_path, job).unwrap();
    }
    // The previous build also journaled these two mid-download fields.
    let mut journal: serde_json::Value =
        serde_json::from_slice(&fs::read(&manager.inner.state_path).unwrap()).unwrap();
    journal["speed"] = 4096.into();
    journal["etaSeconds"] = 77.into();
    fs::write(&manager.inner.state_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    drop(manager);
    let restored = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let job = restored.status().unwrap().unwrap();
    assert_eq!((job.status.as_str(), job.speed), ("failed", 0));
    assert_eq!(job.recovery_records, vec![recovery.clone()]);
    assert_eq!(job.recovery_paths,
        vec![recovery.destination.join(&recovery.name).to_string_lossy().into_owned()]);
}

#[tokio::test]
async fn a_write_the_file_rejects_is_reported_before_any_counter_or_checkpoint_advances() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let record = fixture.interrupted_partial(&manager, &fixture.root.join("output"));
    let path = record.destination.join(&record.name);
    // A handle the operating system refuses to write through stands in for a
    // full volume: the failure happens in Tokio's queued blocking write.
    let read_only = || tokio::fs::File::from_std(fs::File::open(&path).unwrap());
    let mut output = read_only();
    assert!(write_observed(&mut output, b"rejected").await.unwrap_err().starts_with("Map write failed"));
    // A raw write is acknowledged before the file has accepted it. A checkpoint
    // taken then must fail instead of journaling bytes that are not on disk.
    let mut output = read_only();
    let _ = output.write_all(b"rejected").await;
    let recipe = fixture.recipe();
    let control = Control::new();
    let blocks = BlockProgress::new(&recipe, &control).unwrap();
    let failure = manager.checkpoint_output("offline-fixture", &mut output, "fingerprint", 8,
        &Sha256::new(), &blocks).await.unwrap_err();
    assert!(failure.message.starts_with("Map write failed"), "{}", failure.message);
    assert!(failure.retain && !failure.checkpoint_current, "the failure path must not journal these bytes either");
    let job = manager.status().unwrap().unwrap();
    assert!(job.resume.is_none() && !job.resumable && job.written == 0);
    assert_eq!(fs::read(&path).unwrap(), b"unverified interrupted output");
}

#[tokio::test]
async fn a_block_served_without_the_network_clears_the_stale_transfer_rate() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    // An earlier block of this job came over the network at this rate.
    manager.update("offline-fixture", |job| job.speed = 4_000_000);
    let mut control = Control::new();
    control.pause_after = Some(1024);
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &fixture.root.join("output"), vec![fixture.candidate()]).await;
    assert_eq!(result.as_ref().unwrap_err(), CANCELLED);
    // Still the live job: `finish` has not run and would reset the rate anyway.
    assert_eq!(manager.status().unwrap().unwrap().speed, 0);
    manager.finish("offline-fixture", &control, result);
}

#[tokio::test]
async fn a_checkpoint_whose_sync_fails_keeps_the_last_durable_checkpoint() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    // The creation checkpoint (0 bytes) is durable; the next one cannot sync.
    *manager.inner.fail_sync_from.lock().unwrap() = Some(1);
    let control = Control::new();
    let result = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &destination, vec![fixture.candidate()]).await;
    assert!(result.as_ref().unwrap_err().starts_with("Cannot sync map prefix"), "{result:?}");
    manager.finish("offline-fixture", &control, result);
    let failed = manager.status().unwrap().unwrap();
    assert_eq!(failed.status, "failed");
    // Bytes whose sync failed are not journaled by the failure path either.
    assert_eq!(failed.resume.as_ref().unwrap().written, 0);
    assert!(failed.resumable && failed.written == 0);
    // The durable checkpoint still resumes into a correct archive.
    *manager.inner.fail_sync_from.lock().unwrap() = None;
    let control = Arc::new(Control::new());
    manager.inner.state.lock().unwrap().active = Some(control.clone());
    let result = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &destination, vec![fixture.candidate()]).await;
    let published = result.as_ref().unwrap().clone();
    manager.finish("offline-fixture", &control, result);
    assert_eq!(fs::read(published).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn completed_map_cleanup_proves_the_original_inode_and_never_deletes_a_new_path_occupant() {
    for substitute in [false, true] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let control = Control::new();
        let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
        let result = manager.compose("offline-fixture", &control, fixture.recipe(),
            url.as_str(), &url, &fixture.root.join("output"), vec![fixture.candidate()]).await;
        let path = result.as_ref().unwrap().clone();
        manager.finish("offline-fixture", &control, result);
        assert_eq!(manager.validate_terminal_owned_paths("offline-fixture").unwrap(), vec![path.clone()]);
        if substitute {
            fs::rename(&path, fixture.root.join("original-download.zim")).unwrap();
            fs::write(&path, vec![b'v'; fixture.bytes.len()]).unwrap();
            assert!(manager.validate_terminal_owned_paths("offline-fixture").is_err());
            assert!(manager.delete_download("offline-fixture").is_err());
            assert_eq!(fs::read(&path).unwrap(), vec![b'v'; fixture.bytes.len()]);
            assert_eq!(fs::read(fixture.root.join("original-download.zim")).unwrap(), fixture.bytes);
        } else {
            let book = crate::imports::load_book(path.to_str().unwrap(), true, true).unwrap();
            fixture.store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
            assert!(manager.delete_download("offline-fixture").is_err());
            assert_eq!(fs::read(&path).unwrap(), fixture.bytes);
            fixture.store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
            manager.delete_download("offline-fixture").unwrap();
            assert!(!path.exists());
        }
    }
}

async fn replaced_download(fixture: &Fixture) -> (MapManager, PathBuf, PathBuf, String) {
    let manager = fixture.manager();
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let destination = fixture.root.join("output");
    let original = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &destination, vec![fixture.candidate()]).await.unwrap();
    manager.finish("offline-fixture", &control, Ok(original.clone()));
    let book = crate::imports::load_book(original.to_str().unwrap(), true, true).unwrap();
    fixture.store.update(|state| {
        state.books.push(book);
        state.consumed_acquisitions.insert("map:offline-fixture".into());
        Ok(())
    }).unwrap();
    let next = manager.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "replacement".into(), "en".into(), destination.clone(), Vec::new(), None, None,
    ).unwrap();
    // Join the normal network worker before composing the local verified fixture.
    manager.cancel().await.unwrap();
    let control = Control::new();
    let replacement = manager.compose(&next.id, &control, fixture.recipe(),
        url.as_str(), &url, &destination, vec![fixture.candidate()]).await.unwrap();
    manager.finish(&next.id, &control, Ok(replacement.clone()));
    let book = crate::imports::load_book(replacement.to_str().unwrap(), true, true).unwrap();
    fixture.store.update(|state| {
        state.books = vec![book];
        state.consumed_acquisitions.insert(format!("map:{}", next.id));
        Ok(())
    }).unwrap();
    (manager, original, replacement, next.id)
}

#[tokio::test]
async fn replacement_retains_native_publication_across_restart_and_explicit_cleanup() {
    let fixture = Fixture::new();
    let (manager, original, replacement, current_id) = replaced_download(&fixture).await;
    assert_ne!(original, replacement);
    let cached = manager.inner.cache.join("retained-cache-sentinel");
    fs::write(&cached, b"shared verified parts").unwrap();
    let rows = manager.retained_jobs().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "offline-fixture");
    assert_eq!(rows[0].completed_paths, vec![original.to_str().unwrap()]);
    assert!(!rows[0].can_remove_from_list);
    // Retained rows are delete-only; hiding them would strand the file.
    assert!(manager.remove_terminal("offline-fixture").is_err());
    assert_eq!(fs::read(&original).unwrap(), fixture.bytes);
    assert_eq!(manager.status().unwrap().unwrap().id, current_id);
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert_eq!(manager.retained_jobs().unwrap().len(), 1);
    assert_eq!(manager.validate_terminal_owned_paths("offline-fixture").unwrap(), vec![original.clone()]);
    manager.delete_download("offline-fixture").unwrap();
    assert!(!original.exists());
    assert_eq!(fs::read(&replacement).unwrap(), fixture.bytes);
    assert_eq!(fs::read(&cached).unwrap(), b"shared verified parts");
    assert_eq!(manager.status().unwrap().unwrap().id, current_id);
    assert!(manager.retained_jobs().unwrap().is_empty());
    assert!(manager.delete_download(&current_id).is_err(), "installed replacement remains protected");
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert!(manager.retained_jobs().unwrap().is_empty());
    assert!(manager.delete_download("offline-fixture").is_err());
    assert_eq!(fs::read(replacement).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn deleting_current_replacement_does_not_discard_its_retained_predecessor() {
    let fixture = Fixture::new();
    let (manager, original, replacement, current_id) = replaced_download(&fixture).await;
    fixture.store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
    manager.delete_download(&current_id).unwrap();
    assert!(!replacement.exists());
    assert_eq!(fs::read(&original).unwrap(), fixture.bytes);
    assert_eq!(manager.retained_jobs().unwrap().len(), 1);
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert_eq!(manager.retained_jobs().unwrap()[0].id, "offline-fixture");
    manager.delete_download("offline-fixture").unwrap();
    assert!(!original.exists());
    assert!(manager.retained_jobs().unwrap().is_empty());
}

#[tokio::test]
async fn successive_map_jobs_preserve_every_native_predecessor_receipt() {
    let fixture = Fixture::new();
    let (manager, original, replacement, second_id) = replaced_download(&fixture).await;
    manager.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "third-region".into(), "en".into(), fixture.root.join("output"), Vec::new(), None, None,
    ).unwrap();
    manager.cancel().await.unwrap();
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let rows = manager.retained_jobs().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "offline-fixture");
    assert_eq!(rows[1].id, second_id);
    manager.delete_download("offline-fixture").unwrap();
    assert!(!original.exists());
    assert!(manager.delete_download(&second_id).is_err(), "installed predecessor stays protected");
    assert_eq!(fs::read(&replacement).unwrap(), fixture.bytes);
    fixture.store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
    manager.delete_download(&second_id).unwrap();
    assert!(!replacement.exists());
    assert!(manager.retained_jobs().unwrap().is_empty());
}

#[tokio::test]
async fn retained_map_cleanup_rejects_replaced_modified_linked_or_installed_predecessors() {
    for mutation in ["inode", "contents", "directory", "symlink", "hardlink", "installed"] {
        let fixture = Fixture::new();
        let (manager, original, replacement, _) = replaced_download(&fixture).await;
        match mutation {
            "inode" => {
                fs::rename(&original, fixture.root.join("preserved.zim")).unwrap();
                fs::write(&original, &fixture.bytes).unwrap();
            }
            "contents" => { fs::write(&original, vec![b'v'; fixture.bytes.len()]).unwrap(); }
            "directory" => {
                fs::rename(original.parent().unwrap(), fixture.root.join("preserved-directory")).unwrap();
                fs::create_dir(original.parent().unwrap()).unwrap();
                fs::write(&original, &fixture.bytes).unwrap();
                fs::write(&replacement, &fixture.bytes).unwrap();
            }
            "symlink" => {
                fs::rename(&original, fixture.root.join("preserved.zim")).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(fixture.root.join("preserved.zim"), &original).unwrap();
                #[cfg(windows)]
                match std::os::windows::fs::symlink_file(fixture.root.join("preserved.zim"), &original) {
                    Ok(()) => {}
                    // Environment-limited: creating a symlink needs Developer
                    // Mode or SeCreateSymbolicLinkPrivilege (os error 1314).
                    // Only this mutation is skipped; the others still run.
                    Err(error) if error.raw_os_error() == Some(1314) => {
                        eprintln!("skipping symlink mutation: {error}");
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                }
            }
            "hardlink" => { fs::hard_link(&original, fixture.root.join("preserved.zim")).unwrap(); }
            "installed" => {
                let mut book = crate::imports::load_book(original.to_str().unwrap(), true, true).unwrap();
                book.metadata.id = "another-installed-archive".into();
                fixture.store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
            }
            _ => unreachable!(),
        }
        let before = fs::read(&original).unwrap();
        drop(manager);
        let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
        assert_eq!(manager.retained_jobs().unwrap().len(), 1);
        if mutation != "installed" {
            assert!(manager.validate_terminal_owned_paths("offline-fixture").is_err(), "{mutation}");
        }
        assert!(manager.delete_download("offline-fixture").is_err(), "{mutation}");
        assert_eq!(fs::read(&original).unwrap(), before, "{mutation}");
        assert_eq!(fs::read(&replacement).unwrap(), fixture.bytes, "{mutation}");
        assert!(manager.delete_download("unknown-publication").is_err());
        assert!(manager.terminal_owned_publications("unknown-publication").is_err());
    }
}

#[tokio::test]
async fn legacy_imported_and_monitored_map_paths_never_become_retained_deletion_authority() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let imported = fixture.root.join("user-imported.zim");
    fs::write(&imported, &fixture.bytes).unwrap();
    let book = crate::imports::load_book(imported.to_str().unwrap(), true, true).unwrap();
    fixture.store.update(|state| {
        state.books.push(book);
        state.consumed_acquisitions.insert("map:offline-fixture".into());
        Ok(())
    }).unwrap();
    {
        let mut state = manager.inner.state.lock().unwrap();
        state.active = None;
        let job = state.job.as_mut().unwrap();
        job.status = "complete".into();
        job.path = Some(imported.to_str().unwrap().into());
        job.monitored_paths.push(imported.clone());
        persist(&manager.inner.state_path, job).unwrap();
    }
    manager.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "replacement".into(), "en".into(), fixture.root.join("output"), Vec::new(), None, None,
    ).unwrap();
    manager.cancel().await.unwrap();
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert!(manager.retained_jobs().unwrap().is_empty());
    assert!(manager.claims_monitored_path(&imported).unwrap());
    assert!(manager.terminal_owned_publications("offline-fixture").is_err());
    assert!(manager.delete_download("offline-fixture").is_err());
    assert_eq!(fs::read(imported).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn map_cleanup_persists_reclaimed_publication_before_reporting_older_recovery_warnings() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &fixture.root.join("output"), vec![fixture.candidate()]).await;
    let path = result.as_ref().unwrap().clone();
    manager.finish("offline-fixture", &control, result);
    let other = fixture.manager_in(fixture.root.join("older-state"));
    let recovery = fixture.interrupted_partial(&other, &fixture.root.join("older-output"));
    let retained_path = recovery.destination.join(&recovery.name);
    // Not the inode the older download journaled: never Kiwix's to remove.
    fs::rename(&retained_path, fixture.root.join("older-original.part")).unwrap();
    fs::write(&retained_path, b"an unproven file at the journaled name").unwrap();
    let retained_bytes = fs::read(&retained_path).unwrap();
    {
        let mut state = manager.inner.state.lock().unwrap();
        let job = state.job.as_mut().unwrap();
        job.recovery_records.push(recovery);
        persist(&manager.inner.state_path, job).unwrap();
    }
    // Delete removed everything this download owned. The older, unproven file
    // is reported on the job that stays listed, not as a failed Delete.
    manager.delete_download("offline-fixture").unwrap();
    assert!(!path.exists());
    let kept = manager.status().unwrap().unwrap();
    assert_eq!(kept.recovery_paths, vec![retained_path.to_string_lossy().into_owned()]);
    assert!(kept.error.as_deref().is_some_and(|error| error.contains("could not be cleaned up")));
    drop(manager);
    let restored = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let job = restored.status().unwrap().unwrap();
    assert!(job.published.is_none() && job.path.is_none());
    assert!(!job.can_remove_from_list);
    let next = restored.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "another-region".into(), "fr".into(), fixture.root.join("next-output"), Vec::new(), None, None,
    ).unwrap();
    assert_eq!(next.recovery_records.len(), 1);
    restored.cancel().await.unwrap();
    assert_eq!(fs::read(retained_path).unwrap(), retained_bytes);
}

#[tokio::test]
async fn map_cleanup_keeps_legacy_unimported_output_visible_until_explicit_resolution() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let path = fixture.root.join("legacy-map.zim");
    fs::write(&path, &fixture.bytes).unwrap();
    {
        let mut state = manager.inner.state.lock().unwrap();
        state.active = None;
        let job = state.job.as_mut().unwrap();
        job.status = "complete".into();
        job.path = Some(path.to_str().unwrap().into());
        persist(&manager.inner.state_path, job).unwrap();
    }
    let error = manager.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "another-region".into(), "fr".into(), fixture.root.join("next-output"), Vec::new(), None, None,
    ).unwrap_err();
    assert!(error.contains("previous map download"));
    assert_eq!(manager.status().unwrap().unwrap().id, "offline-fixture");
    assert_eq!(fs::read(path).unwrap(), fixture.bytes);
}

#[test]
fn native_pause_signal_does_not_wait_for_the_journal_mutex_or_override_explicit_cancel() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let control = Arc::new(Control::new());
    *manager.inner.pause_control.lock().unwrap() = Some(control.clone());
    let _activity = MapWorkActivity::new(&manager.inner.active_work);
    let state = manager.inner.state.lock().unwrap();
    let worker = manager.clone();
    let (sent, received) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        sent.send((worker.has_active_work(), worker.request_pause())).unwrap();
    });
    let (active, paused) = received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(active.unwrap());
    paused.unwrap();
    assert!(control.is_pausing());
    control.retirement.store(2, std::sync::atomic::Ordering::SeqCst);
    manager.request_pause().unwrap();
    assert!(!control.is_pausing());
    assert_eq!(control.retirement.load(std::sync::atomic::Ordering::SeqCst), 2);
    drop(state);
    thread.join().unwrap();
}

#[tokio::test]
async fn owned_worker_panic_releases_pause_waiters_only_after_preserving_its_checkpoint() {
    let fixture = Fixture::new();
    let (manager, partial, paused) = paused_output(&fixture).await;
    let before = fs::read(&partial).unwrap();
    let control = Arc::new(Control::new());
    manager.inner.state.lock().unwrap().active = Some(control.clone());
    *manager.inner.pause_control.lock().unwrap() = Some(control.clone());
    *manager.inner.before_worker.lock().unwrap() = Some(Box::new(|| panic!("injected worker panic")));
    manager.spawn_run(&tokio::runtime::Handle::current(), paused.id.clone(), control,
        "https://offline-fixture.invalid/index.json".into(),
        "https://offline-fixture.invalid/".into(), fixture.root.join("output"), Vec::new());
    tokio::time::timeout(Duration::from_secs(5), manager.pause()).await.unwrap().unwrap();
    let failed = manager.status().unwrap().unwrap();
    assert_eq!(failed.status, "failed");
    assert!(failed.error.as_deref().unwrap().contains("stopped unexpectedly"));
    assert!(failed.resumable && !manager.has_active_work().unwrap());
    assert_eq!(failed.written, paused.written);
    assert_eq!(fs::read(&partial).unwrap(), before);
    assert!(!manager.remove_terminal(&failed.id).is_ok());
}

#[tokio::test]
async fn historical_recovery_warnings_remain_visible_when_carried_to_another_map() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let recovery = fixture.interrupted_partial(&manager, &fixture.root.join("uncertain-recovery"));
    let path = recovery.destination.join(&recovery.name);
    // Not the inode the earlier download journaled: never Kiwix's to remove.
    fs::rename(&path, fixture.root.join("uncertain-original.part")).unwrap();
    fs::write(&path, b"an unproven file at the journaled name").unwrap();
    let before = fs::read(&path).unwrap();
    {
        let mut state = manager.inner.state.lock().unwrap();
        state.active = None;
        let job = state.job.as_mut().unwrap();
        job.status = "failed".into();
        job.owned_partial = None;
        job.recovery_records.push(recovery);
    }
    assert!(!manager.status().unwrap().unwrap().can_remove_from_list);
    assert!(manager.remove_terminal("offline-fixture").is_err());
    let next = manager.start(
        "https://offline-fixture.invalid/index.json".into(), "https://offline-fixture.invalid/".into(),
        "another-region".into(), "fr".into(), fixture.root.join("another-output"), Vec::new(), None, None,
    ).unwrap();
    assert_eq!(next.recovery_records.len(), 1);
    manager.cancel().await.unwrap();
    assert!(!manager.status().unwrap().unwrap().can_remove_from_list);
    manager.delete_download(&next.id).unwrap();
    let kept = manager.status().unwrap().expect("the warning keeps the download listed");
    assert_eq!(kept.recovery_paths, vec![path.to_string_lossy().into_owned()]);
    assert!(!kept.can_remove_from_list);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn block_progress_counts_unique_required_hash_lengths_including_unreferenced_uuid_header() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let control = Control::new();
    let mut recipe = fixture.recipe();
    let prefix = &fixture.bytes[..fixture.bytes.len() - 16];
    recipe.segments = serde_json::from_value(serde_json::json!([
        {"src":"inline","len":prefix.len(),"data":STANDARD.encode(prefix),
            "sha256":hex::encode(Sha256::digest(prefix))}, {"src":"md5"}
    ])).unwrap();
    recipe.validate(&control).unwrap();
    let mut progress = BlockProgress::new(&recipe, &control).unwrap();
    assert_eq!(progress.required.len(), 1);
    let hash = &recipe.base.sha256[0];
    let length = recipe.base.chunk.min(recipe.base.size);
    progress.mark(hash, length, false, &manager, "offline-fixture", &control).unwrap();
    progress.mark(hash, length, false, &manager, "offline-fixture", &control).unwrap();
    assert_eq!((progress.ready, progress.downloaded), (1, 0));
    progress.mark(hash, length, true, &manager, "offline-fixture", &control).unwrap();
    progress.mark(hash, length, true, &manager, "offline-fixture", &control).unwrap();
    assert_eq!((progress.ready, progress.downloaded), (1, 1));
    control.cancel.send_replace(true);
    assert!(progress.mark(hash, length, true, &manager, "offline-fixture", &control).is_err());
    assert_eq!((progress.ready, progress.downloaded), (1, 1));
}

#[test]
fn installed_map_preflight_rejects_missing_replace_and_stale_installation_before_starting_payload() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    manager.inner.state.lock().unwrap().active = None;
    let source = fixture.candidate();
    let mut installed = crate::imports::load_book(source.to_str().unwrap(), false, false).unwrap();
    let recipe = fixture.recipe();
    installed.metadata.name.clone_from(&recipe.name);
    fixture.store.update(|state| { state.books.push(installed.clone()); Ok(()) }).unwrap();
    let prepared = manager.prepare(MapRecipeMetadata {
        region_id: "fixture".into(), language: "en".into(), name: recipe.name,
        md5: recipe.md5, uuid: recipe.uuid, version: "fixture-version".into(),
        assembled_bytes: recipe.size, cold_download_bytes: recipe.size, reusable_base_download_bytes: 0,
    }, 1).unwrap();
    assert!(prepared.same_version && prepared.installed.is_some());
    assert!(manager.start_prepared("fixture", "en", &prepared.token, false, 1, None).is_err());
    fixture.store.update(|state| { state.books[0].installation_id = crate::store::new_installation_id(); Ok(()) }).unwrap();
    assert!(manager.start_prepared("fixture", "en", &prepared.token, true, 1, None).is_err());
    assert!(!manager.has_active_work().unwrap());
    assert_eq!(manager.status().unwrap().unwrap().written, 0);
    assert_eq!(fs::read(source).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn publication_recovery_preserves_unrenamed_resumable_output_and_admits_only_its_published_inode() {
    for published in [false, true] {
        for restart in [false, true] {
            let fixture = Fixture::new();
            let (manager, partial, paused) = paused_output(&fixture).await;
            fs::write(&partial, &fixture.bytes).unwrap();
            // FlushFileBuffers needs write access on Windows.
            fs::OpenOptions::new().write(true).open(&partial).unwrap().sync_all().unwrap();
            let mut record = paused.owned_partial.clone().unwrap();
            let name = format!("{}.zim", fixture.recipe().output_stem());
            let destination = record.destination.join(&name);
            record.publication = Some(PublicationRecovery {
                name, size: fixture.bytes.len() as u64, sha256: Some(Sha256::digest(&fixture.bytes).into()),
            });
            manager.checkpoint_partial("offline-fixture", Some(record)).unwrap();
            if published { fs::rename(&partial, &destination).unwrap(); }
            else { fs::write(&destination, b"another-owner").unwrap(); }
            let manager = if restart {
                drop(manager);
                MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap()
            } else {
                manager.recover_terminal_publication().unwrap();
                manager
            };
            let recovered = manager.status().unwrap().unwrap();
            if published {
                assert_eq!(recovered.status, "complete");
                assert_eq!(recovered.path.as_deref(), destination.to_str());
                assert!(recovered.published.is_some() && recovered.resume.is_none());
                assert_eq!(fs::read(&destination).unwrap(), fixture.bytes);
            } else {
                assert!(recovered.resumable);
                assert_eq!(recovered.written, paused.written);
                assert!(recovered.owned_partial.as_ref().unwrap().publication.is_none());
                assert_eq!(fs::read(&partial).unwrap(), fixture.bytes);
                assert_eq!(fs::read(&destination).unwrap(), b"another-owner");
            }
            assert!(!recovered.can_remove_from_list);
        }
    }
}

#[tokio::test]
async fn old_cancelled_written_bytes_without_an_owned_checkpoint_are_not_claimed_resumable() {
    let fixture = Fixture::new();
    let directory = fixture.root.join("legacy-state");
    fs::create_dir(&directory).unwrap();
    let job: MapJob = serde_json::from_value(serde_json::json!({
        "id":"old-baltics-job","status":"cancelled","regionId":"baltics","language":"lt",
        "written":620756992u64,"total":1267934996u64,"incoming":0,"incomingTotal":0,
        "error":CANCELLED,"path":null
    })).unwrap();
    persist(&directory.join("map-job.json"), &job).unwrap();
    let manager = MapManager::new(directory, fixture.store.clone()).unwrap();
    let legacy = manager.status().unwrap().unwrap();
    assert!(!legacy.resumable && legacy.can_remove_from_list);
    assert_eq!(legacy.blocks_total, None);
    assert!(manager.resume(&legacy.id, 1, None).unwrap_err().contains("retained output checkpoint"));
    let sentinel = manager.inner.cache.join("other-map-cache");
    fs::write(&sentinel, b"retain-verified-shared-data").unwrap();
    manager.delete_download(&legacy.id).unwrap();
    assert!(manager.status().unwrap().is_none());
    assert_eq!(fs::read(&sentinel).unwrap(), b"retain-verified-shared-data");
}

#[test]
fn terminal_history_dismissal_preserves_imported_archives_and_cache() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let archive = fixture.root.join("published.zim");
    fs::write(&archive, &fixture.bytes).unwrap();
    let cached = manager.inner.cache.join("verified-part");
    fs::write(&cached, b"reusable downloaded part").unwrap();
    {
        let mut state = manager.inner.state.lock().unwrap();
        let job = state.job.as_mut().unwrap();
        job.status = "complete".into();
        job.path = Some(archive.to_string_lossy().into_owned());
        job.owned_partial = None;
        // A terminal-looking checkpoint still cannot retire an active owner.
    }
    assert!(manager.remove_terminal("offline-fixture").is_err());
    manager.inner.state.lock().unwrap().active = None;
    assert!(manager.remove_terminal("stale-job-id").is_err());
    assert!(!manager.status().unwrap().unwrap().can_remove_from_list);
    assert!(manager.remove_terminal("offline-fixture").is_err());
    fixture.store.update(|state| { state.consumed_acquisitions.insert("map:offline-fixture".into()); Ok(()) }).unwrap();
    assert!(manager.status().unwrap().unwrap().can_remove_from_list);
    manager.remove_terminal("offline-fixture").unwrap();
    assert!(manager.status().unwrap().is_none());
    assert!(manager.claims_monitored_path(&archive).unwrap());
    assert_eq!(fs::read(&archive).unwrap(), fixture.bytes);
    assert_eq!(fs::read(&cached).unwrap(), b"reusable downloaded part");
    let state_directory = manager.inner.state_path.parent().unwrap().to_path_buf();
    drop(manager);
    let restored = MapManager::new(state_directory, fixture.store.clone()).unwrap();
    assert!(restored.status().unwrap().is_none());
    assert!(restored.claims_monitored_path(&archive).unwrap());
    let state = restored.inner.state.lock().unwrap();
    let retained = state.job.as_ref().unwrap();
    assert!(retained.history_dismissed);
    assert!(retained.recovery_records.is_empty());
    assert_eq!(retained.path.as_deref(), archive.to_str());
    assert_eq!(fs::read(cached).unwrap(), b"reusable downloaded part");
}

#[test]
fn history_dismissal_rejects_unfinished_maps_and_rolls_back_failed_checkpoint() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    manager.inner.state.lock().unwrap().active = None;
    assert!(manager.remove_terminal("offline-fixture").is_err());
    {
        let mut state = manager.inner.state.lock().unwrap();
        state.job.as_mut().unwrap().status = "failed".into();
    }
    fs::rename(&manager.inner.state_path, manager.inner.state_path.with_extension("retained")).unwrap();
    fs::create_dir(&manager.inner.state_path).unwrap();
    assert!(manager.remove_terminal("offline-fixture").is_err());
    assert!(!manager.status().unwrap().unwrap().history_dismissed);
    fs::remove_dir(&manager.inner.state_path).unwrap();
    manager.remove_terminal("offline-fixture").unwrap();
    assert!(manager.status().unwrap().is_none());
}

#[test]
fn recipe_transfer_estimate_counts_ordered_chunks_not_archive_or_pack_sizes() {
    let fixture = Fixture::new();
    let mut recipe = fixture.recipe();
    let chunk = MAX_CACHE_BYTES + 1;
    recipe.base.size = chunk * 2;
    recipe.base.chunk = chunk;
    recipe.base.sha256 = vec!["a".repeat(64), "b".repeat(64)];
    recipe.segments = vec![
        Segment { src: "base".into(), pack: None, off: Some(chunk), len: Some(1), data: None, sha256: None },
        Segment { src: "base".into(), pack: None, off: Some(0), len: Some(1), data: None, sha256: None },
    ];
    // Header chunk, second chunk, then the evicted header chunk again.
    assert_eq!(recipe.download_estimate(false, &Control::new()).unwrap(), chunk * 3);
    assert_eq!(recipe.download_estimate(true, &Control::new()).unwrap(), 0);
    recipe.base.chunk = 1024;
    recipe.base.size = 2048;
    recipe.segments[0].off = Some(1024);
    // Both small chunks fit the persistent cache, so revisiting is free.
    assert_eq!(recipe.download_estimate(false, &Control::new()).unwrap(), 2048);
}

#[test]
fn recipe_transfer_estimate_includes_search_chunks_but_not_inline_or_unused_ranges() {
    let fixture = Fixture::new();
    let mut recipe = fixture.recipe();
    let base_size = recipe.base.size;
    recipe.base.chunk = base_size;
    recipe.base.sha256 = vec!["a".repeat(64)];
    for (key, hash) in [("es", "b"), ("search", "c")] {
        recipe.packs.insert(key.into(), Source {
            file: key.into(), size: 12, uuid: recipe.base.uuid.clone(),
            chunk: 4, sha256: vec![hash.repeat(64); 3],
            mask: Some(vec![vec![0, 4]]), url: None,
        });
    }
    recipe.segments = vec![
        Segment { src: "inline".into(), pack: None, off: None, len: Some(100), data: None, sha256: None },
        Segment { src: "pack".into(), pack: Some("es".into()), off: Some(4), len: Some(1), data: None, sha256: None },
        Segment { src: "pack".into(), pack: Some("search".into()), off: Some(4), len: Some(1), data: None, sha256: None },
        Segment { src: "md5".into(), pack: None, off: None, len: None, data: None, sha256: None },
    ];
    // Each one-byte copy fetches its whole four-byte source chunk. Unused
    // source chunks and recipe-inline bytes never become payload requests.
    assert_eq!(recipe.download_estimate(false, &Control::new()).unwrap(), base_size + 8);
    assert_eq!(recipe.download_estimate(true, &Control::new()).unwrap(), 8);
}

#[test]
fn recipe_transfer_estimate_rejects_valid_adversarial_segment_expansion() {
    let fixture = Fixture::new();
    let mut recipe = fixture.recipe();
    recipe.base.chunk = 1024;
    recipe.base.size = 8192 * recipe.base.chunk;
    recipe.base.sha256 = vec!["a".repeat(64); 8192];
    recipe.base.mask = None;
    let copies = MAX_ESTIMATE_CHUNK_READS / 8192 + 1;
    recipe.size = copies * recipe.base.size + 16;
    recipe.segments = (0..copies).map(|_| Segment {
        src: "base".into(), pack: None, off: Some(0),
        len: Some(recipe.base.size), data: None, sha256: None,
    }).collect();
    recipe.segments.push(Segment {
        src: "md5".into(), pack: None, off: None,
        len: None, data: None, sha256: None,
    });
    recipe.validate(&Control::new()).unwrap();
    for local in [false, true] {
        assert_eq!(recipe.download_estimate(local, &Control::new()).unwrap_err(),
            "Map recipe is too complex to estimate download size safely");
    }
}

#[test]
fn catalog_unknown_size_is_not_an_incremental_transfer_size() {
    let fixture = Fixture::new();
    let recipe = fixture.recipe();
    let mut region: MapRegion = serde_json::from_value(serde_json::json!({
        "id": "fixture", "title": "Fixture",
        "base": {"file": recipe.base.file, "size": recipe.base.size, "download": "https://example.org/base.zim"},
        "menu": [{"lang": "en", "label": "English", "name": recipe.name,
            "recipe": "recipes/fixture/en.json", "download_bytes": 106214997, "md5": recipe.md5}]
    })).unwrap();
    assert!(region.menu[0].size.is_none());
    assert!(recipe.matches_catalog(&region, &region.menu[0]));
    region.menu[0].size = Some(recipe.size + 1);
    assert!(!recipe.matches_catalog(&region, &region.menu[0]));
    region.menu[0].size = Some(recipe.size);
    region.menu[0].md5 = "0".repeat(32);
    assert!(!recipe.matches_catalog(&region, &region.menu[0]));
}

#[tokio::test]
async fn catalog_retirement_closes_real_requests_and_streaming_bodies() {
    for streaming_body in [false, true] {
        for policy_denial in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = Url::parse(&format!("http://{}/index.json", listener.local_addr().unwrap())).unwrap();
            let (ready, started) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                if streaming_body {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\n{").await.unwrap();
                }
                ready.send(()).unwrap();
                match socket.read(&mut buffer).await {
                    Ok(0) => {}
                    Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                    other => panic!("catalog connection remained active: {other:?}"),
                }
            });
            let control = Arc::new(Control::new());
            let cancel_on_drop = CatalogCancellation(control.clone());
            let (deny, denied) = tokio::sync::oneshot::channel();
            let worker_control = control.clone();
            let worker = tokio::spawn(async move {
                let client = checked_client().unwrap();
                // Exercise the production request/body/parser and policy fence.
                // Only the internal URL bypasses public HTTPS admission for loopback.
                catalog_until_denied(
                    &worker_control,
                    fetch_json::<MapCatalog>(&client, url, &worker_control),
                    async { let _ = denied.await; },
                ).await
            });
            started.await.unwrap();
            if policy_denial {
                deny.send(()).unwrap();
                drop(cancel_on_drop);
            } else {
                // Same guard used when the standalone command future disappears.
                drop(cancel_on_drop);
            }
            let error = tokio::time::timeout(Duration::from_secs(5), worker)
                .await.unwrap().unwrap().unwrap_err();
            assert_eq!(error, if policy_denial { "MOBILE_NETWORK_BLOCKED" } else { CANCELLED });
            assert!(control.check().is_err());
            tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn catalog_policy_retirement_joins_an_existing_json_parser() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/index.json", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        let body = br#"{"format":"streetzim-build-index/1","regions":[]}"#;
        socket.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ).as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
    });
    let control = Arc::new(Control::new());
    let (parsing, parser_started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    *control.before_json_parse.lock().unwrap() = Some(Box::new(move || {
        parsing.send(()).unwrap();
        released.recv().unwrap();
    }));
    let registry = Arc::new(CatalogRegistry::default());
    let worker_control = control.clone();
    let result = registry.spawn(
        crate::transfer_policy::admission_revision().unwrap(), control.clone(),
        async move {
            let client = checked_client().unwrap();
            fetch_json::<MapCatalog>(&client, url, &worker_control).await
        },
    ).unwrap();
    parser_started.await.unwrap();
    let retiring = registry.clone();
    let retirement = tokio::spawn(async move { retiring.retire().await });
    control.cancelled().await;
    assert!(!retirement.is_finished(), "retirement must join the already-running parser");
    release.send(()).unwrap();
    assert_eq!(result.await.unwrap().unwrap_err(), CANCELLED);
    retirement.await.unwrap().unwrap();
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn registered_catalog_is_retired_before_its_first_poll_can_open_a_connection() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("https://{}/index.json", listener.local_addr().unwrap());
    let revision = crate::transfer_policy::admission_revision().unwrap();
    let registry = Arc::new(CatalogRegistry::default());
    let control = Arc::new(Control::new());
    let worker_control = control.clone();
    let result = registry.spawn(revision, control.clone(), async move {
        fetch_catalog_controlled(&checked_client()?, &url, &worker_control).await
    }).unwrap();
    assert_eq!(registry.workers.lock().unwrap().controls.len(), 1);
    // No runtime yield occurred between admission and retirement. Cancellation
    // must reach the registered but not-yet-polled worker.
    registry.retire().await.unwrap();
    assert_eq!(result.await.unwrap().unwrap_err(), CANCELLED);
    assert_eq!(listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(control.check().is_err());
    assert!(registry.workers.lock().unwrap().tasks.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn catalog_retirement_reports_and_retains_worker_join_failure() {
    let registry = Arc::new(CatalogRegistry::default());
    let result = registry.spawn::<MapCatalog>(
        crate::transfer_policy::admission_revision().unwrap(),
        Arc::new(Control::new()),
        async { panic!("catalog worker regression failure") },
    ).unwrap();
    // Let this worker fail before retirement cancels its admission.
    tokio::task::yield_now().await;
    assert!(result.await.is_err());
    assert!(registry.retire().await.unwrap_err().contains("catalog worker regression failure"));
    assert!(registry.retire().await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_retries_failed_terminal_checkpoint_after_storage_recovers() {
    for fail_save in [false, true] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let control = manager.inner.state.lock().unwrap().active.clone().unwrap();
        if fail_save {
            fs::rename(&manager.inner.state_path, manager.inner.state_path.with_extension("retained")).unwrap();
            fs::create_dir(&manager.inner.state_path).unwrap();
        }
        let cancelling = manager.clone();
        let cancellation = tokio::spawn(async move { cancelling.cancel().await });
        control.cancelled().await;
        manager.finish("offline-fixture", &control, Err(CANCELLED.into()));
        let result = cancellation.await.unwrap();
        assert_eq!(result.is_err(), fail_save);
        if fail_save {
            assert!(result.unwrap_err().contains("Cannot save map job state"));
            assert!(manager.cancel().await.unwrap_err().contains("Cannot save map job state"));
            fs::remove_dir(&manager.inner.state_path).unwrap();
            fs::rename(
                manager.inner.state_path.with_extension("retained"),
                &manager.inner.state_path,
            ).unwrap();
        }
        manager.cancel().await.unwrap();
        manager.cancel().await.unwrap();
        let state = manager.inner.state.lock().unwrap();
        assert!(state.active.is_none());
        assert!(state.retirement_error.is_none());
        let durable: MapJob = serde_json::from_slice(
            &fs::read(&manager.inner.state_path).unwrap()
        ).unwrap();
        assert_eq!(durable.status, "cancelled");
    }
}

#[test]
fn volume_comparison_uses_native_directory_handles() {
    let fixture = Fixture::new();
    let mut candidates = vec![fixture.root.clone(), std::env::temp_dir()];
    #[cfg(unix)]
    candidates.push(PathBuf::from("/dev"));
    #[cfg(windows)]
    for drive in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", char::from(drive)));
        if root.is_dir() { candidates.push(root); }
    }
    let device = |path: &Path| Dir::open_ambient_dir(path, cap_std::ambient_authority())
        .unwrap().dir_metadata().unwrap().dev();
    for candidate in candidates {
        assert_eq!(same_volume(&fixture.root, &candidate).unwrap(),
            device(&fixture.root) == device(&candidate), "{}", candidate.display());
    }
}

#[tokio::test]
async fn unverified_local_base_stops_before_fallback_network_and_confirms_real_cache_growth() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    fs::create_dir(&destination).unwrap();
    // The cache budget is larger than the protocol chunk limit; a test chunk
    // must stay within the range a real StreetZim source can request.
    let len = MAX_CHUNK;
    let payload = vec![b'v'; len as usize];
    let expected = hex::encode(Sha256::digest(&payload));
    let candidate = fixture.root.join("wrong-base.zim");
    fs::write(&candidate, vec![b'x'; len as usize]).unwrap();
    // Pin admission capacity instead of sampling the shared volume. The
    // scenario must not move between the hard-capacity and advisory paths
    // because another test or a concurrent build happened to write 1 GB.
    let available = 2 * crate::disk_warning::THRESHOLD_BYTES;
    assert!(crate::fs_ext::available_space(&destination).unwrap() > len * 2,
        "This bounded admission regression needs two cache budgets free, not allocated");
    let mut recipe = fixture.recipe();
    recipe.size = available - crate::disk_warning::THRESHOLD_BYTES - len / 2;
    recipe.base.size = len;
    recipe.base.chunk = len;
    recipe.base.sha256 = vec![expected.clone()];
    recipe.base.mask = None;
    recipe.segments = serde_json::from_value(serde_json::json!([
        {"src":"base", "off":0, "len":len}
    ])).unwrap();
    let mut control = Control::new();
    control.available_bytes = Some(available);
    // Size-only local selection initially needs only output space.
    preflight(&recipe, true, &manager.inner.cache, &destination, &control, recipe.size, 0).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let urls = HashMap::from([("base".to_string(), Url::parse(&format!(
        "http://{}/base.zim", listener.local_addr().unwrap()
    )).unwrap())]);
    let masks = HashMap::from([("base".to_string(), Intervals::default())]);
    let destination_cap = Arc::new(MapDirectory::open(&destination).unwrap());
    let mut reader = ChunkReader {
        manager: &manager, id: "offline-fixture", control: &control, recipe: &recipe,
        masks: &masks, urls: &urls,
        local: vec![LocalCandidate::open(candidate.clone(), len).await.unwrap()],
        local_verified: HashMap::new(), local_failed: HashSet::new(), last_local: None,
        destination: destination_cap.clone(), last: None, written: 0, fallback_space_admitted: false,
        cache: MapChunkCache::open(&manager.inner.cache, || control.check()).unwrap(),
        blocks: BlockProgress::new(&recipe, &control).unwrap(),
    };
    let error = reader.get("base", 0).await.unwrap_err();
    let warning_json = error
        .strip_prefix(crate::disk_warning::WARNING_PREFIX)
        .unwrap_or_else(|| panic!(
            "the unverified local base must stop at the low-disk admission warning, got: {error}"
        ));
    let warning: crate::disk_warning::SpaceWarning =
        serde_json::from_str(warning_json).unwrap_or_else(|parse| panic!(
            "low-disk warning is not valid JSON ({parse}): {warning_json}"
        ));
    assert!(warning.projected_free_bytes < crate::disk_warning::THRESHOLD_BYTES);
    assert_eq!(listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(fs::read_dir(&manager.inner.cache).unwrap().next().is_none());
    drop(reader);
    let mut confirmed = Control::new();
    confirmed.available_bytes = Some(available);
    confirmed.space_acknowledgements = Arc::new(
        serde_json::from_str(&warning.acknowledgement).unwrap()
    );
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0 && request.len() + count <= 16384);
            request.extend_from_slice(&buffer[..count]);
        }
        socket.write_all(format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {len}\r\nContent-Range: bytes 0-{}/{len}\r\nConnection: close\r\n\r\n",
            len - 1
        ).as_bytes()).await.unwrap();
        socket.write_all(&payload).await.unwrap();
    });
    let mut reader = ChunkReader {
        manager: &manager, id: "offline-fixture", control: &confirmed, recipe: &recipe,
        masks: &masks, urls: &urls,
        local: vec![LocalCandidate::open(candidate.clone(), len).await.unwrap()],
        local_verified: HashMap::new(), local_failed: HashSet::new(), last_local: None,
        destination: destination_cap, last: None, written: 0, fallback_space_admitted: false,
        cache: MapChunkCache::open(&manager.inner.cache, || confirmed.check()).unwrap(),
        blocks: BlockProgress::new(&recipe, &confirmed).unwrap(),
    };
    let bytes = tokio::time::timeout(Duration::from_secs(60), reader.get("base", 0))
        .await
        .expect("acknowledged fallback did not finish within 60s")
        .unwrap();
    assert_eq!(hex::encode(Sha256::digest(bytes.as_slice())), expected);
    server.await.unwrap();
}

#[tokio::test]
async fn compose_retry_retains_initial_and_local_fallback_approvals() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("retry-output");
    fs::create_dir(&destination).unwrap();
    let candidate = fixture.root.join("wrong-retry-base.zim");
    // StreetZim ranges cannot exceed MAX_CHUNK even though the cache budget can.
    let chunk = MAX_CHUNK;
    fs::File::create(&candidate).unwrap().set_len(chunk).unwrap();
    // Pin admission capacity: the approval sequence must not depend on
    // what the rest of the suite or a concurrent build writes this second.
    let available = 2 * crate::disk_warning::THRESHOLD_BYTES;
    assert!(crate::fs_ext::available_space(&destination).unwrap() > MAX_CACHE_BYTES * 2,
        "This bounded compose regression needs two cache budgets free, not allocated");
    let size = available - 500_000_000;
    let recipe = || {
        let mut recipe = fixture.recipe();
        recipe.size = size;
        recipe.base.size = chunk;
        recipe.base.chunk = chunk;
        recipe.base.sha256 = vec!["e".repeat(64)];
        recipe.base.mask = None;
        recipe.segments.clear();
        let mut remaining = size - 16;
        while remaining > 0 {
            let length = remaining.min(chunk);
            recipe.segments.push(Segment {
                src: "base".into(), pack: None, off: Some(0), len: Some(length),
                data: None, sha256: None,
            });
            remaining -= length;
        }
        recipe.segments.push(Segment {
            src: "md5".into(), pack: None, off: None, len: None, data: None, sha256: None,
        });
        recipe
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = Url::parse(&format!("https://{}/base.zim", listener.local_addr().unwrap())).unwrap();
    let mut control = Control::new();
    control.available_bytes = Some(available);
    control.space_identity = Some(Arc::new("compose-retry-attempt".into()));
    let mut last_requirement = None;
    // Real composition first requires output-only consent, then discovers
    // the bad local base and asks for the larger fallback requirement.
    for expected_approvals in 1..=2 {
        manager.inner.state.lock().unwrap().active = Some(Arc::new(control.clone()));
        let result = manager.compose(
            "offline-fixture", &control, recipe(), url.as_str(), &url, &destination,
            vec![candidate.clone()],
        ).await;
        let error = result.as_ref().unwrap_err();
        let warning: crate::disk_warning::SpaceWarning = serde_json::from_str(
            error.strip_prefix(crate::disk_warning::WARNING_PREFIX).unwrap()
        ).unwrap();
        if let Some(previous) = last_requirement { assert!(warning.remaining_bytes > previous); }
        last_requirement = Some(warning.remaining_bytes);
        assert_eq!(listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert!(fs::read_dir(&manager.inner.cache).unwrap().next().is_none());
        manager.finish("offline-fixture", &control, result);
        let acknowledgements: MapSpaceAcknowledgements =
            serde_json::from_str(&warning.acknowledgement).unwrap();
        assert_eq!(acknowledgements.output.len(), expected_approvals);
        control.space_acknowledgements = Arc::new(acknowledgements);
    }
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let connections = tokio::spawn(async move {
        // End each actual TLS connection immediately. The existing three
        // transport attempts must be reached, not another approval loop.
        for _ in 0..3 {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        }
    });
    manager.inner.state.lock().unwrap().active = Some(Arc::new(control.clone()));
    let result = tokio::time::timeout(Duration::from_secs(15), manager.compose(
        "offline-fixture", &control, recipe(), url.as_str(), &url, &destination, vec![candidate],
    )).await.unwrap();
    let error = result.as_ref().unwrap_err();
    assert!(!error.starts_with(crate::disk_warning::WARNING_PREFIX), "{error}");
    assert!(error.contains("StreetZim range request failed"), "{error}");
    connections.await.unwrap();
    manager.finish("offline-fixture", &control, result);
}

#[tokio::test]
async fn full_composer_rebuilds_real_zim_from_masked_local_ranges_and_inline_data() {
    let fixture = Fixture::new();
    let manager = fixture.manager_in(fixture.store.data_dir().join("maps"));
    let checking = manager.clone();
    *manager.inner.before_publish_commit.lock().unwrap() = Some(Box::new(move |path| {
        assert!(!path.exists());
        assert!(checking.claims_monitored_path(path).unwrap());
    }));
    let recipe = fixture.recipe();
    // A corrupt content-addressed cache entry must not bypass verification.
    // The real local candidate supplies the correct masked first chunk instead.
    let first_len = recipe.base.chunk.min(recipe.base.size);
    fs::write(
        manager
            .inner
            .cache
            .join(format!("{}-{first_len}", recipe.base.sha256[0])),
        vec![0x55; first_len as usize],
    )
    .unwrap();
    let candidate = fixture.candidate();
    let destination = fixture.root.join("output");
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let path = manager
        .compose(
            "offline-fixture",
            &Control::new(),
            recipe,
            url.as_str(),
            &url,
            &destination,
            vec![candidate.clone()],
        )
        .await
        .unwrap();
    // Completion is durable before the outer worker's finish/acknowledgement.
    let durable: MapJob = serde_json::from_slice(&fs::read(&manager.inner.state_path).unwrap()).unwrap();
    assert_eq!(durable.status, "complete");
    assert_eq!(durable.path.as_deref(), path.to_str());
    assert!(durable.owned_partial.is_none());
    assert!(manager.claims_monitored_path(&path).unwrap());
    let claims = MapManager::saved_monitored_paths(&fixture.store).unwrap();
    assert!(claims.claims_monitored_path(&path).unwrap());
    assert!(!claims.claims_monitored_path(&destination.join("external.zim")).unwrap());
    let composed = fs::read(&path).unwrap();
    assert_eq!(composed, fixture.bytes);
    assert_eq!(
        fs::read(&candidate).unwrap(),
        fixture.bytes,
        "local sources must remain unchanged"
    );
    assert_eq!(
        hex::encode(md5::compute(&composed[..composed.len() - 16]).0),
        hex::encode(&composed[composed.len() - 16..])
    );
    let archive = crate::archive::Archive::open(&path).unwrap();
    let metadata = archive.metadata().unwrap();
    assert!(!metadata.main_path.is_empty());
    assert_eq!(metadata.size, composed.len() as u64);
    assert!(
        archive.entry(&metadata.main_path).is_ok(),
        "published bytes must open through libzim"
    );
    drop(archive);
    // Publication must preserve an already installed file, not replace it.
    let second = manager
        .compose(
            "offline-fixture",
            &Control::new(),
            fixture.recipe(),
            url.as_str(),
            &url,
            &destination,
            vec![candidate],
        )
        .await
        .unwrap();
    assert_ne!(path, second);
    assert_eq!(fs::read(path).unwrap(), fixture.bytes);
    assert_eq!(fs::read(second).unwrap(), fixture.bytes);
    assert_eq!(
        fs::read_dir(destination).unwrap().count(),
        2,
        "no partial files may remain"
    );
}

#[tokio::test]
async fn new_job_waits_for_admission_then_preserves_imported_output_claim_across_restart() {
    let fixture = Fixture::new();
    let state_directory = fixture.store.data_dir().join("maps");
    let manager = fixture.manager_in(state_directory.clone());
    let destination = fixture.root.join("output");
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let output = manager.compose(
        "offline-fixture", &control, fixture.recipe(), url.as_str(), &url,
        &destination, vec![fixture.candidate()],
    ).await.unwrap();
    manager.finish("offline-fixture", &control, Ok(output.clone()));
    // This real verified output has not gone through receipt admission.
    let external = destination.join("external.zim");
    fs::write(&external, &fixture.bytes).unwrap();
    let start_next = |manager: &MapManager, region: &str| manager.start(
        "https://offline-fixture.invalid/index.json".into(),
        "https://offline-fixture.invalid/".into(),
        region.into(), "en".into(), fixture.root.join("next-output"),
        Vec::new(), None, None,
    );
    assert!(start_next(&manager, "second-region").is_err());
    assert!(manager.status().unwrap().unwrap().published.is_some());
    let book = crate::imports::load_book(output.to_str().unwrap(), true, true).unwrap();
    fixture.store.update(|state| {
        state.books.push(book);
        state.consumed_acquisitions.insert("map:offline-fixture".into());
        Ok(())
    }).unwrap();
    let next = start_next(&manager, "second-region").unwrap();
    assert_ne!(next.id, "offline-fixture");
    assert!(next.path.is_none());
    assert!(manager.claims_monitored_path(&output).unwrap());
    assert!(!manager.claims_monitored_path(&external).unwrap());
    let claims = MapManager::saved_monitored_paths(&fixture.store).unwrap();
    assert!(claims.claims_monitored_path(&output).unwrap());
    assert!(!claims.claims_monitored_path(&external).unwrap());
    manager.cancel().await.unwrap();
    drop(manager);

    let manager = MapManager::new(state_directory, fixture.store.clone()).unwrap();
    assert_eq!(manager.status().unwrap().unwrap().region_id, "second-region");
    assert!(manager.claims_monitored_path(&output).unwrap());
    assert!(!manager.claims_monitored_path(&external).unwrap());
    start_next(&manager, "third-region").unwrap();
    assert!(manager.claims_monitored_path(&output).unwrap());
    let claims = MapManager::saved_monitored_paths(&fixture.store).unwrap();
    assert!(claims.claims_monitored_path(&output).unwrap());
    assert!(!claims.claims_monitored_path(&external).unwrap());
    manager.cancel().await.unwrap();
    assert_eq!(fs::read(output).unwrap(), fixture.bytes);
    assert_eq!(fs::read(external).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn full_composer_rejects_bad_checksums_without_publishing_or_leaving_partials() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let candidate = fixture.candidate();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    for whole_file in [false, true] {
        let mut recipe = fixture.recipe();
        if whole_file {
            recipe.file_md5 = Some("0".repeat(32));
        } else {
            recipe.md5 = "0".repeat(32);
        }
        let destination = fixture.root.join(if whole_file {
            "bad-file-md5"
        } else {
            "bad-zim-md5"
        });
        manager
            .compose(
                "offline-fixture",
                &Control::new(),
                recipe,
                url.as_str(),
                &url,
                &destination,
                vec![candidate.clone()],
            )
            .await
            .unwrap_err();
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
        let job = manager.status().unwrap().unwrap();
        assert!(job.path.is_none());
        assert!(job.owned_partial.is_none());
    }
    assert_eq!(fs::read(candidate).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn fresh_publication_never_fills_a_missing_installed_book_path() {
    for register_after_composition in [false, true] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let candidate = fixture.candidate();
        let recipe = fixture.recipe();
        let destination = fixture.root.join("output");
        let output = destination.join(format!("{}.zim", recipe.output_stem()));
        let mut book =
            crate::imports::load_book(candidate.to_str().unwrap(), false, false).unwrap();
        let installation_id = book.installation_id.clone();
        let archive_id = book.metadata.id.clone();
        book.path = output.to_string_lossy().into_owned();
        book.available = false;
        let store = fixture.store.clone();
        let reserve = move || {
            store
                .update(|state| {
                    state.books.push(book);
                    Ok(())
                })
                .unwrap()
        };
        if register_after_composition {
            *manager.inner.before_publish_commit.lock().unwrap() =
                Some(Box::new(move |_| reserve()));
        } else {
            reserve();
        }
        let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
        let result = manager
            .compose(
                "offline-fixture",
                &Control::new(),
                recipe,
                url.as_str(),
                &url,
                &destination,
                vec![candidate.clone()],
            )
            .await;
        let published = result.unwrap();
        assert_ne!(published, output);
        assert_eq!(fs::read(&published).unwrap(), fixture.bytes);
        assert!(!output.exists());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
        assert_eq!(fs::read(candidate).unwrap(), fixture.bytes);
        let books = fixture.store.read().unwrap().books;
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].installation_id, installation_id);
        assert_eq!(books[0].metadata.id, archive_id);
        assert_eq!(books[0].path, output.to_string_lossy());
        let job = manager.status().unwrap().unwrap();
        assert_eq!(job.path.as_deref(), published.to_str());
        assert!(job.owned_partial.is_none());
    }
}

#[tokio::test]
async fn restart_reclaims_uncommitted_publication_and_allows_a_clean_retry() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let record = fixture.publication_checkpoint(&manager, &destination);
    let occupied = destination.join(&record.publication.as_ref().unwrap().name);
    fs::write(&occupied, b"existing archive must survive").unwrap();
    drop(manager);

    // Exact crash window: publication is durable, but rename has not run.
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert!(!destination.join(&record.name).exists());
    assert_eq!(
        fs::read(&occupied).unwrap(),
        b"existing archive must survive"
    );
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.owned_partial.is_none());
    assert!(job.recovery_records.is_empty());
    assert!(job.path.is_none());
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let candidate = fixture.candidate();
    let path = manager
        .compose(
            "offline-fixture",
            &Control::new(),
            fixture.recipe(),
            url.as_str(),
            &url,
            &destination,
            vec![candidate.clone()],
        )
        .await
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), fixture.bytes);
    assert_eq!(fs::read(candidate).unwrap(), fixture.bytes);
    assert_eq!(
        fs::read(occupied).unwrap(),
        b"existing archive must survive"
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 2);
}

#[test]
fn restart_recovers_the_verified_output_after_rename_before_completion_checkpoint() {
    let fixture = Fixture::new();
    let state_directory = fixture.store.data_dir().join("maps");
    let manager = fixture.manager_in(state_directory.clone());
    let destination = fixture.root.join("output");
    let record = fixture.publication_checkpoint(&manager, &destination);
    let partial = destination.join(&record.name);
    let output = destination.join(&record.publication.as_ref().unwrap().name);
    let verified = crate::publication::VerifiedFile::new(
        fs::File::open(&partial).unwrap(),
        fixture.bytes.len() as u64,
    )
    .unwrap();
    crate::publication::publish_verified_noreplace(&partial, &output, &verified).unwrap();
    assert!(manager.claims_monitored_path(&output).unwrap());
    assert!(!manager.claims_monitored_path(&destination.join("external.zim")).unwrap());
    drop(verified);
    drop(manager);
    let claims = MapManager::saved_monitored_paths(&fixture.store).unwrap();
    assert!(claims.claims_monitored_path(&output).unwrap());
    assert!(!claims.claims_monitored_path(&destination.join("external.zim")).unwrap());

    let manager = MapManager::new(state_directory, fixture.store.clone()).unwrap();
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "complete");
    assert_eq!(job.path.as_deref(), output.to_str());
    assert!(job.owned_partial.is_none());
    assert!(job.recovery_records.is_empty());
    assert!(manager.claims_monitored_path(&output).unwrap());
    assert_eq!(fs::read(&output).unwrap(), fixture.bytes);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    let archive = crate::archive::Archive::open(&output).unwrap();
    let metadata = archive.metadata().unwrap();
    assert!(archive.entry(&metadata.main_path).is_ok());
}

#[tokio::test]
async fn failed_completion_save_retains_published_identity_for_restart_recovery() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let state_path = manager.inner.state_path.clone();
    let retained_state = state_path.with_extension("retained");
    let blocked_path = state_path.clone();
    let saved_path = retained_state.clone();
    *manager.inner.before_publish_commit.lock().unwrap() = Some(Box::new(move |_| {
        // Real persistence failure after the publication journal is durable.
        fs::rename(&blocked_path, &saved_path).unwrap();
        fs::create_dir(&blocked_path).unwrap();
    }));
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager.compose(
        "offline-fixture", &control, fixture.recipe(), url.as_str(), &url,
        &destination, vec![fixture.candidate()],
    ).await;
    assert!(result.as_ref().unwrap_err().contains("Cannot publish map job state"));
    let held = manager.status().unwrap().unwrap();
    assert_ne!(held.status, "complete");
    assert!(held.path.is_none());
    let recovery = held.owned_partial.unwrap();
    let output = destination.join(&recovery.publication.as_ref().unwrap().name);
    assert_eq!(fs::read(&output).unwrap(), fixture.bytes);
    assert!(manager.claims_monitored_path(&output).unwrap());
    let durable: MapJob = serde_json::from_slice(&fs::read(&retained_state).unwrap()).unwrap();
    assert!(durable.owned_partial.unwrap().publication.is_some());
    fs::remove_dir(&state_path).unwrap();
    fs::rename(&retained_state, &state_path).unwrap();
    manager.finish("offline-fixture", &control, result);
    assert!(manager.status().unwrap().unwrap().owned_partial.is_some());
    assert!(manager.claims_monitored_path(&output).unwrap());
    drop(manager);
    let recovered = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let job = recovered.status().unwrap().unwrap();
    assert_eq!(job.status, "complete");
    assert_eq!(job.path.as_deref(), output.to_str());
    assert!(job.owned_partial.is_none());
    assert!(recovered.claims_monitored_path(&output).unwrap());
    assert_eq!(fs::read(output).unwrap(), fixture.bytes);
}

#[tokio::test]
async fn start_reclaims_an_uncommitted_publication_instead_of_blocking_forever() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let record = fixture.publication_checkpoint(&manager, &destination);
    manager.finish("offline-fixture", &Control::new(), Err(CANCELLED.into()));
    // The recorded inode still carries its partial name, so the exclusive
    // rename never committed. That is provable ownership, not a reason to
    // veto every later download: reclaim it and start the next map.
    let queued = manager
        .start(
            "https://offline-fixture.invalid/index.json".into(),
            "https://offline-fixture.invalid/".into(),
            "another-region".into(),
            "fr".into(),
            fixture.root.join("another-output"),
            Vec::new(),
            None,
            None,
        )
        .unwrap();
    assert!(queued.owned_partial.is_none());
    assert!(queued.recovery_records.is_empty());
    assert!(queued.error.is_none());
    assert!(!destination.join(record.name).exists());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    manager.cancel().await.unwrap();
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.region_id, "another-region");
    assert!(job.owned_partial.is_none());
    assert!(job.recovery_records.is_empty());
}

#[tokio::test]
async fn unresolved_current_publication_keeps_its_warning_without_vetoing_a_new_region() {
    for replace_directory in [false, true] {
        let fixture = Fixture::new();
        let state_directory = fixture.store.data_dir().join("maps");
        let manager = fixture.manager_in(state_directory.clone());
        let destination = fixture.root.join("output");
        let record = fixture.publication_checkpoint(&manager, &destination);
        let retained = fixture.root.join("retained-original");
        if replace_directory {
            fs::rename(&destination, &retained).unwrap();
            fs::create_dir(&destination).unwrap();
        } else {
            fs::rename(destination.join(&record.name), &retained).unwrap();
        }
        let foreign = destination.join(&record.publication.as_ref().unwrap().name);
        fs::write(&foreign, b"unowned archive must survive").unwrap();
        drop(manager);
        let manager =
            MapManager::new(state_directory.clone(), fixture.store.clone()).unwrap();
        assert!(manager.status().unwrap().unwrap().owned_partial.is_some());
        let started = manager
            .start(
                "https://offline-fixture.invalid/index.json".into(),
                "https://offline-fixture.invalid/".into(),
                "another-region".into(),
                "fr".into(),
                fixture.root.join("another-output"),
                Vec::new(),
                None,
                None,
            )
            .unwrap();
        // The unprovable record moves to the next job's warning list; it
        // never authorizes deletion and no longer blocks the download.
        assert_eq!(started.region_id, "another-region");
        assert!(started.owned_partial.is_none());
        assert_eq!(started.recovery_records.len(), 1);
        assert!(started
            .error
            .as_deref()
            .is_some_and(|error| error.contains("could not be cleaned up")));
        assert!(!started.can_remove_from_list);
        assert!(manager.claims_monitored_path(&foreign).unwrap());
        assert!(!manager.claims_monitored_path(&destination.join("external.zim")).unwrap());
        manager.cancel().await.unwrap();
        drop(manager);
        let claims = MapManager::saved_monitored_paths(&fixture.store).unwrap();
        assert!(claims.claims_monitored_path(&foreign).unwrap());
        assert!(!claims.claims_monitored_path(&destination.join("external.zim")).unwrap());

        let manager =
            MapManager::new(state_directory, fixture.store.clone()).unwrap();
        let job = manager.status().unwrap().unwrap();
        assert_eq!(job.region_id, "another-region");
        assert!(job.owned_partial.is_none());
        assert_eq!(job.recovery_records.len(), 1);
        assert!(job.error.is_some());
        assert!(manager.claims_monitored_path(&foreign).unwrap());
        assert_eq!(fs::read(foreign).unwrap(), b"unowned archive must survive");
        let original = if replace_directory {
            retained.join(&record.name)
        } else {
            retained
        };
        assert_eq!(fs::read(original).unwrap(), fixture.bytes);
    }
}

#[tokio::test]
async fn a_failed_new_job_checkpoint_keeps_the_previous_unresolved_record() {
    let fixture = Fixture::new();
    let state_directory = fixture.store.data_dir().join("maps");
    let manager = fixture.manager_in(state_directory.clone());
    let destination = fixture.root.join("output");
    let record = fixture.publication_checkpoint(&manager, &destination);
    fs::rename(destination.join(&record.name), fixture.root.join("retained-original")).unwrap();
    let foreign = destination.join(&record.publication.as_ref().unwrap().name);
    fs::write(&foreign, b"unowned archive must survive").unwrap();
    drop(manager);
    let manager = MapManager::new(state_directory.clone(), fixture.store.clone()).unwrap();
    assert!(manager.status().unwrap().unwrap().owned_partial.is_some());
    assert!(manager.claims_monitored_path(&foreign).unwrap());
    // The new job's checkpoint cannot be published (e.g. a full volume).
    let state_file = state_directory.join("map-job.json");
    let saved = fs::read(&state_file).unwrap();
    fs::remove_file(&state_file).unwrap();
    fs::create_dir(&state_file).unwrap();
    fs::write(state_file.join("blocker"), b"x").unwrap();
    let start = |manager: &MapManager| manager.start(
        "https://offline-fixture.invalid/index.json".into(),
        "https://offline-fixture.invalid/".into(),
        "another-region".into(),
        "fr".into(),
        fixture.root.join("another-output"),
        Vec::new(),
        None,
        None,
    );
    assert!(start(&manager).is_err());
    // The unresolved record is still owned by the live previous job.
    let previous = manager.status().unwrap().unwrap();
    assert!(previous.owned_partial.is_some(), "the unresolved record was dropped");
    assert!(manager.claims_monitored_path(&foreign).unwrap());
    // After the volume recovers, a retried start still carries it forward.
    fs::remove_dir_all(&state_file).unwrap();
    fs::write(&state_file, saved).unwrap();
    let started = start(&manager).unwrap();
    assert_eq!(started.recovery_records.len(), 1);
    assert!(manager.claims_monitored_path(&foreign).unwrap());
    manager.cancel().await.unwrap();
    assert_eq!(fs::read(&foreign).unwrap(), b"unowned archive must survive");
}

#[test]
fn pause_racing_storage_failure_retains_the_terminal_error_after_restart() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let control = manager.inner.state.lock().unwrap().active.clone().unwrap();
    control.request_pause();
    manager.finish("offline-fixture", &control, Err("Cannot sync composed map".into()));
    let reopened = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let job = reopened.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert_eq!(job.error.as_deref(), Some("Cannot sync composed map"));
    assert!(!job.resumable);
    reopened.remove_terminal(&job.id).unwrap();
    assert!(reopened.status().unwrap().is_none());
}

#[tokio::test]
async fn cancel_never_revokes_a_worker_admitted_while_the_old_one_retired() {
    for status in ["queued", "paused"] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let decision: MapDecision = serde_json::from_value(serde_json::json!({
            "token": "t", "session": "live-session", "generation": 0,
            "catalogUrl": "https://offline-fixture.invalid/index.json",
            "proxyUrl": "https://offline-fixture.invalid/", "destination": "/tmp",
            "regionId": "fixture", "language": "en", "name": "fixture.zim",
            "md5": "0", "uuid": "u", "size": 1, "target": null,
        })).unwrap();
        let old = {
            let mut state = manager.inner.state.lock().unwrap();
            state.job.as_mut().unwrap().decision = Some(decision);
            state.active.clone().unwrap()
        };
        let canceller = manager.clone();
        let cancelling = tokio::spawn(async move { canceller.cancel().await });
        let mut cancel_seen = old.cancel.subscribe();
        tokio::time::timeout(Duration::from_secs(5), cancel_seen.wait_for(|cancelled| *cancelled)).await.unwrap().unwrap();
        {
            // A second surface resumed the same job. Its worker can still be
            // writing or have paused again before the old waiter wakes.
            let mut state = manager.inner.state.lock().unwrap();
            let job = state.job.as_mut().unwrap();
            job.status = status.into();
            state.admission += 1;
            state.active = (status == "queued").then(|| Arc::new(Control::new()));
        }
        old.done.send_replace(Some(Ok(())));
        tokio::time::timeout(Duration::from_secs(5), cancelling).await.unwrap().unwrap().unwrap();
        let state = manager.inner.state.lock().unwrap();
        let job = state.job.as_ref().unwrap();
        assert_eq!(job.status, status);
        assert_eq!(job.decision.as_ref().unwrap().session, "live-session", "the newer admission's authorization was revoked");
    }
}

#[tokio::test]
async fn restart_reclaims_only_recorded_partial_and_rebuilds_from_verified_chunks() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let recipe = fixture.recipe();
    let mut masked = fixture.bytes.clone();
    masked[32..64].fill(0);
    let mut cache = MapChunkCache::open(&manager.inner.cache, || Ok(())).unwrap();
    for (index, bytes) in masked.chunks(recipe.base.chunk as usize).enumerate() {
        let hash = hex::encode(Sha256::digest(bytes));
        assert_eq!(hash, recipe.base.sha256[index]);
        cache.store(&format!("{hash}-{}", bytes.len()), Arc::new(bytes.to_vec()), || Ok(()))
        .await
        .unwrap();
    }
    drop(cache);
    let record = fixture.interrupted_partial(&manager, &destination);
    let unrelated = destination.join(format!(".streetzim-{}.part", Uuid::new_v4()));
    fs::write(&unrelated, b"unrelated sentinel").unwrap();
    drop(manager);

    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert!(!destination.join(&record.name).exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated sentinel");
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.owned_partial.is_none());
    let rebuilt = manager
        .compose(
            "offline-fixture",
            &Control::new(),
            fixture.recipe(),
            url.as_str(),
            &url,
            &destination,
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(fs::read(rebuilt).unwrap(), fixture.bytes);
    assert_eq!(fs::read(unrelated).unwrap(), b"unrelated sentinel");
}

#[test]
fn restart_preserves_a_substituted_partial_or_destination_directory() {
    for replace_directory in [false, true] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let destination = fixture.root.join("output");
        let record = fixture.interrupted_partial(&manager, &destination);
        let retained = fixture.root.join("retained-original");
        if replace_directory {
            fs::rename(&destination, &retained).unwrap();
            fs::create_dir(&destination).unwrap();
        } else {
            fs::rename(destination.join(&record.name), &retained).unwrap();
        }
        let sentinel = destination.join(&record.name);
        fs::write(&sentinel, b"foreign sentinel").unwrap();
        drop(manager);
        MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"foreign sentinel");
        assert!(retained.exists());
    }
}

#[cfg(unix)]
#[test]
fn restart_never_follows_a_substituted_partial_symlink() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let record = fixture.interrupted_partial(&manager, &destination);
    let path = destination.join(&record.name);
    fs::rename(&path, fixture.root.join("retained-original")).unwrap();
    let sentinel = fixture.root.join("sentinel");
    fs::write(&sentinel, b"symlink target sentinel").unwrap();
    std::os::unix::fs::symlink(&sentinel, &path).unwrap();
    drop(manager);
    MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&sentinel).unwrap(), b"symlink target sentinel");
}

#[test]
fn restart_preserves_the_unproven_create_to_checkpoint_crash_reservation() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    fs::create_dir(&destination).unwrap();
    let directory = MapDirectory::open(&destination).unwrap();
    let record = OwnedPartial {
        destination,
        directory_identity: directory.identity,
        name: format!(".streetzim-{}.part", Uuid::new_v4()),
        identity: None,
        capture: None,
        publication: None,
    };
    manager
        .checkpoint_partial("offline-fixture", Some(record.clone()))
        .unwrap();
    let path = record.destination.join(&record.name);
    // A crash after create but before identity persistence cannot distinguish
    // the empty reservation from another actor's entry, so it must not erase it.
    fs::write(&path, b"unproven sentinel").unwrap();
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert_eq!(fs::read(path).unwrap(), b"unproven sentinel");
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.path.is_none());
    assert!(job.owned_partial.is_some());
}

#[tokio::test]
async fn a_vanished_unprovable_partial_is_cleared_and_does_not_block_the_next_map() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("vanished-output");
    fs::create_dir(&destination).unwrap();
    let directory = MapDirectory::open(&destination).unwrap();
    let record = OwnedPartial {
        destination: destination.clone(),
        directory_identity: directory.identity,
        name: format!(".streetzim-{}.part", Uuid::new_v4()),
        identity: None,
        capture: None,
        publication: None,
    };
    manager.checkpoint_partial("offline-fixture", Some(record)).unwrap();
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let job = manager.status().unwrap().unwrap();
    assert!(job.owned_partial.is_none(), "an absent unprovable partial is reclaimed");
    assert!(job.recovery_records.is_empty());
    let started = manager
        .start(
            "https://offline-fixture.invalid/index.json".into(),
            "https://offline-fixture.invalid/".into(),
            "another-region".into(),
            "fr".into(),
            fixture.root.join("another-output"),
            Vec::new(),
            None,
            None,
        )
        .unwrap();
    assert!(started.owned_partial.is_none());
    assert_eq!(started.region_id, "another-region");
    manager.cancel().await.unwrap();
}

#[tokio::test]
async fn a_preserved_unprovable_partial_can_be_forgotten_without_deleting_it() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("unproven-output");
    fs::create_dir(&destination).unwrap();
    let directory = MapDirectory::open(&destination).unwrap();
    let record = OwnedPartial {
        destination: destination.clone(),
        directory_identity: directory.identity,
        name: format!(".streetzim-{}.part", Uuid::new_v4()),
        identity: None,
        capture: None,
        publication: None,
    };
    manager.checkpoint_partial("offline-fixture", Some(record.clone())).unwrap();
    let path = destination.join(&record.name);
    fs::write(&path, b"unproven sentinel").unwrap();
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    assert!(manager.status().unwrap().unwrap().owned_partial.is_some());
    let started = manager
        .start(
            "https://offline-fixture.invalid/index.json".into(),
            "https://offline-fixture.invalid/".into(),
            "another-region".into(),
            "fr".into(),
            fixture.root.join("another-output"),
            Vec::new(),
            None,
            None,
        )
        .unwrap();
    assert_eq!(started.recovery_records.len(), 1);
    manager.forget_recovery(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"unproven sentinel");
    let job = manager.status().unwrap().unwrap();
    assert!(job.recovery_records.is_empty());
    assert!(!job.error.as_deref().is_some_and(|error| error.contains("could not be cleaned up")));
    manager.cancel().await.unwrap();
    assert!(manager.status().unwrap().unwrap().can_remove_from_list);
}

#[tokio::test]
async fn full_composer_rejects_a_foreign_file_substituted_after_checksum_validation() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let retained = fixture.root.join("verified-original.zim");
    let saved_original = retained.clone();
    *manager.inner.before_publish.lock().unwrap() = Some(Box::new(move |partial| {
        fs::rename(partial, &saved_original).unwrap();
        fs::write(partial, b"foreign publication sentinel").unwrap();
    }));
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager
        .compose(
            "offline-fixture",
            &control,
            fixture.recipe(),
            url.as_str(),
            &url,
            &destination,
            vec![fixture.candidate()],
        )
        .await;
    assert!(result.is_err());
    manager.finish("offline-fixture", &control, result);
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.path.is_none());
    assert_eq!(fs::read(retained).unwrap(), fixture.bytes);
    let files: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        fs::read(&files[0]).unwrap(),
        b"foreign publication sentinel"
    );
}

#[test]
fn cleanup_captures_replacements_and_preserves_exclusive_restoration_collisions() {
    for collision in [false, true] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let destination = fixture.root.join("output");
        let mut record = fixture.interrupted_partial(&manager, &destination);
        let source = destination.join(&record.name);
        let original = fixture.root.join("original-partial");
        let mut captured = false;
        let result = reclaim_partial_with_capture(
            &mut record,
            |record| manager.checkpoint_partial("offline-fixture", Some(record.clone())),
            |from, name, to, target| {
                assert!(!captured);
                captured = true;
                // Replace after the public-name check, at atomic capture.
                fs::rename(&source, &original).unwrap();
                fs::write(&source, b"replacement must survive").unwrap();
                capture_partial(from, name, to, target)?;
                if collision {
                    fs::write(&source, b"restoration collision must survive").unwrap();
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(captured);
        assert_eq!(
            fs::read(&original).unwrap(),
            b"unverified interrupted output"
        );
        let recovery = destination
            .join(&record.capture.as_ref().unwrap().name)
            .join(&record.name);
        let assert_preserved = || {
            if collision {
                assert_eq!(
                    fs::read(&source).unwrap(),
                    b"restoration collision must survive"
                );
                assert_eq!(fs::read(&recovery).unwrap(), b"replacement must survive");
            } else {
                assert_eq!(fs::read(&source).unwrap(), b"replacement must survive");
                assert!(!recovery.exists());
            }
        };
        assert_preserved();
        // Live guard cleanup and startup both honor the retained capability.
        drop(OwnedPartialGuard {
            manager: &manager,
            id: "offline-fixture",
            record: record.clone(),
            armed: true,
        });
        assert_preserved();
        drop(manager);
        let manager =
            MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
        assert!(manager.status().unwrap().unwrap().owned_partial.is_some());
        assert_preserved();
    }
}

// The swap needs renaming a directory while the composer holds an open
// capability for it, which only Unix permits. On Windows cap-std directory
// handles never share delete, so the swap itself is refused; the Windows test
// below proves that pin instead.
#[cfg(unix)]
#[tokio::test]
async fn full_composer_rejects_directory_swap_at_capability_publication() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let retained = fixture.root.join("moved-output");
    let moved = retained.clone();
    *manager.inner.before_publish_commit.lock().unwrap() = Some(Box::new(move |path| {
        let parent = path.parent().unwrap();
        fs::rename(parent, &moved).unwrap();
        fs::create_dir(parent).unwrap();
        fs::write(path, b"replacement-directory output must survive").unwrap();
    }));
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager
        .compose(
            "offline-fixture",
            &control,
            fixture.recipe(),
            url.as_str(),
            &url,
            &destination,
            vec![fixture.candidate()],
        )
        .await;
    assert!(result.is_err());
    manager.finish("offline-fixture", &control, result);
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.path.is_none());
    let record = job.owned_partial.unwrap();
    let publication = record.publication.as_ref().unwrap();
    assert_eq!(publication.size, fixture.bytes.len() as u64);
    assert_eq!(
        fs::read(destination.join(&publication.name)).unwrap(),
        b"replacement-directory output must survive"
    );
    assert_eq!(
        fs::read(retained.join(&publication.name)).unwrap(),
        fixture.bytes
    );
    let actual_directory = MapDirectory::open(&retained).unwrap();
    assert_eq!(actual_directory.identity, record.directory_identity);
    let metadata = actual_directory
        .directory
        .symlink_metadata(&publication.name)
        .unwrap();
    assert_eq!(Some(FileIdentity::of(&metadata)), record.identity);
    drop(manager);
    let manager = MapManager::new(fixture.root.join("state"), fixture.store.clone()).unwrap();
    let recovered = manager.status().unwrap().unwrap();
    assert_eq!(recovered.status, "failed");
    assert!(recovered.path.is_none());
    assert!(recovered.owned_partial.unwrap().publication.is_some());
    assert_eq!(
        fs::read(destination.join(&publication.name)).unwrap(),
        b"replacement-directory output must survive"
    );
    assert_eq!(
        fs::read(retained.join(&publication.name)).unwrap(),
        fixture.bytes
    );
}

#[cfg(windows)]
#[tokio::test]
async fn full_composer_destination_capability_pins_its_directory_through_publication() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let retained = fixture.root.join("moved-output");
    let moved = retained.clone();
    *manager.inner.before_publish_commit.lock().unwrap() = Some(Box::new(move |path| {
        let error = fs::rename(path.parent().unwrap(), &moved).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32), "{error}");
    }));
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager.compose("offline-fixture", &control, fixture.recipe(),
        url.as_str(), &url, &destination, vec![fixture.candidate()]).await;
    let published = result.as_ref().unwrap().clone();
    manager.finish("offline-fixture", &control, result);
    assert!(manager.inner.before_publish_commit.lock().unwrap().is_none(), "hook ran");
    assert!(!retained.exists());
    assert_eq!(published.parent().unwrap(), fs::canonicalize(&destination).unwrap());
    assert_eq!(fs::read(&published).unwrap(), fixture.bytes);
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "complete");
    assert!(job.owned_partial.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn full_composer_rejects_a_symlink_substituted_after_checksum_validation() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let destination = fixture.root.join("output");
    let retained = fixture.root.join("verified-original.zim");
    let saved_original = retained.clone();
    let sentinel = fixture.root.join("sentinel");
    fs::write(&sentinel, b"publication symlink target").unwrap();
    let link_target = sentinel.clone();
    *manager.inner.before_publish.lock().unwrap() = Some(Box::new(move |partial| {
        fs::rename(partial, &saved_original).unwrap();
        std::os::unix::fs::symlink(&link_target, partial).unwrap();
    }));
    let control = Control::new();
    let url = checked_url("https://offline-fixture.invalid/base.zim").unwrap();
    let result = manager
        .compose(
            "offline-fixture",
            &control,
            fixture.recipe(),
            url.as_str(),
            &url,
            &destination,
            vec![fixture.candidate()],
        )
        .await;
    assert!(result.is_err());
    manager.finish("offline-fixture", &control, result);
    let job = manager.status().unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.path.is_none());
    assert_eq!(fs::read(retained).unwrap(), fixture.bytes);
    assert_eq!(fs::read(sentinel).unwrap(), b"publication symlink target");
    let files: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    assert!(fs::symlink_metadata(&files[0])
        .unwrap()
        .file_type()
        .is_symlink());
}

#[tokio::test]
async fn fetch_json_decodes_gzip_and_caches_matching_recipes() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let fixture = Fixture::new();
    let recipe = fixture.recipe();
    let expected_md5 = recipe.md5.clone();
    let plain = serde_json::to_vec(&recipe).unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain).unwrap();
    let gzipped = encoder.finish().unwrap();

    // Oversized uncompressed payload that fits comfortably in gzip form.
    let mut bomb_encoder = GzEncoder::new(Vec::new(), Compression::default());
    let oversized = vec![b' '; MAX_JSON + 1024];
    bomb_encoder.write_all(&oversized).unwrap();
    let bomb = bomb_encoder.finish().unwrap();
    assert!(bomb.len() < MAX_JSON);

    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            let head = String::from_utf8_lossy(&request);
            let body = if head.starts_with("GET /bomb.json ") {
                &bomb
            } else {
                server_hits.fetch_add(1, Ordering::SeqCst);
                &gzipped
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        }
    });

    let client = checked_client().unwrap();
    let control = Control::new();
    let url = Url::parse(&format!("http://{addr}/recipe-{}.json", Uuid::new_v4())).unwrap();
    let first = fetch_recipe_cached(&client, url.clone(), &expected_md5, &control).await.unwrap();
    let second = fetch_recipe_cached(&client, url.clone(), &expected_md5, &control).await.unwrap();
    assert_eq!(first.md5, expected_md5);
    assert_eq!(second.md5, expected_md5);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "second fetch should hit the TTL cache");

    // Mismatched expected MD5 must not reuse or populate the cache.
    let mismatch = fetch_recipe_cached(&client, url.clone(), "different-md5", &control).await.unwrap();
    assert_eq!(mismatch.md5, expected_md5);
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // Explicit cache clear must evict cached recipes so subsequent fetches hit the server.
    clear_recipe_cache();
    let refreshed = fetch_recipe_cached(&client, url.clone(), &expected_md5, &control).await.unwrap();
    assert_eq!(refreshed.md5, expected_md5);
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    // Pre-cancelled control must fail immediately even on a cached key.
    let cancelled = Control::new();
    cancelled.cancel.send_replace(true);
    assert_eq!(
        fetch_recipe_cached(&client, url, &expected_md5, &cancelled).await.err().as_deref(),
        Some(CANCELLED)
    );

    // Gzip bomb exceeding MAX_JSON after decompression must be rejected.
    let bomb_url = Url::parse(&format!("http://{addr}/bomb.json")).unwrap();
    let error = fetch_json::<serde_json::Value>(&client, bomb_url, &control).await.unwrap_err();
    assert!(error.contains("size limit"), "{error}");

    server.abort();
}

// Windows: the private cleanup capability is removed through a handle that
// re-proves its journaled identity, after releasing the non-delete-sharing
// cap-std handle. A substitute or a non-empty capability is never removed.
#[cfg(windows)]
#[test]
fn windows_cleanup_capability_removal_is_identity_bound() {
    let fixture = Fixture::new();
    let parent = MapDirectory::open(&fixture.root).unwrap();
    let name = format!(".streetzim-cleanup-{}", Uuid::new_v4());
    parent.directory.create_dir(&name).unwrap();
    let open = |name: &str| parent.directory.open_dir_nofollow(name).unwrap();
    let identity = FileIdentity::of(&open(&name).dir_metadata().unwrap());

    // Holding a cap-std directory handle blocks removal by name (the bug).
    let held = open(&name);
    assert_eq!(parent.directory.remove_dir(&name).unwrap_err().raw_os_error(), Some(32));

    // A non-empty capability fails and is recoverable at its exact identity.
    held.write("entry", b"retained").unwrap();
    let mut staging = Some(held);
    assert!(remove_cleanup_capability(&parent.directory, &name, &mut staging, &identity).is_err());
    assert!(staging.is_none());
    let reopened = reopen_cleanup_capability(&parent.directory, &name, &identity).unwrap();
    assert_eq!(reopened.read("entry").unwrap(), b"retained");
    reopened.remove_file("entry").unwrap();

    // A substituted directory at the journaled name is neither removed nor reacquired.
    drop(reopened);
    let moved = format!("{name}-original");
    parent.directory.rename(&name, &parent.directory, &moved).unwrap();
    parent.directory.create_dir(&name).unwrap();
    let mut staging = Some(open(&name));
    let error = remove_cleanup_capability(&parent.directory, &name, &mut staging, &identity).unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    assert!(parent.directory.symlink_metadata(&name).unwrap().is_dir());
    assert!(reopen_cleanup_capability(&parent.directory, &name, &identity).is_none());

    // The genuine empty capability is removed while its handle was held.
    let mut staging = Some(open(&moved));
    remove_cleanup_capability(&parent.directory, &moved, &mut staging, &identity).unwrap();
    assert!(staging.is_none());
    assert_eq!(
        parent.directory.symlink_metadata(&moved).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}
