// SPDX-License-Identifier: GPL-3.0-or-later
//! Unit tests for `library_files`, kept beside it rather than inline.
#![forbid(unsafe_code)]
use super::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("kiwix-library-files-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn dir(&self) -> Directory {
        Directory::open(&self.0).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn backup_walk_covers_hidden_and_moved_archives_without_admitting_staging() {
    let profile = Temp::new();
    let imported = profile.0.join("archives/mobile-imports-v1/unfinished/book.zim");
    let hidden = profile.0.join(".collection/.book.zim");
    let moved = profile.0.join("chosen-downloads/moved.zimaa");
    let secret = profile.0.join("preferences.json");
    for path in [&imported, &hidden, &moved, &secret] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"retained payload").unwrap();
    }
    // Only publication/Store evidence grants admission. The incomplete
    // import's ordinary .zim basename is deliberately not such evidence.
    let admitted = HashSet::from([hidden.clone(), moved.clone()]);
    let mut included = HashSet::new();
    visit_backup_files(&profile.0, |file, path| {
        assert_eq!(file.metadata().unwrap().len(), 16);
        if admitted.contains(path) { included.insert(path.to_path_buf()); }
        Ok(())
    }).unwrap();
    assert_eq!(included, admitted);
    assert!(!included.contains(&imported));
    assert!(!included.contains(&secret));
}

#[cfg(unix)]
#[test]
fn backup_walk_does_not_follow_provider_symlinks_or_hardlinks() {
    let profile = Temp::new();
    let provider = Temp::new();
    std::fs::write(provider.0.join("outside.zim"), b"provider").unwrap();
    std::os::unix::fs::symlink(&provider.0, profile.0.join("provider")).unwrap();
    std::os::unix::fs::symlink(provider.0.join("outside.zim"), profile.0.join("linked.zim")).unwrap();
    std::fs::hard_link(provider.0.join("outside.zim"), profile.0.join("hardlinked.zim")).unwrap();
    let mut visited = Vec::new();
    visit_backup_files(&profile.0, |_, path| { visited.push(path.to_path_buf()); Ok(()) }).unwrap();
    assert!(visited.is_empty());
}
#[test]
fn backup_receipts_reject_replacement_inodes_and_changed_bytes() {
    let root = Temp::new();
    let bytes = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    let path = root.0.join("book.zim");
    std::fs::write(&path, &bytes).unwrap();
    let original = root.dir().open_file("book.zim").unwrap();
    let metadata = Metadata::from_file(&original).unwrap();
    let identity = (metadata.dev(), metadata.ino());
    let checksum: [u8; 32] = Sha256::digest(&bytes).into();
    let torrent = [BackupEvidence { size: metadata.len(), identity: Some(identity), sha256: Some(checksum) }];
    assert!(BackupEvidence::matches(&torrent, &original).unwrap());
    std::fs::rename(&path, root.0.join("original.zim")).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let replacement = root.dir().open_file("book.zim").unwrap();
    assert!(!BackupEvidence::matches(&torrent, &replacement).unwrap(), "Even identical bytes cannot substitute another inode for an inode-bound receipt");
    let localswarm = [BackupEvidence { size: metadata.len(), identity: Some(identity), sha256: None }];
    assert!(!BackupEvidence::matches(&localswarm, &replacement).unwrap());
    drop(replacement);
    let mut changed = bytes.clone();
    changed[80] ^= 1;
    std::fs::write(&path, changed).unwrap();
    let replacement = root.dir().open_file("book.zim").unwrap();
    let http = [BackupEvidence { size: metadata.len(), identity: None, sha256: Some(checksum) }];
    assert!(!BackupEvidence::matches(&http, &replacement).unwrap(), "A historical path and matching length are not content evidence");
    assert!(BackupEvidence::matches(&http, &original).unwrap(), "Proof reads the retained descriptor, not the replaced pathname");
}

#[test]
fn backup_unbound_history_cannot_promote_even_a_valid_archive() {
    let root = Temp::new();
    let bytes = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
    std::fs::write(root.0.join("book.zim"), &bytes).unwrap();
    let file = root.dir().open_file("book.zim").unwrap();
    let historical_map = [BackupEvidence { size: bytes.len() as u64, identity: None, sha256: None }];
    assert!(!BackupEvidence::matches(&historical_map, &file).unwrap());
    let files = [("book.zim".into(), file)];
    let expected_uuid = Archive::open_files(&root.0, &files).unwrap().metadata().unwrap().id;
    assert!(retained_backup_archive_valid(&root.0, &files, &expected_uuid).unwrap());
    assert!(!retained_backup_archive_valid(&root.0, &files, "unrelated-recorded-uuid").unwrap());
}

#[test]
fn backup_matching_receipt_is_not_erased_by_overlapping_stale_history() {
    let root = Temp::new();
    let bytes = b"ZIM\x04recorded original payload";
    std::fs::write(root.0.join("book.zim"), bytes).unwrap();
    let file = root.dir().open_file("book.zim").unwrap();
    let evidence = [
        BackupEvidence { size: bytes.len() as u64, identity: None, sha256: Some([0; 32]) },
        BackupEvidence { size: bytes.len() as u64, identity: None, sha256: Some(Sha256::digest(bytes).into()) },
        BackupEvidence { size: bytes.len() as u64, identity: None, sha256: None },
    ];
    assert!(BackupEvidence::matches(&evidence, &file).unwrap());
}

