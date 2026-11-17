// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn state() -> OriginState {
    let directory = std::env::temp_dir().join(format!("kiwix-pdf-test-{}", Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let resource = directory.join("resource");
    fs::write(&resource, b"<p>Original archive article</p>").unwrap();
    OriginState {
        host: "127.0.0.1:4567".into(), cookie: "kiwix_pdf_test".into(), secret: "secret".into(), bootstrap: "/bootstrap-secret".into(), root: "/A/article".into(),
        resources: Arc::new(HashMap::from([("A/article".into(), ("text/html".into(), resource, b"<p>Original archive article</p>".len() as u64))])),
        streams: Arc::new(tokio::sync::Semaphore::new(4)),
        files: Files::new(directory, Arc::new(|error| panic!("{error}")), None),
        started: Arc::new(AtomicBool::new(false)), failure: Arc::new(ResourceFailure::default()), cancel: CancellationToken::new(),
    }
}
fn headers(host: &str, authenticated: bool) -> HeaderMap {
    let mut headers = HeaderMap::new(); headers.insert("host", host.parse().unwrap());
    if authenticated { headers.insert("x-kiwix-pdf", "secret".parse().unwrap()); }
    headers
}
#[tokio::test]
async fn pdf_origin_decodes_equivalent_paths_once_before_snapshot_lookup() {
    let mut state = state();
    let resource = state.resources["A/article"].clone();
    state.resources = Arc::new(HashMap::from([
        ("index.html".into(), resource.clone()),
        ("A/?#% café.html".into(), resource.clone()),
        ("/leading.html".into(), resource.clone()),
        ("literal%2Ehtml".into(), resource),
    ]));
    for uri in ["/index.html", "/index%2Ehtml", "/A/%3F%23%25%20caf%C3%A9.html", "/%2Fleading.html", "/literal%252Ehtml"] {
        let reply = serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", true), uri.parse().unwrap()).await;
        assert_eq!(reply.status(), StatusCode::OK, "{uri}");
        assert_eq!(axum::body::to_bytes(reply.into_body(), 1024).await.unwrap(), &b"<p>Original archive article</p>"[..]);
    }
    assert!(!state.failure.failed.load(Ordering::SeqCst));
    let reply = serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", true), "/literal%2Ehtml".parse().unwrap()).await;
    assert_eq!(reply.status(), StatusCode::NOT_FOUND);
    assert!(state.failure.failed.load(Ordering::SeqCst));
}
#[tokio::test]
async fn pdf_origin_requires_exact_port_and_independent_resource_authority() {
    let state = state();
    for (host, authenticated) in [("127.0.0.1:4568", true), ("127.0.0.1:4567", false)] {
        let reply = serve(State(state.clone()), Method::GET, headers(host, authenticated), "/A/article".parse().unwrap()).await;
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
    }
    let reply = serve(State(state), Method::GET, headers("127.0.0.1:4567", true), "/A/article".parse().unwrap()).await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(reply.headers()["cache-control"], "no-store, max-age=0");
    let policy = reply.headers()["content-security-policy"].to_str().unwrap();
    assert!(policy.contains("script-src 'none'"));
    assert!(policy.contains("style-src 'unsafe-inline' data:"));
    assert!(policy.contains("img-src data:"));
    assert_eq!(axum::body::to_bytes(reply.into_body(), 1024).await.unwrap(), &b"<p>Original archive article</p>"[..]);
}
#[tokio::test]
async fn pdf_bootstrap_is_single_use_and_missing_static_assets_fail_closed() {
    let state = state();
    let first = serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", false), "/bootstrap-secret".parse().unwrap()).await;
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    assert_eq!(first.headers()["location"], "/A/article");
    assert!(first.headers()["set-cookie"].to_str().unwrap().contains("HttpOnly; SameSite=Strict"));
    let second = serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", false), "/bootstrap-secret".parse().unwrap()).await;
    assert_eq!(second.status(), StatusCode::FORBIDDEN);
    let missing = serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", true), "/I/missing.png".parse().unwrap()).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(state.failure.failed.load(Ordering::SeqCst));
    assert!(state.failure.message().contains("/I/missing.png"));
}
#[tokio::test]
async fn pdf_resource_concurrency_queues_and_retires_without_false_missing_asset_failure() {
    let state = state();
    let occupied = state.streams.clone().acquire_many_owned(4).await.unwrap();
    let mut waiting = Box::pin(serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", true), "/A/article".parse().unwrap()));
    assert!(futures_util::poll!(&mut waiting).is_pending());
    assert!(!state.failure.failed.load(Ordering::SeqCst));
    drop(occupied);
    let reply = waiting.await;
    assert_eq!(reply.status(), StatusCode::OK);
    axum::body::to_bytes(reply.into_body(), 1024).await.unwrap();
    let _occupied = state.streams.clone().acquire_many_owned(4).await.unwrap();
    let mut waiting = Box::pin(serve(State(state.clone()), Method::GET, headers("127.0.0.1:4567", true), "/A/article".parse().unwrap()));
    assert!(futures_util::poll!(&mut waiting).is_pending());
    state.cancel.cancel();
    assert_eq!(waiting.await.status(), StatusCode::GONE);
    assert!(!state.failure.failed.load(Ordering::SeqCst));
}

#[test]
fn pdf_first_resource_failure_is_bounded_one_shot_and_redacts_bootstrap_capability() {
    let failure = ResourceFailure::default();
    failure.record("missing archive resource", "/.kiwix-pdf-id-secret");
    let first = failure.message();
    assert!(first.contains("<article bootstrap>"));
    assert!(!first.contains("id-secret"));
    failure.record("missing archive resource", "/I/second.png");
    assert_eq!(failure.message(), first);
    let bounded = ResourceFailure::default();
    bounded.record("missing archive resource", &format!("/I/{}", "x".repeat(10000)));
    assert!(bounded.message().len() < 400);
}

#[test]
fn pdf_precise_resource_failure_precedes_native_error_without_masking_unrelated_errors() {
    let failure = ResourceFailure::default();
    assert_eq!(failure.prefer_error("Browsing policy changed".into()), "Browsing policy changed");
    failure.record("missing archive resource", "/I/needed.css");
    let message = failure.prefer_error("An article resource could not be exported".into());
    assert!(message.contains("/I/needed.css"));
    assert!(message.contains("missing archive resource"));
}
