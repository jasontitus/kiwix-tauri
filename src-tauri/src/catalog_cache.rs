// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Profile-scoped, bounded raw OPDS cache. Cached XML always passes the live parsers.
use super::*;
use crate::store::Store;
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, ETAG, IF_NONE_MATCH};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex, OnceLock};
use tokio::sync::RwLock;

// Leave room for a complete index traversal and both navigation feeds.
const MAX_RECORDS: usize = super::index::MAX_PAGES + 2;
const MAX_CACHE: usize = 32 * 1024 * 1024;
const CACHE_FILE: &str = "catalog-opds-v1.json";
const PENDING_FILE: &str = "catalog-opds-v1.pending";
const INDEX_FILE: &str = "catalog-index-v1.json";
const INDEX_PENDING_FILE: &str = "catalog-index-v1.pending";
// Exact compact JSON prefixes produced by the serializers. A legacy fixed-name
// scratch file may only be recovered when its bytes are empty or begin with one.
const CACHE_PAYLOAD_PREFIX: &[u8] = br#"{"version":1,"records":["#;
const INDEX_PAYLOAD_PREFIX: &[u8] = br#"{"version":1,"snapshots":["#;
const MAX_ETAG: usize = 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCacheMetadata {
    /// network: new body; revalidated: conditional 304; offline: failed fetch.
    pub status: String,
    pub stale: bool,
    /// Whether the returned representation is durably available for this profile.
    pub stored: bool,
    /// Network failure for offline results, or a cache availability/write warning.
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedCatalogPage {
    #[serde(flatten)]
    pub page: CatalogPage,
    pub cache: CatalogCacheMetadata,
    /// Actual response origin (or the cached representation's original origin),
    /// retained only inside Rust for acquisition provenance, never IPC input.
    #[serde(skip)]
    representation: Representation,
}

impl CachedCatalogPage {
    pub(crate) fn source_url(&self) -> &str { &self.representation.source_url }
    pub(super) fn representation_digest(&self) -> &str { &self.representation.digest }
    pub(super) fn representation_bytes(&self) -> usize { self.representation.bytes }
}

#[derive(Debug)]
struct Representation {
    source_url: String,
    digest: String,
    bytes: usize,
}

impl From<&Record> for Representation {
    fn from(record: &Record) -> Self {
        Self { source_url: record.base.clone(), digest: record.digest.clone(), bytes: record.xml.len() }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedCatalogFacets {
    #[serde(flatten)]
    pub facets: CatalogFacets,
    pub cache: CatalogCacheMetadata,
}

#[derive(Clone)]
pub struct CatalogCache {
    store: Arc<Store>,
    // Production is strictly HTTPS. Literal loopback HTTP is enabled only by tests.
    #[cfg(test)]
    loopback_http: bool,
    /// Last read of the durable cache, keyed by the cache file's identity.
    /// Populated and consulted only inside Store's catalog policy transaction,
    /// exactly like the disk cache itself, so private mode can never observe a
    /// normal-mode representation through it. A stamp mismatch re-reads, so a
    /// concurrent writer is never hidden behind a stale snapshot.
    memory: Arc<Mutex<Option<Arc<Retained>>>>,
}

/// Identity of the cache file a `Retained` snapshot was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stamp {
    Absent,
    File {
        device: u64,
        inode: u64,
        length: u64,
        modified: Option<std::time::SystemTime>,
    },
}

impl Stamp {
    fn of(metadata: &cap_std::fs::Metadata) -> Self {
        Self::File {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: metadata.modified().ok().map(|modified| modified.into_std()),
        }
    }
}

/// Bounded structural snapshot of the cache file (the same MAX_CACHE/MAX_RECORDS
/// limits as the file). `verified` records that every XML body's digest was
/// checked, which a fetch needs only for the one representation it returns.
struct Retained {
    stamp: Stamp,
    records: Records<Arc<Record>>,
    verified: AtomicBool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Kind { Page, Languages, Categories }

impl Kind {
    fn maximum(self) -> usize {
        match self { Self::Page => MAX_FEED, _ => 2 * 1024 * 1024 }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    url: String,
    language: String,
    kind: Kind,
    base: String,
    etag: Option<String>,
    xml: String,
    digest: String,
    #[serde(skip)]
    encoded_bytes: OnceLock<usize>,
}

impl Record {
    fn matches(&self, url: &str, language: &str, kind: Kind) -> bool {
        self.url == url && self.language == language && self.kind == kind
    }

    fn encoded_bytes(&self) -> usize {
        *self.encoded_bytes.get_or_init(|| {
            struct Count(usize);
            impl Write for Count {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    self.0 += bytes.len();
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
            }
            let mut count = Count(0);
            serde_json::to_writer(&mut count, self).expect("Serializing a catalog record into a byte counter");
            count.0
        })
    }

    fn validate(&self, loopback: bool) -> Result<Parsed, String> {
        checked_url(&self.url, loopback)?;
        let base = checked_url(&self.base, loopback)?;
        if self.xml.len() > self.kind.maximum() || self.language != accept_language(&self.language)
            || self.etag.as_ref().is_some_and(|tag| !valid_etag(tag))
            || self.digest != record_digest(self) {
            return Err("A saved catalog page is damaged. Refresh the catalog".into());
        }
        parse(self.kind, &self.xml, &base, &self.language)
    }
}

fn record_digest(record: &Record) -> String {
    let mut hash = Sha256::new();
    hash.update([record.kind as u8]);
    for field in [record.url.as_str(), record.language.as_str(), record.base.as_str(), record.etag.as_deref().unwrap_or(""), record.xml.as_str()] {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    hex::encode(hash.finalize())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Records<T = Record> {
    version: u32,
    // Insertion/revalidation order, oldest first. Offline reads never write.
    records: Vec<T>,
}

pub(super) enum CacheUpdate {
    Fresh { record: Arc<Record>, previous: Option<String> },
    Revalidated(String),
}

enum Parsed { Page(CatalogPage), Facets(Vec<CatalogFacet>) }

struct CacheDirectory {
    profile_path: std::path::PathBuf,
    profile: Dir,
    directory: Dir,
}

impl CacheDirectory {
    fn unchanged(&self) -> std::io::Result<()> {
        let parent = self.profile.dir_metadata()?;
        let current_parent = crate::storage::open_directory_nofollow(&self.profile_path)?.dir_metadata()?;
        let retained = self.directory.dir_metadata()?;
        let named = self.profile.symlink_metadata("cache")?;
        let opened = self.profile.open_dir_nofollow("cache")?.dir_metadata()?;
        if !current_parent.is_dir() || current_parent.dev() != parent.dev() || current_parent.ino() != parent.ino()
            || !named.is_dir() || named.is_symlink() || named.dev() != retained.dev() || named.ino() != retained.ino()
            || opened.dev() != retained.dev() || opened.ino() != retained.ino() {
            return Err(std::io::Error::other("Catalog cache directory attachment changed"));
        }
        Ok(())
    }

    /// Cheap identity of a durable file, so a retained read is reused only while
    /// the file it came from is unchanged. Publication renames a new inode into
    /// place, so a replacement always changes the stamp.
    fn stamp(&self, name: &str, label: &str) -> Result<Stamp, String> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
        match self.directory.open_with(name, &options) {
            Ok(file) => {
                let metadata = file.metadata().map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > MAX_CACHE as u64 {
                    return Err(format!("{label} is not a bounded, unlinked regular file"));
                }
                Ok(Stamp::of(&metadata))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Stamp::Absent),
            Err(error) => Err(format!("Reading {label}: {error}")),
        }
    }
}

fn parse(kind: Kind, xml: &str, base: &Url, language: &str) -> Result<Parsed, String> {
    match kind {
        Kind::Page => parse_catalog(xml, base).map(Parsed::Page),
        Kind::Languages => parse_facets(xml, base, "lang", language).map(Parsed::Facets),
        Kind::Categories => parse_facets(xml, base, "category", language).map(Parsed::Facets),
    }
}

fn valid_etag(tag: &str) -> bool {
    let value = tag.strip_prefix("W/").unwrap_or(tag);
    tag.len() <= MAX_ETAG && value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1].iter().all(|b| *b == 0x21 || (0x23..=0x7e).contains(b) || *b >= 0x80)
        && reqwest::header::HeaderValue::from_str(tag).is_ok()
}

fn weak_etag_eq(left: &str, right: &str) -> bool {
    valid_etag(left) && valid_etag(right)
        && left.strip_prefix("W/").unwrap_or(left) == right.strip_prefix("W/").unwrap_or(right)
}

impl CatalogCache {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store, #[cfg(test)] loopback_http: false, memory: Arc::new(Mutex::new(None)) }
    }

    #[cfg(test)]
    pub(super) fn with_loopback(store: Arc<Store>) -> Self {
        Self { store, loopback_http: true, memory: Arc::new(Mutex::new(None)) }
    }

    pub(super) fn loopback(&self) -> bool {
        #[cfg(test)] { self.loopback_http }
        #[cfg(not(test))] { false }
    }

    /// The index shares the cache's retained directory and publication discipline.
    /// Callers hold Store's catalog policy transaction for these operations.
    pub(super) fn read_index(&self) -> Result<Option<Vec<u8>>, String> {
        let Some(directory) = self.directory(false)? else { return Ok(None); };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
        let file = match directory.directory.open_with(INDEX_FILE, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Reading catalog index: {error}")),
        };
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > MAX_CACHE as u64 {
            return Err("Catalog index is not a bounded, unlinked regular file".into());
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_CACHE as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_CACHE { return Err("Catalog index exceeds disk limit".into()); }
        directory.unchanged().map_err(|e| e.to_string())?;
        Ok(Some(bytes))
    }

    pub(super) fn write_index(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_CACHE { return Err("Catalog index exceeds disk limit".into()); }
        let directory = self.directory(true)?.ok_or("Catalog cache directory unavailable")?;
        atomic_write_named(&directory, bytes, INDEX_PENDING_FILE, INDEX_FILE, INDEX_PAYLOAD_PREFIX, |_| {}).map(|_| ())
    }

    /// Cheap identity of the durable index, so a retained parse is reused only
    /// while that index is unchanged. Callers hold Store's catalog policy
    /// transaction, like the index read and write themselves.
    pub(super) fn index_stamp(&self) -> Result<Stamp, String> {
        let Some(directory) = self.directory(false)? else { return Ok(Stamp::Absent); };
        directory.stamp(INDEX_FILE, "catalog index")
    }
    pub async fn fetch_page(
        &self, url: &str, locale: &str, gate: &Arc<RwLock<()>>, generation: &AtomicU64, expected_generation: u64,
    ) -> Result<CachedCatalogPage, String> {
        {
            let _guard = gate.read().await;
            check_generation(generation, expected_generation)?;
        }
        let (page, _) = self.fetch_page_impl(url, locale, gate, generation, expected_generation, false).await?;
        Ok(page)
    }

    /// An index traversal publishes accepted fresh bodies together, after it
    /// reacquires the same privacy lease and Store policy for its index commit.
    pub(super) async fn fetch_refresh_page(
        &self, url: &str, locale: &str, gate: &Arc<RwLock<()>>, generation: &AtomicU64, expected: u64,
    ) -> Result<(CachedCatalogPage, Option<CacheUpdate>), String> {
        self.fetch_page_impl(url, locale, gate, generation, expected, true).await
    }

    async fn fetch_page_impl(
        &self, url: &str, locale: &str, gate: &Arc<RwLock<()>>, generation: &AtomicU64, expected: u64, defer: bool,
    ) -> Result<(CachedCatalogPage, Option<CacheUpdate>), String> {
        let (parsed, cache, representation, update) = self.fetch(url, locale, Kind::Page, gate, generation, expected, defer).await?;
        let Parsed::Page(page) = parsed else { unreachable!("Page parser variant") };
        Ok((CachedCatalogPage { page, cache, representation }, update))
    }

    pub async fn fetch_facets(
        &self, source: &str, locale: &str, gate: &Arc<RwLock<()>>, generation: &AtomicU64, expected_generation: u64,
    ) -> Result<CachedCatalogFacets, String> {
        let (languages, categories) = facet_urls(source, self.loopback())?;
        // Both navigation feeds must belong to the same privacy generation.
        {
            let _guard = gate.read().await;
            check_generation(generation, expected_generation)?;
        }
        let (languages, categories) = tokio::try_join!(
            self.fetch(languages.as_str(), locale, Kind::Languages, gate, generation, expected_generation, false),
            self.fetch(categories.as_str(), locale, Kind::Categories, gate, generation, expected_generation, false),
        )?;
        let (Parsed::Facets(languages_values), mut cache, _, _) = languages else { unreachable!("Facet parser variant") };
        let (Parsed::Facets(categories_values), other, _, _) = categories else { unreachable!("Facet parser variant") };
        let _guard = gate.read().await;
        check_generation(generation, expected_generation)?;
        cache.stale |= other.stale;
        cache.stored &= other.stored;
        cache.status = if cache.stale { "offline" } else if cache.status == "network" || other.status == "network" { "network" } else { "revalidated" }.into();
        cache.error = match (cache.error, other.error) {
            (Some(a), Some(b)) if a != b => Some(format!("{a}; {b}")),
            (Some(a), _) => Some(a),
            (_, error) => error,
        };
        Ok(CachedCatalogFacets { facets: CatalogFacets { languages: languages_values, categories: categories_values }, cache })
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch(
        &self, url: &str, locale: &str, kind: Kind, gate: &Arc<RwLock<()>>, generation: &AtomicU64, expected: u64, defer: bool,
    ) -> Result<(Parsed, CatalogCacheMetadata, Representation, Option<CacheUpdate>), String> {
        let url = checked_url(url, self.loopback())?;
        let language = accept_language(locale);
        let (cached, cache_warning, previous) = {
            let lease = gate.clone().read_owned().await;
            check_generation(generation, expected)?;
            let cache = self.clone();
            let request_url = url.to_string();
            let request_language = language.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                let loaded = cache.store.with_catalog_cache_policy(|private, _| {
                    if private { return Ok(None); }
                    // Only the representation that is actually returned is
                    // digest-verified (`Record::validate`); re-hashing every
                    // retained XML body per page made one fetch O(cache).
                    Ok(cache.retained()?.records.records.iter()
                        .find(|r| r.matches(&request_url, &request_language, kind)).cloned())
                });
                match loaded {
                    Ok(Some(record)) => {
                        let previous = defer.then(|| record.digest.clone());
                        match record.validate(cache.loopback()) {
                            Ok(parsed) => (Some((record, parsed)), None, previous),
                            Err(error) => (None, Some(error), previous),
                        }
                    }
                    Ok(None) => (None, None, None),
                    Err(error) => (None, Some(error), None),
                }
            }).await.map_err(|e| format!("Loading catalog cache: {e}"))?;
            let _guard = gate.read().await;
            check_generation(generation, expected)?;
            result
        };
        let fetched = self.request(&url, &language, kind, cached.as_ref().map(|(record, _)| record.as_ref())).await;
        // No result or disk write may escape a retired privacy generation, including
        // normal -> private -> normal transitions during an in-flight request.
        let lease = gate.clone().read_owned().await;
        check_generation(generation, expected)?;
        let cache = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            cache.store.with_catalog_cache_policy(|private, read_only| {
                match fetched {
                    Ok(fetched) => {
                        let (record, parsed, revalidated) = match fetched {
                            Some((record, parsed)) => (record, parsed, false),
                            None => {
                                let (record, parsed) = cached.ok_or("The catalog did not refresh. Try again")?;
                                (record, parsed, true)
                            }
                        };
                        let representation = Representation::from(record.as_ref());
                        let mut metadata = CatalogCacheMetadata {
                            status: if revalidated { "revalidated" } else { "network" }.into(),
                            stale: false, stored: false, error: cache_warning,
                        };
                        if defer {
                            let update = if private || read_only {
                                None
                            } else if revalidated {
                                Some(CacheUpdate::Revalidated(record.digest.clone()))
                            } else {
                                Some(CacheUpdate::Fresh { record, previous })
                            };
                            // Storage will be determined by the traversal's eventual publication.
                            return Ok((parsed, metadata, representation, update));
                        }
                        // A 304 describes the representation sent with this request. A
                        // later 200 may already have committed a different representation.
                        if !private && !read_only && (!revalidated || cache.contains(&record)) {
                            match cache.save_updates(vec![CacheUpdate::Fresh { record: Arc::clone(&record), previous: None }], false) {
                                Ok(warning) => {
                                    metadata.stored = warning.is_none() || cache.contains(&record);
                                    if let Some(warning) = warning {
                                        metadata.error = Some(match metadata.error.take() {
                                            Some(existing) => format!("{existing}; {warning}"),
                                            None => warning,
                                        });
                                    }
                                }
                                Err(error) => {
                                    metadata.stored = cache.contains(&record);
                                    metadata.error = Some(error);
                                }
                            }
                        } else if !private {
                            metadata.stored = cache.contains(&record);
                        }
                        Ok((parsed, metadata, representation, None))
                    }
                    Err(error) => {
                        // Only a representation parsed before the request is eligible.
                        // Private mode must never see a normal-mode history entry.
                        let Some((record, parsed)) = cached.filter(|_| !private) else {
                            return Err(match cache_warning { Some(warning) => format!("{error}; {warning}"), None => error });
                        };
                        let stored = !defer && cache.contains(&record);
                        Ok((parsed, CatalogCacheMetadata { status: "offline".into(), stale: true, stored, error: Some(error) }, Representation::from(record.as_ref()), None))
                    }
                }
            })
        }).await.map_err(|e| format!("Publishing catalog cache: {e}"))?;
        let _guard = gate.read().await;
        check_generation(generation, expected)?;
        result
    }

    /// `None` is a validated 304 response that reuses the cached representation.
    async fn request(
        &self, url: &Url, language: &str, kind: Kind, cached: Option<&Record>,
    ) -> Result<Option<(Arc<Record>, Parsed)>, String> {
        // Retained per transport policy, so pages of one traversal and later
        // catalog work reuse its warm connector pool.
        let client = http_client_with_redirects(self.loopback(), false)?;
        let mut current = url.clone();
        let mut redirects = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        // ETags identify one final resource, not an alias or another redirect
        // target. Discover aliases without a validator and condition only the
        // exact previously validated final URL.
        let response = loop {
            let mut request = client.get(current.clone())
                .header(ACCEPT, if kind == Kind::Page { "application/atom+xml, application/xml;q=0.9" } else { "application/atom+xml" })
                .header(ACCEPT_ENCODING, "identity")
                .header(ACCEPT_LANGUAGE, language)
                .timeout(deadline.saturating_duration_since(std::time::Instant::now()));
            if let Some(etag) = cached.filter(|record| record.base == current.as_str()).and_then(|record| record.etag.as_deref()) {
                request = request.header(IF_NONE_MATCH, etag);
            }
            let response = request.send().await.map_err(|e| format!("Fetching OPDS catalog: {e}"))?;
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if redirects >= 8 { return Err("Too many HTTP redirects".into()); }
                let location = response.headers().get(reqwest::header::LOCATION).and_then(|value| value.to_str().ok())
                    .ok_or("OPDS redirect has no valid Location")?;
                let target = current.join(location).map_err(|e| format!("Invalid OPDS redirect: {e}"))?;
                let target = checked_url(target.as_str(), self.loopback())?;
                if current.scheme() == "https" && target.scheme() == "http" { return Err("Unsafe HTTP redirect".into()); }
                current = target;
                redirects += 1;
                continue;
            }
            break response;
        };
        let conditional = cached.filter(|record| record.base == current.as_str()).and_then(|record| record.etag.as_deref());
        let base = checked_url(response.url().as_str(), self.loopback())?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            cached.filter(|record| conditional.is_some() && record.base == base.as_str())
                .ok_or("The catalog did not refresh. Try again")?;
            if let Some(etag) = response.headers().get(ETAG) {
                if !etag.to_str().ok().zip(conditional).is_some_and(|(received, sent)| weak_etag_eq(received, sent)) {
                    return Err("The catalog did not refresh. Try again".into());
                }
            }
            return Ok(None);
        }
        let etag = response.headers().get(ETAG).and_then(|tag| tag.to_str().ok())
            .filter(|tag| valid_etag(tag)).map(str::to_owned);
        let bytes = bounded_body(response, kind.maximum()).await?;
        let language = language.to_owned();
        let request_url = url.to_string();
        tokio::task::spawn_blocking(move || {
            let xml = String::from_utf8(bytes).map_err(|e| format!("OPDS feed is not UTF-8: {e}"))?;
            let parsed = parse(kind, &xml, &base, &language)?;
            let mut record = Record { url: request_url, language, kind, base: base.to_string(), etag, xml, digest: String::new(), encoded_bytes: OnceLock::new() };
            record.digest = record_digest(&record);
            Ok(Some((Arc::new(record), parsed)))
        }).await.map_err(|e| format!("Parsing OPDS catalog: {e}"))?
    }

    // Invoked only within Store's policy transaction; this capability helper does
    // not acquire the Store mutex. No path-based I/O follows profile admission.
    fn directory(&self, create: bool) -> Result<Option<CacheDirectory>, String> {
        let (profile_path, profile) = self.store.acquisition_profile_directory()?;
        if create {
            #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
            let mut builder = cap_std::fs::DirBuilder::new();
            #[cfg(unix)] { use cap_std::fs::DirBuilderExt; builder.mode(0o700); }
            match profile.create_dir_with("cache", &builder) {
                Ok(()) => sync_directory(&profile)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(format!("Creating catalog cache: {error}")),
            }
        }
        let directory = match profile.open_dir_nofollow("cache") {
            Ok(directory) => directory,
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Opening catalog cache: {error}")),
        };
        let directory = CacheDirectory { profile_path, profile, directory };
        directory.unchanged().map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if create {
            use std::os::unix::fs::PermissionsExt;
            // Linux/Android directory capabilities use O_PATH; fchmod needs
            // a readable descriptor for the same retained directory.
            #[cfg(any(target_os = "linux", target_os = "android"))]
            let permissions_target = {
                use cap_std::fs::OpenOptionsExt;
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No)
                    .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW);
                let readable = directory.directory.open_with(".", &options).map_err(|e| e.to_string())?;
                let retained = directory.directory.dir_metadata().map_err(|e| e.to_string())?;
                let opened = readable.metadata().map_err(|e| e.to_string())?;
                if !opened.is_dir() || opened.dev() != retained.dev() || opened.ino() != retained.ino() {
                    return Err("Catalog cache permission descriptor changed identity".into());
                }
                readable.into_std()
            };
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            let permissions_target = directory.directory.try_clone().map_err(|e| e.to_string())?.into_std_file();
            permissions_target.set_permissions(std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
            directory.unchanged().map_err(|e| e.to_string())?;
        }
        Ok(Some(directory))
    }

    /// Structural snapshot of the durable cache, memoized per cache-file
    /// identity. Callers hold Store's catalog policy transaction.
    fn retained(&self) -> Result<Arc<Retained>, String> {
        let Some(directory) = self.directory(false)? else {
            return Ok(Arc::new(Retained {
                stamp: Stamp::Absent, records: Records { version: 1, records: Vec::new() },
                verified: AtomicBool::new(true),
            }));
        };
        self.retained_in(&directory)
    }

    fn retained_in(&self, directory: &CacheDirectory) -> Result<Arc<Retained>, String> {
        let stamp = directory.stamp(CACHE_FILE, "catalog cache")?;
        {
            let memory = self.memory();
            if let Some(retained) = memory.as_ref().filter(|retained| retained.stamp == stamp) {
                return Ok(retained.clone());
            }
        }
        let (stamp, records) = read_records(&directory.directory, self.loopback())?;
        directory.unchanged().map_err(|e| e.to_string())?;
        if directory.stamp(CACHE_FILE, "catalog cache")? != stamp {
            return Err("Catalog cache changed while reading".into());
        }
        let retained = Arc::new(Retained { stamp, records, verified: AtomicBool::new(false) });
        let mut memory = self.memory();
        *memory = Some(retained.clone());
        Ok(retained)
    }

    /// Verify the retained file once before merging its XML bodies into a write.
    fn verified(retained: Arc<Retained>) -> Result<Arc<Retained>, String> {
        if retained.verified.load(Ordering::Acquire) { return Ok(retained); }
        if retained.records.records.iter().any(|record| record.digest != record_digest(record)) {
            return Err("A saved catalog page is damaged. Refresh the catalog".into());
        }
        retained.verified.store(true, Ordering::Release);
        Ok(retained)
    }

    fn contains(&self, record: &Record) -> bool {
        self.contains_representation(&record.url, &record.language, record.kind, &record.base, &record.digest)
    }

    /// A panic while holding this pure cache slot leaves no invariant broken.
    fn memory(&self) -> std::sync::MutexGuard<'_, Option<Arc<Retained>>> {
        self.memory.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn contains_representation(&self, url: &str, language: &str, kind: Kind, base: &str, digest: &str) -> bool {
        self.retained().and_then(Self::verified).is_ok_and(|retained| retained.records.records.iter().any(|r|
            r.matches(url, language, kind) && r.base == base && r.digest == digest))
    }

    pub(super) fn save_refresh(&self, updates: Vec<CacheUpdate>) -> Result<Option<String>, String> {
        self.save_updates(updates, true)
    }

    // Callers hold Store's policy transaction. Reload any intervening single
    // page writes, then merge in traversal order and encode once for publication.
    // An accepted representation that cannot fit the bounded file is skipped
    // with a warning so the remaining traversal pages still publish.
    fn save_updates(&self, updates: Vec<CacheUpdate>, batch: bool) -> Result<Option<String>, String> {
        if updates.is_empty() { return Ok(None); }
        let directory = self.directory(true)?.ok_or("Catalog cache directory unavailable")?;
        let retained = self.retained_in(&directory).and_then(Self::verified).ok();
        let mut records = retained.as_ref().map(|cache| cache.records.records.clone()).unwrap_or_default();
        const ENVELOPE_BYTES: usize = br#"{"version":1,"records":[]}"#.len();
        let mut skipped = 0usize;
        for update in updates {
            match update {
                CacheUpdate::Fresh { record, previous } => {
                    if ENVELOPE_BYTES + record.encoded_bytes() > MAX_CACHE {
                        skipped += 1;
                        continue;
                    }
                    if let Some(index) = records.iter().position(|r| r.matches(&record.url, &record.language, record.kind)) {
                        if batch && records[index].digest != record.digest
                            && previous.as_ref() != Some(&records[index].digest)
                        {
                            continue; // A concurrent interactive request superseded this body.
                        }
                        records.remove(index);
                    }
                    records.push(record);
                }
                CacheUpdate::Revalidated(digest) => {
                    if let Some(index) = records.iter().position(|r| r.digest == digest) {
                        let record = records.remove(index);
                        records.push(record);
                    }
                }
            }
        }
        let warning = skipped_warning(skipped);
        if records.is_empty() { return Ok(warning); }
        if batch && retained.as_ref().is_some_and(|old| old.records.records.iter().map(|r| &r.digest)
            .eq(records.iter().map(|r| &r.digest))) {
            return Ok(warning);
        }
        let mut bytes = ENVELOPE_BYTES + records.iter().map(|r| r.encoded_bytes()).sum::<usize>()
            + records.len().saturating_sub(1);
        let mut evicted = 0;
        while records.len() - evicted > MAX_RECORDS || bytes > MAX_CACHE {
            bytes -= records[evicted].encoded_bytes() + 1;
            evicted += 1;
        }
        records.drain(..evicted);
        let mut encoded = Vec::with_capacity(bytes);
        serde_json::to_writer(&mut encoded, &Records { version: 1, records: records.iter().map(Arc::as_ref).collect() })
            .map_err(|e| format!("Encoding catalog cache: {e}"))?;
        debug_assert_eq!(encoded.len(), bytes);
        let stamp = atomic_write_named(&directory, &encoded, PENDING_FILE, CACHE_FILE, CACHE_PAYLOAD_PREFIX, |_| {})?;
        *self.memory() = Some(Arc::new(Retained {
            stamp, records: Records { version: 1, records }, verified: AtomicBool::new(true),
        }));
        Ok(warning)
    }
}

