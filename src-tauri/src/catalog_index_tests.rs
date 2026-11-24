// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
use axum::{extract::State, http::{HeaderMap, StatusCode, Uri}, response::{IntoResponse, Response}, routing::get, Router};
use std::{path::PathBuf, sync::atomic::AtomicUsize};

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self { Self(std::env::temp_dir().join(format!("kiwix-catalog-index-{}", uuid::Uuid::new_v4()))) }
    fn file(&self) -> PathBuf { self.0.join("cache/catalog-index-v1.json") }
}
impl Drop for Profile { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

#[derive(Default)]
struct Network {
    mode: AtomicUsize,
    requests: Mutex<Vec<(String, Option<String>)>>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
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
    fn root(&self) -> String { format!("{}/feed?collection=all", self.base) }
    async fn stop(self) { self.task.abort(); let _ = self.task.await; }
}

fn entry(id: &str, title: &str) -> String {
    format!(r#"<entry><id>{id}</id><title>{title}</title><name>example_en</name><language>eng</language><date>2026-01-01</date><tags>_flavour:maxi;_pictures:yes</tags><link type="application/x-zim" href="https://example.org/{id}.zim"/></entry>"#)
}
fn feed(entries: &str, next: Option<&str>) -> String {
    format!(r#"<feed xmlns="http://www.w3.org/2005/Atom">{entries}{}</feed>"#,
        next.map(|next| format!(r#"<link rel="next" href="{next}"/>"#)).unwrap_or_default())
}
async fn serve(State(state): State<Arc<Network>>, uri: Uri, headers: HeaderMap) -> Response {
    let conditional = headers.get("if-none-match").and_then(|value| value.to_str().ok()).map(str::to_owned);
    state.requests.lock().unwrap().push((uri.to_string(), conditional.clone()));
    let mode = state.mode.load(Ordering::SeqCst);
    if mode == 1 { return (StatusCode::SERVICE_UNAVAILABLE, "offline fixture").into_response(); }
    if mode == 7 {
        state.entered.notify_one();
        state.release.notified().await;
    }
    let second = uri.path() == "/second";
    if mode == 9 && second { return (StatusCode::SERVICE_UNAVAILABLE, "missing downstream page").into_response(); }
    // Mode 2 moves A to page two and removes B; mode 3 removes next entirely.
    let (body, version) = match mode {
        2 if second => (feed(&entry("urn:uuid:a", "A"), None), 2),
        2 => (feed(&entry("c", "C"), Some("/second")), 2),
        3 => (feed(&entry("c", "C"), None), 3),
        4 if second => (feed(&entry("urn:uuid:a", "Conflicting A"), None), 4),
        4 => (feed(&entry("urn:uuid:a", "A"), Some("/second")), 4),
        5 if second => (feed(&entry("b", "B"), Some("/feed?collection=all")), 5),
        5 => (feed(&entry("urn:uuid:a", "A"), Some("/second")), 5),
        6 if second => (feed(&entry("urn:uuid:a", "A"), None), 6),
        6 => (feed(&entry("urn:uuid:a", "A"), Some("/second")), 6),
        8 => {
            let offset = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .find(|(key, _)| key == "page").and_then(|(_, value)| value.parse::<usize>().ok()).unwrap_or(0);
            (feed(&entry("urn:uuid:a", "A"), Some(&format!("/feed?page={}", offset + 1))), 8)
        },
        10 | 11 => {
            let offset = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .find(|(key, _)| key == "page").and_then(|(_, value)| value.parse::<usize>().ok()).unwrap_or(0);
            let next = (offset + 1 < MAX_PAGES).then(|| format!("/feed?page={}", offset + 1));
            if mode == 10 && next.is_none() {
                state.entered.notify_one();
                state.release.notified().await;
            }
            (feed(&entry(&format!("page-{offset}"), "Book"), next.as_deref()), 10)
        }
        _ if second => (feed(&entry("b", "B"), None), 1),
        _ => (feed(&entry("urn:uuid:a", "A"), Some("/second")), 1),
    };
    let etag = format!("\"page-{version}\"");
    if conditional.as_deref() == Some(etag.as_str()) {
        return (StatusCode::NOT_MODIFIED, [("etag", etag)], "").into_response();
    }
    (StatusCode::OK, [("etag", etag)], body).into_response()
}
fn policy() -> (Arc<RwLock<()>>, Arc<AtomicU64>) { (Arc::new(RwLock::new(())), Arc::new(AtomicU64::new(1))) }
fn cache(store: Arc<Store>) -> CatalogCache { CatalogCache::with_loopback(store) }

#[tokio::test(flavor = "current_thread")]
async fn current_chain_revalidates_all_pages_and_offline_is_never_traversed() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let index = CatalogIndex::new(store);
    let (gate, generation) = policy();
    let first = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(first.completeness, "traversed");
    assert_eq!(first.pages.len(), 2);
    assert_eq!(first.entries.len(), 2);
    assert!(first.stored && !first.stale);
    assert_eq!(first.entries.iter().find(|entry| entry.entry.title == "A").unwrap().entry.id, "urn:uuid:a");
    fixture.network.requests.lock().unwrap().clear();
    let second = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(second.completeness, "traversed");
    assert!(second.pages.iter().all(|page| page.status == "revalidated" && page.generation == second.generation));
    assert_eq!(first.pages[0].representation_digest, second.pages[0].representation_digest);
    assert_ne!(first.generation, second.generation);
    assert_eq!(fixture.network.requests.lock().unwrap().len(), 2, "a root 304 must not bless its descendants");
    assert!(index.lookup(&first.source_id, &first.entries[0].entry_id, first.generation).is_err());
    assert_eq!(index.lookup_current(&first.source_id, &first.entries[0].entry_id).unwrap(),
        (second.generation, first.entries[0].entry.clone()));
    let restarted = CatalogIndex::new(index.store.clone());
    assert!(restarted.lookup_current(&first.source_id, &first.entries[0].entry_id).unwrap_err().contains("HTTPS"),
        "production restart lookup must not inherit the fixture transport's loopback exception");
    fixture.network.mode.store(1, Ordering::SeqCst);
    let offline = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(offline.completeness, "partial");
    assert!(offline.stale && offline.error.is_some());
    assert_eq!(offline.entries.len(), 2);
    assert!(offline.pages.iter().all(|page| page.stale && page.status == "offline"));
    assert!(index.lookup(&offline.source_id, &offline.entries[0].entry_id, offline.generation).is_err());
    assert!(index.lookup_current(&offline.source_id, &offline.entries[0].entry_id).is_err());
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn maximum_traversal_publishes_raw_history_once_and_revalidates_every_page() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    fixture.network.mode.store(10, Ordering::SeqCst);
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let index = Arc::new(CatalogIndex::new(store));
    let (gate, generation) = policy();
    let root = fixture.root();
    let pending = {
        let (index, cache, gate, generation, root) = (index.clone(), cache.clone(), gate.clone(), generation.clone(), root.clone());
        tokio::spawn(async move { index.refresh(&cache, &root, "en", &gate, &generation, 1).await.unwrap() })
    };
    fixture.network.entered.notified().await;
    let raw = profile.0.join("cache/catalog-opds-v1.json");
    assert!(!raw.exists(), "the traversal must not publish an incomplete set of raw pages");
    fixture.network.release.notify_one();
    let first = pending.await.unwrap();
    assert!(first.stored && !first.stale);
    assert_eq!(first.pages.len(), MAX_PAGES);
    assert_eq!(first.completeness, "traversed");
    assert!(raw.is_file());

    fixture.network.mode.store(11, Ordering::SeqCst);
    let second = index.refresh(&cache, &root, "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(second.pages.len(), MAX_PAGES);
    assert!(second.pages.iter().all(|page| page.status == "revalidated"));
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn movement_replaces_membership_and_removed_next_drops_old_descendants() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let index = CatalogIndex::new(store.clone());
    let (gate, generation) = policy();
    let library_before = store.read().unwrap();
    let first = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    let a = first.entries.iter().find(|entry| entry.entry.title == "A").unwrap().entry_id.clone();
    let b = first.entries.iter().find(|entry| entry.entry.title == "B").unwrap().entry_id.clone();
    fixture.network.mode.store(2, Ordering::SeqCst);
    let moved = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(moved.completeness, "traversed");
    assert!(!moved.pages[0].entry_ids.contains(&a));
    assert!(moved.pages[1].entry_ids.contains(&a));
    assert!(moved.entries.iter().all(|entry| entry.entry_id != b));
    assert!(index.lookup(&moved.source_id, &b, moved.generation).is_err());
    fixture.network.mode.store(3, Ordering::SeqCst);
    fixture.network.requests.lock().unwrap().clear();
    let shortened = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(shortened.pages.len(), 1);
    assert_eq!(shortened.entries.len(), 1);
    assert!(shortened.entries.iter().all(|entry| entry.entry_id != a));
    assert_eq!(fixture.network.requests.lock().unwrap().len(), 1);
    assert_eq!(serde_json::to_value(store.read().unwrap()).unwrap(), serde_json::to_value(library_before).unwrap(), "catalog absence must never mutate installed state");
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn conflicting_duplicates_are_unselectable_identical_duplicates_dedupe_and_cycles_are_partial() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let index = CatalogIndex::new(store);
    let (gate, generation) = policy();
    fixture.network.mode.store(4, Ordering::SeqCst);
    let conflict = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(conflict.completeness, "partial");
    assert!(conflict.entries.is_empty());
    assert!(conflict.error.as_deref().unwrap().contains("conflicting"));
    assert!(index.lookup(&conflict.source_id, &conflict.pages[0].entry_ids[0], conflict.generation).is_err());
    fixture.network.mode.store(6, Ordering::SeqCst);
    let duplicate = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(duplicate.completeness, "traversed");
    assert_eq!(duplicate.entries.len(), 1);
    assert_eq!(duplicate.pages[0].entry_ids, duplicate.pages[1].entry_ids);
    fixture.network.mode.store(5, Ordering::SeqCst);
    fixture.network.requests.lock().unwrap().clear();
    let cycle = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(cycle.completeness, "partial");
    assert!(cycle.error.as_deref().unwrap().contains("cycle"));
    assert_eq!(fixture.network.requests.lock().unwrap().len(), 2);
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn source_scope_and_forged_ids_cannot_select_another_native_artifact() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    store.update(|state| {
        state.settings.catalog_url = fixture.root();
        state.settings.opds_sources = vec![format!("{}/other", fixture.base)];
        Ok(())
    }).unwrap();
    let cache = cache(store.clone());
    let index = CatalogIndex::new(store);
    let (gate, generation) = policy();
    let first = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    let second = index.refresh(&cache, &format!("{}/other", fixture.base), "en", &gate, &generation, 1).await.unwrap();
    let a = first.entries.iter().find(|entry| entry.entry.title == "A").unwrap();
    let other_a = second.entries.iter().find(|entry| entry.entry.title == "A").unwrap();
    assert_ne!(first.source_id, second.source_id);
    assert_ne!(a.entry_id, other_a.entry_id);
    assert!(index.lookup(&second.source_id, &a.entry_id, second.generation).is_err());
    assert!(index.lookup(&first.source_id, &"0".repeat(64), first.generation).is_err());
    assert_eq!(index.lookup(&first.source_id, &a.entry_id, first.generation).unwrap().url, a.entry.url);
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn all_configured_sources_remain_addressable_beyond_durable_cache_capacity() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let roots: Vec<_> = (0..33).map(|n| format!("{}/source-{n}", fixture.base)).collect();
    store.update(|state| {
        state.settings.catalog_url = roots[0].clone();
        state.settings.opds_sources = roots[1..].to_vec();
        Ok(())
    }).unwrap();
    let cache = cache(store.clone());
    let index = CatalogIndex::new(store);
    let (gate, generation) = policy();
    let mut snapshots = Vec::new();
    for root in &roots {
        snapshots.push(index.refresh(&cache, root, "en", &gate, &generation, 1).await.unwrap());
    }
    assert_eq!(load(&cache).unwrap().snapshots.len(), MAX_SOURCES);
    for snapshot in snapshots {
        for selected in snapshot.entries {
            assert_eq!(index.lookup(&snapshot.source_id, &selected.entry_id, snapshot.generation).unwrap(), selected.entry);
            assert_eq!(index.lookup_current(&snapshot.source_id, &selected.entry_id).unwrap().0, snapshot.generation);
        }
    }
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn private_readonly_and_retired_refresh_do_not_publish_history() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let index = Arc::new(CatalogIndex::new(store.clone()));
    let (gate, generation) = policy();
    let first = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    let before = std::fs::read(profile.file()).unwrap();
    store.set_runtime_mode(true, false).unwrap();
    let private = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert!(!private.stored);
    assert!(index.lookup(&first.source_id, &first.entries[0].entry_id, first.generation).is_err());
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    fixture.network.mode.store(1, Ordering::SeqCst);
    let private_offline = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert!(private_offline.entries.is_empty() && private_offline.stale);
    assert!(!private_offline.stored);
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    fixture.network.mode.store(0, Ordering::SeqCst);
    store.set_runtime_mode(false, true).unwrap();
    let readonly = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert!(!readonly.stored);
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    store.set_runtime_mode(false, false).unwrap();
    fixture.network.mode.store(7, Ordering::SeqCst);
    let task = {
        let (index, cache, root, gate, generation) = (index.clone(), cache.clone(), fixture.root(), gate.clone(), generation.clone());
        tokio::spawn(async move { index.refresh(&cache, &root, "en", &gate, &generation, 1).await })
    };
    fixture.network.entered.notified().await;
    {
        let _lease = gate.write().await;
        generation.fetch_add(1, Ordering::SeqCst);
        store.set_runtime_mode(true, false).unwrap();
    }
    fixture.network.release.notify_one();
    assert!(task.await.unwrap().unwrap_err().contains("privacy settings changed"));
    assert_eq!(std::fs::read(profile.file()).unwrap(), before);
    fixture.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn page_limit_and_missing_downstream_are_honest_partial_snapshots() {
    let profile = Profile::new();
    let fixture = Fixture::new().await;
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = cache(store.clone());
    let index = CatalogIndex::new(store);
    let (gate, generation) = policy();
    fixture.network.mode.store(9, Ordering::SeqCst);
    let interrupted = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(interrupted.completeness, "partial");
    assert!(interrupted.stale && interrupted.error.is_some());
    assert_eq!(interrupted.pages.len(), 1);
    assert_eq!(interrupted.entries.len(), 1);
    assert!(index.lookup(&interrupted.source_id, &interrupted.entries[0].entry_id, interrupted.generation).is_err());
    fixture.network.mode.store(8, Ordering::SeqCst);
    fixture.network.requests.lock().unwrap().clear();
    let bounded = index.refresh(&cache, &fixture.root(), "en", &gate, &generation, 1).await.unwrap();
    assert_eq!(bounded.completeness, "partial");
    assert_eq!(bounded.pages.len(), MAX_PAGES);
    assert_eq!(fixture.network.requests.lock().unwrap().len(), MAX_PAGES);
    assert!(bounded.error.as_deref().unwrap().contains("page limit"));
    fixture.stop().await;
}

#[test]
fn persisted_lookup_revalidates_source_scope_and_membership_after_restart() {
    let profile = Profile::new();
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let root = "https://catalog.example/feed?collection=all";
    let source = source_id(root).unwrap();
    let entry = super::super::parse_catalog(&feed(&entry("opaque", "Book"), None), &url::Url::parse(root).unwrap()).unwrap().entries.remove(0);
    let id = scoped_entry_id(&source, &entry.id);
    let snapshot = CatalogSnapshot {
        source_id: source.clone(), source_url: root.into(), locale: "en".into(), generation: 42,
        completeness: "traversed".into(), stale: false, stored: true, error: None,
        entries: vec![IndexedCatalogEntry { source_id: source.clone(), entry_id: id.clone(), entry: entry.clone() }],
        pages: vec![PageMembership { request_url: root.into(), response_url: root.into(), representation_digest: "a".repeat(64),
            next: None, entry_ids: vec![id.clone()], generation: 42, stale: false, status: "network".into() }],
    };
    let cache = CatalogCache::new(store.clone());
    store.with_catalog_cache_policy(|_, _| cache.write_index(&serde_json::to_vec(&SavedIndex { version: 1, snapshots: vec![snapshot.clone()] }).unwrap())).unwrap();
    let reopened = CatalogIndex::new(store.clone());
    assert_eq!(reopened.lookup(&source, &id, 42).unwrap(), entry);
    assert_eq!(reopened.lookup_current(&source, &id).unwrap(), (42, entry.clone()));
    assert!(reopened.lookup(&source, &id, 41).is_err());
    let mut forged = snapshot;
    forged.entries[0].source_id = source_id("https://other.example/feed").unwrap();
    store.with_catalog_cache_policy(|_, _| cache.write_index(&serde_json::to_vec(&SavedIndex { version: 1, snapshots: vec![forged] }).unwrap())).unwrap();
    let reopened = CatalogIndex::new(store);
    assert!(reopened.lookup(&source, &id, 42).is_err());
    assert!(reopened.lookup_current(&source, &id).is_err());
}

#[test]
fn official_transport_requires_native_root_and_every_containing_response_origin() {
    let profile = Profile::new();
    let store = Arc::new(Store::new(profile.0.clone()).unwrap());
    let cache = CatalogCache::new(store.clone());
    let root = "https://library.kiwix.org/catalog/v2/entries";
    let source = source_id(root).unwrap();
    let entry = super::super::parse_catalog(&feed(&entry("opaque", "Book"), None), &url::Url::parse(root).unwrap()).unwrap().entries.remove(0);
    let id = scoped_entry_id(&source, &entry.id);
    let mut snapshot = CatalogSnapshot {
        source_id: source.clone(), source_url: root.into(), locale: "en".into(), generation: 42,
        completeness: "traversed".into(), stale: false, stored: true, error: None,
        entries: vec![IndexedCatalogEntry { source_id: source.clone(), entry_id: id.clone(), entry }],
        pages: vec![PageMembership { request_url: root.into(), response_url: root.into(), representation_digest: "a".repeat(64),
            next: None, entry_ids: vec![id.clone()], generation: 42, stale: false, status: "network".into() }],
    };
    let save = |snapshot: &CatalogSnapshot| {
        store.with_catalog_cache_policy(|_, _| cache.write_index(&serde_json::to_vec(&SavedIndex {
            version: 1, snapshots: vec![snapshot.clone()],
        }).unwrap())).unwrap();
    };
    save(&snapshot);
    let index = CatalogIndex::new(store.clone());
    assert!(index.lookup_official(&source, &id, 42).unwrap());
    assert!(index.lookup_official(&source, &id, 41).is_err());
    assert!(index.lookup_official(&source, &"f".repeat(64), 42).is_err());
    snapshot.pages[0].response_url = "https://custom.example/feed".into();
    save(&snapshot);
    assert!(!index.lookup_official(&source, &id, 42).unwrap(), "official alias redirected to a custom feed must not confer authority");
    snapshot.pages[0].response_url = root.into();
    let second = format!("{root}?start=2");
    snapshot.pages[0].next = Some(second.clone());
    snapshot.pages.push(PageMembership { request_url: second, response_url: "https://custom.example/feed".into(),
        representation_digest: "b".repeat(64), next: None, entry_ids: vec![id.clone()],
        generation: 42, stale: false, status: "network".into() });
    save(&snapshot);
    assert!(!index.lookup_official(&source, &id, 42).unwrap(), "one official occurrence cannot bless a duplicate from a custom response");
    snapshot.pages.pop();
    snapshot.pages[0].next = None;
    snapshot.source_url = "https://custom.example/feed".into();
    snapshot.source_id = source_id(&snapshot.source_url).unwrap();
    let custom_id = scoped_entry_id(&snapshot.source_id, &snapshot.entries[0].entry.id);
    snapshot.entries[0].source_id = snapshot.source_id.clone();
    snapshot.entries[0].entry_id = custom_id.clone();
    snapshot.pages[0].request_url = snapshot.source_url.clone();
    snapshot.pages[0].entry_ids = vec![custom_id.clone()];
    save(&snapshot);
    assert!(!index.lookup_official(&snapshot.source_id, &custom_id, 42).unwrap(), "a custom configured source cannot inherit official source identity");
    store.set_runtime_mode(true, false).unwrap();
    assert!(index.lookup_official(&snapshot.source_id, &custom_id, 42).is_err());
}

#[test]
fn source_identity_and_series_dates_do_not_guess_lineage() {
    assert_eq!(source_id("https://EXAMPLE.org:443/feed?q=one").unwrap(), source_id("https://example.org/feed?q=one").unwrap());
    assert_ne!(source_id("https://example.org/feed?q=one").unwrap(), source_id("https://example.org/feed?q=two").unwrap());
    assert!(source_id("http://example.org/feed").is_err());
    let tags = vec!["_pictures:yes".into(), "_flavour:maxi".into(), "_ftindex:yes".into()];
    let maxi = series_key("wikipedia_en", "ENG", &tags).unwrap();
    assert_eq!(maxi, series_key("wikipedia_en", "eng", &tags.iter().cloned().rev().collect::<Vec<_>>()).unwrap());
    assert_ne!(Some(maxi), series_key("wikipedia_en", "eng", &["_flavour:mini".into(), "_pictures:yes".into(), "_ftindex:yes".into()]));
    assert!(series_key("", "eng", &tags).is_none());
    assert!(series_key("x", "eng", &["_flavour:maxi".into(), "_flavour:mini".into()]).is_none());
    assert!(series_key("x", "eng", &["_pictures:yes".into(), "_pictures:no".into()]).is_none());
    assert!(edition_date("2026-02-30").is_none());
    assert!(edition_date("latest").is_none());
    assert!(edition_date("2026-02").is_none(), "month-only dates must not invent day precision");
    assert!(edition_date("2026-02-01").unwrap() > edition_date("2026-01-31").unwrap());
    assert_eq!(edition_date("2026-02-01T23:00:00Z"), edition_date("2026-02-01"));
}

#[test]
fn edition_ordering_preserves_month_and_day_precision() {
    assert!(edition_is_newer("2026-02", "2026-01"));
    assert!(edition_is_newer("2026-02-01", "2026-01"));
    assert!(edition_is_newer("2026-02", "2026-01-31"));
    assert!(edition_is_newer("2026-02-02", "2026-02-01"));
    assert!(edition_is_newer("2024-03", "2024-02-29"));
    assert!(edition_is_newer("2027-01", "2026-12"));
    assert!(edition_is_newer("2026-02-02T00:01:00Z", "2026-02-01T23:59:00Z"));
    for (candidate, installed) in [
        ("2026-02", "2026-02"),
        ("2026-02-28", "2026-02"),
        ("2026-02", "2026-02-01"),
        ("2026-02-01", "2026-02-01"),
        ("2026-02-01T23:59:00Z", "2026-02-01T00:01:00Z"),
        ("2026-02-02T00:01:00+14:00", "2026-02-01T23:59:00-12:00"),
        ("2026-01", "2026-02"),
        ("2026-13", "2026-02"),
        ("2026-02-30", "2026-02"),
        ("latest", "2026-02"),
        ("2026-02", "unknown"),
        ("+026-02", "0025-02"),
    ] {
        assert!(!edition_is_newer(candidate, installed), "{candidate} is not provably newer than {installed}");
    }
}
