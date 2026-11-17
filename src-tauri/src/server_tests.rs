// SPDX-License-Identifier: GPL-3.0-or-later
//! Unit tests for `server`, kept beside it rather than inline.
use super::parse_range;
use super::{ArchiveWorkers, ReaderConnection, ReaderListener, ReaderStorage, Store};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use uuid::Uuid;

#[test]
fn archive_lookup_tries_exact_then_one_trailing_slash_variant() {
    let missing = || format!("{}: missing", crate::archive::ENTRY_NOT_FOUND);
    let mut calls = Vec::new();
    let value = super::lookup_entry_with_slash_fallback("A/Foo", |path| {
        calls.push(path.to_string());
        if path == "A/Foo/" { Ok("slash") } else { Err(missing()) }
    }).unwrap();
    assert_eq!(value, "slash");
    assert_eq!(calls, vec!["A/Foo".to_string(), "A/Foo/".to_string()]);

    // Exact wins when both entries exist.
    let mut calls = Vec::new();
    let value = super::lookup_entry_with_slash_fallback("A/Foo", |path| {
        calls.push(path.to_string());
        Ok("exact")
    }).unwrap();
    assert_eq!(value, "exact");
    assert_eq!(calls, vec!["A/Foo".to_string()]);

    // The reverse direction removes exactly one trailing slash.
    let mut calls = Vec::new();
    let value = super::lookup_entry_with_slash_fallback("A/Foo/", |path| {
        calls.push(path.to_string());
        if path == "A/Foo" { Ok("plain") } else { Err(missing()) }
    }).unwrap();
    assert_eq!(value, "plain");
    assert_eq!(calls, vec!["A/Foo/".to_string(), "A/Foo".to_string()]);

    // Non-entry failures are never converted into a second lookup.
    let mut calls = Vec::new();
    let error = super::lookup_entry_with_slash_fallback::<()>("A/Foo", |path| {
        calls.push(path.to_string());
        Err("Malformed archive".into())
    }).unwrap_err();
    assert_eq!(error, "Malformed archive");
    assert_eq!(calls, vec!["A/Foo".to_string()]);

    // A non-entry failure on the alternate lookup is preserved, not hidden as
    // the original content miss.
    let mut calls = Vec::new();
    let error = super::lookup_entry_with_slash_fallback::<()>("A/Foo", |path| {
        calls.push(path.to_string());
        if path == "A/Foo/" { Err("Malformed archive".into()) } else { Err(missing()) }
    }).unwrap_err();
    assert_eq!(error, "Malformed archive");
    assert_eq!(calls, vec!["A/Foo".to_string(), "A/Foo/".to_string()]);

    // The empty/root path has no alternate.
    let mut calls = Vec::new();
    let error = super::lookup_entry_with_slash_fallback::<()>("", |path| {
        calls.push(path.to_string());
        Err(missing())
    }).unwrap_err();
    assert!(crate::archive::is_entry_not_found(&error));
    assert_eq!(calls, vec![String::new()]);
}

#[tokio::test]
async fn reader_bootstrap_never_splits_a_bounded_text_prefix() {
    for (text, tail) in [
        ("a".repeat(65_535), "é<script>window.__kiwixReaderBridge=true</script>"),
        ("&".repeat(65_535), "amp;<script>window.__kiwixReaderBridge=true</script>"),
    ] {
        let html = format!("{text}{tail}");
        let body = super::injected_html_body(
            html.as_bytes().to_vec(), "<script src=\"/trusted-bridge.js\"></script>".into(),
        );
        let bytes = axum::body::to_bytes(body, 128 * 1024).await.unwrap();
        let served = std::str::from_utf8(&bytes).expect("injection split a UTF-8 sequence");
        assert!(served.contains(&html), "leading text was corrupted at the scan limit");
        let document = dom_query::Document::from(served);
        assert_eq!(document.select("script").first().attr("src").as_deref(),
            Some("/trusted-bridge.js"), "bootstrap was not placed first");
    }
}

