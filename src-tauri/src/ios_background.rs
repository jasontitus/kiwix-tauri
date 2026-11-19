// SPDX-License-Identifier: GPL-3.0-or-later
//! OS-owned transport for trusted official-catalog acquisitions only.
//!
//! Unlike the custom-URL Rust client, iOS background URLSession follows redirects
//! using the OS policy without app per-hop callbacks. Eligibility is granted by
//! the canonical acquisition owner, never by an IPC-supplied flag. Native files
//! are private transport receipts, not published archives; Rust still owns the
//! durable part, size/hash checks and no-clobber publication.
use crate::catalog::{official_artifact_url, official_catalog_url, CatalogEntry, CatalogPage};
use std::collections::{BTreeMap, VecDeque};

const OFFICIAL_ENTRY_LIMIT: usize = 2048;
const OFFICIAL_METADATA_LIMIT: usize = 8 * 1024 * 1024;

/// In-memory authority created only from a Rust-fetched official catalog page.
/// A user-supplied download URL or `official` flag cannot populate this ledger.
#[derive(Default)]
pub(crate) struct OfficialCatalogRegistry {
    entries: BTreeMap<String, (CatalogEntry, usize)>,
    order: VecDeque<String>,
    bytes: usize,
}

impl OfficialCatalogRegistry {
    pub(crate) fn record(&mut self, request_url: &str, source_url: &str, page: &CatalogPage) {
        if !official_catalog_url(request_url) || !official_catalog_url(source_url) { return; }
        for entry in &page.entries {
            if !official_artifact_url(&entry.url) { continue; }
            let bytes = entry_bytes(entry);
            if bytes > OFFICIAL_METADATA_LIMIT { continue; }
            if let Some((_, previous)) = self.entries.remove(&entry.url) {
                self.bytes -= previous;
                self.order.retain(|url| url != &entry.url);
            }
            while self.entries.len() >= OFFICIAL_ENTRY_LIMIT || self.bytes + bytes > OFFICIAL_METADATA_LIMIT {
                let Some(oldest) = self.order.pop_front() else { break; };
                if let Some((_, removed)) = self.entries.remove(&oldest) { self.bytes -= removed; }
            }
            self.bytes += bytes;
            self.order.push_back(entry.url.clone());
            self.entries.insert(entry.url.clone(), (entry.clone(), bytes));
        }
    }

    pub(crate) fn lookup(&self, url: &str) -> Option<CatalogEntry> {
        self.entries.get(url).map(|(entry, _)| entry.clone())
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear(); self.order.clear(); self.bytes = 0;
    }
}

fn entry_bytes(entry: &CatalogEntry) -> usize {
    std::mem::size_of::<CatalogEntry>() + entry.id.len() + entry.title.len() + entry.description.len()
        + entry.language.len() + entry.creator.len() + entry.publisher.len() + entry.date.len()
        + entry.name.len() + entry.url.len() * 3
        + entry.thumbnail_url.as_ref().map_or(0, String::len)
        + entry.torrent_url.as_ref().map_or(0, String::len)
        + entry.tags.iter().map(|tag| tag.len() + std::mem::size_of::<String>()).sum::<usize>()
}


#[cfg(any(target_os = "ios", all(test, unix)))]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "ios")]
use crate::apple_replies::Replies;
#[cfg(target_os = "ios")]
use objc2_foundation::NSData;
#[cfg(target_os = "ios")]
use serde_json::{json, Value};
#[cfg(target_os = "ios")]
use std::sync::LazyLock;

#[cfg(any(target_os = "ios", all(test, unix)))]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeRequest {
    pub id: String,
    pub generation: String,
    pub url: String,
    pub expected_size: Option<u64>,
    pub allows_cellular: bool,
}

#[cfg(any(target_os = "ios", all(test, unix)))]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSnapshot {
    pub state: String,
    pub staging_directory: String,
    pub copied: u64,
    pub total: Option<u64>,
    pub path: Option<String>,
    pub error: Option<String>,
    pub response_status: Option<u16>,
    pub source_url: Option<String>,
    pub final_url: Option<String>,
    #[serde(default)]
    // Host fixtures deserialize the native schema; iOS reconciliation reads this
    // retained start-time allowance before resuming an OS-owned transfer.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    pub allows_cellular: bool,
    pub device: Option<u64>,
    pub inode: Option<u64>,
}

