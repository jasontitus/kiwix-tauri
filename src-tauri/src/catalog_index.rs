// SPDX-License-Identifier: GPL-3.0-or-later
//! Current source membership, not publisher tombstones or installed-library authority.
//! Every traversal starts at the configured root and revalidates every current next
//! page. Offset pagination is never claimed to be an atomic publisher snapshot.
use super::cache::Stamp;
use super::{accept_language, checked_url, CatalogCache, CatalogEntry};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex};
use tokio::sync::RwLock;

const MAX_PAGES: usize = 64;
const MAX_ENTRIES: usize = 20_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_SOURCES: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexedCatalogEntry {
    pub source_id: String,
    pub entry_id: String,
    pub entry: CatalogEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageMembership {
    pub request_url: String,
    pub response_url: String,
    pub representation_digest: String,
    pub next: Option<String>,
    pub entry_ids: Vec<String>,
    pub generation: u64,
    pub stale: bool,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogSnapshot {
    pub source_id: String,
    pub source_url: String,
    pub locale: String,
    /// Nonzero, JavaScript-safe identity; never an ordering or authorization token.
    pub generation: u64,
    /// partial or traversed. Neither implies authenticated publisher deletions.
    pub completeness: String,
    pub stale: bool,
    pub stored: bool,
    pub pages: Vec<PageMembership>,
    pub entries: Vec<IndexedCatalogEntry>,
    pub error: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedIndex {
    version: u32,
    snapshots: Vec<CatalogSnapshot>,
}

pub struct CatalogIndex {
    store: Arc<Store>,
    refresh_gate: Arc<tokio::sync::Mutex<()>>,
    // Only normal-mode publications enter memory. Access is inside Store policy.
    current: Arc<Mutex<BTreeMap<String, CatalogSnapshot>>>,
    /// The durable index parsed for the identity in `Stamp`, so a restarted
    /// process reads and parses the index once instead of once per selection.
    /// Never consulted in private mode, and never across a changed index file.
    durable: DurableIndex,
}

type DurableIndex = Arc<Mutex<Option<(Stamp, Arc<SavedIndex>)>>>;

fn digest(fields: &[&str]) -> String {
    let mut hash = Sha256::new();
    for field in fields {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    hex::encode(hash.finalize())
}

/// The configured URL, not a redirect target, defines source identity. Queries
/// remain meaningful; credentials/fragments and insecure schemes are rejected.
pub fn source_id(source_root: &str) -> Result<String, String> {
    let url = checked_url(source_root, false)?;
    Ok(digest(&["catalog-source-v1", url.as_str()]))
}

fn scoped_entry_id(source: &str, atom_id: &str) -> String {
    digest(&["catalog-entry-v1", source, atom_id])
}

fn check_generation(generation: &AtomicU64, expected: u64) -> Result<(), String> {
    if generation.load(Ordering::SeqCst) != expected {
        return Err("Your privacy settings changed, so the catalog did not update. Open the catalog again".into());
    }
    Ok(())
}

impl CatalogIndex {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store, refresh_gate: Arc::new(tokio::sync::Mutex::new(())), current: Arc::new(Mutex::new(BTreeMap::new())),
            durable: Arc::new(Mutex::new(None)) }
    }

    pub async fn refresh(
        &self, cache: &CatalogCache, source_root: &str, locale: &str,
        gate: &Arc<RwLock<()>>, generation: &AtomicU64, expected: u64,
    ) -> Result<CatalogSnapshot, String> {
        let refresh = self.refresh_gate.clone().lock_owned().await;
        {
            let _lease = gate.read().await;
            check_generation(generation, expected)?;
        }
        let root = checked_url(source_root, cache.loopback())?.to_string();
        // Test-only loopback feeds use the exact same digest domain as HTTPS.
        let source = digest(&["catalog-source-v1", &root]);
        let revision = (uuid::Uuid::new_v4().as_u128() as u64 & ((1u64 << 53) - 1)).max(1);
        let mut snapshot = CatalogSnapshot {
            source_id: source.clone(), source_url: root.clone(), locale: accept_language(locale),
            generation: revision, completeness: "partial".into(), stale: false, stored: false,
            pages: Vec::new(), entries: Vec::new(), error: None,
        };
        let mut next = Some(root);
        let mut visited = HashSet::new();
        let mut entries = BTreeMap::<String, CatalogEntry>::new();
        let mut conflicts = HashSet::new();
        let mut bytes = 0usize;
        let mut traversed = false;
        while let Some(request) = next.take() {
            if snapshot.pages.len() >= MAX_PAGES {
                append_error(&mut snapshot, "Catalog traversal reached its page limit");
                break;
            }
            if !visited.insert(request.clone()) {
                append_error(&mut snapshot, "Catalog pagination contains a cycle");
                break;
            }
            let result = cache.fetch_page(&request, locale, gate, generation, expected).await;
            // A retired operation must not publish even a partial result.
            check_generation(generation, expected)?;
            let page = match result {
                Ok(page) => page,
                Err(error) => { snapshot.stale = true; append_error(&mut snapshot, &error); break; }
            };
            let response_url = page.source_url().to_owned();
            bytes = bytes.saturating_add(page.representation_bytes());
            if bytes > MAX_BYTES || entries.len().saturating_add(conflicts.len()).saturating_add(page.page.entries.len()) > MAX_ENTRIES {
                append_error(&mut snapshot, "Catalog traversal reached its metadata limit");
                break;
            }
            snapshot.stale |= page.cache.stale;
            if let Some(error) = &page.cache.error { append_error(&mut snapshot, error); }
            let representation_digest = page.representation_digest().to_owned();
            let mut membership = Vec::new();
            let mut page_seen = HashSet::new();
            for mut entry in page.page.entries {
                // Tag order is not identity and should not create false conflicts.
                entry.tags.sort();
                let id = scoped_entry_id(&source, &entry.id);
                if page_seen.insert(id.clone()) { membership.push(id.clone()); }
                if conflicts.contains(&id) { continue; }
                if let Some(previous) = entries.get(&id) {
                    if previous != &entry {
                        entries.remove(&id);
                        conflicts.insert(id);
                        append_error(&mut snapshot, "Catalog contains conflicting representations of an entry ID");
                    }
                } else { entries.insert(id, entry); }
            }
            next = page.page.next;
            snapshot.pages.push(PageMembership {
                request_url: request, response_url, representation_digest, next: next.clone(),
                entry_ids: membership, generation: revision, stale: page.cache.stale, status: page.cache.status,
            });
            if next.is_none() { traversed = true; }
        }
        snapshot.entries = entries.into_iter().map(|(entry_id, entry)| IndexedCatalogEntry {
            source_id: source.clone(), entry_id, entry,
        }).collect();
        if traversed && !snapshot.stale && conflicts.is_empty() { snapshot.completeness = "traversed".into(); }
        let lease = gate.clone().read_owned().await;
        check_generation(generation, expected)?;
        let store = self.store.clone();
        let cache = cache.clone();
        let current = self.current.clone();
        // Detached blocking I/O retains the policy lease even if its caller is
        // cancelled. No normal history write can outlive a privacy transition.
        let snapshot = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let _refresh = refresh;
            let settings = store.read()?.settings;
            let configured: HashSet<String> = std::iter::once(&settings.catalog_url)
                .chain(settings.opds_sources.iter())
                .filter_map(|root| checked_url(root, cache.loopback()).ok())
                .map(|root| digest(&["catalog-source-v1", root.as_str()]))
                .collect();
            store.with_catalog_cache_policy(|private, read_only| {
                if private { return Ok(()); }
                if !read_only {
                    let mut saved = match load(&cache) {
                        Ok(saved) => saved,
                        Err(error) => { append_error(&mut snapshot, &error); SavedIndex { version: 1, snapshots: Vec::new() } }
                    };
                    saved.snapshots.retain(|old| old.source_id != source);
                    saved.snapshots.push(snapshot.clone());
                    while saved.snapshots.len() > MAX_SOURCES { saved.snapshots.remove(0); }
                    let encoded = loop {
                        let encoded = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
                        if encoded.len() <= MAX_BYTES { break Some(encoded); }
                        if saved.snapshots.len() <= 1 { break None; }
                        saved.snapshots.remove(0);
                    };
                    match encoded.ok_or_else(|| "Catalog snapshot exceeds durable index capacity".to_string()).and_then(|bytes| cache.write_index(&bytes)) {
                        Ok(()) => snapshot.stored = true,
                        Err(error) => append_error(&mut snapshot, &error),
                    }
                }
                let mut current = current.lock().map_err(|_| "Catalog index lock failed")?;
                // Durable cache capacity is not the addressability limit for
                // configured sources advertised by this process.
                current.retain(|id, _| configured.contains(id));
                current.insert(source, snapshot.clone());
                Ok(())
            })?;
            Ok::<_, String>(snapshot)
        }).await.map_err(|e| format!("Publishing catalog index: {e}"))??;
        let _lease = gate.read().await;
        check_generation(generation, expected)?;
        Ok(snapshot)
    }

    /// IPC IDs only select an exact native-owned representation. They cannot
    /// supply an artifact URL, impersonate another source, or revive old pages.
    /// The caller additionally validates configured-source and edition authority.
    pub fn lookup(&self, source: &str, entry: &str, revision: u64) -> Result<CatalogEntry, String> {
        self.with_selection(source, entry, revision, |_, selected| Ok(selected.clone()))
    }

    /// Reauthorize retained intent IDs against native current metadata, never
    /// against metadata supplied by IPC or an obsolete catalog generation.
    pub fn lookup_current(&self, source: &str, entry: &str) -> Result<(u64, CatalogEntry), String> {
        self.with_selected_snapshot(source, entry, None, |snapshot, selected| {
            Ok((snapshot.generation, selected.clone()))
        })
    }

    /// Native transport provenance only. An official configured alias must not
    /// turn a redirected custom feed into an official acquisition authority.
    #[cfg(any(target_os = "ios", test))]
    pub fn lookup_official(&self, source: &str, entry: &str, revision: u64) -> Result<bool, String> {
        self.with_selection(source, entry, revision, |snapshot, _| {
            if !super::official_catalog_url(&snapshot.source_url) { return Ok(false); }
            let mut found = false;
            for page in snapshot.pages.iter().filter(|page| page.entry_ids.iter().any(|id| id == entry)) {
                found = true;
                if page.stale || page.generation != revision || !super::official_catalog_url(&page.response_url) {
                    return Ok(false);
                }
            }
            Ok(found)
        })
    }

    fn with_selection<T>(
        &self, source: &str, entry: &str, revision: u64,
        work: impl FnOnce(&CatalogSnapshot, &CatalogEntry) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_selected_snapshot(source, entry, Some(revision), work)
    }

    fn with_selected_snapshot<T>(
        &self, source: &str, entry: &str, revision: Option<u64>,
        work: impl FnOnce(&CatalogSnapshot, &CatalogEntry) -> Result<T, String>,
    ) -> Result<T, String> {
        if source.len() != 64 || entry.len() != 64 || revision == Some(0) {
            return Err("Unknown catalog selection. Refresh the catalog".into());
        }
        self.store.with_catalog_cache_policy(|private, _| {
            if private { return Err("Catalog edition selection is unavailable in private mode".into()); }
            {
                let current = self.current.lock().map_err(|_| "Catalog index lock failed")?;
                if let Some(snapshot) = current.get(source) {
                    return work(snapshot, lookup_snapshot(snapshot, source, entry, revision.unwrap_or(snapshot.generation))?);
                }
            }
            // Nothing was published to memory yet (a fresh process, or a source
            // this process never refreshed). Read the durable index without
            // holding either lock across the read, and retain the parsed result
            // so later selections re-parse nothing. The retained read is keyed by
            // the index file's identity, so a newer durable index is never
            // shadowed by an older one.
            let cache = CatalogCache::new(self.store.clone());
            let stamp = cache.index_stamp()?;
            let retained = {
                let durable = self.durable.lock().map_err(|_| "Catalog index lock failed")?;
                match durable.as_ref() {
                    Some((known, saved)) if *known == stamp => Some(saved.clone()),
                    _ => None,
                }
            };
            let saved = match retained {
                Some(saved) => saved,
                None => {
                    let saved = Arc::new(load(&cache)?);
                    let mut durable = self.durable.lock().map_err(|_| "Catalog index lock failed")?;
                    *durable = Some((stamp, saved.clone()));
                    saved
                }
            };
            let snapshot = saved.snapshots.iter().find(|snapshot| snapshot.source_id == source)
                .ok_or("Unknown catalog source. Refresh the catalog")?;
            work(snapshot, lookup_snapshot(snapshot, source, entry, revision.unwrap_or(snapshot.generation))?)
        })
    }
}