#[tokio::test]
async fn reader_bootstrap_precedes_pre_head_scripts_and_comment_lookalikes() {
    for html in [
        "<!doctype html><script>window.__kiwixReaderBridge=true</script><head><title>Archive</title>",
        "<!doctype html><script>const fake='<head>';</script><head><title>Archive</title>",
        "<!-- --!><!doctype html><script>window.__kiwixReaderBridge=true</script><head>",
        "<!--><!doctype html><script>window.__kiwixReaderBridge=true</script><head>",
        "\u{feff} \n<!-- note --><!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"https://example.invalid/dtd?a>b\"><script>window.__kiwixReaderBridge=true</script><head>",
        "<!doctype html><html style=\"--article-width:42rem\" data-lookalike=\"<head>\"><script>window.__kiwixReaderBridge=true</script><head>",
        "<!doctype html><html data-value=unquoted\" ><script>window.__kiwixReaderBridge=true</script><head>",
        "<!DOCTYPE html SYSTEM 'https://example.invalid/dtd?a>b'><html><script>window.__kiwixReaderBridge=true</script><head>",
        "<!DOCTYPE html SYSTEM \"id><script>window.__kiwixReaderBridge=true</script><head>\"><html>",
        "<?xml version=\"1.0\"?><!doctype html><html><script>window.__kiwixReaderBridge=true</script><head>",
        "<!bogus comment><!doctype html><html><script>window.__kiwixReaderBridge=true</script><head>",
        "<script>window.__kiwixReaderBridge=true</script><head>",
    ] {
        let body = super::injected_html_body(
            html.as_bytes().to_vec(), "<script src=\"/trusted-bridge.js\"></script>".into(),
        );
        let bytes = axum::body::to_bytes(body, 16384).await.unwrap();
        let served = std::str::from_utf8(&bytes).unwrap();
        let document = dom_query::Document::from(served);
        assert_eq!(document.select("script").first().attr("src").as_deref(),
            Some("/trusted-bridge.js"), "archive script ran before bootstrap: {html}");
    }
}


#[tokio::test]
async fn cancelled_archive_consumer_and_drain_still_join_owned_work() {
    let workers = Arc::new(ArchiveWorkers::default());
    let retained = Arc::new(());
    let retained_weak = Arc::downgrade(&retained);
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let consumer_workers = workers.clone();
    let consumer = tokio::spawn(async move {
        consumer_workers.run(move || {
            let _retained = retained;
            started.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        }).await
    });
    started_rx.await.unwrap();
    consumer.abort();
    assert!(consumer.await.unwrap_err().is_cancelled());
    workers.close();
    assert!(workers.run(|| Ok(())).await.is_err());
    let mut draining = Box::pin(workers.drain());
    assert!(futures_util::poll!(draining.as_mut()).is_pending());
    drop(draining);
    assert!(retained_weak.upgrade().is_some());
    release.send(()).unwrap();
    workers.drain().await.unwrap();
    assert!(retained_weak.upgrade().is_none());
}

#[tokio::test]
async fn archive_worker_panics_survive_normal_completion_reaping() {
    let workers = ArchiveWorkers::default();
    assert!(workers.run(|| -> Result<(), String> {
        panic!("archive ownership regression");
    }).await.is_err());
    workers.run(|| Ok(())).await.unwrap();
    let failure = workers.drain().await.unwrap_err();
    assert!(failure.contains("archive ownership regression"), "{failure}");
}

#[tokio::test]
async fn reader_retirement_wakes_pending_socket_reads() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let mut connection = ReaderConnection {
        stream: Some(stream),
        cancelled: Box::pin(cancellation.clone().cancelled_owned()),
    };
    let mut buffer = [0; 1];
    let mut reading = Box::pin(connection.read(&mut buffer));
    assert!(futures_util::poll!(reading.as_mut()).is_pending());
    cancellation.cancel();
    assert_eq!(reading.await.unwrap_err().kind(), std::io::ErrorKind::ConnectionAborted);
    assert!(connection.stream.is_none());
    assert_eq!(client.read(&mut buffer).await.unwrap(), 0);
    assert!(connection.write_all(b"retired").await.is_err());
}

#[tokio::test]
async fn reader_retirement_disconnects_nonreading_http_consumers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let accepted = Arc::new(tokio::sync::Notify::new());
    let handler_accepted = accepted.clone();
    let app = axum::Router::new().fallback(axum::routing::any(move || {
        let accepted = handler_accepted.clone();
        async move {
            accepted.notify_one();
            axum::body::Body::from_stream(futures_util::stream::repeat_with(|| {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(&[0; 65536]))
            }))
        }
    }));
    let listener = ReaderListener { listener, cancellation: cancellation.clone() };
    let stopping = cancellation.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).with_graceful_shutdown(stopping.cancelled_owned()).await
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    accepted.notified().await;
    let mut first = [0; 1];
    client.read_exact(&mut first).await.unwrap();
    // No further socket reads: retirement must not depend on peer progress.
    cancellation.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .expect("retirement waited for a stalled HTTP consumer")
        .unwrap()
        .unwrap();
}