fn skipped_warning(skipped: usize) -> Option<String> {
    match skipped {
        0 => None,
        1 => Some("A catalog page exceeds durable cache capacity and was not saved".into()),
        skipped => Some(format!("{skipped} catalog pages exceed durable cache capacity and were not saved")),
    }
}

fn check_generation(generation: &AtomicU64, expected: u64) -> Result<(), String> {
    if generation.load(Ordering::SeqCst) != expected {
        Err("Your privacy settings changed, so the catalog did not update. Open the catalog again".into())
    } else { Ok(()) }
}

/// Read the cache file and validate every record's bounds, transport and key
/// uniqueness, but not its XML digest: the digest of the one representation a
/// caller returns is checked by `Record::validate`, and whole-cache digests are
/// checked once per file identity by `CatalogCache::load`.
fn read_records(directory: &Dir, loopback: bool) -> Result<(Stamp, Records<Arc<Record>>), String> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)] {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match directory.open_with(CACHE_FILE, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Stamp::Absent, Records { version: 1, records: Vec::new() })),
        Err(error) => return Err(format!("Reading catalog cache: {error}")),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > MAX_CACHE as u64 {
        return Err("Catalog cache is not a bounded, unlinked regular file".into());
    }
    let stamp = Stamp::of(&metadata);
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&file).take(MAX_CACHE as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("Reading catalog cache: {e}"))?;
    if bytes.len() > MAX_CACHE { return Err("Catalog cache exceeds disk limit".into()); }
    if Stamp::of(&file.metadata().map_err(|e| e.to_string())?) != stamp {
        return Err("Catalog cache changed while reading".into());
    }
    let records: Records = serde_json::from_slice(&bytes).map_err(|e| format!("Catalog cache unavailable: {e}"))?;
    if records.version != 1 || records.records.len() > MAX_RECORDS { return Err("Unsupported or oversized catalog cache".into()); }
    let mut keys = HashSet::new();
    for record in &records.records {
        // Validate bounds/transport for all entries, parsing only the requested
        // XML below to avoid doing unrelated XML work on every fetch.
        checked_url(&record.url, loopback)?;
        checked_url(&record.base, loopback)?;
        if record.xml.len() > record.kind.maximum() || record.language != accept_language(&record.language)
            || record.etag.as_ref().is_some_and(|tag| !valid_etag(tag))
            || !keys.insert((&record.url, &record.language, record.kind as u8)) {
            return Err("A saved catalog page is damaged. Refresh the catalog".into());
        }
    }
    Ok((stamp, Records { version: records.version, records: records.records.into_iter().map(Arc::new).collect() }))
}