#[test]
fn backup_exclusion_is_confined_to_archive_and_operation_payload_namespaces() {
    let profile = Temp::new();
    let id = Uuid::new_v4();
    for relative in [
        ".collection/.book.zim".to_string(),
        "archives/mobile-imports-v1/unfinished/book.zimaa".to_string(),
        format!("chosen/.kiwix-{id}.part"),
        format!("chosen/.streetzim-{id}.part"),
        format!("chosen/.kiwix-publish-{id}"),
        "maps/map-parts/chunk-cache".to_string(),
    ] {
        assert!(backup_payload_path(&profile.0, &profile.0.join(relative)));
    }
    for relative in ["state.json", "credentials.json", "preferences/theme.json", "private/token", ".kiwix-settings.part"] {
        assert!(!backup_payload_path(&profile.0, &profile.0.join(relative)));
    }
    assert!(!backup_payload_path(&profile.0, &profile.0.parent().unwrap().join("foreign.zim")));
}
fn header(size: u64) -> Vec<u8> {
    let mut header = vec![0u8; 80];
    header[..4].copy_from_slice(b"ZIM\x04");
    header[72..80].copy_from_slice(&(size - 16).to_le_bytes());
    header
}
#[test]
fn read_admission_keeps_legacy_single_files_separate_from_mutation_admission() {
    let temp = Temp::new();
    let mut legacy = header(120);
    legacy[56..64].copy_from_slice(&72u64.to_le_bytes());
    legacy[72..80].copy_from_slice(b"text/htm");
    legacy.extend_from_slice(b"l\0\0");
    std::fs::write(temp.0.join("book.zim"), &legacy).unwrap();
    std::fs::write(temp.0.join("book.zimaa"), &legacy).unwrap();
    let directory = temp.dir();
    assert_eq!(read_part_names(&directory, "book.zim", true).unwrap(), vec!["book.zim"]);
    assert!(part_names(&directory, "book.zim").is_err());
    assert!(read_part_names(&directory, "book.zimaa", true).is_err());
}
#[test]
fn split_set_uses_checksum_boundary_not_similarly_prefixed_files() {
    let temp = Temp::new();
    let mut first = header(120);
    first.resize(90, 1);
    std::fs::write(temp.0.join("book.zimaa"), first).unwrap();
    std::fs::write(temp.0.join("book.zimab"), [2u8; 30]).unwrap();
    std::fs::write(temp.0.join("book.zimac"), b"unrelated surplus").unwrap();
    std::fs::write(temp.0.join("book.zimaa.backup"), b"keep").unwrap();
    assert_eq!(
        part_names(&temp.dir(), "book.zimaa").unwrap(),
        vec!["book.zimaa", "book.zimab"]
    );
    assert!(part_names(&temp.dir(), "book.zimab").is_err());
}
#[test]
fn split_set_preserves_mixed_case_names_and_ignores_surplus_parts() {
    let temp = Temp::new();
    let mut first = header(120);
    first.resize(90, 1);
    std::fs::write(temp.0.join("BOOK.ZIMAA"), first).unwrap();
    std::fs::write(temp.0.join("book.ZimAb"), [2u8; 30]).unwrap();
    std::fs::write(temp.0.join("BOOK.ZIMAC"), b"unrelated surplus").unwrap();
    std::fs::write(temp.0.join("BOOK.ZIMAA.backup"), b"keep").unwrap();
    assert_eq!(part_names(&temp.dir(), "BOOK.ZIMAA").unwrap(), vec!["BOOK.ZIMAA", "book.ZimAb"]);
    assert!(part_names(&temp.dir(), "book.ZimAb").is_err());
    assert_eq!(std::fs::read(temp.0.join("BOOK.ZIMAC")).unwrap(), b"unrelated surplus");
}
#[test]
fn split_set_rejects_missing_or_oversized_parts() {
    let temp = Temp::new();
    std::fs::write(temp.0.join("book.zimaa"), header(120)).unwrap();
    assert!(part_names(&temp.dir(), "book.zimaa").is_err());
    std::fs::write(temp.0.join("book.zimab"), [1u8; 41]).unwrap();
    assert!(part_names(&temp.dir(), "book.zimaa").is_err());
}
#[test]
fn exclusive_move_preserves_identity_and_never_overwrites() {
    let source = Temp::new();
    let target = Temp::new();
    std::fs::write(source.0.join("book.zim"), b"owned").unwrap();
    std::fs::write(target.0.join("collision.zim"), b"user").unwrap();
    let stamp = source.dir().stamp("book.zim").unwrap();
    assert!(move_exclusive(&source.dir(), "book.zim", &target.dir(), "collision.zim").is_err());
    assert_eq!(
        std::fs::read(target.0.join("collision.zim")).unwrap(),
        b"user"
    );
    move_exclusive(&source.dir(), "book.zim", &target.dir(), "book.zim").unwrap();
    assert_eq!(target.dir().stamp("book.zim").unwrap(), stamp);
    assert!(!source.0.join("book.zim").exists());
}
#[cfg(unix)]
#[test]
fn capabilities_reject_file_and_directory_symlinks_and_replacement() {
    let root = Temp::new();
    let outside = Temp::new();
    std::fs::write(outside.0.join("book.zim"), b"user").unwrap();
    std::os::unix::fs::symlink(outside.0.join("book.zim"), root.0.join("book.zim")).unwrap();
    assert!(root.dir().stamp("book.zim").is_err());
    std::os::unix::fs::symlink(&outside.0, root.0.join("link")).unwrap();
    assert!(Directory::open(&root.0.join("link")).is_err());
    let pinned = outside.dir();
    let renamed = root.0.join("renamed");
    std::fs::rename(&outside.0, &renamed).unwrap();
    std::fs::create_dir(&outside.0).unwrap();
    assert!(pinned.unchanged().is_err());
}
#[test]
fn untrusted_mimes_and_names_never_become_native_targets() {
    for mime in [
        "text/html",
        "image/svg+xml",
        "application/javascript",
        "application/octet-stream",
        "application/x-msdownload",
        "text/x-shellscript",
        "application/vnd.ms-word.document.macroEnabled.12",
    ] {
        assert!(attachment_extension(mime).is_none(), "{mime}");
    }
    for name in [
        "../book.zim",
        "book.zim/child",
        "C:book.zim",
        "..",
        "book.zim ",
        "book\0.zim",
    ] {
        assert!(filename(name).is_err());
    }
    assert_eq!(
        attachment_extension("application/pdf; charset=binary"),
        Some("pdf")
    );
    assert!(!signature_matches("pdf", b"#!/bin/sh"));
    assert!(signature_matches("pdf", b"%PDF-1.7"));
}
#[test]
fn copied_file_digest_is_bound_to_identity_not_just_length() {
    let root = Temp::new();
    std::fs::write(root.0.join("book.zim"), b"first").unwrap();
    let (stamp, _) = digest(&root.dir(), "book.zim", None).unwrap();
    std::fs::rename(root.0.join("book.zim"), root.0.join("original.zim")).unwrap();
    std::fs::write(root.0.join("book.zim"), b"other").unwrap();
    assert!(digest(&root.dir(), "book.zim", Some(&stamp)).is_err());
    assert_eq!(
        std::fs::read(root.0.join("original.zim")).unwrap(),
        b"first"
    );
}
#[test]
fn read_only_recovery_does_not_create_or_repair_owned_directories() {
    let root = Temp::new();
    let store = Store::new(root.0.clone()).unwrap();
    store.set_runtime_mode(false, true).unwrap();
    assert_eq!(recover(&store).unwrap()["readOnly"], true);
    assert!(!root.0.join("library-migrations-v1").exists());
    assert!(!root.0.join("attachment-exports-v1").exists());
}
#[test]
fn journal_owner_and_paths_are_validated_before_file_operations() {
    let root = Temp::new();
    let store = Store::new(root.0.clone()).unwrap();
    let journals = Journals::open(&store, true).unwrap().unwrap();
    let source = Temp::new();
    let destination = Temp::new();
    std::fs::write(source.0.join("book.zim"), b"preserve user file").unwrap();
    let (stamp, hash) = digest(&source.dir(), "book.zim", None).unwrap();
    let mut journal = Journal {
        version: 1,
        id: Uuid::new_v4().to_string(),
        owner: Uuid::new_v4().to_string(),
        book_id: "not-a-managed-book".into(),
        title: "Untrusted".into(),
        source_directory: source.0.clone(),
        destination_directory: destination.0.clone(),
        source_directory_identity: source.dir().identity,
        destination_directory_identity: destination.dir().identity,
        primary: "book.zim".into(),
        same_filesystem: true,
        staging_identity: None,
        cleanup_staging_identity: None,
        parts: vec![Part {
            name: "book.zim".into(),
            source: stamp,
            sha256: hash,
            target: None,
        }],
    };
    assert!(finish_move(&store, &journals, &mut journal, &mut Vec::new()).is_err());
    journal.owner = journals.owner.clone();
    assert!(finish_move(&store, &journals, &mut journal, &mut Vec::new()).is_err());
    journal.parts[0].name = "../outside.zim".into();
    assert!(validate_journal(&journal, &journals, &store).is_err());
    assert_eq!(
        std::fs::read(source.0.join("book.zim")).unwrap(),
        b"preserve user file"
    );
    assert!(!destination.0.join("book.zim").exists());
}
#[test]
fn owned_journal_roundtrip_is_durable_bounded_and_no_follow() {
    let root = Temp::new();
    let store = Store::new(root.0.clone()).unwrap();
    let journals = Journals::open(&store, true).unwrap().unwrap();
    let source = Temp::new();
    let target = Temp::new();
    std::fs::write(source.0.join("book.zim"), b"owned").unwrap();
    let (stamp, hash) = digest(&source.dir(), "book.zim", None).unwrap();
    let journal = Journal {
        version: 1,
        id: Uuid::new_v4().to_string(),
        owner: journals.owner.clone(),
        book_id: "test".into(),
        title: "Test".into(),
        source_directory: source.0.clone(),
        destination_directory: target.0.clone(),
        source_directory_identity: source.dir().identity,
        destination_directory_identity: target.dir().identity,
        primary: "book.zim".into(),
        same_filesystem: true,
        staging_identity: None,
        cleanup_staging_identity: None,
        parts: vec![Part {
            name: "book.zim".into(),
            source: stamp,
            sha256: hash,
            target: None,
        }],
    };
    journals.write(&journal).unwrap();
    let bytes =
        std::fs::read(journals.directory.path.join(format!("{}.json", journal.id))).unwrap();
    let restored: Journal = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored.owner, journals.owner);
    assert_eq!(restored.parts[0].source, journal.parts[0].source);
    assert_eq!(restored.parts[0].sha256, journal.parts[0].sha256);
    journals.remove(&journal.id).unwrap();
    assert!(!journals
        .directory
        .path
        .join(format!("{}.json", journal.id))
        .exists());
}
fn split_fixture(root: &Temp) -> Book {
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
    )
    .unwrap();
    let midpoint = bytes.len() / 2;
    let path = root.0.join("download.zimaa");
    std::fs::write(&path, &bytes[..midpoint]).unwrap();
    std::fs::write(root.0.join("download.zimab"), &bytes[midpoint..]).unwrap();
    Book {
        metadata: Archive::open(&path).unwrap().metadata().unwrap(),
        installation_id: crate::store::new_installation_id(),
        path: path.to_string_lossy().into_owned(),
        added_at: crate::store::now(),
        available: true,
        managed: true,
        source_reference: None,
        integrity: None,
    }
}
#[cfg(all(unix, desktop))]
#[tokio::test]
async fn retained_reads_preserve_hardlinks_and_platform_names_without_destructive_authority() {
    for (split, stem, hardlink) in [
        (false, "linked", true), (true, "linked", true),
        (false, "book:2026", false), (true, "book:2026", false),
        (false, "book\\2026", false), (true, "book\\2026", false),
    ] {
        let profile = Temp::new();
        let service = crate::service::Service::new(profile.0.clone(), false, false).unwrap();
        let directory = profile.0.join("archives");
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim")).unwrap();
        let names = if split { vec![format!("{stem}.zimaa"), format!("{stem}.zimab")] } else { vec![format!("{stem}.zim")] };
        for (index, name) in names.iter().enumerate() {
            let path = directory.join(name);
            let start = index * bytes.len() / names.len();
            let end = (index + 1) * bytes.len() / names.len();
            std::fs::write(&path, &bytes[start..end]).unwrap();
            if hardlink {
                std::fs::hard_link(&path, directory.join(format!("{name}.saved"))).unwrap();
            }
        }
        let primary = directory.join(&names[0]);
        let imported = service.import_paths(vec![primary.to_string_lossy().into_owned()], true).unwrap();
        assert_eq!(imported["errors"], serde_json::json!([]));
        let book = service.store.read().unwrap().books[0].clone();
        let archive = service.archive(&book.metadata.id).unwrap();
        assert!(archive.check_integrity().unwrap());
        let titles = archive.title_batch(0, 1).unwrap();
        let query = titles.titles.first().unwrap();
        let results = archive.search(query, false, 0, 50).unwrap();
        let hit = results.hits.first().expect("Imported platform-valid archive must remain searchable");
        assert!(!archive.entry(&hit.path).unwrap().path.is_empty());
        let retained = crate::archive_source::retain(&book).unwrap();
        retained.unchanged().unwrap();
        assert!(retain_archive_files(&book).is_err());
        assert!(pin_archive_files(&book).is_err());
        assert!(delete_book(&book, false).is_err());
        for name in names {
            assert!(directory.join(&name).exists());
            if hardlink {
                assert!(directory.join(format!("{name}.saved")).exists());
            }
        }
    }
}
#[test]
fn split_deletion_removes_only_verified_exact_parts_and_rejects_wrong_uuid() {
    let root = Temp::new();
    let book = split_fixture(&root);
    std::fs::write(root.0.join("download.zimaa.backup"), b"user backup").unwrap();
    std::fs::write(root.0.join("download.zimab.notes"), b"user notes").unwrap();
    let mut wrong = book.clone();
    wrong.metadata.id = Uuid::new_v4().to_string();
    assert!(delete_book(&wrong, false).is_err());
    assert!(Path::new(&book.path).exists());
    assert!(root.0.join("download.zimab").exists());
    delete_book(&book, false).unwrap();
    assert!(!Path::new(&book.path).exists());
    assert!(!root.0.join("download.zimab").exists());
    assert_eq!(
        std::fs::read(root.0.join("download.zimaa.backup")).unwrap(),
        b"user backup"
    );
    assert_eq!(
        std::fs::read(root.0.join("download.zimab.notes")).unwrap(),
        b"user notes"
    );
}
#[test]
fn deletion_and_listing_prove_identity_without_reading_archive_bytes() {
    let root = Temp::new();
    let mut bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
    )
    .unwrap();
    // Damage a payload byte while the header, uuid and declared part
    // boundary stay intact. A full content checksum rejects this archive;
    // the ownership proof must not, so the user can delete what they see.
    let index = bytes.len() / 2;
    bytes[index] ^= 0xff;
    let path = root.0.join("damaged.zim");
    std::fs::write(&path, &bytes).unwrap();
    let book = Book {
        metadata: Archive::open(&path).unwrap().metadata().unwrap(),
        installation_id: crate::store::new_installation_id(),
        path: path.to_string_lossy().into_owned(),
        added_at: crate::store::now(),
        available: true,
        managed: true,
        source_reference: None,
        integrity: None,
    };
    let damaged = Archive::open(&path).unwrap();
    assert!(
        damaged.check_integrity().map_or(true, |valid| !valid),
        "fixture must fail a content checksum"
    );
    // The delete confirmation lists exact parts without hashing the archive.
    assert_eq!(archive_files(&book).unwrap(), vec![path.clone()]);
    // The library-relocation gate keeps its full integrity requirement.
    assert!(verified_archive_files(&book).is_err());
    // Windows refuses to move a part that an open reader still leases, so the
    // service retires every reader there first. Unix deletes under the reader.
    #[cfg(windows)]
    drop(damaged);
    delete_book(&book, false).unwrap();
    assert!(!path.exists());
}
#[test]
fn a_failed_staged_rename_restores_every_part_and_leaves_no_copy() {
    let root = Temp::new();
    let book = split_fixture(&root);
    let first = std::fs::read(&book.path).unwrap();
    let second = std::fs::read(root.0.join("download.zimab")).unwrap();
    // The second staged rename fails the way a cross-device move would:
    // deletion must restore the first part instead of copying or erasing.
    MOVE_FAILURE_AT.set(Some(2));
    let error = delete_book(&book, false).unwrap_err();
    assert!(error.contains("simulated cross-device move"), "{error}");
    assert_eq!(std::fs::read(&book.path).unwrap(), first);
    assert_eq!(std::fs::read(root.0.join("download.zimab")).unwrap(), second);
    let mut entries: Vec<_> = std::fs::read_dir(&root.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            std::ffi::OsString::from("download.zimaa"),
            std::ffi::OsString::from("download.zimab"),
        ],
        "no staging directory or copied duplicate may survive an aborted delete"
    );
    // The restored archive is still complete and deletable.
    assert!(Archive::open(Path::new(&book.path)).unwrap().check_integrity().unwrap());
    delete_book(&book, false).unwrap();
    assert!(!Path::new(&book.path).exists());
}