struct StorageFixture {
    storage: Arc<ReaderStorage>,
    directory: PathBuf,
}
impl StorageFixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("kiwix-storage-test-{}", Uuid::new_v4()));
        let store = Arc::new(Store::new(directory.clone()).unwrap());
        Self {
            storage: Arc::new(ReaderStorage {
                installation_id: "archive".into(),
                active: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                store,
                privacy_generation: Arc::new(AtomicU64::new(7)),
                generation: 7,
                documents: Mutex::new(HashMap::new()),
            }),
            directory,
        }
    }
}
impl Drop for StorageFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn batch(document: &str, sequence: u64, operations: Value) -> Value {
    json!({"id":Uuid::new_v4().to_string(),"documentId":document,"sequence":sequence,"operations":operations})
}

#[test]
fn storage_retries_preserve_other_documents_and_reject_reordering() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    storage.store.set_runtime_mode(true, false).unwrap();
    let a = Uuid::new_v4().to_string();
    let b = Uuid::new_v4().to_string();
    let clear = batch(&a, 1, json!([{"kind":"local-clear"}]));
    storage.write_batch(&clear).unwrap();
    let peer = batch(
        &b,
        1,
        json!([{"kind":"local-set","key":"peer","value":"survives"}]),
    );
    storage.write_batch(&peer).unwrap();
    storage.write_batch(&clear).unwrap();
    assert_eq!(
        storage.store.read().unwrap().reader_storage["archive"]["localStorage"]["peer"],
        "survives"
    );
    let later = batch(
        &a,
        2,
        json!([{"kind":"local-set","key":"latest","value":"yes"}]),
    );
    storage.write_batch(&later).unwrap();
    storage.write_batch(&clear).unwrap();
    let reordered = batch(&a, 4, json!([{"kind":"local-clear"}]));
    assert!(storage.write_batch(&reordered).is_err());
    let conflicting_retry = batch(&a, 2, json!([{"kind":"local-clear"}]));
    assert!(storage.write_batch(&conflicting_retry).is_err());
    let saved = storage.store.read().unwrap();
    assert_eq!(
        saved.reader_storage["archive"]["localStorage"]["peer"],
        "survives"
    );
    assert_eq!(
        saved.reader_storage["archive"]["localStorage"]["latest"],
        "yes"
    );
}

#[test]
fn storage_large_batches_use_full_quota_and_fail_atomically() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    let document = Uuid::new_v4().to_string();
    let first = batch(
        &document,
        1,
        json!([{"kind":"local-set","key":"large","value":"x".repeat(700000)}]),
    );
    storage.write_batch(&first).unwrap();
    let original = std::fs::read(fixture.directory.join("state.json")).unwrap();
    let oversized = batch(
        &document,
        2,
        json!([
            {"kind":"local-set","key":"partial","value":"must not appear"},
            {"kind":"local-set","key":"too-large","value":"y".repeat(400000)}
        ]),
    );
    assert!(storage.write_batch(&oversized).is_err());
    assert_eq!(
        std::fs::read(fixture.directory.join("state.json")).unwrap(),
        original
    );
    assert!(
        storage.store.read().unwrap().reader_storage["archive"]["localStorage"]
            .get("partial")
            .is_none()
    );
    let malformed = batch(
        &document,
        2,
        json!([{"kind":"local-clear","command":"open_external"}]),
    );
    assert!(storage.write_batch(&malformed).is_err());
    let wrong_cookie = batch(
        &document,
        2,
        json!([{
            "kind":"cookie-set","key":"/\nother",
            "cookie":{"name":"name","path":"/","value":"value","expires":9999999999999.0}
        }]),
    );
    assert!(storage.write_batch(&wrong_cookie).is_err());
    let valid = batch(
        &document,
        2,
        json!([{"kind":"local-set","key":"small","value":"accepted"}]),
    );
    storage.write_batch(&valid).unwrap();
}