#[cfg(test)]
fn atomic_write(cache: &CacheDirectory, bytes: &[u8]) -> Result<(), String> {
    atomic_write_with_hook(cache, bytes, |_| {})
}

#[cfg(test)]
fn atomic_write_with_hook(cache: &CacheDirectory, bytes: &[u8], before_publish: impl FnOnce(&str)) -> Result<(), String> {
    atomic_write_named(cache, bytes, PENDING_FILE, CACHE_FILE, CACHE_PAYLOAD_PREFIX, before_publish).map(|_| ())
}

/// A scratch file is app-owned only if it is a private, single-link regular
/// file created by this user.
fn app_owned_scratch(metadata: &cap_std::fs::Metadata) -> bool {
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > MAX_CACHE as u64 { return false; }
    #[cfg(unix)]
    if cap_std::fs::MetadataExt::uid(metadata) != crate::fs_ext::effective_uid()
        || cap_std::fs::MetadataExt::mode(metadata) & 0o077 != 0 {
        return false;
    }
    true
}

fn scratch_uuid(name: &str) -> bool {
    name.len() == 36 && name.bytes().enumerate().all(|(index, byte)|
        if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() })
}

fn open_read_nofollow(directory: &Dir, name: &str) -> std::io::Result<cap_std::fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
    directory.open_with(name, &options)
}

