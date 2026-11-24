// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Bounded OPDS/Atom acquisition feeds. Feed URLs and every resolved link require HTTPS.
//! Metalink acquisition retains exact transport metadata; OpenSearch offsets provide
//! pagination when a producer omits an explicit next link without losing filters.
//! Distinct tags are limited to 1,024 per acquisition entry and facets to 16,384
//! per navigation feed; overflow rejects the feed rather than hiding metadata.

use futures_util::StreamExt;
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;
use url::Url;

#[path = "catalog_cache.rs"]
mod cache;
pub use cache::CatalogCache;

#[path = "catalog_index.rs"]
mod index;
pub use index::{CatalogIndex, CatalogSnapshot, SeriesKey, edition_date, edition_is_newer, series_compatible, series_key, source_id};

const ATOM: &str = "http://www.w3.org/2005/Atom";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const MAX_FEED: usize = 16 * 1024 * 1024;
const MAX_ENTRY_TAGS: usize = 1024;
const MAX_FACETS: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: String,
    pub title: String,
    pub description: String,
    pub language: String,
    pub creator: String,
    pub publisher: String,
    pub date: String,
    pub name: String,
    pub tags: Vec<String>,
    pub size: u64,
    pub article_count: u64,
    pub media_count: u64,
    pub url: String,
    pub thumbnail_url: Option<String>,
    /// Explicit feed-provided BitTorrent acquisition, never a guessed sibling URL.
    pub torrent_url: Option<String>,
    /// Official producer endpoint resolved only after an explicit user action.
    #[serde(default)]
    pub torrent_resolver_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPage {
    pub entries: Vec<CatalogEntry>,
    pub next: Option<String>,
    pub total: Option<u64>,
}

pub(crate) fn official_catalog_url(value: &str) -> bool {
    checked_url(value, false).is_ok_and(|url| {
        matches!(url.host_str(), Some("library.kiwix.org" | "opds.library.kiwix.org"))
            && url.port_or_known_default() == Some(443) && url.path() == "/catalog/v2/entries"
    })
}

fn official_download_origin(url: &Url) -> bool {
    matches!(url.host_str(), Some("download.kiwix.org" | "lb.download.kiwix.org"))
        && url.port_or_known_default() == Some(443) && url.path().starts_with("/zim/")
}

#[cfg(any(target_os = "ios", test))]
pub(crate) fn official_artifact_url(value: &str) -> bool {
    checked_url(value, false).is_ok_and(|url| {
        official_download_origin(&url)
            && (url.path().ends_with(".zim") || url.path().ends_with(".zim.meta4"))
    })
}

pub(crate) fn offer_official_torrents(page: &mut CatalogPage, verified_official_source: bool) {
    for entry in &mut page.entries {
        // Cached/custom feeds cannot manufacture producer provenance.
        entry.torrent_resolver_url = None;
        if !verified_official_source || entry.torrent_url.is_some() { continue; }
        let Ok(mut url) = checked_url(&entry.url, false) else { continue; };
        if !official_download_origin(&url) || url.query().is_some() { continue; }
        let path = url.path().strip_suffix(".meta4").unwrap_or(url.path());
        if !path.ends_with(".zim") { continue; }
        // libkiwix's published Download action fetches this endpoint on demand.
        // This is a resolver capability, not proof that a torrent exists.
        let path = format!("{path}.magnet");
        url.set_path(&path);
        entry.torrent_resolver_url = Some(url.into());
    }
}

const MAX_MAGNET: usize = 8192;

fn official_magnet_endpoint(value: &str) -> Result<Url, String> {
    let url = checked_url(value, false)?;
    if !official_download_origin(&url) || url.query().is_some() || !url.path().ends_with(".zim.magnet") {
        return Err("This is not an official BitTorrent download endpoint".into());
    }
    Ok(url)
}

fn producer_magnet(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_MAGNET { return Err("The BitTorrent download link is too large".into()); }
    let text = std::str::from_utf8(bytes).map_err(|_| "The publisher returned an invalid BitTorrent link")?.trim();
    let mut url = Url::parse(text).map_err(|_| "The publisher returned an invalid BitTorrent link")?;
    if url.scheme() != "magnet" || url.has_host() || !url.username().is_empty()
        || url.password().is_some() || url.fragment().is_some() {
        return Err("The publisher returned an invalid BitTorrent link".into());
    }
    let mut hashes = 0;
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url.query_pairs() {
        if key == "xt" {
            // MirrorBrain also supplies an MD5 xt. The engine supports the
            // BitTorrent identity, not that optional non-BitTorrent topic.
            if !value.starts_with("urn:btih:") { continue; }
            hashes += 1;
        }
        query.append_pair(&key, &value);
    }
    if hashes != 1 { return Err("The publisher returned no unambiguous BitTorrent v1 download".into()); }
    url.set_query(Some(&query.finish()));
    librqbit::Magnet::parse(url.as_str()).map_err(|_| "The publisher returned an invalid BitTorrent info hash")?;
    Ok(url.into())
}