#[test]
fn duplicate_intake_cleanup_removes_only_the_matching_archive() {
    let root = Temp::new();
    let single = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
    )
    .unwrap();
    let other = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/delimiters.zim"),
    )
    .unwrap();
    let duplicate = root.0.join("foo.zim");
    std::fs::write(&duplicate, &single).unwrap();
    let uuid = Archive::open(&duplicate).unwrap().metadata().unwrap().id;

    // Same stem, different archive, admitted in the same batch: the old
    // stem-prefix sweep deleted these parts and destroyed the new book.
    let midpoint = other.len() / 2;
    std::fs::write(root.0.join("foo.zimaa"), &other[..midpoint]).unwrap();
    std::fs::write(root.0.join("foo.zimab"), &other[midpoint..]).unwrap();
    std::fs::write(root.0.join("foo.zimaa.backup"), b"user backup").unwrap();

    remove_duplicate_intake_copy(duplicate.to_str().unwrap(), &uuid).unwrap();
    assert!(!duplicate.exists());
    assert!(root.0.join("foo.zimaa").exists());
    assert!(root.0.join("foo.zimab").exists());
    assert_eq!(
        std::fs::read(root.0.join("foo.zimaa.backup")).unwrap(),
        b"user backup"
    );

    // A differently cased exact copy is still the duplicate itself; the
    // old lowercase `.zim` search leaked it in staging.
    let upper = root.0.join("baz.ZIM");
    std::fs::write(&upper, &single).unwrap();
    remove_duplicate_intake_copy(upper.to_str().unwrap(), &uuid).unwrap();
    assert!(!upper.exists());

    // A split duplicate removes its own cohort, never a same-stem single
    // file that belongs to another archive.
    let sibling = root.0.join("bar.zim");
    std::fs::write(&sibling, &single).unwrap();
    std::fs::write(root.0.join("bar.zimaa"), &other[..midpoint]).unwrap();
    std::fs::write(root.0.join("bar.zimab"), &other[midpoint..]).unwrap();
    let split_uuid = Archive::open(&root.0.join("bar.zimaa"))
        .unwrap()
        .metadata()
        .unwrap()
        .id;
    remove_duplicate_intake_copy(
        root.0.join("bar.zimaa").to_str().unwrap(),
        &split_uuid,
    )
    .unwrap();
    assert!(!root.0.join("bar.zimaa").exists());
    assert!(!root.0.join("bar.zimab").exists());
    assert_eq!(std::fs::read(&sibling).unwrap(), single);
}

#[test]
fn duplicate_intake_cleanup_never_deletes_a_replacement() {
    let root = Temp::new();
    let single = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
    )
    .unwrap();
    let other = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/delimiters.zim"),
    )
    .unwrap();
    let staged = root.0.join("staged.zim");
    std::fs::write(&staged, &single).unwrap();
    let uuid = Archive::open(&staged).unwrap().metadata().unwrap().id;
    let replacement = root.0.join("replacement.zim");
    std::fs::write(&replacement, &other).unwrap();
    let target = staged.clone();
    *DUPLICATE_INTAKE_HOOK.lock().unwrap() = Some((
        std::thread::current().id(),
        Box::new(move || {
            std::fs::rename(&replacement, &target).unwrap();
        }),
    ));
    // The identity was captured before the archive was opened, so the
    // swap is detected and the replacement survives.
    let error = remove_duplicate_intake_copy(staged.to_str().unwrap(), &uuid).unwrap_err();
    assert!(
        error.contains("changed while it was being removed"),
        "{error}"
    );
    assert_eq!(std::fs::read(&staged).unwrap(), other);
}

fn imported_book(path: &Path) -> Book {
    Book {
        metadata: Archive::open(path).unwrap().metadata().unwrap(),
        installation_id: crate::store::new_installation_id(),
        path: path.to_string_lossy().into_owned(),
        added_at: crate::store::now(),
        available: true,
        managed: false,
        source_reference: None,
        integrity: None,
    }
}