#[test]
fn storage_utf8_cookie_bounds_reject_without_consuming_sequence() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    let document = Uuid::new_v4().to_string();
    for (name, value, path) in [
        ("name".to_string(), "é".repeat(3000), "/".to_string()),
        ("n".repeat(3000), "v".repeat(2000), "/".to_string()),
        (
            "name".to_string(),
            "value".to_string(),
            format!("/{}", "é".repeat(4096)),
        ),
    ] {
        let invalid = batch(
            &document,
            1,
            json!([{
                "kind":"cookie-set", "key":format!("{path}\n{name}"),
                "cookie":{"name":name,"value":value,"path":path,"expires":9999999999999_u64}
            }]),
        );
        assert!(matches!(
            storage.write_batch(&invalid),
            Err(super::ReaderStorageError::Rejected(_))
        ));
        assert!(storage.documents.lock().unwrap().is_empty());
    }
    storage
        .write_batch(&batch(
            &document,
            1,
            json!([{"kind":"local-set","key":"after-invalid","value":"accepted"}]),
        ))
        .unwrap();
}

#[test]
fn storage_combines_cookie_and_local_quota_without_blocking_other_documents() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    let document = Uuid::new_v4().to_string();
    storage
        .write_batch(&batch(
            &document,
            1,
            json!([{"kind":"local-set","key":"large","value":"x".repeat(1024 * 1024 - 2000)}]),
        ))
        .unwrap();
    let rejected = batch(
        &document,
        2,
        json!([{
            "kind":"cookie-set","key":"/\nname",
            "cookie":{"name":"name","value":"é".repeat(1500),"path":"/","expires":9999999999999_u64}
        }]),
    );
    assert!(matches!(
        storage.write_batch(&rejected),
        Err(super::ReaderStorageError::Rejected(_))
    ));
    let other = Uuid::new_v4().to_string();
    storage
        .write_batch(&batch(
            &other,
            1,
            json!([{"kind":"local-set","key":"peer","value":"survives"}]),
        ))
        .unwrap();
    let saved = storage.store.read().unwrap();
    assert_eq!(
        saved.reader_storage["archive"]["localStorage"]["peer"],
        "survives"
    );
    assert!(saved.reader_storage["archive"]["cookies"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn storage_reclaims_expired_cookie_quota_without_rejecting_accepted_writes() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    storage.store.update(|state| {
        state.reader_storage.insert("archive".into(), json!({
            "localStorage":{"large":"x".repeat(1_044_000)},
            "cookies":[
                {"name":"expired","path":"/old/","value":"e".repeat(3000),"expires":1},
                {"name":"live","path":"/kept/","value":"v".repeat(512),"expires":9999999999999_u64}
            ]
        }));
        Ok(true)
    }).unwrap();
    let document = Uuid::new_v4().to_string();
    storage.write_batch(&batch(&document, 1, json!([
        {"kind":"local-set","key":"large","value":"y".repeat(1_047_000)}
    ]))).unwrap();
    // Also reclaim an otherwise-valid cookie that expires while queued.
    storage.write_batch(&batch(&document, 2, json!([
        {"kind":"cookie-set","key":"/old/\nexpired",
         "cookie":{"name":"expired","path":"/old/","value":"e".repeat(3000),"expires":1}}
    ]))).unwrap();
    storage.write_batch(&batch(&document, 3, json!([
        {"kind":"local-set","key":"after-expiry","value":"accepted"}
    ]))).unwrap();
    let state = storage.store.read().unwrap();
    let saved = &state.reader_storage["archive"];
    assert_eq!(saved["localStorage"]["large"].as_str().unwrap().len(), 1_047_000);
    assert_eq!(saved["localStorage"]["after-expiry"], "accepted");
    let cookies = saved["cookies"].as_array().unwrap();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0]["name"], "live");
    assert_eq!(cookies[0]["path"], "/kept/");
    assert_eq!(cookies[0]["value"], "v".repeat(512));
}

#[test]
fn storage_io_failure_is_retryable_with_original_uuid_and_sequence() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    let destination = fixture.directory.join("state.json");
    let original = std::fs::read(&destination).unwrap();
    std::fs::remove_file(&destination).unwrap();
    std::fs::create_dir(&destination).unwrap();
    let document = Uuid::new_v4().to_string();
    let request = batch(
        &document,
        1,
        json!([{"kind":"local-set","key":"retry","value":"saved-once"}]),
    );
    assert!(matches!(
        storage.write_batch(&request),
        Err(super::ReaderStorageError::Retryable(_))
    ));
    assert!(storage.documents.lock().unwrap().is_empty());
    std::fs::remove_dir(&destination).unwrap();
    std::fs::write(&destination, original).unwrap();
    storage.write_batch(&request).unwrap();
    storage.write_batch(&request).unwrap();
    let documents = storage.documents.lock().unwrap();
    assert_eq!(documents[&document].0, 1);
    assert_eq!(documents[&document].1, request["id"].as_str().unwrap());
}