pub(crate) async fn resolve_official_torrent(endpoint: &str) -> Result<String, String> {
    crate::transfer_policy::require_internet()?;
    let endpoint = official_magnet_endpoint(endpoint)?;
    let response = http_client(false)?.get(endpoint).timeout(Duration::from_secs(5))
        .send().await.map_err(|error| format!("Could not get the BitTorrent download link: {error}"))?
        .error_for_status().map_err(|_| "The publisher has no BitTorrent download available for this archive")?;
    official_magnet_endpoint(response.url().as_str())?;
    if response.content_length().is_some_and(|size| size > MAX_MAGNET as u64) {
        return Err("The BitTorrent download link is too large".into());
    }
    let mut bytes = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        crate::transfer_policy::require_internet()?;
        let chunk = chunk.map_err(|error| format!("Could not read the BitTorrent download link: {error}"))?;
        if chunk.len() > MAX_MAGNET - bytes.len() {
            return Err("The BitTorrent download link is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    producer_magnet(&bytes)
}

pub(crate) fn checked_url(value: &str, loopback_http: bool) -> Result<Url, String> {
    if value.len() > 8192 {
        return Err("URL exceeds 8192 bytes".into());
    }
    let url = Url::parse(value).map_err(|e| format!("Invalid URL: {e}"))?;
    let local = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.scheme() != "https" && !(loopback_http && url.scheme() == "http" && local) {
        return Err(
            "HTTPS is required (downloads additionally permit literal loopback HTTP fixture URLs)"
                .into(),
        );
    }
    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("URL must have a host and no credentials or fragment".into());
    }
    // Feed-supplied literals must not reach local services. Names are checked
    // against the addresses they actually resolve to by [`PublicDns`], and this
    // also rejects every redirect target because the redirect policy reuses
    // this function. The permissive fixture policy keeps the user-facing
    // download path unchanged: only the HTTPS-only catalog policy is fenced.
    let literal_public = match url.host() {
        Some(url::Host::Ipv4(ip)) => public_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => public_ip(ip.into()),
        Some(url::Host::Domain(_)) | None => true,
    };
    if !literal_public && !loopback_http {
        return Err("URL must be a public internet destination".into());
    }
    Ok(url)
}

/// Public unicast destinations only: loopback, private, link-local, shared,
/// benchmarking and reserved ranges are never reachable from a catalog feed.
fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => public_ipv4(ip),
        std::net::IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_ipv4(mapped);
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                // Documentation (2001:db8::/32).
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

fn public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [first, second, third, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        // "This network" (0/8), shared address space (100.64/10), IETF
        // protocol assignments (192.0.0/24), benchmarking (198.18/15) and
        // reserved (240/4).
        || first == 0
        || (first == 100 && (64..128).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && matches!(second, 18 | 19))
        || first >= 240)
}

/// Resolves catalog hostnames to public addresses only, at connect time: the
/// addresses refused here are the ones the connector would dial, so a DNS
/// answer that changes between validation and use (rebinding) is refused too.
/// Residual: an HTTP proxy configured in the environment resolves the target
/// itself; the literal host check in [`checked_url`] still applies there.
#[derive(Debug)]
struct PublicDns;

impl reqwest::dns::Resolve for PublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(public_addresses(name))
    }
}

async fn public_addresses(
    name: reqwest::dns::Name,
) -> Result<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>> {
    let addrs = tokio::net::lookup_host((name.as_str(), 0)).await?;
    let public: Vec<std::net::SocketAddr> = addrs.filter(|addr| public_ip(addr.ip())).collect();
    if public.is_empty() {
        let error: Box<dyn std::error::Error + Send + Sync> = Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "The catalog host resolves only to private or loopback addresses",
        ));
        return Err(error);
    }
    let addrs: reqwest::dns::Addrs = Box::new(public.into_iter());
    Ok(addrs)
}

pub(crate) fn http_client(loopback_http: bool) -> Result<reqwest::Client, String> {
    http_client_with_redirects(loopback_http, true)
}

/// One retained client per transport policy. The redirect decision and the
/// loopback exception are baked into a built client, so they are part of its
/// identity and clients are never shared across policies. Cloning a client
/// shares its connector pool, which is why it is retained: catalog traversals,
/// thumbnails and torrent resolution reuse warm connections instead of paying a
/// new TCP/TLS handshake per call.
fn http_client_with_redirects(loopback_http: bool, follow_redirects: bool) -> Result<reqwest::Client, String> {
    static HTTPS_REDIRECTING: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| build_http_client(false, true));
    static HTTPS_DIRECT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| build_http_client(false, false));
    static LOOPBACK_REDIRECTING: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| build_http_client(true, true));
    static LOOPBACK_DIRECT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| build_http_client(true, false));
    let shared = match (loopback_http, follow_redirects) {
        (false, true) => &HTTPS_REDIRECTING,
        (false, false) => &HTTPS_DIRECT,
        (true, true) => &LOOPBACK_REDIRECTING,
        (true, false) => &LOOPBACK_DIRECT,
    };
    let shared: &Result<reqwest::Client, String> = shared;
    shared.clone()
}

fn build_http_client(loopback_http: bool, follow_redirects: bool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .user_agent(concat!("Kiwix-Tauri/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if !follow_redirects { return attempt.stop(); }
            if attempt.previous().len() >= 8 {
                return attempt.error("Too many HTTP redirects");
            }
            if checked_url(attempt.url().as_str(), loopback_http).is_err()
                || (attempt.url().scheme() == "http"
                    && attempt.previous().iter().any(|u| u.scheme() == "https"))
            {
                return attempt.error("Unsafe HTTP redirect");
            }
            attempt.follow()
        }));
    // Literal loopback fixture URLs (test transport policy) keep the system
    // resolver; every production client refuses a name that resolves to a
    // local or private address.
    let builder = if loopback_http { builder } else { builder.dns_resolver(std::sync::Arc::new(PublicDns)) };
    builder
        .build()
        .map_err(|e| format!("Cannot construct HTTP client: {e}"))
}