#[test]
fn permanent_delete_removes_only_the_emptied_import_directory() {
    let profile = Temp::new();
    let collection = profile.0.join("archives/mobile-imports-v1");
    let staging = collection.join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&staging).unwrap();
    let path = staging.join("imported.zim");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
        &path,
    )
    .unwrap();
    let book = imported_book(&path);
    // A neighbouring import directory with content is not ours to remove.
    let sibling = collection.join(Uuid::new_v4().to_string());
    std::fs::create_dir(&sibling).unwrap();
    std::fs::write(sibling.join("retained.zim"), b"retained payload").unwrap();
    delete_book(&book, false).unwrap();
    assert!(!staging.exists(), "the emptied import directory must be removed");
    assert_eq!(
        std::fs::read(sibling.join("retained.zim")).unwrap(),
        b"retained payload"
    );
    assert!(collection.exists());
}

#[test]
fn empty_import_directory_cleanup_requires_the_app_owned_shape() {
    let profile = Temp::new();
    let collection = profile.0.join("archives/mobile-imports-v1");
    // An empty directory whose name is not a canonical UUID is left alone.
    let foreign = collection.join("not-an-import");
    std::fs::create_dir_all(&foreign).unwrap();
    remove_empty_import_directory(&foreign).unwrap();
    assert!(foreign.exists());
    // A canonical UUID directory outside the import collection is left alone.
    let elsewhere = profile.0.join("archives").join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&elsewhere).unwrap();
    remove_empty_import_directory(&elsewhere).unwrap();
    assert!(elsewhere.exists());
}

#[test]
fn download_bound_deletion_rejects_partial_authority_and_same_path_replacement() {
    let root = Temp::new();
    let book = split_fixture(&root);
    let paths = [root.0.join("download.zimaa"), root.0.join("download.zimab")];
    let bytes: Vec<_> = paths.iter().map(|path| std::fs::read(path).unwrap()).collect();
    let ownership: Vec<_> = paths.iter().map(|path| {
        (path.clone(), crate::torrent::CleanupEntry::retain(path, &File::open(path).unwrap()).unwrap())
    }).collect();
    assert!(delete_book_with_ownership(&book, false, &ownership[..1]).is_err());
    for (path, expected) in paths.iter().zip(&bytes) {
        assert_eq!(&std::fs::read(path).unwrap(), expected);
    }
    let original = root.0.join("retained-original.zimaa");
    std::fs::rename(&paths[0], &original).unwrap();
    std::fs::write(&paths[0], &bytes[0]).unwrap();
    assert!(delete_book_with_ownership(&book, false, &ownership).is_err());
    assert_eq!(std::fs::read(&original).unwrap(), bytes[0]);
    for (path, expected) in paths.iter().zip(&bytes) {
        assert_eq!(&std::fs::read(path).unwrap(), expected);
    }
}