#[test]
fn storage_private_and_readonly_modes_never_write_disk() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    let original = std::fs::read(fixture.directory.join("state.json")).unwrap();
    storage.store.set_runtime_mode(true, false).unwrap();
    let document = Uuid::new_v4().to_string();
    let secret = batch(
        &document,
        1,
        json!([{"kind":"local-set","key":"secret","value":"private"}]),
    );
    storage.write_batch(&secret).unwrap();
    assert_eq!(
        storage.store.read().unwrap().reader_storage["archive"]["localStorage"]["secret"],
        "private"
    );
    assert_eq!(
        std::fs::read(fixture.directory.join("state.json")).unwrap(),
        original
    );
    storage.store.set_runtime_mode(false, true).unwrap();
    assert!(storage
        .write_batch(&batch(&document, 2, json!([{"kind":"local-clear"}])))
        .is_err());
    assert!(!storage
        .store
        .read()
        .unwrap()
        .reader_storage
        .contains_key("archive"));
    assert_eq!(
        std::fs::read(fixture.directory.join("state.json")).unwrap(),
        original
    );
}

#[test]
fn storage_generation_is_checked_after_acquiring_store_transaction() {
    let fixture = StorageFixture::new();
    let storage = fixture.storage.clone();
    let worker_storage = storage.clone();
    let request = batch(
        &Uuid::new_v4().to_string(),
        1,
        json!([{"kind":"local-set","key":"stale","value":"no"}]),
    );
    let mut worker = None;
    storage
        .store
        .update(|_| {
            worker = Some(std::thread::spawn(move || {
                worker_storage.write_batch(&request)
            }));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            // The writer holds its document registry while waiting on this Store
            // mutex, so generation changes here reproduce the retirement race.
            while storage.documents.try_lock().is_ok() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "storage writer did not enter"
                );
                std::thread::yield_now();
            }
            storage.privacy_generation.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        worker.unwrap().join().unwrap(),
        Err(super::ReaderStorageError::Retired)
    ));
    assert!(!storage
        .store
        .read()
        .unwrap()
        .reader_storage
        .contains_key("archive"));
}

#[test]
fn storage_retirement_precedes_store_readonly_gate_classification() {
    let fixture = StorageFixture::new();
    let storage = &fixture.storage;
    storage.store.set_runtime_mode(false, true).unwrap();
    storage.privacy_generation.store(8, Ordering::SeqCst);
    let request = batch(
        &Uuid::new_v4().to_string(),
        1,
        json!([{"kind":"local-set","key":"retired","value":"never-persist"}]),
    );
    assert!(matches!(
        storage.write_batch(&request),
        Err(super::ReaderStorageError::Retired)
    ));
    assert!(storage.documents.lock().unwrap().is_empty());
    assert!(!storage
        .store
        .read()
        .unwrap()
        .reader_storage
        .contains_key("archive"));
}

#[test]
fn readonly_binary_wrapper_has_no_download_affordance() {
    let readonly = super::resource_fallback("Book", "/resource", "file.pdf", true);
    assert!(!readonly.contains("<a"));
    assert!(!readonly.contains("download"));
    assert!(
        super::resource_fallback("Book", "/resource", "file.pdf", false).contains(" download")
    );
}