#[cfg(target_os = "ios")]
static REPLIES: LazyLock<Replies<Result<Value, String>>> = LazyLock::new(Replies::default);

#[cfg(target_os = "ios")]
#[deny(improper_ctypes)]
unsafe extern "C" {
    // Swift snapshots the bytes before returning and borrows immutable reply
    // data only for the callback. The request ID carries no Rust ownership.
    safe fn kiwix_ios_background_call(
        id: u64,
        input: &NSData,
        callback: extern "C" fn(u64, Option<&NSData>),
    );
    safe fn kiwix_ios_background_install_delegate_handoff() -> i32;
}

/// Call on main after Tao constructs its runtime, before App::run invokes
/// UIApplicationMain. Adding the selector before delegate assignment avoids both
/// UIKit optional-selector caches and disturbing its non-retaining delegate.
#[cfg(target_os = "ios")]
pub(crate) fn install_delegate_handoff() -> Result<(), String> {
    if kiwix_ios_background_install_delegate_handoff() == 1 { Ok(()) }
    else { Err("Native application delegate class is unavailable before application run".into()) }
}

#[cfg(target_os = "ios")]
extern "C" fn receive(id: u64, response: Option<&NSData>) {
    REPLIES.finish(id, || {
        let response = response.ok_or("Native background transport returned no response")?;
        serde_json::from_slice::<Value>(&response.to_vec())
            .map_err(|error| format!("Reading native background response: {error}"))
            .and_then(|value| match value.get("failure").and_then(Value::as_str) {
                Some(error) => Err(error.to_owned()),
                None => Ok(value),
            })
    });
}

#[cfg(target_os = "ios")]
async fn call(command: &'static str, args: Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(&json!({"command": command, "args": args}))
        .map_err(|error| error.to_string())?;
    let reply = REPLIES.register().ok_or("Native background request IDs exhausted")?;
    // Swift still owns the submitted command if the Rust future is cancelled.
    // In particular, dropping a waiter cannot retract a native privacy barrier.
    kiwix_ios_background_call(reply.id(), &NSData::with_bytes(&bytes), receive);
    reply.wait().await.map_err(|_| "Native background transport did not acknowledge operation".to_string())?
}

#[cfg(target_os = "ios")]
pub(crate) async fn start(request: &NativeRequest) -> Result<(), String> {
    call("start", serde_json::to_value(request).map_err(|error| error.to_string())?).await.map(|_| ())
}

#[cfg(target_os = "ios")]
pub(crate) async fn snapshot(id: &str, generation: &str) -> Result<NativeSnapshot, String> {
    serde_json::from_value(call("snapshot", json!({"id":id,"generation":generation})).await?)
        .map_err(|error| format!("Reading native background snapshot: {error}"))
}

#[cfg(target_os = "ios")]
/// Pause waits for native task cancellation and durable opaque resume data.
/// Cancellation additionally removes only native-owned transport artifacts.
pub(crate) async fn stop(id: &str, generation: &str, cancel: bool) -> Result<(), String> {
    call("stop", json!({"id":id,"generation":generation,"cancel":cancel})).await.map(|_| ())
}

#[cfg(target_os = "ios")]
/// Acknowledge only after the canonical part and its ownership ledger are synced.
pub(crate) async fn acknowledge(id: &str, generation: &str) -> Result<(), String> {
    call("acknowledge", json!({"id":id,"generation":generation})).await.map(|_| ())
}

#[cfg(target_os = "ios")]
/// Revoke orphan tasks/artifacts without starting or resuming any task.
pub(crate) async fn reconcile(live: &[(String, String)]) -> Result<(), String> {
    let leases: Vec<Value> = live.iter().map(|(id, generation)| json!({"id":id,"generation":generation})).collect();
    call("reconcile", json!({"live":leases})).await.map(|_| ())
}