fn append_error(snapshot: &mut CatalogSnapshot, error: &str) {
    match &mut snapshot.error {
        Some(previous) if !previous.contains(error) => { previous.push_str("; "); previous.push_str(error); }
        None => snapshot.error = Some(error.to_owned()),
        _ => {},
    }
}

fn lookup_snapshot<'a>(snapshot: &'a CatalogSnapshot, source: &str, entry: &str, revision: u64) -> Result<&'a CatalogEntry, String> {
    if snapshot.source_id != source || snapshot.generation != revision || snapshot.stale {
        return Err("The catalog selection is stale. Refresh the catalog".into());
    }
    let selected = snapshot.entries.iter().find(|candidate| candidate.entry_id == entry)
        .ok_or("Unknown or conflicting catalog entry. Refresh the catalog")?;
    if selected.source_id != source || scoped_entry_id(source, &selected.entry.id) != entry {
        return Err("The catalog selection has invalid source identity".into());
    }
    Ok(&selected.entry)
}

fn load(cache: &CatalogCache) -> Result<SavedIndex, String> {
    let Some(bytes) = cache.read_index()? else { return Ok(SavedIndex { version: 1, snapshots: Vec::new() }); };
    let saved: SavedIndex = serde_json::from_slice(&bytes).map_err(|e| format!("Catalog index unavailable: {e}"))?;
    if saved.version != 1 || saved.snapshots.len() > MAX_SOURCES { return Err("Unsupported catalog index".into()); }
    let mut sources = HashSet::new();
    for snapshot in &saved.snapshots {
        let root = checked_url(&snapshot.source_url, cache.loopback())?;
        if snapshot.source_id != digest(&["catalog-source-v1", root.as_str()]) || !sources.insert(&snapshot.source_id)
            || snapshot.generation == 0 || snapshot.generation >= (1u64 << 53)
            || snapshot.locale != accept_language(&snapshot.locale)
            || !matches!(snapshot.completeness.as_str(), "partial" | "traversed")
            || snapshot.pages.len() > MAX_PAGES || snapshot.entries.len() > MAX_ENTRIES {
            return Err("Saved catalog index has invalid source identity or bounds".into());
        }
        if snapshot.completeness == "traversed" && (snapshot.stale || snapshot.pages.is_empty()
            || snapshot.pages.last().is_some_and(|page| page.next.is_some())) {
            return Err("Saved catalog traversal has invalid completeness".into());
        }
        let mut members = HashSet::new();
        let mut requests = HashSet::new();
        for (offset, page) in snapshot.pages.iter().enumerate() {
            checked_url(&page.request_url, cache.loopback())?;
            checked_url(&page.response_url, cache.loopback())?;
            if let Some(next) = &page.next { checked_url(next, cache.loopback())?; }
            if page.generation != snapshot.generation || page.representation_digest.len() != 64
                || !page.representation_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                || !matches!(page.status.as_str(), "network" | "revalidated" | "offline")
                || page.stale != (page.status == "offline") || (page.stale && !snapshot.stale)
                || page.entry_ids.iter().any(|id| id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
                || page.entry_ids.len() > MAX_ENTRIES || !requests.insert(&page.request_url)
                || (offset == 0 && page.request_url != snapshot.source_url)
                || (offset > 0 && snapshot.pages[offset - 1].next.as_ref() != Some(&page.request_url)) {
                return Err("Saved catalog page membership is invalid".into());
            }
            members.extend(page.entry_ids.iter().map(String::as_str));
        }
        let mut ids = HashSet::new();
        for entry in &snapshot.entries {
            if entry.source_id != snapshot.source_id || entry.entry.id.is_empty()
                || entry.entry_id != scoped_entry_id(&snapshot.source_id, &entry.entry.id)
                || !ids.insert(&entry.entry_id) || !members.contains(entry.entry_id.as_str()) {
                return Err("Saved catalog entry has invalid source membership".into());
            }
            checked_url(&entry.entry.url, false)?;
        }
    }
    Ok(saved)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesKey {
    pub name: String,
    pub language: String,
    pub flavour: Option<String>,
    pub variants: Vec<String>,
}

/// Compatibility is not association authority: a missing flavour requires
/// explicit confirmation or a durable native source binding at the consumer.
pub fn series_compatible(left: &SeriesKey, right: &SeriesKey) -> bool {
    left.name == right.name && left.language == right.language && left.variants == right.variants
        && match (&left.flavour, &right.flavour) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
}

/// Matching aid only, never lineage or installation authority. Missing flavours
/// remain distinct from explicit flavours; conflicting metadata is unmatchable.
pub fn series_key(name: &str, language: &str, tags: &[String]) -> Option<SeriesKey> {
    if name.trim().is_empty() || language.trim().is_empty() { return None; }
    let mut flavour = None;
    let mut variants = BTreeMap::new();
    for tag in tags {
        if let Some(value) = tag.strip_prefix("_flavour:") {
            if value.is_empty() || flavour.as_ref().is_some_and(|old| old != value) { return None; }
            flavour = Some(value.to_owned());
        }
        if let Some((key, value)) = tag.split_once(':') {
            if matches!(key, "_pictures" | "_videos" | "_details" | "_ftindex") {
                if value.is_empty() || variants.get(key).is_some_and(|old| *old != value) { return None; }
                variants.insert(key, value);
            }
        }
    }
    Some(SeriesKey { name: name.trim().to_owned(), language: language.trim().to_ascii_lowercase(),
        flavour, variants: variants.into_iter().map(|(key, value)| format!("{key}:{value}")).collect() })
}

/// Complete calendar dates and RFC3339 timestamps only. A month-only value has
/// unknown day precision and cannot safely establish a newer edition.
pub fn edition_date(value: &str) -> Option<chrono::NaiveDate> {
    if value.len() == 10 { return chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok(); }
    chrono::DateTime::parse_from_rfc3339(value).ok().map(|date| date.naive_utc().date())
}

/// Compare only non-overlapping calendar precision intervals. In particular,
/// an edition issued somewhere in a month is not known newer than a dated
/// edition in that same month (nor is the inverse).
pub fn edition_is_newer(candidate: &str, installed: &str) -> bool {
    edition_interval(candidate).zip(edition_interval(installed))
        .is_some_and(|((candidate_start, _), (_, installed_end))| candidate_start >= installed_end)
}

fn edition_interval(value: &str) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
    if value.len() == 7 {
        let bytes = value.as_bytes();
        if bytes[4] != b'-' || !bytes[..4].iter().chain(&bytes[5..]).all(u8::is_ascii_digit) {
            return None;
        }
        let year = value.get(..4)?.parse().ok()?;
        let month = value.get(5..7)?.parse().ok()?;
        let start = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
        let end = start.checked_add_months(chrono::Months::new(1))?;
        return Some((start, end));
    }
    let start = edition_date(value)?;
    Some((start, start.succ_opt()?))
}

#[cfg(test)]
#[path = "catalog_index_tests.rs"]
mod tests;
