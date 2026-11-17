// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use axum::{extract::State, http::{HeaderMap, StatusCode, Uri}, response::{IntoResponse, Response}, routing::get, Router};
use std::{path::PathBuf, sync::{atomic::AtomicUsize, Mutex}};
use tokio::sync::Notify;

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self { Self(std::env::temp_dir().join(format!("kiwix-opds-cache-{}", uuid::Uuid::new_v4()))) }
    fn file(&self) -> PathBuf { self.0.join("cache").join(CACHE_FILE) }
}
impl Drop for Profile { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

#[derive(Default)]
struct Network {
    mode: AtomicUsize,
    requests: Mutex<Vec<(String, String, Option<String>)>>,
    started: Notify,
    release: Notify,
}
struct Fixture {
    base: String,
    network: Arc<Network>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let network = Arc::new(Network::default());
        let app = Router::new().route("/{*path}", get(serve)).with_state(network.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        Self { base, network, task }
    }
    fn url(&self, path: &str) -> String { format!("{}{path}", self.base) }
    async fn stop(self) { self.task.abort(); let _ = self.task.await; }
}

async fn serve(State(state): State<Arc<Network>>, uri: Uri, headers: HeaderMap) -> Response {
    let language = headers.get(ACCEPT_LANGUAGE).and_then(|h| h.to_str().ok()).unwrap_or("").to_owned();
    let conditional = headers.get(IF_NONE_MATCH).and_then(|h| h.to_str().ok()).map(str::to_owned);
    state.requests.lock().unwrap().push((uri.to_string(), language.clone(), conditional.clone()));
    let mode = state.mode.load(Ordering::SeqCst);
    if uri.path() == "/redirect" {
        let target = if mode == 7 { "/replacement/feed?filter=new&count=1" } else { "/nested/feed?filter=kept&count=1" };
        return (StatusCode::FOUND, [("location", target)], "").into_response();
    }
    if mode == 4 {
        state.started.notify_one();
        state.release.notified().await;
    }
    if mode == 1 { return (StatusCode::SERVICE_UNAVAILABLE, "offline fixture").into_response(); }
    let version = if mode == 8 { 2 } else { 1 };
    let etag = format!("{}\"{language}-v{version}\"", if mode == 5 { "W/" } else { "" });
    if mode == 2 || conditional.as_deref().is_some_and(|value| value.strip_prefix("W/").unwrap_or(value) == etag.strip_prefix("W/").unwrap_or(&etag)) {
        return (StatusCode::NOT_MODIFIED, [("etag", etag)], "").into_response();
    }
    let xml = if uri.path().ends_with("/languages") || uri.path().ends_with("/categories") {
        let (key, value) = if uri.path().ends_with("/languages") { ("lang", "fra") } else { ("category", "science") };
        let label = if language == "fr" { "Sciences françaises" } else { "English science" };
        format!(r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title xml:lang="{language}">{label}</title><link rel="subsection" href="https://example.org/feed?{key}={value}"/></entry></feed>"#)
    } else {
        let title = (if mode == 8 { "replacement" } else { uri.query().unwrap_or("unfiltered") }).replace('&', "&amp;");
        format!(r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:o="http://a9.com/-/spec/opensearch/1.1/"><o:totalResults>2</o:totalResults><o:startIndex>0</o:startIndex><o:itemsPerPage>1</o:itemsPerPage><entry><id>book</id><title>{language}:{title}</title><link type="application/x-zim" href="https://example.org/book.zim"/></entry></feed>"#)
    };
    (StatusCode::OK, [("etag", etag)], xml).into_response()
}

fn cache(store: Arc<Store>) -> CatalogCache {
    let mut cache = CatalogCache::new(store);
    cache.loopback_http = true;
    cache
}
fn policy() -> (Arc<RwLock<()>>, Arc<AtomicU64>) {
    (Arc::new(RwLock::new(())), Arc::new(AtomicU64::new(1)))
}
async fn mode(store: &Store, gate: &Arc<RwLock<()>>, generation: &AtomicU64, private: bool, read_only: bool) {
    let _guard = gate.write().await;
    generation.fetch_add(1, Ordering::SeqCst);
    store.set_runtime_mode(private, read_only).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn late_not_modified_does_not_replace_the_newer_offline_page() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let url = fixture.url("/feed");
    let (gate, generation) = policy();
    cache.fetch_page(&url, "en", &gate, &generation, 1).await.unwrap();
    fixture.network.mode.store(4, Ordering::SeqCst);
    let pending = {
        let (cache, url, gate, generation) = (cache.clone(), url.clone(), gate.clone(), generation.clone());
        tokio::spawn(async move { cache.fetch_page(&url, "en", &gate, &generation, 1).await.unwrap() })
    };
    fixture.network.started.notified().await;
    fixture.network.mode.store(8, Ordering::SeqCst);
    let fresh = cache.fetch_page(&url, "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(fresh.page.entries[0].title, "en:replacement");
    fixture.network.release.notify_one();
    let older = pending.await.unwrap();
    assert_eq!(older.cache.status, "revalidated");
    assert!(!older.cache.stored);
    fixture.network.mode.store(1, Ordering::SeqCst);
    let offline = cache.fetch_page(&url, "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(offline.page.entries[0].title, "en:replacement");
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn conditional_redirect_and_restart_offline_keep_final_base_and_filters() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let url = fixture.url("/redirect");
    let (gate, generation) = policy();
    {
        let store = Arc::new(Store::new(profile.0.clone()).unwrap());
        let cache = cache(store);
        let first = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
        assert_eq!(first.cache.status, "network");
        assert!(first.cache.stored && !first.cache.stale);
        assert_eq!(first.page.next.as_deref(), Some(fixture.url("/nested/feed?filter=kept&count=1&start=1").as_str()));
        assert_eq!(first.source_url(), fixture.url("/nested/feed?filter=kept&count=1"));
        let second = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
        assert_eq!(second.cache.status, "revalidated");
        assert!(!second.cache.stale);
        assert_eq!(second.source_url(), first.source_url());
        let requests = fixture.network.requests.lock().unwrap();
        assert!(requests.iter().any(|(url, language, tag)| url == "/nested/feed?filter=kept&count=1" && language == "en" && tag.as_deref() == Some("\"en-v1\"")));
        let records: Records = serde_json::from_slice(&std::fs::read(profile.file()).unwrap()).unwrap();
        assert_eq!(records.records[0].base, fixture.url("/nested/feed?filter=kept&count=1"));
    }
    let final_source = fixture.url("/nested/feed?filter=kept&count=1");
    fixture.stop().await;
    let reopened = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let offline = reopened.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(offline.cache.status, "offline");
    assert!(offline.cache.stale && offline.cache.stored && offline.cache.error.is_some());
    assert_eq!(offline.source_url(), final_source);
    assert_eq!(offline.page.entries[0].url, "https://example.org/book.zim");
    assert!(offline.page.next.unwrap().contains("filter=kept&count=1&start=1"));
}

#[tokio::test(flavor = "current_thread")]
async fn page_query_and_accept_language_and_localized_facets_are_isolated_offline() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    let one = fixture.url("/feed?q=one&start=0");
    let two = fixture.url("/feed?q=two&start=0");
    for (url, language) in [(&one, "en"), (&one, "fr"), (&two, "en")] {
        assert_eq!(cache.fetch_page(url, language, &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().cache.status, "network");
    }
    for language in ["en", "fr"] {
        let facets = cache.fetch_facets(&one, language, &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
        assert!(facets.cache.stored);
        let revalidated = cache.fetch_facets(&one, language, &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
        assert_eq!(revalidated.cache.status, "revalidated");
    }
    fixture.network.mode.store(1, Ordering::SeqCst);
    assert_eq!(cache.fetch_page(&one, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().page.entries[0].title, "en:q=one&start=0");
    assert_eq!(cache.fetch_page(&one, "fr", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().page.entries[0].title, "fr:q=one&start=0");
    assert_eq!(cache.fetch_page(&two, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().page.entries[0].title, "en:q=two&start=0");
    for (language, label) in [("en", "English science"), ("fr", "Sciences françaises")] {
        let facets = cache.fetch_facets(&one, language, &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
        assert!(facets.cache.stale && facets.cache.stored);
        assert_eq!(facets.facets.languages[0].label, label);
        assert_eq!(facets.facets.categories[0].label, label);
    }
    assert!(cache.fetch_page(&one, "de", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    assert!(cache.fetch_facets(&one, "de", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn private_never_reads_or_writes_normal_history_and_readonly_is_nonmutating() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let (gate, generation) = policy();
    let url = fixture.url("/feed?q=normal");
    cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    cache.fetch_facets(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    let before = std::fs::read(profile.file()).unwrap();
    mode(&store, &gate, &generation, true, false).await;
    fixture.network.requests.lock().unwrap().clear();
    let private = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(private.cache.status, "network");
    assert!(!private.cache.stored);
    let private_facets = cache.fetch_facets(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert!(!private_facets.cache.stored);
    assert!(fixture.network.requests.lock().unwrap().iter().all(|(_, _, tag)| tag.is_none()));
    fixture.network.mode.store(1, Ordering::SeqCst);
    assert!(cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    assert!(cache.fetch_facets(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    mode(&store, &gate, &generation, false, true).await;
    assert!(cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().cache.stale);
    assert!(cache.fetch_facets(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().cache.stale);
    fixture.network.mode.store(0, Ordering::SeqCst);
    let uncached = cache.fetch_page(&fixture.url("/feed?q=readonly-new"), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert!(!uncached.cache.stored);
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn readonly_empty_profile_does_not_create_cache_directory() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    store.set_runtime_mode(false, true).unwrap();
    let cache = cache(store);
    let (gate, generation) = policy();
    assert!(!cache.fetch_page(&fixture.url("/feed"), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().cache.stored);
    assert!(!cache.fetch_facets(&fixture.url("/feed"), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap().cache.stored);
    assert!(!profile.0.join("cache").exists());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn privacy_roundtrip_during_http_retires_response_before_disk_or_publication() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    fixture.network.mode.store(4, Ordering::SeqCst);
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let (gate, generation) = policy();
    let task = {
        let gate = gate.clone();
        let generation = generation.clone();
        let url = fixture.url("/feed?q=retired");
        tokio::spawn(async move { cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await })
    };
    fixture.network.started.notified().await;
    mode(&store, &gate, &generation, true, false).await;
    mode(&store, &gate, &generation, false, false).await;
    fixture.network.release.notify_one();
    assert!(task.await.unwrap().unwrap_err().contains("privacy settings changed"));
    assert!(!profile.0.join("cache").exists());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn corrupt_cache_and_unsolicited_304_never_become_success_or_trusted_links() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    let url = fixture.url("/feed");
    fixture.network.mode.store(2, Ordering::SeqCst);
    assert!(cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap_err().contains("catalog did not refresh"));
    fixture.network.mode.store(0, Ordering::SeqCst);
    cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    let good = std::fs::read(profile.file()).unwrap();
    let mut records: Records = serde_json::from_slice(&good).unwrap();
    records.records[0].base = "https://unexpected.example/changed/".into();
    std::fs::write(profile.file(), serde_json::to_vec(&records).unwrap()).unwrap();
    fixture.network.mode.store(1, Ordering::SeqCst);
    assert!(cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    // Even an internally checksummed but forged cache must pass the live URL parser.
    records.records[0].xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>unsafe</id><title>Unsafe</title><link type="application/x-zim" href="file:///private/book.zim"/></entry></feed>"#.into();
    records.records[0].digest = record_digest(&records.records[0]);
    std::fs::write(profile.file(), serde_json::to_vec(&records).unwrap()).unwrap();
    fixture.network.requests.lock().unwrap().clear();
    assert!(cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    assert!(fixture.network.requests.lock().unwrap().iter().all(|(_, _, tag)| tag.is_none()));
    // A real successful refresh repairs unusable cache, without swallowing warning.
    fixture.network.mode.store(0, Ordering::SeqCst);
    let repaired = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(repaired.cache.status, "network");
    assert!(repaired.cache.stored && repaired.cache.error.is_some());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn finite_entry_eviction_does_not_mix_distinct_queries() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    for i in 0..=MAX_RECORDS {
        cache.fetch_page(&fixture.url(&format!("/feed?q={i}")), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    }
    let records: Records = serde_json::from_slice(&std::fs::read(profile.file()).unwrap()).unwrap();
    assert_eq!(records.records.len(), MAX_RECORDS);
    assert!(std::fs::metadata(profile.file()).unwrap().len() <= MAX_CACHE as u64);
    fixture.network.mode.store(1, Ordering::SeqCst);
    assert!(cache.fetch_page(&fixture.url("/feed?q=0"), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.is_err());
    let newest = cache.fetch_page(&fixture.url(&format!("/feed?q={MAX_RECORDS}")), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(newest.page.entries[0].title, format!("en:q={MAX_RECORDS}"));
    fixture.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn symlink_cache_never_reads_or_overwrites_external_file() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    std::fs::create_dir(profile.0.join("cache")).unwrap();
    let external = profile.0.join("external.json");
    std::fs::write(&external, b"external sentinel").unwrap();
    std::os::unix::fs::symlink(&external, profile.file()).unwrap();
    let result = cache.fetch_page(&fixture.url("/feed"), "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(result.cache.status, "network");
    assert_eq!(std::fs::read(&external).unwrap(), b"external sentinel");
    // Atomic rename may replace the cache symlink itself, never its target.
    assert!(result.cache.error.is_some());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_cache_load_keeps_policy_lease_until_blocking_work_retires() {
    let profile = Profile::new();
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let (gate, generation) = policy();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || store.with_catalog_cache_policy(|_, _| {
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        Ok(())
    }).unwrap());
    entered_rx.recv().unwrap();
    let task = {
        let gate = gate.clone();
        tokio::spawn(async move {
            cache.fetch_page("https://example.org/feed", "en", &gate, &generation, generation.load(Ordering::SeqCst)).await
        })
    };
    // On this current-thread executor the fetch runs until its first blocking
    // task await; that task owns the read lease while waiting on the Store lock.
    tokio::task::yield_now().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(gate.try_write().is_err(), "cancellation must not detach unleased cache work");
    release_tx.send(()).unwrap();
    let _writer = tokio::time::timeout(Duration::from_secs(5), gate.write()).await.unwrap();
    blocker.join().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn changed_redirect_target_receives_no_other_resources_validator() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    let url = fixture.url("/redirect");
    cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    fixture.network.requests.lock().unwrap().clear();
    fixture.network.mode.store(7, Ordering::SeqCst);
    let changed = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(changed.cache.status, "network");
    assert!(!changed.cache.stale && changed.cache.stored);
    assert_eq!(changed.page.entries[0].title, "en:filter=new&count=1");
    assert!(fixture.network.requests.lock().unwrap().iter().all(|(_, _, tag)| tag.is_none()));
    let same = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(same.cache.status, "revalidated");
    {
        let requests = fixture.network.requests.lock().unwrap();
        assert!(requests.iter().any(|(url, _, tag)| url.starts_with("/replacement/feed") && tag.as_deref() == Some("\"en-v1\"")));
        assert!(requests.iter().filter(|(url, _, _)| url == "/redirect").all(|(_, _, tag)| tag.is_none()));
    }
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn if_none_match_uses_weak_comparison_in_both_directions() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    fixture.network.mode.store(5, Ordering::SeqCst);
    let weak = fixture.url("/feed?q=weak");
    cache.fetch_page(&weak, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    fixture.network.mode.store(0, Ordering::SeqCst);
    let strong_header = cache.fetch_page(&weak, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(strong_header.cache.status, "revalidated");
    assert!(!strong_header.cache.stale && strong_header.cache.error.is_none());
    let strong = fixture.url("/feed?q=strong");
    cache.fetch_page(&strong, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    fixture.network.mode.store(5, Ordering::SeqCst);
    let weak_header = cache.fetch_page(&strong, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(weak_header.cache.status, "revalidated");
    assert!(!weak_header.cache.stale && weak_header.cache.error.is_none());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn failed_revalidation_rewrite_preserves_actual_existing_durability() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    let url = fixture.url("/feed");
    cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    let before = std::fs::read(profile.file()).unwrap();
    std::fs::create_dir(profile.0.join("cache").join(PENDING_FILE)).unwrap();
    let revalidated = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(revalidated.cache.status, "revalidated");
    assert!(revalidated.cache.stored && !revalidated.cache.stale);
    assert!(revalidated.cache.error.as_deref().is_some_and(|error| error.contains("Persisting")));
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    fixture.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn cache_history_permissions_are_private_on_creation_and_pending_reuse() {
    use std::os::unix::fs::PermissionsExt;
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let cache = cache(Arc::new(Store::new(profile.0.clone()).unwrap()));
    let (gate, generation) = policy();
    let url = fixture.url("/feed?q=private-history");
    cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(std::fs::metadata(profile.file()).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(profile.0.join("cache")).unwrap().permissions().mode() & 0o777, 0o700);
    let pending = profile.0.join("cache").join(PENDING_FILE);
    std::fs::set_permissions(profile.0.join("cache"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&pending, b"interrupted old scratch").unwrap();
    std::fs::set_permissions(&pending, std::fs::Permissions::from_mode(0o644)).unwrap();
    let reused = cache.fetch_page(&url, "en", &gate, &generation, generation.load(Ordering::SeqCst)).await.unwrap();
    assert_eq!(reused.cache.status, "revalidated");
    assert!(reused.cache.stored);
    assert_eq!(std::fs::metadata(profile.file()).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(profile.0.join("cache")).unwrap().permissions().mode() & 0o777, 0o700);
    fixture.stop().await;
}

// Unix lets another actor rename a directory this process holds open, so the
// attachment checks must catch a detached capability. Windows refuses that
// rename while the capability is open (cap-std directory handles never share
// delete), so the detached state cannot arise; see the Windows test below.
#[cfg(unix)]
#[test]
fn cache_attachment_replacement_prevents_mutation_and_detached_publication() {
    let profile = Profile::new();
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let directory = store.with_catalog_cache_policy(|_, _| cache.directory(true)).unwrap().unwrap();
    let detached = profile.0.join("detached-cache");
    std::fs::rename(profile.0.join("cache"), &detached).unwrap();
    std::fs::create_dir(profile.0.join("cache")).unwrap();
    assert!(atomic_write(&directory, b"must not write").is_err());
    assert!(!detached.join(PENDING_FILE).exists());
    assert!(!profile.file().exists());
    let attached = store.with_catalog_cache_policy(|_, _| cache.directory(true)).unwrap().unwrap();
    let late_detached = profile.0.join("late-detached-cache");
    let error = atomic_write_with_hook(&attached, b"fully written but unpublished", || {
        std::fs::rename(profile.0.join("cache"), &late_detached).unwrap();
        std::fs::create_dir(profile.0.join("cache")).unwrap();
        std::fs::write(profile.file(), b"replacement sentinel").unwrap();
    }).unwrap_err();
    assert!(error.contains("attachment"));
    assert!(!late_detached.join(CACHE_FILE).exists());
    assert_eq!(std::fs::read(profile.file()).unwrap(), b"replacement sentinel");
}

#[cfg(windows)]
#[test]
fn held_cache_directory_cannot_be_detached_before_or_during_publication() {
    let profile = Profile::new();
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let directory = store.with_catalog_cache_policy(|_, _| cache.directory(true)).unwrap().unwrap();
    let error = std::fs::rename(profile.0.join("cache"), profile.0.join("detached-cache")).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32), "{error}");
    atomic_write_with_hook(&directory, b"published in the attached directory", || {
        let error = std::fs::rename(profile.0.join("cache"), profile.0.join("late-detached-cache")).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32), "{error}");
    })
    .unwrap();
    assert_eq!(std::fs::read(profile.file()).unwrap(), b"published in the attached directory");
    assert!(!profile.0.join("detached-cache").exists());
    assert!(!profile.0.join("late-detached-cache").exists());
}

#[tokio::test(flavor = "current_thread")]
async fn caller_generation_retired_before_admission_causes_no_http_or_cache_history() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let (gate, generation) = policy();
    let caller_generation = generation.load(Ordering::SeqCst);
    // The request was queued before this complete privacy roundtrip but has not
    // entered the cache yet. It cannot adopt the replacement normal generation.
    mode(&store, &gate, &generation, true, false).await;
    mode(&store, &gate, &generation, false, false).await;
    let url = fixture.url("/feed?q=old-session-history");
    let page = cache.fetch_page(&url, "en", &gate, &generation, caller_generation).await.unwrap_err();
    let facets = cache.fetch_facets(&url, "en", &gate, &generation, caller_generation).await.unwrap_err();
    assert!(page.contains("privacy settings changed") && facets.contains("privacy settings changed"));
    assert!(fixture.network.requests.lock().unwrap().is_empty());
    assert!(!profile.0.join("cache").exists());
    fixture.stop().await;
}