#[test]
fn an_empty_or_directory_archive_path_never_widens_deletion() {
    // Upstream kiwix-desktop #816: an empty basename became a wildcard and
    // deleted unrelated files in the working directory.
    let root = Temp::new();
    let book = split_fixture(&root);
    let parts = [root.0.join("download.zimaa"), root.0.join("download.zimab")];
    let sentinel = root.0.join("unrelated.zim");
    std::fs::write(&sentinel, b"keep").unwrap();
    for path in [String::new(), root.0.to_string_lossy().into_owned()] {
        let mut malformed = book.clone();
        malformed.path = path;
        assert!(delete_book(&malformed, false).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
        for part in &parts {
            assert!(part.exists(), "{} was removed by a malformed archive path", part.display());
        }
    }
}

#[test]
fn download_bound_split_deletion_reclaims_owned_parts_but_keeps_neighbor_files() {
    let root = Temp::new();
    let book = split_fixture(&root);
    let paths = [root.0.join("download.zimaa"), root.0.join("download.zimab")];
    let ownership: Vec<_> = paths.iter().map(|path| {
        (path.clone(), crate::torrent::CleanupEntry::retain(path, &File::open(path).unwrap()).unwrap())
    }).collect();
    let neighbor = root.0.join("download.zimab.notes");
    std::fs::write(&neighbor, b"user notes").unwrap();
    delete_book_with_ownership(&book, false, &ownership).unwrap();
    assert!(!paths[0].exists());
    assert!(!paths[1].exists());
    assert_eq!(std::fs::read(&neighbor).unwrap(), b"user notes");
}
#[test]
fn startup_recovers_a_split_archive_interrupted_between_exclusive_renames() {
    let profile = Temp::new();
    let source = Temp::new();
    let target = Temp::new();
    let book = split_fixture(&source);
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.books.push(book.clone());
            state.settings.download_directory = target.0.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    let journals = Journals::open(&store, true).unwrap().unwrap();
    let directory = source.dir();
    let files = verified(&directory, "download.zimaa", &book.metadata.id).unwrap();
    let parts = files
        .into_iter()
        .map(|(name, stamp)| {
            let (_, sha256) = digest(&directory, &name, Some(&stamp)).unwrap();
            Part {
                name,
                source: stamp,
                sha256,
                target: None,
            }
        })
        .collect::<Vec<_>>();
    let first_identity = parts[0].source.clone();
    let journal = Journal {
        version: 1,
        id: Uuid::new_v4().to_string(),
        owner: journals.owner.clone(),
        book_id: book.metadata.id.clone(),
        title: book.metadata.title.clone(),
        source_directory: source.0.clone(),
        destination_directory: target.0.clone(),
        source_directory_identity: source.dir().identity,
        destination_directory_identity: target.dir().identity,
        primary: "download.zimaa".into(),
        same_filesystem: true,
        staging_identity: None,
        cleanup_staging_identity: None,
        parts,
    };
    journals.write(&journal).unwrap();
    commit_path(&store, &journal, &target.0.join("download.zimaa")).unwrap();
    move_exclusive(
        &source.dir(),
        "download.zimaa",
        &target.dir(),
        "download.zimaa",
    )
    .unwrap();
    source.dir().sync().unwrap();
    target.dir().sync().unwrap();
    drop(journals);
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap();
    let result = recover(&reopened).unwrap();
    assert_eq!(result["errors"].as_array().unwrap().len(), 0);
    assert_eq!(result["moved"].as_array().unwrap().len(), 1);
    assert_eq!(
        target.dir().stamp("download.zimaa").unwrap(),
        first_identity
    );
    assert!(Archive::open(&target.0.join("download.zimaa"))
        .unwrap()
        .check_integrity()
        .unwrap());
    assert!(!source.0.join("download.zimab").exists());
    assert_eq!(
        reopened.read().unwrap().books[0].metadata.id,
        book.metadata.id
    );
    assert_eq!(
        reopened.read().unwrap().books[0].path,
        target.0.join("download.zimaa").to_string_lossy()
    );
    assert!(recover(&reopened).unwrap()["moved"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn migration_skips_imported_archives_and_never_clobbers_existing_destinations() {
    let profile = Temp::new();
    let source = Temp::new();
    let target = Temp::new();
    let book = split_fixture(&source);
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    store
        .update(|state| {
            let mut imported = book.clone();
            imported.managed = false;
            state.books.push(imported);
            state.settings.download_directory = target.0.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    let result = move_archives(store.clone(), source.0.clone(), target.0.clone()).unwrap();
    assert_eq!(result["skipped"], 1);
    assert!(result["moved"].as_array().unwrap().is_empty());
    store
        .update(|state| {
            state.books[0].managed = true;
            Ok(())
        })
        .unwrap();
    std::fs::write(target.0.join("download.zimab"), b"existing user file").unwrap();
    let result = move_archives(store.clone(), source.0.clone(), target.0.clone()).unwrap();
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
    assert_eq!(
        std::fs::read(target.0.join("download.zimab")).unwrap(),
        b"existing user file"
    );
    assert!(Path::new(&book.path).exists());
    assert!(source.0.join("download.zimab").exists());
    assert_eq!(store.read().unwrap().books[0].path, book.path);
}
#[test]
fn copied_archive_cleanup_requires_verified_destination_and_durable_profile_first() {
    let profile = Temp::new();
    let source = Temp::new();
    let target = Temp::new();
    let book = split_fixture(&source);
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.books.push(book.clone());
            Ok(())
        })
        .unwrap();
    let mut parts = Vec::new();
    for (name, stamp) in verified(&source.dir(), "download.zimaa", &book.metadata.id).unwrap() {
        let (_, sha256) = digest(&source.dir(), &name, Some(&stamp)).unwrap();
        std::fs::copy(source.0.join(&name), target.0.join(&name)).unwrap();
        let copied = target.dir().stamp(&name).unwrap();
        parts.push(Part {
            name,
            source: stamp,
            sha256,
            target: Some(copied),
        });
    }
    let journals = Journals::open(&store, true).unwrap().unwrap();
    let mut journal = Journal {
        version: 1,
        id: Uuid::new_v4().to_string(),
        owner: journals.owner.clone(),
        book_id: book.metadata.id.clone(),
        title: book.metadata.title.clone(),
        source_directory: source.0.clone(),
        destination_directory: target.0.clone(),
        source_directory_identity: source.dir().identity,
        destination_directory_identity: target.dir().identity,
        primary: "download.zimaa".into(),
        same_filesystem: false,
        staging_identity: None,
        cleanup_staging_identity: None,
        parts,
    };
    store.set_runtime_mode(false, true).unwrap();
    assert!(cleanup_migration_sources(
        &store,
        &journals,
        &mut journal,
        &source.dir(),
        &target.dir(),
        &[]
    )
    .is_err());
    assert_eq!(store.read().unwrap().books[0].path, book.path);
    assert!(source.0.join("download.zimaa").exists());
    assert!(source.0.join("download.zimab").exists());
    store.set_runtime_mode(false, false).unwrap();
    let copied_bytes = std::fs::read(target.0.join("download.zimab")).unwrap();
    std::fs::write(target.0.join("download.zimab"), b"corruption").unwrap();
    assert!(cleanup_migration_sources(
        &store,
        &journals,
        &mut journal,
        &source.dir(),
        &target.dir(),
        &[]
    )
    .is_err());
    assert!(source.0.join("download.zimaa").exists());
    assert!(source.0.join("download.zimab").exists());
    std::fs::write(target.0.join("download.zimab"), copied_bytes).unwrap();
    let mut recovered = journal;
    recovered.parts[1].target = Some(target.dir().stamp("download.zimab").unwrap());
    cleanup_migration_sources(
        &store,
        &journals,
        &mut recovered,
        &source.dir(),
        &target.dir(),
        &[],
    )
    .unwrap();
    assert!(!source.0.join("download.zimaa").exists());
    assert!(!source.0.join("download.zimab").exists());
    assert_eq!(
        store.read().unwrap().books[0].path,
        target.0.join("download.zimaa").to_string_lossy()
    );
    assert!(Archive::open(&target.0.join("download.zimaa"))
        .unwrap()
        .check_integrity()
        .unwrap());
}
fn migration_fixture(
    profile: &Temp,
    source: &Temp,
    target: &Temp,
    copy: bool,
) -> (Store, Journals, Journal, Book) {
    let book = split_fixture(source);
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.books.push(book.clone());
            Ok(())
        })
        .unwrap();
    let journals = Journals::open(&store, true).unwrap().unwrap();
    let directory = source.dir();
    let parts = verified(&directory, "download.zimaa", &book.metadata.id)
        .unwrap()
        .into_iter()
        .map(|(name, stamp)| {
            let (_, sha256) = digest(&directory, &name, Some(&stamp)).unwrap();
            let target_stamp = if copy {
                std::fs::copy(source.0.join(&name), target.0.join(&name)).unwrap();
                Some(target.dir().stamp(&name).unwrap())
            } else {
                None
            };
            Part {
                name,
                source: stamp,
                sha256,
                target: target_stamp,
            }
        })
        .collect();
    let journal = Journal {
        version: 1,
        id: Uuid::new_v4().to_string(),
        owner: journals.owner.clone(),
        book_id: book.metadata.id.clone(),
        title: book.metadata.title.clone(),
        source_directory: source.0.clone(),
        destination_directory: target.0.clone(),
        source_directory_identity: directory.identity,
        destination_directory_identity: target.dir().identity,
        primary: "download.zimaa".into(),
        same_filesystem: !copy,
        staging_identity: None,
        cleanup_staging_identity: None,
        parts,
    };
    journals.write(&journal).unwrap();
    (store, journals, journal, book)
}
#[test]
fn failed_first_rename_restores_source_path_on_every_recovery_attempt() {
    let profile = Temp::new();
    let source = Temp::new();
    let target = Temp::new();
    let (store, journals, mut journal, book) =
        migration_fixture(&profile, &source, &target, false);
    for attempt in 0..2 {
        let mut attempts = 0;
        let result = finish_move_with_rename(&store, &journals, &mut journal, &mut Vec::new(), |_, _, _, _| {
            attempts += 1;
            // Model a native permission failure after the durable profile
            // commit, without depending on the test runner's privileges.
            Err("Exclusive archive rename: Permission denied".into())
        });
        let error = result.unwrap_err();
        assert!(error.contains("Permission denied"), "{error}");
        assert_eq!(attempts, 1);
        assert_eq!(store.read().unwrap().books[0].path, book.path);
        assert!(Archive::open(Path::new(&book.path))
            .unwrap()
            .check_integrity()
            .unwrap());
        assert!(target.dir().absent("download.zimaa").unwrap());
        // With the archive whole in its source folder, the library pointing
        // at it again and no destination name taken, nothing is left to
        // recover: the journal goes, so a rename that can never succeed (two
        // mounts of one device) is not re-hashed and re-tried at every launch.
        assert!(journal_files(&journals).is_empty());
        assert_eq!(error.contains("was not moved and is still in"), attempt == 0, "{error}");
    }
    // A real recovery collision keeps the journal and must also leave the
    // still-complete source usable rather than repointing the profile on
    // each startup.
    journals.write(&journal).unwrap();
    std::fs::write(target.0.join("download.zimaa"), b"unrelated collision").unwrap();
    drop(journals);
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap();
    assert_eq!(
        recover(&reopened).unwrap()["errors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(reopened.read().unwrap().books[0].path, book.path);
    assert_eq!(
        std::fs::read(target.0.join("download.zimaa")).unwrap(),
        b"unrelated collision"
    );
    std::fs::remove_file(target.0.join("download.zimaa")).unwrap();
    assert_eq!(
        recover(&reopened).unwrap()["moved"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reopened.read().unwrap().books[0].path,
        target.0.join("download.zimaa").to_string_lossy()
    );
    assert!(recover(&reopened).unwrap()["moved"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn a_fresh_same_volume_move_proves_parts_by_identity_instead_of_rehashing() {
    let profile = Temp::new();
    let source = Temp::new();
    let target = Temp::new();
    let (store, journals, mut journal, _book) = migration_fixture(&profile, &source, &target, false);
    // Exactly what `move_archives` records while planning: every part was
    // hashed in this process at this stamp, so no later stage re-reads it.
    let mut proven = journal
        .parts
        .iter()
        .map(|part| VerifiedPart {
            name: part.name.clone(),
            stamp: part.source.clone(),
        })
        .collect();
    finish_move(&store, &journals, &mut journal, &mut proven).unwrap();
    for part in &journal.parts {
        // Same device, inode, size and mtime after the rename, so the planning
        // digest still covers the published bytes.
        assert_eq!(target.dir().stamp(&part.name).unwrap(), part.source);
        assert!(!source.0.join(&part.name).exists());
    }
    assert!(Archive::open(&target.0.join("download.zimaa"))
        .unwrap()
        .check_integrity()
        .unwrap());
    assert!(!journals
        .directory
        .path
        .join(format!("{}.json", journal.id))
        .exists());
    assert_eq!(
        store.read().unwrap().books[0].path,
        target.0.join("download.zimaa").to_string_lossy()
    );
}
#[test]
fn cleanup_captures_and_preserves_a_source_replaced_at_the_rename_boundary() {
    for collision in [false, true] {
        let profile = Temp::new();
        let source = Temp::new();
        let target = Temp::new();
        let (store, journals, mut journal, _) =
            migration_fixture(&profile, &source, &target, true);
        let original = std::fs::read(source.0.join("download.zimaa")).unwrap();
        let mut replaced = false;
        let result = cleanup_migration_sources_with_rename(
            &store,
            &journals,
            &mut journal,
            &source.dir(),
            &target.dir(),
            &[],
            |from, name, to, target_name| {
                if !replaced {
                    replaced = true;
                    std::fs::rename(from.path.join(name), from.path.join("original-part"))
                        .unwrap();
                    std::fs::write(from.path.join(name), b"replacement must survive").unwrap();
                }
                move_exclusive(from, name, to, target_name)?;
                if collision {
                    std::fs::write(from.path.join(name), b"new source collision").unwrap();
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(replaced);
        assert_eq!(
            std::fs::read(source.0.join("original-part")).unwrap(),
            original
        );
        assert!(source.0.join("download.zimab").exists());
        let staging_path = source.0.join(format!(".kiwix-cleanup-{}", journal.id));
        if collision {
            assert_eq!(
                std::fs::read(source.0.join("download.zimaa")).unwrap(),
                b"new source collision"
            );
            assert_eq!(
                std::fs::read(staging_path.join("download.zimaa")).unwrap(),
                b"replacement must survive"
            );
        } else {
            assert_eq!(
                std::fs::read(source.0.join("download.zimaa")).unwrap(),
                b"replacement must survive"
            );
            assert!(!staging_path.join("download.zimaa").exists());
        }
        assert!(cleanup_migration_sources(
            &store,
            &journals,
            &mut journal,
            &source.dir(),
            &target.dir(),
            &[]
        )
        .is_err());
        assert_eq!(
            store.read().unwrap().books[0].path,
            target.0.join("download.zimaa").to_string_lossy()
        );
        assert!(Archive::open(&target.0.join("download.zimaa"))
            .unwrap()
            .check_integrity()
            .unwrap());
    }
}
#[test]
fn cleanup_recovers_staged_parts_and_an_already_removed_staging_directory() {
    let profile = Temp::new();
    let source = Temp::new();
    let target = Temp::new();
    let (store, journals, mut journal, _) = migration_fixture(&profile, &source, &target, true);
    let staging_name = format!(".kiwix-cleanup-{}", journal.id);
    let staging = private_child(&source.dir(), &staging_name, true).unwrap();
    journal.cleanup_staging_identity = Some(staging.identity.clone());
    journals.write(&journal).unwrap();
    move_exclusive(&source.dir(), "download.zimaa", &staging, "download.zimaa").unwrap();
    source.dir().sync().unwrap();
    staging.sync().unwrap();
    // The interrupted process no longer holds its capability (Windows would
    // otherwise refuse to remove the directory, as for any open handle).
    drop(staging);
    // Read the actual durable manifest, as startup does after interruption.
    let bytes =
        std::fs::read(journals.directory.path.join(format!("{}.json", journal.id))).unwrap();
    let mut recovered: Journal = serde_json::from_slice(&bytes).unwrap();
    cleanup_migration_sources(
        &store,
        &journals,
        &mut recovered,
        &source.dir(),
        &target.dir(),
        &[],
    )
    .unwrap();
    assert!(source.dir().absent("download.zimaa").unwrap());
    assert!(source.dir().absent("download.zimab").unwrap());
    assert!(source.dir().absent(&staging_name).unwrap());
    cleanup_migration_sources(
        &store,
        &journals,
        &mut recovered,
        &source.dir(),
        &target.dir(),
        &[],
    )
    .unwrap();
    assert!(Archive::open(&target.0.join("download.zimaa"))
        .unwrap()
        .check_integrity()
        .unwrap());
}
/// Windows denies removing a directory while its own cap-std capability is
/// open (no FILE_SHARE_DELETE). Staging removal therefore releases the
/// capability first, and only acts on, or re-adopts, the verified identity.
#[cfg(windows)]
#[test]
fn windows_staging_release_is_identity_bound_and_permits_removal() {
    let root = Temp::new();
    let parent = root.dir();
    let staging = private_child(&parent, ".kiwix-delete-staging", true).unwrap();
    let identity = staging.identity.clone();
    let error = parent.dir.remove_dir(".kiwix-delete-staging").unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32), "{error}");
    // A different directory under the released name is never adopted.
    std::fs::create_dir(root.0.join(".kiwix-delete-other")).unwrap();
    let mut slot = Some(staging);
    let error = release_directory(&parent, &mut slot, ".kiwix-delete-other").unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    assert!(slot.is_none(), "the capability is released even when the name is foreign");
    let error = reopen_child(&parent, ".kiwix-delete-other", &identity).err().unwrap();
    assert!(error.contains("replaced"), "{error}");
    assert!(root.0.join(".kiwix-delete-other").is_dir());
    // The verified directory is re-adopted, then removable once released.
    let mut slot = Some(reopen_child(&parent, ".kiwix-delete-staging", &identity).unwrap());
    release_directory(&parent, &mut slot, ".kiwix-delete-staging").unwrap();
    parent.dir.remove_dir(".kiwix-delete-staging").unwrap();
    assert!(parent.absent(".kiwix-delete-staging").unwrap());
}
#[test]
fn permanent_split_delete_leaves_no_staging_directory() {
    let root = Temp::new();
    let book = split_fixture(&root);
    delete_book(&book, false).unwrap();
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0, "parts and staging are gone");
}

#[cfg(any(all(unix, desktop), target_os = "linux"))]
fn single_fixture(root: &Temp, name: &str) -> Book {
    fixture_book(root, "small.zim", name)
}

fn fixture_book(root: &Temp, fixture: &str, name: &str) -> Book {
    let path = root.0.join(name);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture),
        &path,
    )
    .unwrap();
    let mut book = imported_book(&path);
    book.managed = true;
    book
}

fn directory_names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn journal_files(journals: &Journals) -> Vec<String> {
    directory_names(&journals.directory.path)
        .into_iter()
        .filter(|name| name.ends_with(".json"))
        .collect()
}

#[cfg(all(unix, desktop))]
#[test]
fn a_platform_valid_name_can_be_revealed_while_mutation_explains_its_refusal() {
    let root = Temp::new();
    // A slash typed in Finder is stored as a colon; reading accepts both.
    let colon = single_fixture(&root, "a:b.zim");
    assert_eq!(reveal_target(&colon).unwrap(), root.0);
    let long = single_fixture(&root, &format!("{}.zim", "x".repeat(245)));
    assert_eq!(reveal_target(&long).unwrap(), root.0);
    for (book, cause) in [(&colon, "contains a colon"), (&long, "is longer than 240 bytes")] {
        for error in [
            archive_files(book).unwrap_err(),
            delete_book(book, false).unwrap_err(),
            retain_archive_files(book).err().unwrap(),
        ] {
            assert!(error.contains(cause), "{error}");
            assert!(error.contains("Rename the file") && error.contains("Locate"), "{error}");
        }
        // The stricter rule still protects every operation that changes files.
        assert!(Path::new(&book.path).exists());
    }
    let mut trailing = colon.clone();
    trailing.path = root.0.join("book.zim ").to_string_lossy().into_owned();
    assert!(archive_files(&trailing).unwrap_err().contains("ends with a dot or a space"));
    // Reveal still proves that the file is the library's archive.
    let mut other = colon.clone();
    other.metadata.id = Uuid::new_v4().to_string();
    assert!(reveal_target(&other).unwrap_err().contains("different archive"));
    assert_eq!(directory_names(&root.0).len(), 2);
}

#[test]
fn the_trashed_folder_is_visible_and_a_permanent_delete_stays_hidden() {
    assert_eq!(deletion_staging_prefix(true), "kiwix-deleted");
    assert!(!deletion_staging_prefix(true).starts_with('.'));
    assert_eq!(deletion_staging_prefix(false), ".kiwix-delete");
}

#[cfg(desktop)]
#[test]
fn a_failed_trash_names_a_recovery_folder_only_while_one_exists() {
    let root = Temp::new();
    let book = split_fixture(&root);
    // The Trash is refused under test after both parts were staged, so the
    // delete restores them. Nothing is retained anywhere else.
    let error = delete_book(&book, true).unwrap_err();
    assert!(error.contains("Moving archive to Trash"), "{error}");
    assert!(error.contains("The archive was left where it is"), "{error}");
    assert!(!error.contains("recoverable at"), "{error}");
    assert_eq!(directory_names(&root.0), ["download.zimaa", "download.zimab"]);
    assert!(Archive::open(Path::new(&book.path)).unwrap().check_integrity().unwrap());
    // When a part cannot be put back, the folder that still holds it is named.
    MOVE_FAILURE_AT.set(Some(3));
    let error = delete_book(&book, true).unwrap_err();
    assert!(error.contains("recoverable at"), "{error}");
    let names = directory_names(&root.0);
    let staging = names.iter().find(|name| name.starts_with("kiwix-deleted-")).expect("visible staging folder");
    assert!(error.contains(staging.as_str()), "{error}");
    assert_eq!(directory_names(&root.0.join(staging)), ["download.zimaa"]);
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn a_copy_that_cannot_fit_is_refused_and_unknown_capacity_is_not_zero() {
    let folder = Path::new("/library");
    assert!(ensure_copy_space(Some(100), 100, folder).is_ok());
    let error = ensure_copy_space(Some(99), 100, folder).unwrap_err();
    assert!(error.contains("Not enough free space") && error.contains("100 bytes are needed but 99 bytes are free"), "{error}");
    assert!(ensure_copy_space(None, u64::MAX, folder).is_ok());
}

/// A journaled cross-volume move whose first copy was interrupted: a private
/// staging folder with a partial first part and nothing published.
fn interrupted_copy(profile: &Temp, source: &Temp, target: &Temp) -> (Store, Journals, Journal, Book) {
    let (store, journals, mut journal, book) = migration_fixture(profile, source, target, false);
    journal.same_filesystem = false;
    let staging = private_child(&target.dir(), &format!(".kiwix-move-{}", journal.id), true).unwrap();
    journal.staging_identity = Some(staging.identity.clone());
    std::fs::write(staging.path.join("download.zimaa"), b"partial copy").unwrap();
    journals.write(&journal).unwrap();
    (store, journals, journal, book)
}

#[test]
fn an_unpublished_copy_is_abandoned_with_its_owned_staging_and_journal() {
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, book) = interrupted_copy(&profile, &source, &target);
    let error = failed_move(&store, &journals, &journal, "No space left on device (os error 28)".into());
    assert!(error.contains("No space left on device"), "{error}");
    assert!(error.contains("was not moved and is still in"), "{error}");
    assert!(journal_files(&journals).is_empty(), "the journal must not be replayed at every launch");
    assert!(directory_names(&target.0).is_empty(), "no staging folder or partial copy may remain");
    assert_eq!(directory_names(&source.0), ["download.zimaa", "download.zimab"]);
    assert_eq!(store.read().unwrap().books[0].path, book.path);
    assert!(Archive::open(Path::new(&book.path)).unwrap().check_integrity().unwrap());
}

#[test]
fn a_journal_is_never_abandoned_once_anything_of_the_move_exists() {
    // A published destination part: its copy identity was journaled.
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, mut journal, _) = interrupted_copy(&profile, &source, &target);
    std::fs::copy(source.0.join("download.zimab"), target.0.join("download.zimab")).unwrap();
    journal.parts[1].target = Some(target.dir().stamp("download.zimab").unwrap());
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    assert_eq!(journal_files(&journals).len(), 1);
    let staging = target.0.join(format!(".kiwix-move-{}", journal.id));
    assert_eq!(std::fs::read(staging.join("download.zimaa")).unwrap(), b"partial copy");
    assert_eq!(failed_move(&store, &journals, &journal, "copy failed".into()), "copy failed");

    // The library already points at the destination.
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, _) = interrupted_copy(&profile, &source, &target);
    commit_path(&store, &journal, &target.0.join("download.zimaa")).unwrap();
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    assert_eq!(journal_files(&journals).len(), 1);
    assert!(target.0.join(format!(".kiwix-move-{}/download.zimaa", journal.id)).exists());

    // A same-volume move keeps its journal for recovery, as before.
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, _) = migration_fixture(&profile, &source, &target, false);
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    assert_eq!(journal_files(&journals).len(), 1);
}

#[test]
fn abandoning_removes_only_staging_entries_that_are_still_the_recorded_ones() {
    // A finished staged copy that is no longer the recorded file is kept.
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, mut journal, _) = interrupted_copy(&profile, &source, &target);
    let staging = target.0.join(format!(".kiwix-move-{}", journal.id));
    journal.parts[0].target = Some(source.dir().stamp("download.zimaa").unwrap());
    let error = abandon_unpublished_copy(&store, &journals, &journal).unwrap_err();
    assert!(error.contains("preserved"), "{error}");
    assert_eq!(std::fs::read(staging.join("download.zimaa")).unwrap(), b"partial copy");
    assert_eq!(journal_files(&journals).len(), 1);
    let message = failed_move(&store, &journals, &journal, "copy failed".into());
    assert!(message.contains("kept and will be tried again"), "{message}");

    // The recorded copy itself is removed; a file the move never wrote keeps
    // the folder, and with it the journal.
    journal.parts[0].target = Some(Directory::open(&staging).unwrap().stamp("download.zimaa").unwrap());
    std::fs::write(staging.join("notes.txt"), b"not written by the move").unwrap();
    let error = abandon_unpublished_copy(&store, &journals, &journal).unwrap_err();
    assert!(error.contains("could not be removed and was preserved"), "{error}");
    assert_eq!(directory_names(&staging), ["notes.txt"]);
    assert_eq!(journal_files(&journals).len(), 1);

    // A staging folder that is not the journaled one is not entered at all.
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, mut journal, _) = interrupted_copy(&profile, &source, &target);
    journal.staging_identity = Some(source.dir().identity);
    let error = abandon_unpublished_copy(&store, &journals, &journal).unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    assert!(target.0.join(format!(".kiwix-move-{}/download.zimaa", journal.id)).exists());
    assert_eq!(journal_files(&journals).len(), 1);
    assert_eq!(directory_names(&source.0), ["download.zimaa", "download.zimab"]);
}

#[test]
fn recovery_drops_the_journal_of_a_book_that_left_the_library_and_names_it_once() {
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, book) = interrupted_copy(&profile, &source, &target);
    // The person removed this book; the rest of the library is still there.
    let other = Temp::new();
    let kept = fixture_book(&other, "delimiters.zim", "kept.zim");
    store.update(|state| { state.books.clear(); state.books.push(kept.clone()); Ok(()) }).unwrap();
    drop(journals);
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap();
    let result = recover(&reopened).unwrap();
    let errors = result["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{result}");
    assert_eq!(errors[0]["title"], book.metadata.title);
    assert_eq!(errors[0]["from"], crate::storage::display_path(&source.0));
    assert_eq!(errors[0]["to"], crate::storage::display_path(&target.0));
    assert!(errors[0]["error"].as_str().unwrap().contains("no longer in the library"), "{result}");
    // Archive files are never removed for a book that is gone; only the
    // move's own unfinished copy is.
    assert_eq!(directory_names(&source.0), ["download.zimaa", "download.zimab"]);
    assert!(directory_names(&target.0).is_empty());
    let journals = Journals::open(&reopened, false).unwrap().unwrap();
    assert!(journal_files(&journals).is_empty());
    assert!(!journals.directory.path.join(format!("{}.json", journal.id)).exists());
    let again = recover(&reopened).unwrap();
    assert!(again["errors"].as_array().unwrap().is_empty(), "the warning must not repeat: {again}");
}

