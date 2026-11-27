// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
fn fixture() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim") }
fn state(store: Arc<Store>) -> LibraryIndex {
    let cancellation = SharingCancellation::new();
    let archives = Arc::new(LibraryArchives::default());
    let session_hosts = SessionHosts::new(&[IpAddr::V4(Ipv4Addr::LOCALHOST)], &SharingBinding::Loopback {});
    LibraryIndex {
        token: "secret".into(), authorities: session_hosts.for_port(8080), port: 8080,
        store: store.clone(), cancellation: cancellation.clone(), archives: archives.clone(),
        availability: Arc::new(AvailabilityCache::default()),
        session: Arc::new(SharingSession::new(None, None).unwrap()),
        hosts: LibraryHosts { token: "secret".into(), bind: IpAddr::V4(Ipv4Addr::LOCALHOST), binding: SharingBinding::Loopback {}, hosts: session_hosts, store, cancellation, archives, entries: Arc::new(Mutex::new(HashMap::new())), seq: Arc::new(std::sync::atomic::AtomicU64::new(0)), retired: Arc::new(Mutex::new(Vec::new())) },
    }
}
fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("host", "127.0.0.1:8080".parse().unwrap());
    headers
}
async fn bytes(response: Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap().to_vec()
}
#[tokio::test]
async fn library_index_page_is_bounded_and_continues() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-index-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let total = OPDS_PAGE_MAX + 1;
    store.update(|s| {
        for index in 1..total {
            let mut extra = book.clone();
            extra.metadata.id = Uuid::new_v4().to_string();
            extra.installation_id = crate::store::new_installation_id();
            extra.metadata.title = format!("Title {index:04}");
            s.books.push(extra);
        }
        Ok(())
    }).unwrap();
    let state = state(store);
    let first = String::from_utf8(bytes(library_index_page(&state, IpAddr::V4(Ipv4Addr::LOCALHOST), &"/secret/".parse().unwrap(), &headers())).await).unwrap();
    assert_eq!(first.matches("<article class=\"book\"").count(), OPDS_PAGE_MAX);
    assert!(first.contains(&format!("/secret/?start={OPDS_PAGE_MAX}")), "first page must link onward");
    let second = String::from_utf8(bytes(library_index_page(&state, IpAddr::V4(Ipv4Addr::LOCALHOST), &format!("/secret/?start={OPDS_PAGE_MAX}").parse().unwrap(), &headers())).await).unwrap();
    assert_eq!(second.matches("<article class=\"book\"").count(), total - OPDS_PAGE_MAX);
    assert!(!second.contains("rel=\"next\""), "the last page has no continuation");
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn complete_pagination_url_survives_strict_xml_and_filter_roundtrip() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-opds-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (store, mut book) = tests::library_store(&fixture(), &root);
    store.update(|s| {
        s.books[0].metadata.title = "Alpha & Beta".into();
        s.books[0].metadata.language = "eng".into();
        s.books[0].metadata.tags = vec!["reference".into()];
        book = s.books[0].clone();
        book.metadata.id = Uuid::new_v4().to_string();
        book.installation_id = crate::store::new_installation_id();
        s.books.push(book.clone());
        Ok(())
    }).unwrap();
    let state = state(store);
    let response = opds_entries(&state, IpAddr::V4(Ipv4Addr::LOCALHOST), &"/secret/catalog/v2/entries?count=1&q=Alpha%20%26&lang=eng&tag=reference".parse().unwrap(), &headers());
    let xml = String::from_utf8(bytes(response).await).unwrap();
    let document = roxmltree::Document::parse(&xml).unwrap();
    let next = document.descendants().find(|n| n.attribute("rel") == Some("next")).unwrap().attribute("href").unwrap();
    let url = url::Url::parse(next).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["start"], "1");
    assert_eq!(query["count"], "1");
    assert_eq!(query["q"], "Alpha &");
    assert_eq!(query["lang"], "eng");
    assert_eq!(query["tag"], "reference");
    let next: Uri = next.parse().unwrap();
    let xml = String::from_utf8(bytes(opds_entries(&state, IpAddr::V4(Ipv4Addr::LOCALHOST), &next, &headers())).await).unwrap();
    let document = roxmltree::Document::parse(&xml).unwrap();
    assert!(!document.descendants().any(|n| n.attribute("rel") == Some("next")));
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn sharing_predicate_excludes_only_session_provider_references() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-provider-share-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (store, mut book) = tests::library_store(&fixture(), &root);
    assert!(shareable_library_book(&book));
    book.source_reference = Some(crate::provider_reference::ProviderReference {
        reference: Uuid::new_v4().to_string(),
        display_name: "archive.zim".into(),
        persistent: true,
    });
    assert!(shareable_library_book(&book), "a saved external archive remains an ordinary library entry");
    book.source_reference.as_mut().unwrap().persistent = false;
    assert!(!shareable_library_book(&book), "a session-only overlay must not become LAN-visible");
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn split_acquisition_ranges_head_and_retirement_preserve_one_logical_zim() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-split-share-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (store, mut book) = tests::library_store(&fixture(), &root);
    let expected = std::fs::read(&book.path).unwrap();
    let boundary = 97;
    for (name, data) in [("split.zimaa", &expected[..boundary]), ("split.zimab", &expected[boundary..])] {
        std::fs::write(root.join(name), data).unwrap();
    }
    std::fs::write(root.join("split.zimac"), b"not part of the archive").unwrap();
    book.path = root.join("split.zimaa").to_string_lossy().into_owned();
    store.update(|s| { s.books[0] = book.clone(); Ok(()) }).unwrap();
    let state = state(store.clone());
    let uri: Uri = format!("/secret/download/{}", book.metadata.id).parse().unwrap();
    for range in [None, Some("bytes=95-100"), Some("bytes=97-97"), Some("bytes=-16")] {
        let mut headers = headers();
        if let Some(range) = range { headers.insert("range", range.parse().unwrap()); }
        let response = library_request(State(state.clone()), Method::GET, uri.clone(), headers.clone(), Body::empty()).await;
        assert_eq!(response.status(), if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK });
        let (start, length) = range.map(|r| parse_range(r, expected.len() as u64).unwrap()).unwrap_or((0, expected.len() as u64));
        assert_eq!(response.headers()["content-length"], length.to_string());
        let get_headers = response.headers().clone();
        assert_eq!(bytes(response).await, expected[start as usize..(start + length) as usize]);
        let response = library_request(State(state.clone()), Method::HEAD, uri.clone(), headers, Body::empty()).await;
        assert_eq!(response.headers(), &get_headers);
        assert!(bytes(response).await.is_empty());
    }
    let response = library_request(State(state.clone()), Method::GET, uri.clone(), headers(), Body::empty()).await;
    store.update(|s| { s.books.clear(); Ok(()) }).unwrap();
    assert!(axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.is_err(), "Retired ownership must stop even an already-open download");
    let response = library_request(State(state.clone()), Method::GET, uri, headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn provider_reference_replacement_retires_the_previous_download_owner() {
    let mut book = tests::feed_book();
    book.source_reference = Some(crate::provider_reference::ProviderReference { reference: "old-native-reference".into(), display_name: "archive.zim".into(), persistent: true });
    let mut replacement = book.clone();
    assert!(same_owner(Some(&book), Some(&replacement)));
    replacement.source_reference.as_mut().unwrap().reference = "new-native-reference".into();
    assert!(!same_owner(Some(&book), Some(&replacement)));
    assert!(!same_owner(Some(&book), None));
    replacement = book.clone();
    replacement.available = false;
    assert!(!same_owner(Some(&book), Some(&replacement)));
}
#[tokio::test]
async fn reader_endpoints_keep_host_origin_token_and_live_scope_authority() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-lan-search-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let state = state(store.clone());
    let uri: Uri = format!("/secret/suggest?q=a&book={}", book.metadata.id).parse().unwrap();
    for (host, origin, path) in [("evil.test:8080", None, uri.clone()), ("127.0.0.1:8080", Some("http://127.0.0.1:8081"), uri.clone()), ("127.0.0.1:8080", None, "/wrong/suggest?q=a".parse().unwrap())] {
        let mut headers = HeaderMap::new();
        headers.insert("host", host.parse().unwrap());
        if let Some(origin) = origin { headers.insert("origin", origin.parse().unwrap()); }
        assert_eq!(library_request(State(state.clone()), Method::GET, path, headers, Body::empty()).await.status(), StatusCode::FORBIDDEN);
    }
    let response = library_request(State(state.clone()), Method::GET, uri.clone(), headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert!(value["suggestions"].is_array());
    let archive = Arc::new(Archive::open(&fixture()).unwrap());
    let selected = Index {
        token: "selected".into(),
        authorities: AcceptedAuthorities::new(&[IpAddr::V4(Ipv4Addr::LOCALHOST)], &SharingBinding::Loopback {}, 8080),
        entries: Arc::new(vec![IndexBook { id: book.metadata.id.clone(), title: book.metadata.title.clone(), port: 8081, archive: Arc::downgrade(&archive), fulltext: book.metadata.has_fulltext_index }]),
        store: store.clone(), cancellation: SharingCancellation::new(),
        library: state.clone(),
    };
    let response = index_request(State(selected.clone()), Method::GET, "/selected/search?q=a&book=unshared-book".parse().unwrap(), headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = index_request(State(selected), Method::GET, format!("/selected/suggest?q=a&book={}", book.metadata.id).parse().unwrap(), headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert!(value["suggestions"].as_array().unwrap().iter().all(|row| row["bookId"] == book.metadata.id));
    store.update(|s| { s.books.clear(); Ok(()) }).unwrap();
    assert_eq!(library_request(State(state.clone()), Method::GET, uri, headers(), Body::empty()).await.status(), StatusCode::BAD_REQUEST);
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn repeated_no_match_suggestions_do_not_flush_the_archive_cache() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-lan-suggest-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let mut extra = Vec::new();
    for name in ["delimiters", "indexed_search", "large_article", "main_path", "media", "no_main_page", "preview", "zimit_warc"] {
        let fixture = fixture().with_file_name(format!("{name}.zim"));
        let path = root.join(format!("{name}.zim"));
        std::fs::copy(fixture, &path).unwrap();
        let mut other = book.clone();
        other.metadata = Archive::open(&path).unwrap().metadata().unwrap();
        other.path = path.to_string_lossy().into_owned();
        other.installation_id = crate::store::new_installation_id();
        extra.push(other);
    }
    let count = 1 + extra.len();
    store.update(|state| {
        state.books.extend(extra);
        Ok(())
    }).unwrap();
    // Under the default desktop cache (32), all 9 fixtures stay warm with zero
    // re-opens on the second suggestion pass.
    if count <= LIBRARY_ARCHIVE_CACHE {
        let warmed = state(store.clone());
        for prefix in ["qzqxnoarticleone", "qzqxnoarticletwo"] {
            let uri = format!("/secret/suggest?q={prefix}").parse().unwrap();
            let response = library_request(State(warmed.clone()), Method::GET, uri, headers(), Body::empty()).await;
            assert_eq!(response.status(), StatusCode::OK);
            let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
            assert_eq!(value["suggestions"], json!([]));
            assert_eq!(value["errors"], json!([]));
            assert_eq!(warmed.archives.opens(), count);
        }
        drop(warmed);
    }
    // When a library exceeds cache capacity, search misses still do not flush
    // the warmed slots.
    let small_cap = 8usize;
    assert!(count > small_cap);
    let mut state = state(store);
    state.archives = Arc::new(LibraryArchives::with_capacity(small_cap));
    for (pass, prefix) in ["qzqxnoarticleone", "qzqxnoarticletwo"].into_iter().enumerate() {
        let uri = format!("/secret/suggest?q={prefix}").parse().unwrap();
        let response = library_request(State(state.clone()), Method::GET, uri, headers(), Body::empty()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
        assert_eq!(value["suggestions"], json!([]));
        assert_eq!(value["errors"], json!([]));
        assert_eq!(state.archives.opens(), count + pass * (count - small_cap));
    }
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_download_refuses_same_content_path_replacement() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-sharing-replacement-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (_store, book) = tests::library_store(&fixture(), &root);
    let (mut download, _, _) = open_download(book.clone()).unwrap();
    let replacement = root.join("replacement.zim");
    std::fs::copy(&book.path, &replacement).unwrap();
    std::fs::rename(&book.path, root.join("original.zim")).unwrap();
    std::fs::rename(&replacement, &book.path).unwrap();
    assert!(download.read(0, 80).is_err(), "Identical replacement bytes do not transfer retained pathname ownership");
    drop(download);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn bootstrap_and_inventory_keep_reserved_and_long_archive_names_acquirable() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-share-names-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    store.update(|s| {
        s.books[0].metadata.name = "CON".into();
        for name in [format!("{}.zim", "a".repeat(117)), "ordinary".into()] {
            let mut additional = book.clone();
            additional.metadata.id = Uuid::new_v4().to_string();
            additional.installation_id = crate::store::new_installation_id();
            additional.metadata.name = name;
            s.books.push(additional);
        }
        Ok(())
    }).unwrap();
    let state = state(store);
    let bootstrap: Uri = format!("/nearby/{}", state.session.id).parse().unwrap();
    let response = library_request(State(state.clone()), Method::POST, bootstrap, headers(), Body::from("{}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bootstrap: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    let books = bootstrap["books"].as_array().unwrap();
    assert_eq!(books.len(), 3);
    for book in books {
        let name = book["fileName"].as_str().unwrap();
        assert!(name.len() <= 120);
        crate::acquisition::shared_file_name(name).unwrap();
    }
    assert!(books.iter().any(|book| book["fileName"] == "_CON.zim"));
    assert!(books.iter().any(|book| book["fileName"] == format!("{}.zim", "a".repeat(116))));
    assert!(books.iter().any(|book| book["fileName"] == "ordinary.zim"));
    let response = library_request(State(state.clone()), Method::GET, "/secret/inventory".parse().unwrap(), headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let inventory: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(inventory["books"], bootstrap["books"]);

    let download: Uri = format!("/secret/download/{}", book.metadata.id).parse().unwrap();
    let response = library_request(State(state.clone()), Method::GET, download, headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-disposition"], "attachment; filename=\"_CON.zim\"");
    assert_eq!(bytes(response).await, std::fs::read(&book.path).unwrap());
    state.cancellation.cancel();
    state.hosts.shutdown().await.unwrap();
    state.cancellation.0.workers.drain().await.unwrap();
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn open_bootstrap_issues_only_read_only_scoped_capability_and_revokes_on_stop() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-bootstrap-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let state = state(store);
    let uri: Uri = format!("/nearby/{}", state.session.id).parse().unwrap();
    let response = library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from("{}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert!(!response.headers().contains_key("access-control-allow-origin"));
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(value["sessionId"], state.session.id);
    assert_eq!(value["token"], state.token);
    assert_ne!(value["sessionId"], value["token"]);
    assert_eq!(value["name"], "Shared library");
    assert_eq!(value["books"][0]["id"], book.metadata.id);
    assert_eq!(value["books"][0]["archiveId"], book.metadata.id);
    assert_eq!(value["books"][0]["size"], book.metadata.size);
    assert!(value["books"][0]["fileName"].as_str().unwrap().ends_with(".zim"));
    assert!(value["books"][0].get("path").is_none());

    for (path, host, origin) in [
        (uri.clone(), "attacker.test:8080", None),
        (uri.clone(), "127.0.0.1:8080", Some("http://attacker.test")),
        ("/nearby/wrong-session".parse().unwrap(), "127.0.0.1:8080", None),
    ] {
        let mut headers = headers();
        headers.insert("host", host.parse().unwrap());
        if let Some(origin) = origin { headers.insert("origin", origin.parse().unwrap()); }
        assert_eq!(library_request(State(state.clone()), Method::POST, path, headers, Body::from("{}")).await.status(), StatusCode::FORBIDDEN);
    }
    let download: Uri = format!("/secret/download/{}", query_value(&book.metadata.id)).parse().unwrap();
    let response = library_request(State(state.clone()), Method::GET, download.clone(), headers(), Body::empty()).await;
    state.cancellation.cancel();
    // Direct Body polling sees take_until cancellation as clean EOF, without
    // HTTP's Content-Length enforcement turning the truncation into an error.
    assert!(bytes(response).await.is_empty(), "Cancellation must deliver no archive bytes");
    for (method, path, body) in [
        (Method::POST, uri, Body::from("{}")),
        (Method::GET, download, Body::empty()),
        (Method::GET, "/secret/inventory".parse().unwrap(), Body::empty()),
    ] {
        assert_eq!(library_request(State(state.clone()), method, path, headers(), body).await.status(), StatusCode::FORBIDDEN);
    }
    state.hosts.shutdown().await.unwrap();
    state.cancellation.0.workers.drain().await.unwrap();
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn protected_browser_and_native_bootstrap_never_issue_before_pin_authentication() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-protected-share-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let mut state = state(store);
    state.session = Arc::new(SharingSession::new(Some("493781"), None).unwrap());
    let uri: Uri = format!("/nearby/{}", state.session.id).parse().unwrap();
    let response = library_request(State(state.clone()), Method::GET, uri.clone(), headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key("set-cookie"));
    assert!(!response.headers().contains_key("location"));
    let html = String::from_utf8(bytes(response).await).unwrap();
    assert!(html.contains("type=\"password\""));
    assert!(!html.contains(&state.token));
    assert!(!html.contains(&book.metadata.id));
    assert!(!html.contains("493781"));
    assert!(!html.contains("<script"));
    for body in [r#"{}"#, r#"{"pin":"000000"}"#] {
        let response = library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from(body)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!String::from_utf8(bytes(response).await).unwrap().contains(&state.token));
    }
    let response = library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from(r#"{"pin":"493781"}"#)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(value["token"], "secret");
    assert_eq!(value["name"], "Protected library");
    let mut form = headers();
    form.insert("content-type", "application/x-www-form-urlencoded".parse().unwrap());
    let response = library_request(State(state.clone()), Method::POST, uri.clone(), form.clone(), Body::from("pin=000000")).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()["content-type"], "text/html; charset=utf-8");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(!response.headers().contains_key("set-cookie"));
    assert!(!response.headers().contains_key("location"));
    let html = String::from_utf8(bytes(response).await).unwrap();
    assert!(html.contains("The PIN was not accepted. Try again."));
    assert!(html.contains("type=\"password\""));
    assert!(html.contains("role=\"alert\""));
    assert!(html.contains("aria-describedby=\"pin-error\""));
    assert!(!html.contains(&state.token));
    assert!(!html.contains("000000"));
    assert!(!html.contains("493781"));
    let response = library_request(State(state.clone()), Method::POST, uri, form, Body::from("pin=493781")).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/secret/");
    assert_eq!(response.headers()["cache-control"], "no-store");
    state.cancellation.cancel();
    state.hosts.shutdown().await.unwrap();
    state.cancellation.0.workers.drain().await.unwrap();
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn bootstrap_limits_pin_attempts_and_bodies_without_reflecting_credentials() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-pin-limits-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, _) = tests::library_store(&fixture(), &root);
    let mut state = state(store);
    state.session = Arc::new(SharingSession::new(Some("493781"), None).unwrap());
    let uri: Uri = format!("/nearby/{}", state.session.id).parse().unwrap();
    for _ in 0..PIN_ATTEMPTS {
        assert_eq!(library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from(r#"{"pin":"000000"}"#)).await.status(), StatusCode::FORBIDDEN);
    }
    let response = library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from(r#"{"pin":"493781"}"#)).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "30");
    *state.session.attempts.lock().unwrap() = PinAttempts { next: Instant::now() - PIN_WINDOW, remaining: 0, lockout: PIN_WINDOW };
    assert_eq!(library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from(r#"{"pin":"493781"}"#)).await.status(), StatusCode::OK);
    for body in [r#"{"pin":"493781","other":true}"#, r#"{"pin":"493781","pin":"493781"}"#, r#"{"pin":1234}"#] {
        let response = library_request(State(state.clone()), Method::POST, uri.clone(), headers(), Body::from(body)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = String::from_utf8(bytes(response).await).unwrap();
        assert!(!body.contains("493781"));
        assert!(!body.contains(&state.token));
    }
    *state.session.attempts.lock().unwrap() = PinAttempts::new();
    let body = Body::from_stream(stream::iter([Ok::<_, io::Error>(vec![b' '; BOOTSTRAP_BODY_BYTES + 1])]));
    assert_eq!(library_request(State(state.clone()), Method::POST, uri.clone(), headers(), body).await.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let mut oversized = headers();
    oversized.insert("content-length", "1000000".parse().unwrap());
    assert_eq!(library_request(State(state.clone()), Method::POST, uri.clone(), oversized, Body::from("{}")).await.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let mut cross_site = headers();
    cross_site.insert("sec-fetch-site", "cross-site".parse().unwrap());
    assert_eq!(library_request(State(state.clone()), Method::POST, uri, cross_site, Body::from(r#"{"pin":"493781"}"#)).await.status(), StatusCode::FORBIDDEN);
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn selected_scope_filters_bootstrap_catalog_download_reader_and_relinked_installations() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-selected-session-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let mut unshared = book.clone();
    unshared.metadata.id = Uuid::new_v4().to_string();
    unshared.metadata.title = "Never disclose this unselected title".into();
    unshared.installation_id = crate::store::new_installation_id();
    store.update(|state| { state.books.push(unshared.clone()); Ok(()) }).unwrap();
    let mut library = state(store.clone());
    library.session = Arc::new(SharingSession::new(None, Some(HashMap::from([(book.metadata.id.clone(), book.clone())]))).unwrap());
    let archive = Arc::new(Archive::open(&fixture()).unwrap());
    let selected = Index {
        token: library.token.clone(), authorities: library.authorities.clone(),
        entries: Arc::new(vec![IndexBook { id: book.metadata.id.clone(), title: book.metadata.title.clone(), port: 8081, archive: Arc::downgrade(&archive), fulltext: book.metadata.has_fulltext_index }]),
        store: store.clone(), cancellation: library.cancellation.clone(), library: library.clone(),
    };
    let bootstrap: Uri = format!("/nearby/{}", library.session.id).parse().unwrap();
    let response = index_request(State(selected.clone()), Method::POST, bootstrap.clone(), headers(), Body::from("{}")).await;
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(value["books"].as_array().unwrap().len(), 1);
    assert_eq!(value["books"][0]["id"], book.metadata.id);
    let response = index_request(State(selected.clone()), Method::GET, "/secret/catalog/v2/entries".parse().unwrap(), headers(), Body::empty()).await;
    let xml = String::from_utf8(bytes(response).await).unwrap();
    assert!(xml.contains(&book.metadata.id));
    assert!(!xml.contains(&unshared.metadata.id));
    assert!(!xml.contains(&unshared.metadata.title));
    for route in ["open", "download"] {
        let uri = format!("/secret/{route}/{}", query_value(&unshared.metadata.id)).parse().unwrap();
        assert_eq!(index_request(State(selected.clone()), Method::GET, uri, headers(), Body::empty()).await.status(), StatusCode::FORBIDDEN);
    }
    let uri = format!("/secret/search?q=a&book={}", unshared.metadata.id).parse().unwrap();
    assert_eq!(index_request(State(selected.clone()), Method::GET, uri, headers(), Body::empty()).await.status(), StatusCode::BAD_REQUEST);
    let response = index_request(State(selected.clone()), Method::GET, "/secret/".parse().unwrap(), headers(), Body::empty()).await;
    let html = String::from_utf8(bytes(response).await).unwrap();
    assert!(html.contains(&format!("/secret/download/{}", query_value(&book.metadata.id))));
    assert!(html.contains(">Download</a>"));
    assert!(!html.contains(&unshared.metadata.title));
    let response = index_request(State(selected.clone()), Method::GET, format!("/secret/download/{}", book.metadata.id).parse().unwrap(), headers(), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(bytes(response).await, std::fs::read(&book.path).unwrap());

    store.update(|state| { state.books[0].installation_id = crate::store::new_installation_id(); Ok(()) }).unwrap();
    let response = index_request(State(selected.clone()), Method::POST, bootstrap, headers(), Body::from("{}")).await;
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert!(value["books"].as_array().unwrap().is_empty());
    assert_eq!(index_request(State(selected.clone()), Method::GET, format!("/secret/download/{}", book.metadata.id).parse().unwrap(), headers(), Body::empty()).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(index_request(State(selected.clone()), Method::GET, format!("/secret/suggest?q=a&book={}", book.metadata.id).parse().unwrap(), headers(), Body::empty()).await.status(), StatusCode::BAD_REQUEST);
    library.cancellation.cancel();
    library.hosts.shutdown().await.unwrap();
    library.cancellation.0.workers.drain().await.unwrap();
    drop(selected);
    drop(library);
    drop(archive);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn selected_browser_activation_is_revoked_with_its_installation() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-selected-reader-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let archive = Arc::new(Archive::open(&fixture()).unwrap());
    let state = SharedBook {
        archive: BookArchive::Selected { archive: Arc::downgrade(&archive), owner: Arc::new(book.clone()), store: store.clone(), stamp: archive_stamp(&book).unwrap() },
        token: "secret".into(), authorities: AcceptedAuthorities::new(&[IpAddr::V4(Ipv4Addr::LOCALHOST)], &SharingBinding::Loopback {}, 8080),
        port: 8080, store: store.clone(), cancellation: SharingCancellation::new(), activity: None,
    };
    let uri: Uri = "/.kiwix-access/secret".parse().unwrap();
    assert_eq!(shared_request(State(state.clone()), Method::GET, uri.clone(), headers()).await.status(), StatusCode::OK);
    store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
    let response = shared_request(State(state.clone()), Method::GET, uri, headers()).await;
    assert_eq!(response.status(), StatusCode::GONE);
    assert!(!response.headers().contains_key("set-cookie"));
    state.cancellation.cancel();
    state.cancellation.0.workers.drain().await.unwrap();
    drop(state);
    drop(archive);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn session_defaults_and_pin_validation_do_not_widen_network_authority() {
    assert_eq!(SharingScope::default(), SharingScope::Library);
    assert_eq!(query_value("id with +/percent%"), "id%20with%20%2B%2Fpercent%25");
    let addresses = ["127.0.0.1", "169.254.1.2", "0.0.0.0", "8.8.8.8", "::1", "fe80::1", "192.168.7.9"];
    let addresses: Vec<IpAddr> = addresses.iter().map(|address| address.parse().unwrap()).collect();
    assert_eq!(auto_lan_binding(&addresses).unwrap(), SharingBinding::Interface { address: "192.168.7.9".into() });
    assert!(auto_lan_binding(&addresses[..6]).is_err());
    for pin in ["123", "12345", "1234567890123", "12ab", "１２３４", "1234\n"] {
        assert!(SharingSession::new(Some(pin), None).is_err());
    }
    assert!(!SharingSession::new(Some(""), None).unwrap().locked());
    let session = SharingSession::new(Some("000000"), None).unwrap();
    assert!(session.authenticates(Some("000000")));
    assert!(!session.authenticates(Some("000001")));
    assert!(!session.authenticates(None));
    assert_eq!(session.id.len(), 32);
    assert!(session.id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
}

#[test]
fn pin_lockout_escalates_and_a_success_restores_the_budget() {
    let mut attempts = PinAttempts::new();
    for _ in 0..PIN_ATTEMPTS {
        assert!(attempts.attempt());
    }
    // The spent budget locks the form instead of resetting on a fixed timer.
    assert!(!attempts.attempt());
    assert_eq!(attempts.retry_after(), PIN_WINDOW.as_secs());
    // After the lockout the next budget is smaller in rate: the wait doubles.
    attempts.next = Instant::now() - PIN_WINDOW;
    assert!(attempts.attempt());
    assert_eq!(attempts.lockout, PIN_WINDOW * 2);
    for _ in 1..PIN_ATTEMPTS {
        assert!(attempts.attempt());
    }
    assert!(!attempts.attempt());
    assert_eq!(attempts.retry_after(), (PIN_WINDOW * 2).as_secs());
    attempts.succeeded();
    assert_eq!(attempts.lockout, PIN_WINDOW);
    for _ in 0..PIN_ATTEMPTS {
        assert!(attempts.attempt());
    }
}

#[tokio::test]
async fn shutdown_joins_retired_book_listener_tasks() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("kiwix-retired-hosts-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let (store, book) = tests::library_store(&fixture(), &root);
    let state = state(store);
    let ports = state.hosts.ensure(std::slice::from_ref(&book), std::slice::from_ref(&book)).unwrap();
    assert!(ports[&book.metadata.id] != 0);
    assert_eq!(state.hosts.lock().len(), 1);
    state.hosts.ensure(&[], &[]).unwrap();
    assert!(state.hosts.lock().is_empty());
    assert_eq!(state.hosts.retired.lock().unwrap().len(), 1);
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(dropped.clone());
    let (started, ready) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let _guard = guard;
        let _ = started.send(());
        std::future::pending::<()>().await;
        Ok::<(), String>(())
    });
    ready.await.unwrap();
    handle.abort();
    state.hosts.retired.lock().unwrap().push(handle);
    state.cancellation.cancel();
    state.hosts.shutdown().await.unwrap();
    assert!(dropped.load(Ordering::Acquire));
    assert!(state.hosts.retired.lock().unwrap().is_empty());
    assert!(state.hosts.ensure(&[], &[]).is_err());
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}