/// Global native denial barrier, available before canonical managers exist.
/// Enter denial before committing settings. The returned acknowledgement means
/// native OS tasks have retired and late delegates cannot stage app-owned files.
/// Leaving denial never starts or resumes transfers.
#[cfg(target_os = "ios")]
pub(crate) async fn set_policy(private_mode: bool, read_only: bool) -> Result<(), String> {
    call("policy", json!({"privateMode":private_mode,"readOnly":read_only})).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = "https://library.kiwix.org/catalog/v2/entries?count=1";
    const ACTUAL: &str = "https://opds.library.kiwix.org/catalog/v2/entries?count=1";
    const ARTIFACT: &str = "https://lb.download.kiwix.org/zim/videos/cest-pas-sorcier_fr_zapping-sauvage_2024-11.zim.meta4";

    fn entry(url: &str) -> CatalogEntry {
        CatalogEntry { id: "urn:uuid:001eb72b-e815-af2c-7225-f9ad60b5c891".into(), title: "Zapping Sauvage".into(),
            description: String::new(), language: "fra".into(), creator: String::new(), publisher: "openZIM".into(),
            date: String::new(), name: "cest-pas-sorcier_fr_zapping-sauvage".into(), tags: vec![], size: 374175744,
            article_count: 70, media_count: 142, url: url.into(), thumbnail_url: None, torrent_url: None, torrent_resolver_url: None }
    }

    fn page(url: &str) -> CatalogPage { CatalogPage { entries: vec![entry(url)], next: None, total: Some(1) } }

    #[test]
    fn current_official_redirect_and_metalink_are_authenticated_without_rewriting_url() {
        let mut registry = OfficialCatalogRegistry::default();
        registry.record(CATALOG, ACTUAL, &page(ARTIFACT));
        let trusted = registry.lookup(ARTIFACT).unwrap();
        assert_eq!(trusted.url, ARTIFACT);
        assert_eq!(trusted.size, 374175744);
        assert!(registry.lookup(ARTIFACT.trim_end_matches(".meta4")).is_none());
        assert!(registry.lookup(&format!("{ARTIFACT}?different=request")).is_none());
    }

    #[test]
    fn forged_catalog_or_actual_redirect_cannot_grant_native_authority() {
        for (request, actual) in [
            ("https://custom.example/catalog/v2/entries", ACTUAL),
            (CATALOG, "https://custom.example/catalog/v2/entries"),
            ("https://library.kiwix.org.evil.example/catalog/v2/entries", ACTUAL),
            (CATALOG, "https://opds.library.kiwix.org:444/catalog/v2/entries"),
            (CATALOG, "https://opds.library.kiwix.org/untrusted"),
            ("https://user@library.kiwix.org/catalog/v2/entries", ACTUAL),
        ] {
            let mut registry = OfficialCatalogRegistry::default();
            registry.record(request, actual, &page(ARTIFACT));
            assert!(registry.lookup(ARTIFACT).is_none(), "{request} / {actual}");
        }
    }

    #[test]
    fn custom_hosts_or_nonarchive_acquisitions_remain_foreground_only() {
        for artifact in [
            "https://example.org/zim/book.zim", "http://download.kiwix.org/zim/book.zim",
            "https://download.kiwix.org.evil.example/zim/book.zim", "https://download.kiwix.org:444/zim/book.zim",
            "https://download.kiwix.org/other/book.zim", "https://download.kiwix.org/zim/book.torrent",
            "https://user@download.kiwix.org/zim/book.zim", "https://download.kiwix.org/zim/book.zim#fragment",
        ] {
            let mut registry = OfficialCatalogRegistry::default();
            registry.record(CATALOG, ACTUAL, &page(artifact));
            assert!(registry.lookup(artifact).is_none(), "{artifact}");
        }
    }

    #[test]
    fn registry_bounds_replaces_and_clears_trusted_metadata() {
        let mut registry = OfficialCatalogRegistry::default();
        for index in 0..=OFFICIAL_ENTRY_LIMIT {
            registry.record(CATALOG, ACTUAL, &page(&format!("https://download.kiwix.org/zim/book-{index}.zim")));
        }
        assert_eq!(registry.entries.len(), OFFICIAL_ENTRY_LIMIT);
        assert!(registry.lookup("https://download.kiwix.org/zim/book-0.zim").is_none());
        let mut updated = page("https://download.kiwix.org/zim/book-1.zim");
        updated.entries[0].size = 123;
        registry.record(CATALOG, ACTUAL, &updated);
        assert_eq!(registry.lookup(&updated.entries[0].url).unwrap().size, 123);
        assert_eq!(registry.entries.len(), OFFICIAL_ENTRY_LIMIT);
        registry.clear();
        assert!(registry.entries.is_empty());
        assert!(registry.lookup(&updated.entries[0].url).is_none());
    }
}