/// Older builds used one fixed scratch name and truncated it before writing.
/// Recover only a file this app could have written (private, owned, and an
/// empty or truncated copy of the expected payload); never touch a foreign file.
fn recover_legacy_scratch(directory: &Dir, pending: &str, expected: &[u8]) -> std::io::Result<()> {
    let named = match directory.symlink_metadata(pending) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !app_owned_scratch(&named) {
        return Err(std::io::Error::other("Catalog cache scratch file is not app-owned"));
    }
    let opened = open_read_nofollow(directory, pending)?;
    let metadata = opened.metadata()?;
    if !app_owned_scratch(&metadata) || metadata.dev() != named.dev() || metadata.ino() != named.ino() {
        return Err(std::io::Error::other("Catalog cache scratch file changed"));
    }
    let mut prefix = Vec::with_capacity(expected.len() + 1);
    (&opened).take(expected.len() as u64 + 1).read_to_end(&mut prefix)?;
    if !prefix.is_empty() && !prefix.starts_with(expected) && !expected.starts_with(&prefix) {
        return Err(std::io::Error::other("Catalog cache scratch file is not app-owned"));
    }
    drop(opened);
    match directory.remove_file(pending) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Interrupted writes leave UUID-named scratch files behind. Remove only
/// app-owned leftovers before claiming a new, unguessable scratch name.
fn remove_stale_scratch(directory: &Dir, pending: &str, expected: &[u8]) -> std::io::Result<()> {
    let prefix = format!("{pending}.");
    let mut stale = Vec::new();
    for entry in directory.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue; };
        if text.strip_prefix(prefix.as_str()).is_some_and(scratch_uuid) { stale.push(text.to_owned()); }
    }
    for name in stale {
        let named = match directory.symlink_metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !app_owned_scratch(&named) { continue; }
        let opened = open_read_nofollow(directory, &name)?;
        let metadata = opened.metadata()?;
        if !app_owned_scratch(&metadata) || metadata.dev() != named.dev() || metadata.ino() != named.ino() { continue; }
        let mut prefix = Vec::with_capacity(expected.len() + 1);
        (&opened).take(expected.len() as u64 + 1).read_to_end(&mut prefix)?;
        if !prefix.is_empty() && !prefix.starts_with(expected) && !expected.starts_with(&prefix) { continue; }
        drop(opened);
        match directory.remove_file(&name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn atomic_write_named(cache: &CacheDirectory, bytes: &[u8], pending: &str, destination: &str, legacy_prefix: &[u8], before_publish: impl FnOnce(&str)) -> Result<Stamp, String> {
    let directory = &cache.directory;
    let io = || -> std::io::Result<Stamp> {
        cache.unchanged()?;
        recover_legacy_scratch(directory, pending, legacy_prefix)?;
        remove_stale_scratch(directory, pending, legacy_prefix)?;
        // A fresh, unguessable scratch name is created exclusively, so a
        // pre-existing file is never opened, truncated, or reused.
        let scratch = format!("{pending}.{}", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true).follow(FollowSymlinks::No);
        #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NONBLOCK); }
        let file = directory.open_with(&scratch, &options)?;
        let before = file.metadata()?;
        if !before.is_file() || before.nlink() != 1 || before.len() != 0 {
            drop(file);
            if let Ok(named) = directory.symlink_metadata(&scratch) {
                if named.is_file() && named.dev() == before.dev() && named.ino() == before.ino() {
                    let _ = directory.remove_file(&scratch);
                }
            }
            return Err(std::io::Error::other("Catalog cache scratch file is unsafe"));
        }
        let named = match directory.symlink_metadata(&scratch) {
            Ok(metadata) => metadata,
            Err(error) => { drop(file); return Err(error); }
        };
        if !named.is_file() || named.nlink() != 1 || named.dev() != before.dev() || named.ino() != before.ino() {
            drop(file);
            return Err(std::io::Error::other("Catalog cache scratch file changed"));
        }
        let mut file = file.into_std();
        let result = (|| -> std::io::Result<Stamp> {
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            cache.unchanged()?;
            file.write_all(bytes)?;
            file.sync_all()?;
            let verified = crate::publication::VerifiedFile::new(file, bytes.len() as u64)?;
            before_publish(&scratch);
            cache.unchanged()?;
            if !verified.matches_in(directory, std::path::Path::new(&scratch))? {
                return Err(std::io::Error::other("Catalog cache scratch file changed"));
            }
            let metadata = cap_std::fs::Metadata::from_file(verified.file())?;
            if metadata.nlink() != 1 {
                return Err(std::io::Error::other("Catalog cache scratch file changed"));
            }
            directory.rename(&scratch, directory, destination)?;
            if !verified.matches_in(directory, std::path::Path::new(destination))? {
                return Err(std::io::Error::other("Catalog cache publication changed"));
            }
            #[cfg(unix)] crate::storage::sync_cap_directory(directory)?;
            cache.unchanged()?;
            Ok(Stamp::of(&metadata))
        })();
        if result.is_err() {
            // Remove only the scratch inode this call created; a replaced name
            // belongs to someone else and is preserved.
            if let Ok(named) = directory.symlink_metadata(&scratch) {
                if named.is_file() && named.nlink() == 1 && named.dev() == before.dev() && named.ino() == before.ino() {
                    let _ = directory.remove_file(&scratch);
                }
            }
        }
        result
    };
    io().map_err(|e| format!("Persisting catalog cache: {e}"))
}

fn sync_directory(directory: &Dir) -> Result<(), String> {
    #[cfg(unix)] crate::storage::sync_cap_directory(directory).map_err(|e| format!("Synchronizing catalog cache directory: {e}"))?;
    #[cfg(not(unix))] let _ = directory;
    Ok(())
}

#[cfg(test)]
#[path = "catalog_cache_tests.rs"]
mod tests;