#[test]
fn a_failed_recovery_names_the_archive_and_both_folders() {
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, book) = migration_fixture(&profile, &source, &target, false);
    std::fs::write(target.0.join("download.zimaa"), b"unrelated collision").unwrap();
    drop(journals);
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap();
    let result = recover(&reopened).unwrap();
    let errors = result["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{result}");
    assert_eq!(errors[0]["id"], format!("{}.json", journal.id));
    assert_eq!(errors[0]["title"], book.metadata.title);
    assert_eq!(errors[0]["from"], crate::storage::display_path(&source.0));
    assert_eq!(errors[0]["to"], crate::storage::display_path(&target.0));
    assert_eq!(std::fs::read(target.0.join("download.zimaa")).unwrap(), b"unrelated collision");
}

/// A second volume for real cross-volume moves. Only Linux offers one to an
/// unprivileged test (`/dev/shm`); `None` when it is the temporary volume too.
#[cfg(target_os = "linux")]
fn second_volume(first: &Temp) -> Option<Temp> {
    let shared = Path::new("/dev/shm").canonicalize().ok()?;
    if Directory::open(&shared).ok()?.identity.device == first.dir().identity.device {
        return None;
    }
    let root = shared.join(format!("kiwix-library-files-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).ok()?;
    Some(Temp(root))
}

#[cfg(target_os = "linux")]
#[test]
fn a_cross_volume_move_that_fails_before_publishing_leaves_nothing_to_replay() {
    let (profile, source) = (Temp::new(), Temp::new());
    let Some(target) = second_volume(&source) else {
        eprintln!("skipped: no second volume is available");
        return;
    };
    let book = single_fixture(&source, "download.zim");
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    store
        .update(|state| {
            state.books.push(book.clone());
            state.settings.download_directory = target.0.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    // The copy is written, hashed and staged; publishing it then fails.
    MOVE_FAILURE_AT.set(Some(1));
    let result = move_archives(store.clone(), source.0.clone(), target.0.clone()).unwrap();
    let errors = result["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{result}");
    let error = errors[0]["error"].as_str().unwrap();
    assert!(error.contains("simulated cross-device move"), "{error}");
    assert!(error.contains("was not moved and is still in"), "{error}");
    assert!(directory_names(&target.0).is_empty(), "{:?}", directory_names(&target.0));
    assert_eq!(directory_names(&source.0), ["download.zim"]);
    assert_eq!(store.read().unwrap().books[0].path, book.path);
    let journals = Journals::open(&store, false).unwrap().unwrap();
    assert!(journal_files(&journals).is_empty());
    drop(journals);
    // Startup has nothing to re-hash, re-copy or warn about.
    let recovered = recover(&store).unwrap();
    assert!(recovered["errors"].as_array().unwrap().is_empty(), "{recovered}");
    assert!(recovered["moved"].as_array().unwrap().is_empty(), "{recovered}");
    // The same move can simply be asked for again.
    let result = move_archives(store.clone(), source.0.clone(), target.0.clone()).unwrap();
    assert_eq!(result["moved"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(directory_names(&target.0), ["download.zim"]);
    assert!(directory_names(&source.0).is_empty());
    assert!(Archive::open(&target.0.join("download.zim")).unwrap().check_integrity().unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn a_cross_volume_move_is_refused_up_front_when_the_archives_cannot_fit() {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            FREE_SPACE_OVERRIDE.set(None);
        }
    }
    let (profile, source) = (Temp::new(), Temp::new());
    let Some(target) = second_volume(&source) else {
        eprintln!("skipped: no second volume is available");
        return;
    };
    let book = single_fixture(&source, "download.zim");
    let size = std::fs::metadata(&book.path).unwrap().len();
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    store
        .update(|state| {
            state.books.push(book.clone());
            state.settings.download_directory = target.0.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    let _restore = Restore;
    FREE_SPACE_OVERRIDE.set(Some(size - 1));
    let error = move_archives(store.clone(), source.0.clone(), target.0.clone()).unwrap_err();
    assert!(error.contains("Not enough free space"), "{error}");
    assert!(error.contains("Nothing was moved"), "{error}");
    assert!(directory_names(&target.0).is_empty());
    assert!(Journals::open(&store, false).unwrap().is_none_or(|journals| journal_files(&journals).is_empty()));
    assert_eq!(store.read().unwrap().books[0].path, book.path);
    // A move within one volume is a rename and needs no room for a copy.
    let sibling = Temp::new();
    store
        .update(|state| {
            state.settings.download_directory = sibling.0.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    let result = move_archives(store.clone(), source.0.clone(), sibling.0.clone()).unwrap();
    assert_eq!(result["moved"].as_array().unwrap().len(), 1, "{result}");
    // Exactly enough room is enough.
    FREE_SPACE_OVERRIDE.set(Some(size));
    store
        .update(|state| {
            state.settings.download_directory = target.0.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    let result = move_archives(store.clone(), sibling.0.clone(), target.0.clone()).unwrap();
    assert_eq!(result["moved"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(directory_names(&target.0), ["download.zim"]);
}

/// Review regression (FILES-R1). The staged copy may be the only verified
/// copy left when the source volume is unplugged, failed or was moved away:
/// nothing is cleared and the journal stays, and no message claims the
/// archive is still in a folder that is not there.
#[test]
fn a_staged_copy_and_its_journal_survive_while_the_source_is_not_provably_present() {
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, mut journal, _book) = migration_fixture(&profile, &source, &target, false);
    journal.same_filesystem = false;
    let staging = private_child(&target.dir(), &format!(".kiwix-move-{}", journal.id), true).unwrap();
    journal.staging_identity = Some(staging.identity.clone());
    std::fs::copy(source.0.join("download.zimaa"), staging.path.join("download.zimaa")).unwrap();
    journal.parts[0].target = Some(staging.stamp("download.zimaa").unwrap());
    journals.write(&journal).unwrap();
    let staged = staging.path.join("download.zimaa");
    let saved = journals.directory.path.join(format!("{}.json", journal.id));
    drop(staging);

    // The whole source folder is gone.
    let away = source.0.with_extension("away");
    std::fs::rename(&source.0, &away).unwrap();
    let result = recover(&store).unwrap();
    let error = result["errors"][0]["error"].as_str().unwrap().to_owned();
    assert!(staged.exists(), "verified staged copy deleted while the source was not present: {result}");
    assert!(saved.exists(), "journal dropped while the source was not present: {result}");
    assert!(!error.contains("still in"), "{error}");
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    std::fs::rename(&away, &source.0).unwrap();

    // The folder is there but one part is missing, or is another file.
    let part = source.0.join("download.zimab");
    let bytes = std::fs::read(&part).unwrap();
    std::fs::rename(&part, source.0.join("aside")).unwrap();
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    std::fs::write(&part, &bytes).unwrap();
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    assert!(staged.exists() && saved.exists());
    std::fs::remove_file(&part).unwrap();
    std::fs::rename(source.0.join("aside"), &part).unwrap();

    // The same folder path, recreated: not the journaled directory.
    std::fs::rename(&source.0, &away).unwrap();
    std::fs::create_dir(&source.0).unwrap();
    for name in ["download.zimaa", "download.zimab"] {
        std::fs::copy(away.join(name), source.0.join(name)).unwrap();
    }
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(false));
    assert!(staged.exists() && saved.exists());
    std::fs::remove_dir_all(&source.0).unwrap();
    std::fs::rename(&away, &source.0).unwrap();

    // With every source part back as recorded, the move can be given up.
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(true));
    assert!(!staged.exists() && !saved.exists());
    assert_eq!(directory_names(&source.0), ["download.zimaa", "download.zimab"]);
}

/// Review regression (FILES-R2). A profile whose state was lost (a fresh
/// state has no books at all) or partly unreadable proves nothing about one
/// book, and a same-volume move split across its two folders needs its
/// journal: neither is ever dropped as an orphan.
#[test]
fn a_lost_profile_or_a_split_move_never_makes_a_journal_an_orphan() {
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, _book) = migration_fixture(&profile, &source, &target, false);
    assert!(journal.same_filesystem);
    commit_path(&store, &journal, &target.0.join("download.zimaa")).unwrap();
    std::fs::rename(source.0.join("download.zimaa"), target.0.join("download.zimaa")).unwrap();
    drop(journals);
    drop(store);
    std::fs::remove_file(profile.0.join("state.json")).unwrap();
    let reopened = Store::new(profile.0.clone()).unwrap();
    let result = recover(&reopened).unwrap();
    assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
    let journals = Journals::open(&reopened, false).unwrap().unwrap();
    assert_eq!(journal_files(&journals), [format!("{}.json", journal.id)], "journal of a split same-volume move was dropped");
    assert_eq!(directory_names(&source.0), ["download.zimab"]);
    assert_eq!(directory_names(&target.0), ["download.zimaa"]);

    // The library lists other books and this one is gone, but the archive
    // is split across the two folders: still not an orphan.
    let other = Temp::new();
    let kept = fixture_book(&other, "delimiters.zim", "kept.zim");
    reopened.update(|state| { state.books.push(kept.clone()); Ok(()) }).unwrap();
    assert_eq!(journal_orphaned(&journal, &journals, &reopened), Ok(false));
    recover(&reopened).unwrap();
    assert_eq!(journal_files(&journals).len(), 1);

    // Whole in the source folder again and the profile intact: an orphan.
    std::fs::rename(target.0.join("download.zimaa"), source.0.join("download.zimaa")).unwrap();
    assert_eq!(journal_orphaned(&journal, &journals, &reopened), Ok(true));
    // An empty library is what a fresh profile looks like.
    reopened.update(|state| { state.books.clear(); Ok(()) }).unwrap();
    assert_eq!(journal_orphaned(&journal, &journals, &reopened), Ok(false));
}

/// Review regression (FILES-R7). A file at the name of a part whose copy was
/// never journaled cannot be this move's publication; it is somebody else's,
/// stays untouched, and does not keep the move alive.
#[test]
fn a_foreign_file_at_an_unpublished_part_name_does_not_block_abandoning() {
    let (profile, source, target) = (Temp::new(), Temp::new(), Temp::new());
    let (store, journals, journal, _) = interrupted_copy(&profile, &source, &target);
    std::fs::write(target.0.join("download.zimab"), b"somebody else's file").unwrap();
    assert_eq!(abandon_unpublished_copy(&store, &journals, &journal), Ok(true));
    assert!(journal_files(&journals).is_empty());
    assert_eq!(directory_names(&target.0), ["download.zimab"]);
    assert_eq!(std::fs::read(target.0.join("download.zimab")).unwrap(), b"somebody else's file");
    assert_eq!(directory_names(&source.0), ["download.zimaa", "download.zimab"]);
}

/// Review regression (FILES-R4). An interrupted exclusive move on a
/// filesystem without RENAME_NOREPLACE leaves the archive a hidden second
/// name; the refusal to act on a hard-linked file must point at it.
#[cfg(target_os = "linux")]
#[test]
fn a_hard_linked_archive_refusal_points_at_a_leftover_hidden_name() {
    let root = Temp::new();
    let book = single_fixture(&root, "book.zim");
    std::fs::hard_link(&book.path, root.0.join(".kiwix-renamed-1-2")).unwrap();
    for error in [delete_book(&book, false).unwrap_err(), archive_files(&book).unwrap_err()] {
        assert!(error.contains("without hard links"), "{error}");
        assert!(error.contains(".kiwix-renamed-"), "{error}");
    }
    assert_eq!(directory_names(&root.0), [".kiwix-renamed-1-2", "book.zim"]);
    // Removing the hidden entry is all it takes.
    std::fs::remove_file(root.0.join(".kiwix-renamed-1-2")).unwrap();
    delete_book(&book, false).unwrap();
    assert!(directory_names(&root.0).is_empty());
}

/// Review regression (FILES-R7). A delete that stops after some parts are
/// gone says how many, and that the rest came back; it never says "deleted"
/// for parts that are back, nor "left where it is" unless all of them are.
#[test]
fn a_settled_delete_failure_says_what_is_gone_and_what_came_back() {
    assert_eq!(settled_delete_failure("Disk error", 3, 3, false), "Disk error. The archive was left where it is");
    assert_eq!(settled_delete_failure("Disk error", 0, 3, false), "Disk error. The archive files had already been deleted");
    assert_eq!(settled_delete_failure("Disk error", 0, 3, true), "Disk error. The archive files had already been moved to the Trash");
    assert_eq!(
        settled_delete_failure("Disk error", 2, 3, false),
        "Disk error. 1 of the archive's 3 files had already been deleted; the others were put back"
    );
}