#[test]
fn media_ranges_preserve_boundaries() {
    assert_eq!(parse_range("bytes=0-0", 100).unwrap(), (0, 1));
    assert_eq!(parse_range("bytes=50-", 100).unwrap(), (50, 50));
    assert_eq!(parse_range("bytes=-10", 100).unwrap(), (90, 10));
    assert_eq!(parse_range("bytes=90-900", 100).unwrap(), (90, 10));
    for bad in [
        "bytes=100-",
        "bytes=20-10",
        "bytes=-0",
        "bytes=0-1,3-4",
        "items=0-1",
    ] {
        assert!(parse_range(bad, 100).is_err());
    }
    assert!(parse_range("bytes=0-", 0).is_err());
}
#[test]
fn encoded_locations_preserve_literal_delimiters_without_changing_authority() {
    let raw = "A/chapter#1?100% + café.html";
    let location = format!("{}?query=%23kept#route", super::encode_path(raw));
    assert_eq!(super::decode_location_path(&location).unwrap(), raw);
    assert_eq!(super::normalize_location(&location).unwrap(), location);
    assert_eq!(
        super::decode_location_path("A/literal%2523.html#route").unwrap(),
        "A/literal%23.html"
    );
    for path in [
        "//foreign.example/path",
        "/literal-leading-slash",
        "A/normal",
    ] {
        let address = url::Url::parse(&format!(
            "http://archive.localhost:1234/{}",
            super::encode_path(path)
        ))
        .unwrap();
        assert_eq!(
            address.origin().ascii_serialization(),
            "http://archive.localhost:1234"
        );
        assert_eq!(super::decode_location_path(address.path()).unwrap(), path);
    }
    assert!(super::decode_location_path("A/%FF.html").is_err());
}

#[test]
fn split_location_requires_an_encoded_path() {
    let raw = "A/Foo?bar#hash.html";
    let location = super::encode_path(raw);
    assert_eq!(location, "A/Foo%3Fbar%23hash%2Ehtml");
    assert_eq!(super::split_location(&location), (location.as_str(), ""));
    assert_eq!(super::decode_location_path(&location).unwrap(), raw);
    // A genuine archive-authored suffix stays a suffix and is never decoded.
    let routed = format!("{location}?q=1#heading");
    assert_eq!(
        super::split_location(&routed),
        (location.as_str(), "?q=1#heading")
    );
    assert_eq!(super::decode_location_path(&routed).unwrap(), raw);
    assert_eq!(super::normalize_location(&routed).unwrap(), routed);
    // Splitting a raw libzim path is exactly the producer bug: `?` and `#`
    // are path text there, so a producer must encode before it builds a
    // location.
    assert_eq!(super::split_location(raw), ("A/Foo", "?bar#hash.html"));
}

#[test]
fn malformed_locations_are_rejected_instead_of_resolved() {
    for location in [
        "A/%ZZ.html",
        "A/%2.html",
        "A/trailing%",
        "A/100%.html",
        "A/%FF.html",
    ] {
        assert!(
            super::decode_location_path(location).is_err(),
            "decoded {location}"
        );
        assert!(
            super::normalize_location(location).is_err(),
            "normalized {location}"
        );
    }
    // Percent signs encoded exactly once still round-trip.
    let raw = "A/100% + café.html";
    assert_eq!(
        super::decode_location_path(&super::encode_path(raw)).unwrap(),
        raw
    );
}

/// A live reader origin over the real archive fixture, so the streaming
/// path is exercised end to end instead of through a hand-built handler.
struct ReaderFixture {
    _server: super::ReaderServer,
    host: String,
    port: u16,
    directory: PathBuf,
    archive: crate::archive::Archive,
}

impl ReaderFixture {
    async fn start() -> Self {
        Self::start_with("media.zim").await
    }

    async fn start_with(name: &str) -> Self {
        Self::start_path(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .await
    }

    async fn start_path(fixture: PathBuf) -> Self {
        let archive = crate::archive::Archive::open(&fixture).unwrap();
        let directory = std::env::temp_dir().join(format!("kiwix-reader-test-{}", Uuid::new_v4()));
        let store = Arc::new(Store::new(directory.clone()).unwrap());
        let server = super::ReaderServer::start(
            archive.clone(),
            "reader-fixture".into(),
            "installation".into(),
            "http://shell.example".into(),
            store,
            Arc::new(AtomicU64::new(1)),
        )
        .await
        .unwrap();
        let host = server
            .origin
            .strip_prefix("http://")
            .expect("reader origins are plain loopback HTTP")
            .to_string();
        let port = host
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
            .expect("reader origin names its port");
        Self { _server: server, host, port, directory, archive }
    }

    fn url(&self, path: &str) -> String {
        format!("/{path}")
    }
}

impl Drop for ReaderFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// One request over a fresh connection. The archive host is addressed
/// directly so the test never depends on resolving `*.localhost`.
async fn fetch(
    port: u16,
    host: &str,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
) -> HttpResponse {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response has a header block");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("HTTP response has a status code");
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    HttpResponse { status, headers, body }
}

const MEDIA_PATH: &str = "media/probe.mp4";

#[tokio::test]
async fn zimit2_static_replay_and_warc_asset_are_served() {
    let fixture = ReaderFixture::start_with("zimit_warc.zim").await;
    let article = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        "/example.org/guide/index.html",
        &[],
    )
    .await;
    assert_eq!(article.status, 200);
    let html = String::from_utf8(article.body).unwrap();
    assert!(html.contains("Zimit2 static replay"));
    assert!(html.contains("No service worker is required."));
    assert!(html.contains("wombatSetup.js"));
    assert!(html.contains("bridge.js"), "archive HTML was not instrumented");

