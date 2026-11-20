// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
fn source() -> (std::path::PathBuf, Arc<Store>, Book) {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-quic-library-source-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
    let path = root.join("library.zim"); std::fs::copy(&fixture, &path).unwrap();
    let book = Book { metadata: crate::archive::Archive::open(&path).unwrap().metadata().unwrap(),
        installation_id: crate::store::new_installation_id(), path: path.to_string_lossy().into_owned(), source_reference: None,
        added_at: "2026-01-01T00:00:00Z".into(), available: true, managed: false, integrity: None };
    let store = Arc::new(Store::new(root.join("profile")).unwrap());
    store.update(|s| { s.books.push(book.clone()); Ok(()) }).unwrap();
    (root, store, book)
}
#[test]
fn explicit_snapshot_never_adds_received_installations_and_revokes_reinstalled_members() {
    let (root, store, book) = source();
    let sources = Sources::new(store.clone(), BTreeMap::from([(book.metadata.id.clone(), book.clone())]),
        "a".repeat(32), "sender-peer".into(), "Sender".into(),
        Arc::new(RateBudget::new(0)), CancellationToken::new());
    let mut received = book.clone(); received.metadata.id = uuid::Uuid::new_v4().to_string(); received.installation_id = crate::store::new_installation_id();
    store.update(|s| { s.books.push(received.clone()); Ok(()) }).unwrap();
    let inventory = sources.inventory().unwrap();
    assert_eq!(inventory.books.len(), 1); assert_eq!(inventory.books[0].id, book.metadata.id);
    store.update(|s| { s.books[0].installation_id = crate::store::new_installation_id(); Ok(()) }).unwrap();
    assert!(sources.inventory().is_err()); assert!(sources.principal(&book).is_err());
    drop(sources); drop(store); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn identical_path_replacement_changes_validator_and_old_stream_refuses_bytes() {
    let (root, store, book) = source();
    let (mut original, _, _) = crate::sharing::open_library_download(book.clone()).unwrap();
    let first = source_etag(&book, &original).unwrap();
    let replacement = root.join("replacement.zim"); std::fs::copy(&book.path, &replacement).unwrap();
    std::fs::rename(&book.path, root.join("old.zim")).unwrap(); std::fs::rename(&replacement, &book.path).unwrap();
    assert!(original.read(0, 80).is_err());
    let (current, _, _) = crate::sharing::open_library_download(book.clone()).unwrap();
    assert_ne!(first, source_etag(&book, &current).unwrap());
    let mut reinstalled = book.clone(); reinstalled.installation_id = crate::store::new_installation_id();
    assert_ne!(source_etag(&book, &current).unwrap(), source_etag(&reinstalled, &current).unwrap());
    let mut provider = book; provider.source_reference = Some(crate::provider_reference::ProviderReference { reference: "native-owned-other".into(), display_name: "library.zim".into(), persistent: true });
    assert_ne!(source_etag(&reinstalled, &current).unwrap(), source_etag(&provider, &current).unwrap());
    drop(current); drop(original); drop(store); std::fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn same_length_write_with_restored_mtime_cannot_reuse_retained_identity() {
    use std::io::{Seek, SeekFrom, Write};
    let (root, store, book) = source();
    let (mut download, _, _) = crate::sharing::open_library_download(book.clone()).unwrap();
    let modified = std::fs::metadata(&book.path).unwrap().modified().unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&book.path).unwrap();
    file.seek(SeekFrom::Start(80)).unwrap(); file.write_all(&[0xff]).unwrap(); file.sync_all().unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
    assert!(download.read(0, 80).is_err(), "ctime changes must reject even reads outside the changed range");
    drop(file); drop(download); drop(store); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn response_identity_and_range_metadata_reject_cross_session_and_overflow() {
    let peer = LibraryPeerDescriptor { session_id: "a".repeat(32), peer_id: "sender-peer".into(), name: "Sender".into(),
        port: 1234, addresses: vec!["192.168.1.2".into()], native_services: vec![], direct: false,
        route: None, cancel: CancellationToken::new(), authority: Default::default() };
    let mut reply = wire::Reply { version: 1, session_id: peer.session_id.clone(), peer_id: peer.peer_id.clone(), status: 206,
        length: 8, total: 10, start: 2, etag: Some("\"source\"".into()), inventory: None, error: None };
    validate_reply(&peer, &reply).unwrap();
    reply.session_id = "b".repeat(32); assert!(validate_reply(&peer, &reply).is_err()); reply.session_id = peer.session_id.clone();
    reply.start = u64::MAX; assert!(validate_reply(&peer, &reply).is_err()); reply.start = 2;
    reply.length = 0; assert!(validate_reply(&peer, &reply).is_err()); reply.length = 8;
    reply.status = 416; assert!(validate_reply(&peer, &reply).is_err());
}

#[tokio::test]
#[ignore = "Uses the actual physical LAN route and encrypted QUIC listener"]
async fn exact_socket_rebind_keeps_session_snapshot_and_retires_old_streams() {
    let (root, store, book) = source();
    let mut server = LibraryServer::start_default(store.clone(), "sender-peer".into(), "Sender".into(), CancellationToken::new()).await.unwrap();
    let old = server.descriptor.clone();
    let mut response = open(&old, &book.metadata.id, HeaderMap::new(), CancellationToken::new()).await.unwrap();
    let original = browse(&old, CancellationToken::new()).await.unwrap();
    let mut received = book.clone(); received.metadata.id = uuid::Uuid::new_v4().to_string(); received.installation_id = crate::store::new_installation_id();
    store.update(|s| { s.books.push(received); Ok(()) }).unwrap();
    let address = old.addresses.iter().find_map(|s| s.parse::<std::net::Ipv4Addr>().ok()).expect("physical IPv4 interface");
    let socket = std::net::UdpSocket::bind((address, 0)).unwrap();
    server.rebind(socket, vec![address.to_string()]).await.unwrap();
    assert!(old.cancel.is_cancelled());
    assert!(response.chunk().await.is_err());
    assert_eq!(server.descriptor.session_id, old.session_id);
    assert_ne!(server.descriptor.port, old.port);
    let current = browse(&server.descriptor, CancellationToken::new()).await.unwrap();
    assert_eq!(current.books.len(), 1);
    assert_eq!(current.books[0].id, original.books[0].id);
    drop(response); server.shutdown().await; drop(store); std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Uses the actual physical LAN route and encrypted QUIC listener"]
async fn four_exact_length_ranges_release_admission_without_an_eof_poll() {
    let (root, store, book) = source();
    let expected = std::fs::read(&book.path).unwrap();
    let server = LibraryServer::start_default(store.clone(), "sender-peer".into(), "Sender".into(), CancellationToken::new()).await.unwrap();
    let peer = server.descriptor.clone();
    let mut other_receipt = peer.clone();
    other_receipt.authority = Default::default();
    let probe = open(&peer, &book.metadata.id, HeaderMap::new(), CancellationToken::new()).await.unwrap();
    let validator = probe.headers().get("etag").unwrap().clone();
    // Acquisition discards this probe before starting its parallel workers.
    drop(probe);
    let mut workers = tokio::task::JoinSet::new();
    for index in 0..4usize {
        let descriptor = if index % 2 == 0 { peer.clone() } else { other_receipt.clone() };
        let id = book.metadata.id.clone();
        let validator = validator.clone();
        workers.spawn(async move {
            let start = index * 1024;
            let mut headers = HeaderMap::new();
            headers.insert("range", HeaderValue::from_str(&format!("bytes={start}-{}", start + 1023)).unwrap());
            headers.insert("if-range", validator);
            let mut response = open(&descriptor, &id, headers, CancellationToken::new()).await.unwrap();
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            let length: usize = response.headers().get("content-length").unwrap().to_str().unwrap().parse().unwrap();
            assert_eq!(length, 1024);
            let mut bytes = Vec::with_capacity(length);
            while bytes.len() < length {
                let chunk = response.chunk().await.unwrap().expect("range ended before its declared length");
                assert!(bytes.len() + chunk.len() <= length);
                bytes.extend_from_slice(&chunk);
            }
            // Keep the response alive without an extra EOF poll, just as the
            // length-bounded segmented consumer does.
            (start, bytes, response)
        });
    }
    let mut retained = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Some(result) = workers.join_next().await {
            let (start, bytes, response) = result.unwrap();
            assert_eq!(bytes, expected[start..start + 1024]);
            retained.push(response);
        }
        let inventory = browse(&other_receipt, CancellationToken::new()).await.unwrap();
        assert_eq!(inventory.books[0].id, book.metadata.id);
        assert_eq!(inventory.books[0].size, expected.len() as u64);
    }).await.expect("parallel ranges or the subsequent catalog were blocked by retained response admission");
    drop(retained);
    server.shutdown().await;
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