pub(crate) async fn bounded_body(
    response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), response.url()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > maximum as u64)
    {
        return Err(format!("Response exceeds {maximum} bytes"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Reading HTTP response: {e}"))?;
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            return Err(format!("Response exceeds {maximum} bytes"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFacet {
    pub value: String,
    pub label: String,
    pub count: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFacets {
    pub languages: Vec<CatalogFacet>,
    pub categories: Vec<CatalogFacet>,
}

fn accept_language(locale: &str) -> String {
    locale.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect()
}

fn facet_urls(source: &str, loopback: bool) -> Result<(Url, Url), String> {
    let source = checked_url(source, loopback)?;
    let prefix = source.path().rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let mut languages = source.clone();
    languages.set_path(&format!("{prefix}/languages"));
    languages.set_query(None);
    let mut categories = languages.clone();
    categories.set_path(&format!("{prefix}/categories"));
    Ok((languages, categories))
}

fn parse_facets(
    xml: &str,
    base: &Url,
    key: &str,
    locale: &str,
) -> Result<Vec<CatalogFacet>, String> {
    let document = Document::parse(xml).map_err(|e| format!("Malformed facet XML: {e}"))?;
    let feed = document.root_element();
    if !atom(feed, "feed") {
        return Err("Facet root must be an Atom feed".into());
    }
    let mut facets = Vec::new();
    let mut seen = HashSet::new();
    for entry in feed.children().filter(|n| atom(*n, "entry")) {
        let mut value = String::new();
        for link in entry
            .children()
            .filter(|n| atom(*n, "link") && relation(*n, "subsection"))
        {
            if let Some(href) = link.attribute("href") {
                let url = Url::parse(&resolved(link, href, base)?).map_err(|e| e.to_string())?;
                if let Some((_, found)) = url.query_pairs().find(|(name, _)| name == key) {
                    value = found.into_owned();
                    break;
                }
            }
        }
        if value.is_empty() && key == "lang" {
            value = field(entry, &["language"]);
        }
        if value.is_empty() || seen.contains(value.as_str()) {
            continue;
        }
        if facets.len() >= MAX_FACETS {
            return Err(format!("Facet feed exceeds {MAX_FACETS} distinct values"));
        }
        let title = entry
            .children()
            .filter(|n| atom(*n, "title"))
            .max_by_key(|node| match node.attribute((XML, "lang")) {
                Some(language) if language.eq_ignore_ascii_case(locale) => 3,
                Some(language)
                    if language.split('-').next().is_some_and(|l| {
                        l.eq_ignore_ascii_case(locale.split('-').next().unwrap_or(""))
                    }) =>
                {
                    2
                }
                None => 1,
                _ => 0,
            });
        let label = title
            .map(text)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| value.clone());
        let count = entry
            .children()
            .find(|n| {
                n.is_element()
                    && n.tag_name().name() == "count"
                    && n.tag_name().namespace() == Some("http://purl.org/syndication/thread/1.0")
            })
            .and_then(|n| text(n).parse().ok());
        seen.insert(value.clone());
        facets.push(CatalogFacet {
            value,
            label,
            count,
        });
    }
    Ok(facets)
}

const MAX_THUMBNAIL_BYTES: usize = 256 * 1024;
const MAX_THUMBNAILS_IN_FLIGHT: usize = 4;
const MAX_CACHED_THUMBNAILS: usize = 64;

type Thumbnail = Result<Option<Arc<String>>, String>;

#[derive(Default)]
struct ThumbnailState {
    cached: VecDeque<(Arc<str>, Arc<String>)>,
    pending: HashMap<Arc<str>, Weak<ThumbnailFlight>>,
}

struct ThumbnailCache {
    state: Arc<Mutex<ThumbnailState>>,
    network: tokio::sync::Semaphore,
}

struct ThumbnailFlight {
    key: Arc<str>,
    state: Weak<Mutex<ThumbnailState>>,
    result: tokio::sync::OnceCell<Thumbnail>,
}

impl Drop for ThumbnailFlight {
    fn drop(&mut self) {
        let Some(state) = self.state.upgrade() else { return };
        let mut state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending.get(&self.key).is_some_and(|flight| std::ptr::eq(flight.as_ptr(), self)) {
            state.pending.remove(&self.key);
        }
    }
}

impl ThumbnailCache {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ThumbnailState::default())),
            network: tokio::sync::Semaphore::new(MAX_THUMBNAILS_IN_FLIGHT),
        }
    }

    async fn fetch(
        &self, url: &str, loopback: bool, admit: impl Fn() -> Result<(), String>,
    ) -> Result<Option<String>, String> {
        admit()?;
        let url = checked_url(url, loopback)?;
        let flight = {
            let mut state = self.state.lock().map_err(|_| "Thumbnail cache unavailable")?;
            if let Some(index) = state.cached.iter().rposition(|(key, _)| &**key == url.as_str()) {
                let item = state.cached.remove(index).expect("Known cached thumbnail");
                let value = Arc::clone(&item.1);
                state.cached.push_back(item);
                drop(state);
                return Ok(Some(value.as_ref().clone()));
            }
            if let Some(flight) = state.pending.get(url.as_str()).and_then(Weak::upgrade) {
                flight
            } else {
                let key: Arc<str> = Arc::from(url.as_str());
                let flight = Arc::new(ThumbnailFlight {
                    key: key.clone(), state: Arc::downgrade(&self.state), result: tokio::sync::OnceCell::new(),
                });
                state.pending.insert(key, Arc::downgrade(&flight));
                flight
            }
        };
        // Equal URLs wait on the same initializer. Retirement of its caller
        // does not publish a response or cache its admission error for others.
        let result = flight.result.get_or_try_init(|| async {
            let _permit = self.network.acquire().await.map_err(|_| "Thumbnail proxy is unavailable".to_owned())?;
            admit()?;
            let result: Thumbnail = async {
                let response = http_client(loopback)?.get(url).timeout(Duration::from_secs(60)).send().await
                    .map_err(|e| format!("Fetching catalog thumbnail: {e}"))?;
                let bytes = bounded_body(response, MAX_THUMBNAIL_BYTES).await?;
                Ok(raster_data_url(&bytes).map(Arc::new))
            }.await;
            admit()?;
            if let Ok(Some(value)) = &result {
                let mut state = self.state.lock().map_err(|_| "Thumbnail cache unavailable".to_owned())?;
                if state.cached.len() == MAX_CACHED_THUMBNAILS { state.cached.pop_front(); }
                state.cached.push_back((flight.key.clone(), value.clone()));
            }
            Ok::<_, String>(result)
        }).await?;
        admit()?;
        result.as_ref().map(|value| value.as_ref().map(|value| value.as_ref().clone())).map_err(Clone::clone)
    }
}

/// Native-only, bounded raster proxy: untrusted feeds never gain a browser request.
/// The request has no user locale header, so its URL fully identifies the image input.
pub async fn fetch_thumbnail(url: &str, admit: impl Fn() -> Result<(), String>) -> Result<Option<String>, String> {
    static CACHE: LazyLock<ThumbnailCache> = LazyLock::new(ThumbnailCache::new);
    CACHE.fetch(url, false, admit).await
}

/// Bounded, inert image payload shared by catalog and offline article previews.
pub(crate) fn raster_data_url(bytes: &[u8]) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    if bytes.len() > MAX_THUMBNAIL_BYTES {
        return None;
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        "image/webp"
    } else {
        return None;
    };
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn atom(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(ATOM)
}

fn text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn field(node: Node<'_, '_>, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| {
            node.children()
                .find(|n| {
                    n.is_element()
                        && n.tag_name().name() == *name
                        && allowed_metadata_namespace(n.tag_name().namespace())
                })
                .map(text)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default()
}

fn people(node: Node<'_, '_>, names: &[&str]) -> String {
    for name in names {
        let values: Vec<_> = node
            .children()
            .filter(|n| {
                n.is_element()
                    && n.tag_name().name() == *name
                    && allowed_metadata_namespace(n.tag_name().namespace())
            })
            .map(|person| {
                let name = field(person, &["name"]);
                if name.is_empty() {
                    text(person)
                } else {
                    name
                }
            })
            .filter(|name| !name.is_empty())
            .collect();
        if !values.is_empty() {
            return values.join(", ");
        }
    }
    String::new()
}

fn allowed_metadata_namespace(namespace: Option<&str>) -> bool {
    matches!(
        namespace,
        None | Some(ATOM)
            | Some("http://purl.org/dc/terms/")
            | Some("http://purl.org/dc/elements/1.1/")
            | Some("http://www.kiwix.org/metadata")
            | Some("http://kiwix.org/metadata")
    )
}

fn numeric(node: Node<'_, '_>, names: &[&str]) -> Result<u64, String> {
    let value = field(node, names);
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse::<u64>()
        .map_err(|_| format!("Invalid {} count: {value}", names[0]))
}

fn resolved(node: Node<'_, '_>, href: &str, base: &Url) -> Result<String, String> {
    // Literal loopback HTTP feeds exist only in native transport regression
    // fixtures. HTTPS production feeds cannot opt into this exception.
    let loopback = cfg!(test) && base.scheme() == "http" && checked_url(base.as_str(), true).is_ok();
    let mut current = base.clone();
    let mut ancestry: Vec<_> = node.ancestors().filter(|n| n.is_element()).collect();
    ancestry.reverse();
    for ancestor in ancestry {
        if let Some(relative) = ancestor.attribute((XML, "base")) {
            current = current
                .join(relative)
                .map_err(|e| format!("Invalid xml:base: {e}"))?;
            checked_url(current.as_str(), loopback)?;
        }
    }
    let url = current
        .join(href)
        .map_err(|e| format!("Invalid OPDS link: {e}"))?;
    Ok(checked_url(url.as_str(), loopback)?.to_string())
}

fn relation(node: Node<'_, '_>, value: &str) -> bool {
    node.attribute("rel")
        .unwrap_or("")
        .split_ascii_whitespace()
        .any(|rel| rel == value)
}

fn acquisition(node: Node<'_, '_>) -> bool {
    if !atom(node, "link") {
        return false;
    }
    let mime = node
        .attribute("type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    let rel = node.attribute("rel").unwrap_or("");
    let archive = mime.eq_ignore_ascii_case("application/x-zim")
        || (metalink_link(node)
            && (mime.eq_ignore_ascii_case("application/metalink4+xml")
                || mime.eq_ignore_ascii_case("application/metalink+xml")));
    archive
        && (rel.is_empty()
            || rel.split_ascii_whitespace().any(|r| {
                [
                    "http://opds-spec.org/acquisition",
                    "https://opds-spec.org/acquisition",
                ]
                .iter()
                .any(|prefix| {
                    r == *prefix
                        || r.strip_prefix(*prefix)
                            .is_some_and(|rest| rest.starts_with('/'))
                })
            }))
}

fn metalink_link(node: Node<'_, '_>) -> bool {
    let path = node
        .attribute("href")
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    path.ends_with(".meta4") || path.ends_with(".metalink")
}

fn page_number(feed: Node<'_, '_>, name: &str) -> Result<Option<u64>, String> {
    feed.children()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == name
                && matches!(
                    n.tag_name().namespace(),
                    Some(ATOM)
                        | Some("http://a9.com/-/spec/opensearch/1.1/")
                        | Some("http://a9.com/-/spec/opensearch/1.0/")
                )
        })
        .map(|n| {
            text(n)
                .parse::<u64>()
                .map_err(|_| format!("Invalid OPDS {name}"))
        })
        .transpose()
}

fn parse_catalog(xml: &str, base: &Url) -> Result<CatalogPage, String> {
    // roxmltree rejects DTDs by default, so external entities are never fetched or expanded.
    let document = Document::parse(xml).map_err(|e| format!("Malformed OPDS XML: {e}"))?;
    let feed = document.root_element();
    if !atom(feed, "feed") {
        return Err("Catalog root must be an Atom feed".into());
    }
    // Feed-level metadata is the fallback for every entry, so resolve each one
    // once. Rescanning the feed's direct children per entry made parsing
    // quadratic in the page size.
    let feed_language = field(feed, &["language"]);
    let feed_creator = people(feed, &["author", "creator"]);
    let feed_publisher = people(feed, &["publisher"]);
    let mut entries = Vec::new();
    for entry in feed.children().filter(|n| atom(*n, "entry")) {
        let links: Vec<_> = entry.children().filter(|n| acquisition(*n)).collect();
        // Navigation entries do not represent downloadable archives.
        if links.is_empty() {
            continue;
        }
        // Metalink supplies the exact archive size/hash; OPDS display lengths can
        // be rounded, and must not reject a correctly sized archive response.
        let link = links
            .iter()
            .find(|n| metalink_link(**n))
            .copied()
            .unwrap_or(links[0]);
        let href = link
            .attribute("href")
            .filter(|v| !v.is_empty())
            .ok_or("Acquisition link has no href")?;
        let url = resolved(link, href, base)?;
        let id = field(entry, &["id"]);
        let title = field(entry, &["title"]);
        if id.is_empty() || title.is_empty() {
            return Err("Acquisition entry requires an id and title".into());
        }
        let thumbnail = entry.children().find(|n| {
            atom(*n, "link")
                && [
                    "http://opds-spec.org/image/thumbnail",
                    "https://opds-spec.org/image/thumbnail",
                    "http://opds-spec.org/image",
                    "https://opds-spec.org/image",
                ]
                .iter()
                .any(|rel| relation(*n, rel))
        });
        let thumbnail_url = thumbnail
            .map(|n| {
                resolved(
                    n,
                    n.attribute("href").ok_or("Image link has no href")?,
                    base,
                )
            })
            .transpose()?;
        let torrent_url = entry
            .children()
            .find(|n| {
                if !atom(*n, "link") {
                    return false;
                }
                let mime = n
                    .attribute("type")
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim();
                mime.eq_ignore_ascii_case("application/x-bittorrent")
                    || (n.attribute("href").unwrap_or("").starts_with("magnet:")
                        && n.attribute("rel")
                            .unwrap_or("")
                            .split_ascii_whitespace()
                            .any(|rel| {
                                rel.starts_with("http://opds-spec.org/acquisition")
                                    || rel.starts_with("https://opds-spec.org/acquisition")
                            }))
            })
            .map(|n| {
                let href = n
                    .attribute("href")
                    .ok_or("Torrent acquisition has no href")?;
                if href.starts_with("magnet:") {
                    let magnet =
                        Url::parse(href).map_err(|e| format!("Invalid magnet URL: {e}"))?;
                    if href.len() > 8192
                        || magnet.fragment().is_some()
                        || !magnet.query_pairs().any(|(key, value)| {
                            key == "xt"
                                && (value.starts_with("urn:btih:")
                                    || value.starts_with("urn:btmh:"))
                        })
                    {
                        return Err("Invalid magnet acquisition link".into());
                    }
                    Ok(magnet.to_string())
                } else {
                    resolved(n, href, base)
                }
            })
            .transpose()?;
        let raw_tags = field(entry, &["tags"]);
        let mut tags = Vec::new();
        let mut seen_tags = HashSet::new();
        for tag in raw_tags
            .split(';')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .chain(
                entry
                    .children()
                    .filter(|n| atom(*n, "category"))
                    .filter_map(|category| category.attribute("term"))
                    .filter(|term| !term.is_empty()),
            )
        {
            if seen_tags.insert(tag) {
                if tags.len() >= MAX_ENTRY_TAGS {
                    return Err(format!(
                        "Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags"
                    ));
                }
                tags.push(tag.to_string());
            }
        }
        for (field_name, prefix) in [("flavour", "_flavour:"), ("category", "_category:")] {
            let value = field(entry, &[field_name]);
            if !value.is_empty() {
                let tag = format!("{prefix}{value}");
                if !seen_tags.contains(tag.as_str()) {
                    if tags.len() >= MAX_ENTRY_TAGS {
                        return Err(format!(
                            "Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags"
                        ));
                    }
                    tags.push(tag);
                }
            }
        }
        let size = match link.attribute("length") {
            Some(value) => value
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("Invalid acquisition length: {value}"))?,
            None => numeric(entry, &["size"])?,
        };
        let mut language = field(entry, &["language"]);
        if language.is_empty() {
            language = entry
                .attribute((XML, "lang"))
                .or_else(|| feed.attribute((XML, "lang")))
                .map(str::to_string)
                .unwrap_or_else(|| feed_language.clone());
        }
        let mut creator = people(entry, &["author", "creator"]);
        if creator.is_empty() {
            creator = feed_creator.clone();
        }
        let mut publisher = people(entry, &["publisher"]);
        if publisher.is_empty() {
            publisher = feed_publisher.clone();
        }
        entries.push(CatalogEntry {
            id,
            title,
            description: field(entry, &["summary", "content", "description"]),
            language,
            creator,
            publisher,
            date: field(entry, &["issued", "date", "updated"]),
            name: field(entry, &["name"]),
            tags,
            size,
            article_count: numeric(entry, &["articleCount", "article_count"])?,
            media_count: numeric(entry, &["mediaCount", "media_count"])?,
            url,
            thumbnail_url,
            torrent_url,
            torrent_resolver_url: None,
        });
    }
    let mut next = feed
        .children()
        .find(|n| atom(*n, "link") && relation(*n, "next"))
        .map(|n| resolved(n, n.attribute("href").ok_or("Next link has no href")?, base))
        .transpose()?;
    let total = page_number(feed, "totalResults")?;
    if next.is_none() {
        // Kiwix's default feed publishes OpenSearch offsets, but no rel=next.
        // Keep the request's filters/count and replace only its pagination offset.
        if let (Some(total), Some(start), Some(count)) = (
            total,
            page_number(feed, "startIndex")?,
            page_number(feed, "itemsPerPage")?,
        ) {
            if let Some(offset) = start
                .checked_add(count)
                .filter(|offset| count > 0 && *offset < total)
            {
                let parameters: Vec<_> = base
                    .query_pairs()
                    .filter(|(key, _)| key != "start")
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect();
                let mut url = base.clone();
                url.set_query(None);
                url.query_pairs_mut()
                    .extend_pairs(parameters)
                    .append_pair("start", &offset.to_string());
                next = Some(url.to_string());
            }
        }
    }
    Ok(CatalogPage {
        entries,
        next,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simultaneous_thumbnail_failures_share_one_request_and_allow_retry() {
        use axum::{http::StatusCode, routing::get, Router};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let requests = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let app = Router::new().route("/image", get({
            let (requests, started, release) = (requests.clone(), started.clone(), release.clone());
            move || {
                let (requests, started, release) = (requests.clone(), started.clone(), release.clone());
                async move {
                    if requests.fetch_add(1, Ordering::SeqCst) == 0 {
                        started.notify_one();
                        release.notified().await;
                        (StatusCode::SERVICE_UNAVAILABLE, &b"unavailable"[..])
                    } else {
                        (StatusCode::OK, &b"\x89PNG\r\n\x1a\n"[..])
                    }
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let cache = ThumbnailCache::new();
        let wait_for_flight = async {
            started.notified().await;
            let state = cache.state.lock().unwrap();
            assert_eq!(state.pending[url.as_str()].strong_count(), 2);
            release.notify_one();
        };
        let (first, second, ()) = tokio::join!(cache.fetch(&url, true, || Ok(())), cache.fetch(&url, true, || Ok(())), wait_for_flight);
        assert!(first.as_ref().unwrap_err().contains("503"));
        assert_eq!(first, second);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        let retry = cache.fetch(&url, true, || Ok(())).await.unwrap().unwrap();
        assert!(retry.starts_with("data:image/png;base64,"));
        assert_eq!(cache.fetch(&url, true, || Ok(())).await.unwrap(), Some(retry));
        assert_eq!(requests.load(Ordering::SeqCst), 2);

        let queued = format!("{url}?queued");
        let permits = cache.network.acquire_many(MAX_THUMBNAILS_IN_FLIGHT as u32).await.unwrap();
        let retired = std::cell::Cell::new(false);
        let mut request = std::pin::pin!(cache.fetch(&queued, true, || {
            if retired.get() { Err("Thumbnail request retired".into()) } else { Ok(()) }
        }));
        assert!(futures_util::poll!(&mut request).is_pending());
        retired.set(true);
        drop(permits);
        assert_eq!(request.await.unwrap_err(), "Thumbnail request retired");
        assert_eq!(requests.load(Ordering::SeqCst), 2, "retired queued work must not fetch");
        server.abort();
    }

    #[test]
    fn official_torrent_offers_require_provenance_and_an_official_artifact() {
        let base = Url::parse("https://library.kiwix.org/catalog/v2/entries").unwrap();
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
          <entry><id>official</id><title>Official</title>
            <link type="application/x-zim" href="https://lb.download.kiwix.org/zim/test/book.zim.meta4"/></entry>
          <entry><id>foreign</id><title>Foreign</title>
            <link type="application/x-zim" href="https://custom.example/book.zim"/></entry>
          <entry><id>explicit</id><title>Explicit</title>
            <link type="application/x-zim" href="https://download.kiwix.org/zim/test/explicit.zim"/>
            <link type="application/x-bittorrent" href="https://custom.example/explicit.torrent"/></entry>
        </feed>"#;
        let mut page = parse_catalog(xml, &base).unwrap();
        offer_official_torrents(&mut page, false);
        assert!(page.entries.iter().all(|entry| entry.torrent_resolver_url.is_none()));
        offer_official_torrents(&mut page, true);
        assert_eq!(page.entries[0].torrent_resolver_url.as_deref(),
            Some("https://lb.download.kiwix.org/zim/test/book.zim.magnet"));
        assert!(page.entries[1].torrent_resolver_url.is_none());
        assert!(page.entries[2].torrent_resolver_url.is_none());
        assert_eq!(page.entries[2].torrent_url.as_deref(), Some("https://custom.example/explicit.torrent"));
        offer_official_torrents(&mut page, false);
        assert!(page.entries.iter().all(|entry| entry.torrent_resolver_url.is_none()));
    }

    #[test]
    fn torrent_resolver_rejects_foreign_origins_and_nonproducer_routes() {
        for endpoint in [
            "http://download.kiwix.org/zim/book.zim.magnet",
            "https://download.kiwix.org.evil.example/zim/book.zim.magnet",
            "https://download.kiwix.org:444/zim/book.zim.magnet",
            "https://user@download.kiwix.org/zim/book.zim.magnet",
            "https://download.kiwix.org/other/book.zim.magnet",
            "https://download.kiwix.org/zim/book.zim.magnet?redirect=other",
            "https://download.kiwix.org/zim/book.zim.magnet#fragment",
            "https://download.kiwix.org/zim/book.zim.torrent",
        ] {
            assert!(official_magnet_endpoint(endpoint).is_err(), "{endpoint}");
        }
        assert!(official_magnet_endpoint("https://lb.download.kiwix.org/zim/test/book.zim.magnet").is_ok());
    }

    #[test]
    fn producer_magnet_keeps_engine_identity_and_trackers_without_md5_topics() {
        let hash = "0b9b5075708dbfa9fe2237967c714cf3b9b34687";
        let response = format!("magnet:?xt=urn:btih:{hash}&xt=urn:md5:e595035d0a5c12a57db33c209509d8e9&dn=book.zim&tr=https%3A%2F%2Ftracker.openzim.org%2Fannounce\n");
        let resolved = producer_magnet(response.as_bytes()).unwrap();
        let parsed = librqbit::Magnet::parse(&resolved).unwrap();
        assert_eq!(parsed.as_id20().unwrap().as_string(), hash);
        assert_eq!(parsed.trackers, vec!["https://tracker.openzim.org/announce"]);
        assert_eq!(parsed.name.as_deref(), Some("book.zim"));
    }

    #[test]
    fn producer_magnet_rejects_invalid_ambiguous_and_oversized_responses() {
        for response in [
            "<html>Not found</html>",
            "https://custom.example/book.torrent",
            "magnet:?xt=urn:md5:1234",
            "magnet:?xt=urn:btih:not-a-hash",
            "magnet:?xt=urn:btih:0b9b5075708dbfa9fe2237967c714cf3b9b34687&xt=urn:btih:1111111111111111111111111111111111111111",
        ] {
            assert!(producer_magnet(response.as_bytes()).is_err(), "{response}");
        }
        assert!(producer_magnet(&vec![b'x'; MAX_MAGNET + 1]).is_err());
    }

    #[test]
    fn preserves_explicit_service_worker_requirements_on_every_feed_page() {
        let base = Url::parse("https://custom.example/catalog?start=48").unwrap();
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:m="http://www.kiwix.org/metadata">
          <link rel="next" href="?start=96"/>
          <entry><id>legacy</id><title>Legacy replay</title><m:tags>_sw:yes;_category:web</m:tags>
            <link type="application/x-zim" href="/legacy.zim"/></entry>
          <entry><id>modern</id><title>Zimit 2</title><category term="_sw:no"/>
            <link type="application/x-zim" href="/zimit2.zim"/></entry>
          <entry><id>unknown</id><title>Unknown requirements</title>
            <link type="application/x-zim" href="/sw-only-name.zim"/></entry>
        </feed>"#;
        let page = parse_catalog(xml, &base).unwrap();
        assert!(page.entries[0].tags.iter().any(|tag| tag == "_sw:yes"));
        assert_eq!(page.entries[1].tags, vec!["_sw:no"]);
        assert!(page.entries[2].tags.is_empty());
        assert_eq!(page.next.as_deref(), Some("https://custom.example/catalog?start=96"));
    }

    #[test]
    fn preserves_namespaces_base_pagination_and_large_lengths() {
        let xml = r#"<a:feed xmlns:a="http://www.w3.org/2005/Atom" xmlns:d="http://purl.org/dc/terms/" xmlns:o="http://a9.com/-/spec/opensearch/1.1/" xml:base="/catalog/">
          <o:totalResults>4294967300</o:totalResults><a:link rel="next" href="?start=2"/>
          <a:entry xml:base="../zim/"><a:id>urn:uuid:abc</a:id><a:title>École</a:title><d:language>fra</d:language><a:articleCount>5000000000</a:articleCount>
          <a:link rel="http://opds-spec.org/acquisition/open-access" type="application/x-zim" href="school.zim" length="9000000000"/></a:entry></a:feed>"#;
        let page = parse_catalog(xml, &Url::parse("https://example.org/start").unwrap()).unwrap();
        assert_eq!(page.entries[0].id, "urn:uuid:abc");
        assert_eq!(page.entries[0].url, "https://example.org/zim/school.zim");
        assert_eq!(page.entries[0].language, "fra");
        assert_eq!(page.entries[0].size, 9_000_000_000);
        assert_eq!(page.entries[0].article_count, 5_000_000_000);
        assert_eq!(page.total, Some(4_294_967_300));
        assert_eq!(
            page.next.as_deref(),
            Some("https://example.org/catalog/?start=2")
        );
    }

    #[test]
    fn rejects_file_links_and_remote_plaintext() {
        assert!(checked_url("file:///tmp/a.zim", true).is_err());
        assert!(checked_url("http://example.org/a.zim", true).is_err());
        assert!(checked_url("http://127.0.0.1:4321/a.zim", true).is_ok());
        assert!(checked_url("http://127.0.0.1/a.zim", false).is_err());
        assert!(checked_url("https://name:secret@example.org/a.zim", true).is_err());
    }

    #[test]
    fn rejects_literal_local_destinations_but_keeps_public_catalog_urls() {
        for url in [
            "https://127.0.0.1/a.zim",
            "https://[::1]/a.zim",
            "https://10.1.2.3/a.zim",
            "https://172.16.0.9/a.zim",
            "https://192.168.1.1/a.zim",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/a.zim",
            "https://198.18.0.1/a.zim",
            "https://[fd00::1]/a.zim",
            "https://[fe80::1]/a.zim",
            "https://[::ffff:127.0.0.1]/a.zim",
        ] {
            assert!(checked_url(url, false).is_err(), "{url}");
        }
        for url in [
            "https://library.kiwix.org/catalog/v2/entries",
            "https://1.1.1.1/a.zim",
            "https://[2606:4700:4700::1111]/a.zim",
            "https://example.org:8443/catalog?start=48",
        ] {
            assert!(checked_url(url, false).is_ok(), "{url}");
        }
        // The permissive fixture/download policy keeps its previous reach; only
        // the HTTPS-only catalog policy is fenced.
        assert!(checked_url("https://192.168.1.5/a.zim", true).is_ok());
    }

    #[test]
    fn derives_kiwix_offsets_without_losing_filters_or_overriding_explicit_next() {
        let feed = |start: u64, count: u64, total: u64, link: &str| {
            format!(
                r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:o="http://a9.com/-/spec/opensearch/1.1/"><o:totalResults>{total}</o:totalResults><o:startIndex>{start}</o:startIndex><o:itemsPerPage>{count}</o:itemsPerPage>{link}</feed>"#
            )
        };
        let base = Url::parse(
            "https://library.kiwix.org/catalog/v2/entries?count=48&q=water+birds&lang=fra&start=0",
        )
        .unwrap();
        let page = parse_catalog(&feed(0, 48, 3658, ""), &base).unwrap();
        let next = Url::parse(page.next.as_deref().unwrap()).unwrap();
        let pairs: std::collections::BTreeMap<_, _> = next.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("start").map(String::as_str), Some("48"));
        assert_eq!(pairs.get("count").map(String::as_str), Some("48"));
        assert_eq!(pairs.get("q").map(String::as_str), Some("water birds"));
        assert_eq!(pairs.get("lang").map(String::as_str), Some("fra"));
        assert!(parse_catalog(&feed(3648, 48, 3658, ""), &base)
            .unwrap()
            .next
            .is_none());
        assert!(parse_catalog(&feed(0, 0, 3658, ""), &base)
            .unwrap()
            .next
            .is_none());
        let page = parse_catalog(
            &feed(0, 48, 3658, r#"<link rel="next" href="?cursor=opaque"/>"#),
            &base,
        )
        .unwrap();
        assert_eq!(
            page.next.as_deref(),
            Some("https://library.kiwix.org/catalog/v2/entries?cursor=opaque")
        );
    }

    #[test]
    fn prefers_exact_metalink_metadata_over_rounded_archive_length() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>book</id><title>Book</title>
            <link rel="http://opds-spec.org/acquisition" type="application/x-zim" href="book.zim" length="100"/>
            <link rel="http://opds-spec.org/acquisition" type="application/metalink4+xml" href="book.zim.meta4" length="100"/>
            </entry></feed>"#;
        let page = parse_catalog(xml, &Url::parse("https://example.org/catalog").unwrap()).unwrap();
        assert_eq!(page.entries[0].url, "https://example.org/book.zim.meta4");
    }

    #[test]
    fn kiwix_facets_use_subsection_values_and_translated_titles() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:thr="http://purl.org/syndication/thread/1.0">
            <entry><id>opaque-uuid</id><title>wikipedia</title><title xml:lang="fr">Encyclopédies</title>
            <thr:count>42</thr:count><link rel="subsection" href="entries?category=wikipedia"/></entry>
            <entry><title>Duplicate</title><link rel="subsection" href="entries?category=wikipedia"/></entry>
            </feed>"#;
        let facets = parse_facets(
            xml,
            &Url::parse("https://example.org/catalog/v2/categories").unwrap(),
            "category",
            "fr-CA",
        )
        .unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(facets[0].value, "wikipedia");
        assert_eq!(facets[0].label, "Encyclopédies");
        assert_eq!(facets[0].count, Some(42));
    }

    #[test]
    fn facet_languages_fall_back_to_language_metadata_and_reject_unsafe_links() {
        let base = Url::parse("https://example.org/catalog/v2/languages").unwrap();
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/">
            <entry><title>العربية</title><dc:language>ara</dc:language></entry></feed>"#;
        let facets = parse_facets(xml, &base, "lang", "en").unwrap();
        assert_eq!(facets[0].value, "ara");
        assert_eq!(facets[0].label, "العربية");
        let unsafe_xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>Unsafe</title>
            <link rel="subsection" href="file:///tmp/entries?category=unsafe"/></entry></feed>"#;
        assert!(parse_facets(unsafe_xml, &base, "category", "en").is_err());
        assert!(parse_facets("<html/>", &base, "lang", "en").is_err());
    }

    #[test]
    fn catalog_preserves_flavor_and_category_without_duplicate_tags() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>book</id><title>Book</title>
            <flavour>nopic</flavour><category>wikipedia</category><tags>_category:wikipedia;_pictures:no</tags>
            <link type="application/x-zim" href="book.zim"/></entry></feed>"#;
        let page = parse_catalog(xml, &Url::parse("https://example.org/entries").unwrap()).unwrap();
        assert!(page.entries[0].tags.contains(&"_flavour:nopic".to_string()));
        assert_eq!(
            page.entries[0]
                .tags
                .iter()
                .filter(|tag| *tag == "_category:wikipedia")
                .count(),
            1
        );
    }

    fn acquisition_feed(metadata: &str) -> String {
        format!(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>book</id><title>Book</title>
            {metadata}<link type="application/x-zim" href="book.zim"/></entry></feed>"#
        )
    }

    #[test]
    fn catalog_tag_deduplication_preserves_first_encounter_order() {
        let xml = acquisition_feed(
            r#"<tags>zeta; alpha;zeta;_category:reference</tags>
            <category term="alpha"/><category term="beta"/><category term="zeta"/>
            <flavour>nopic</flavour>"#,
        );
        let page =
            parse_catalog(&xml, &Url::parse("https://example.org/entries").unwrap()).unwrap();
        assert_eq!(
            page.entries[0].tags,
            [
                "zeta",
                "alpha",
                "_category:reference",
                "beta",
                "_flavour:nopic"
            ]
        );
    }

    #[test]
    fn catalog_category_limit_rejects_the_feed_instead_of_returning_partial_tags() {
        let base = Url::parse("https://example.org/entries").unwrap();
        let mut categories: String = (0..MAX_ENTRY_TAGS)
            .map(|index| format!(r#"<category term="tag-{index}"/>"#))
            .collect();
        // Repeating an existing value at the boundary consumes no additional slot.
        categories.push_str(r#"<category term="tag-0"/>"#);
        let page = parse_catalog(&acquisition_feed(&categories), &base).unwrap();
        assert_eq!(page.entries[0].tags.len(), MAX_ENTRY_TAGS);
        assert_eq!(page.entries[0].tags.first().unwrap(), "tag-0");
        assert_eq!(
            page.entries[0].tags.last().unwrap(),
            &format!("tag-{}", MAX_ENTRY_TAGS - 1)
        );
        categories.push_str(r#"<category term="overflow"/>"#);
        assert_eq!(
            parse_catalog(&acquisition_feed(&categories), &base).unwrap_err(),
            format!("Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags")
        );
    }

    #[test]
    fn catalog_metadata_tags_and_derived_tags_share_the_category_limit() {
        let base = Url::parse("https://example.org/entries").unwrap();
        let tags = (0..MAX_ENTRY_TAGS)
            .map(|index| format!("tag-{index}"))
            .collect::<Vec<_>>()
            .join(";");
        let metadata = format!("<tags>{tags};tag-0</tags>");
        assert_eq!(
            parse_catalog(&acquisition_feed(&metadata), &base)
                .unwrap()
                .entries[0]
                .tags
                .len(),
            MAX_ENTRY_TAGS
        );
        for extra in [
            "<tags>unused</tags><category term=\"overflow\"/>",
            "<flavour>nopic</flavour>",
            "<category>reference</category>",
        ] {
            assert_eq!(
                parse_catalog(&acquisition_feed(&format!("{metadata}{extra}")), &base).unwrap_err(),
                format!("Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags")
            );
        }
        assert_eq!(
            parse_catalog(
                &acquisition_feed(&format!("<tags>{tags};overflow</tags>")),
                &base
            )
            .unwrap_err(),
            format!("Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags")
        );
    }

    #[test]
    fn facet_deduplication_keeps_first_metadata_and_encounter_order() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:thr="http://purl.org/syndication/thread/1.0">
            <entry><title>First Z</title><thr:count>7</thr:count><link rel="subsection" href="?category=z"/></entry>
            <entry><title>First A</title><link rel="subsection" href="?category=a"/></entry>
            <entry><title>Replacement Z</title><thr:count>99</thr:count><link rel="subsection" href="?category=z"/></entry>
            </feed>"#;
        let facets = parse_facets(
            xml,
            &Url::parse("https://example.org/categories").unwrap(),
            "category",
            "en",
        )
        .unwrap();
        assert_eq!(
            facets
                .iter()
                .map(|facet| facet.value.as_str())
                .collect::<Vec<_>>(),
            ["z", "a"]
        );
        assert_eq!(facets[0].label, "First Z");
        assert_eq!(facets[0].count, Some(7));
        assert_eq!(facets[1].label, "First A");
    }

    #[test]
    fn facet_limit_rejects_the_feed_instead_of_returning_partial_facets() {
        let base = Url::parse("https://example.org/facets").unwrap();
        for key in ["category", "lang"] {
            let mut entries: String = (0..MAX_FACETS)
                .map(|index| {
                    format!(
                        r#"<entry><link rel="subsection" href="?{key}=value-{index}"/></entry>"#
                    )
                })
                .collect();
            entries.push_str(&format!(
                r#"<entry><link rel="subsection" href="?{key}=value-0"/></entry>"#
            ));
            let feed = |entries: &str| {
                format!(r#"<feed xmlns="http://www.w3.org/2005/Atom">{entries}</feed>"#)
            };
            let facets = parse_facets(&feed(&entries), &base, key, "en").unwrap();
            assert_eq!(facets.len(), MAX_FACETS);
            assert_eq!(facets.first().unwrap().value, "value-0");
            assert_eq!(
                facets.last().unwrap().value,
                format!("value-{}", MAX_FACETS - 1)
            );
            entries.push_str(&format!(
                r#"<entry><link rel="subsection" href="?{key}=overflow"/></entry>"#
            ));
            assert_eq!(
                parse_facets(&feed(&entries), &base, key, "en").unwrap_err(),
                format!("Facet feed exceeds {MAX_FACETS} distinct values")
            );
        }
    }
}