    for (path, expected) in [
        ("/_zim_static/wombat.js", b"window.__kiwixZimitWombat = true;\n".as_slice()),
        ("/_zim_static/wombatSetup.js", b"window.__kiwixWombatSetup = true;\n".as_slice()),
    ] {
        let script = fetch(fixture.port, &fixture.host, "GET", path, &[]).await;
        assert_eq!(script.status, 200);
        assert_eq!(script.header("content-type"), Some("text/javascript; charset=utf-8"));
        assert_eq!(script.body.as_slice(), expected);
    }

    let warc = fetch(fixture.port, &fixture.host, "GET", "/capture.warc", &[]).await;
    assert_eq!(warc.status, 200);
    assert_eq!(warc.header("content-type"), Some("application/warc"));
    assert_eq!(warc.header("content-length"), Some("104"));
    assert_eq!(
        warc.body,
        b"WARC/1.0\r\nWARC-Type: response\r\nWARC-Target-URI: https://example.org/\r\nContent-Length: 12\r\n\r\nhello world\n"
    );
}

#[tokio::test]
#[ignore = "requires KIWIX_ZIMIT2_FIXTURE set to a real Zimit2 ZIM"]
async fn real_zimit2_fixture_serves_main_page_and_wombat() {
    let path = std::env::var_os("KIWIX_ZIMIT2_FIXTURE")
        .expect("set KIWIX_ZIMIT2_FIXTURE to a real Zimit2 ZIM");
    let fixture = ReaderFixture::start_path(PathBuf::from(path)).await;
    let main = fixture.archive.main_entry().unwrap();
    let article = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &format!("/{}", main.path),
        &[],
    )
    .await;
    assert_eq!(article.status, 200);
    let html = String::from_utf8_lossy(&article.body);
    assert!(html.contains("wombat.js"));
    assert!(html.contains("bridge.js"), "archive HTML was not instrumented");

    for path in ["/_zim_static/wombat.js", "/_zim_static/wombatSetup.js"] {
        let script = fetch(fixture.port, &fixture.host, "GET", path, &[]).await;
        assert_eq!(script.status, 200);
        assert!(script
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/javascript")));
        assert!(!script.body.is_empty());
    }
}

#[tokio::test]
async fn ranged_media_request_returns_exact_bytes_and_headers() {
    let fixture = ReaderFixture::start().await;
    let size = fixture.archive.entry(MEDIA_PATH).unwrap().size;
    assert!(size > 2 * 1024 * 1024, "fixture media spans streaming ranges");
    let response = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("Range", "bytes=1000-1999")],
    )
    .await;
    assert_eq!(response.status, 206);
    assert_eq!(response.header("content-range"), Some(format!("bytes 1000-1999/{size}").as_str()));
    assert_eq!(response.header("content-length"), Some("1000"));
    assert_eq!(response.header("accept-ranges"), Some("bytes"));
    assert_eq!(response.header("content-type"), Some("video/mp4"));
    assert!(response.header("etag").is_some_and(|etag| etag.starts_with('"')));
    assert_eq!(
        response.body,
        fixture.archive.read(MEDIA_PATH, 1000, 1000).unwrap()
    );
}

#[tokio::test]
async fn media_seek_returns_the_requested_middle_slice() {
    let fixture = ReaderFixture::start().await;
    let size = fixture.archive.entry(MEDIA_PATH).unwrap().size;
    let start = 2 * 1024 * 1024 + 12345;
    let response = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("Range", &format!("bytes={start}-{}", start + 4095))],
    )
    .await;
    assert_eq!(response.status, 206);
    assert_eq!(
        response.header("content-range"),
        Some(format!("bytes {start}-{}/{size}", start + 4095).as_str())
    );
    assert_eq!(response.body.len(), 4096);
    assert_eq!(
        response.body,
        fixture.archive.read(MEDIA_PATH, start, 4096).unwrap()
    );
    // The slice is the middle of the entry, not its first bytes.
    assert_ne!(response.body, fixture.archive.read(MEDIA_PATH, 0, 4096).unwrap());
}

#[tokio::test]
async fn full_media_get_streams_the_whole_entry() {
    let fixture = ReaderFixture::start().await;
    let size = fixture.archive.entry(MEDIA_PATH).unwrap().size;
    let response = fetch(fixture.port, &fixture.host, "GET", &fixture.url(MEDIA_PATH), &[]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-length"), Some(size.to_string().as_str()));
    assert_eq!(response.header("accept-ranges"), Some("bytes"));
    assert_eq!(response.body.len() as u64, size);
    assert_eq!(response.body, fixture.archive.read(MEDIA_PATH, 0, size).unwrap());
}

#[tokio::test]
async fn head_reports_media_metadata_without_a_body() {
    let fixture = ReaderFixture::start().await;
    let size = fixture.archive.entry(MEDIA_PATH).unwrap().size;
    let response = fetch(fixture.port, &fixture.host, "HEAD", &fixture.url(MEDIA_PATH), &[]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-length"), Some(size.to_string().as_str()));
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn concurrent_article_request_is_not_serialized_behind_media() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = ReaderFixture::start().await;
    // Open a whole-entry media response and never drain it, so the server's
    // write side stays pending while other requests arrive.
    let mut stalled = tokio::net::TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .unwrap();
    stalled
        .write_all(
            format!(
                "GET /{MEDIA_PATH} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                fixture.host
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut first = [0_u8; 1024];
    let read = stalled.read(&mut first).await.unwrap();
    assert!(read > 0, "the media response starts before the client drains it");
    // The article is a separate connection served by the same archive.
    let started = std::time::Instant::now();
    let article = fetch(fixture.port, &fixture.host, "GET", "/index.html", &[]).await;
    let elapsed = started.elapsed();
    assert_eq!(article.status, 200);
    assert!(article.body.windows(6).any(|window| window == b"<video"));
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "article request waited {elapsed:?} behind an undrained media stream"
    );
    drop(stalled);
}

#[tokio::test]
async fn revalidating_a_cached_media_entry_returns_not_modified() {
    let fixture = ReaderFixture::start().await;
    let first = fetch(fixture.port, &fixture.host, "GET", &fixture.url(MEDIA_PATH), &[]).await;
    let etag = first.header("etag").expect("media responses carry a validator").to_string();
    let cached = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("If-None-Match", &etag)],
    )
    .await;
    assert_eq!(cached.status, 304);
    assert!(cached.body.is_empty());
    assert_eq!(cached.header("etag"), Some(etag.as_str()));
    // A different validator is a miss and must stream the entry again.
    let changed = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("If-None-Match", "\"stale\"")],
    )
    .await;
    assert_eq!(changed.status, 200);
    assert_eq!(changed.body, first.body);
}

#[tokio::test]
async fn if_range_selects_partial_or_full_media() {
    let fixture = ReaderFixture::start().await;
    let size = fixture.archive.entry(MEDIA_PATH).unwrap().size;
    let etag = fetch(fixture.port, &fixture.host, "GET", &fixture.url(MEDIA_PATH), &[])
        .await
        .header("etag")
        .expect("media responses carry a validator")
        .to_string();
    // A matching validator keeps the seek partial.
    let partial = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("Range", "bytes=0-1023"), ("If-Range", &etag)],
    )
    .await;
    assert_eq!(partial.status, 206);
    assert_eq!(partial.header("content-range"), Some(format!("bytes 0-1023/{size}").as_str()));
    // A stale validator must never splice a cached prefix into new bytes.
    let whole = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("Range", "bytes=0-1023"), ("If-Range", "\"stale\"")],
    )
    .await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.header("content-range"), None);
    assert_eq!(whole.body.len() as u64, size);
}

#[tokio::test]
async fn unsatisfiable_range_is_rejected_with_the_entry_size() {
    let fixture = ReaderFixture::start().await;
    let size = fixture.archive.entry(MEDIA_PATH).unwrap().size;
    let response = fetch(
        fixture.port,
        &fixture.host,
        "GET",
        &fixture.url(MEDIA_PATH),
        &[("Range", &format!("bytes={}-", size + 10))],
    )
    .await;
    assert_eq!(response.status, 416);
    assert_eq!(response.header("content-range"), Some(format!("bytes */{size}").as_str()));
    assert!(response.body.is_empty());
}
