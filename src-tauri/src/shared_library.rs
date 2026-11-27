// SPDX-License-Identifier: GPL-3.0-or-later
//! Ephemeral selected-peer QUIC receipts. Durable URLs are correlation, never authority.
#![forbid(unsafe_code)]
use crate::acquisition::{DownloadManager, DownloadSnapshot};
use crate::localswarm::{library::{self, LibraryPeerDescriptor}, LocalSwarmManager};
use crate::store::Store;
use crate::transfer_policy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Exact durable correlation only. Reopening this URL cannot restore a selected route.
pub(crate) fn download_identity(session: &str, peer: &str, book: &str) -> String {
    let mut url = Url::parse("kiwix-library://public/").expect("constant library URL");
    url.path_segments_mut().expect("library URL has path segments")
        .push(session).push(peer).push(book);
    url.to_string()
}

/// How long an open list is kept when room is needed for a newer one. Age is
/// never authority: a list left open stays usable for as long as its grant
/// passes `check()`, which every admission and request repeats.
const RECEIPT_TTL: Duration = Duration::from_secs(300);
/// Revocation poll for a receipt that is not serving a request. Every admission
/// and every request re-checks the grant, so an idle receipt only has to stop
/// being reusable; a request in flight is watched by `denied()` at the active
/// cadence and interrupts within that window.
const IDLE_REVOCATION_POLL: Duration = Duration::from_secs(2);
const MAX_RECEIPTS: usize = 64;
const REVOKED: &str = "Shared library changed or disconnected; browse it again";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedBook {
    pub id: String,
    pub title: String,
    pub file_name: String,
    pub size: u64,
    #[serde(default)]
    pub download_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLibraryBrowse {
    pub receipt_id: String,
    pub session_id: String,
    pub name: String,
    /// Shared with the retained receipt: one validated inventory, not a copy
    /// per open browse.
    pub books: Arc<[SharedBook]>,
}

struct Receipt {
    created: Instant,
    grant: ScopedDownloadGrant,
    books: Arc<[SharedBook]>,
}

pub struct SharedLibraryClient {
    store: Arc<Store>,
    discovery: LocalSwarmManager,
    state: Mutex<(CancellationToken, HashMap<String, Receipt>)>,
}

fn require_store(store: &Store) -> Result<(), String> {
    if store.private_mode()? || store.read_only()? {
        return Err("Shared library access is unavailable in private or read-only mode".into());
    }
    Ok(())
}

impl SharedLibraryClient {
    pub fn new(store: Arc<Store>, discovery: LocalSwarmManager) -> Self {
        Self { store, discovery, state: Mutex::new((CancellationToken::new(), HashMap::new())) }
    }

    /// Retire all selected peers, including paused jobs holding a receipt grant.
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.0.cancel();
        state.0 = CancellationToken::new();
        state.1.clear();
    }

    /// Resolve the selected current discovery record, never an address supplied by UI.
    pub async fn browse(&self, peer_id: &str) -> Result<SharedLibraryBrowse, String> {
        require_store(&self.store)?;
        let peer = self.discovery.library_peer(peer_id).await?;
        let descriptor = LibraryPeerDescriptor::from_peer(&peer)?;
        self.browse_peer(descriptor, Some(peer.id)).await
    }

    /// The public-link controller supplies its current selected descriptor and owner token.
    pub(crate) async fn browse_selected(&self, descriptor: LibraryPeerDescriptor) -> Result<SharedLibraryBrowse, String> {
        self.browse_peer(descriptor, None).await
    }

    async fn browse_peer(&self, descriptor: LibraryPeerDescriptor, discovery_id: Option<String>) -> Result<SharedLibraryBrowse, String> {
        require_store(&self.store)?;
        let selected_route = descriptor.route.is_some()
            || (cfg!(any(target_os = "ios", target_os = "macos")) && !descriptor.native_services.is_empty());
        let revision = if selected_route { None } else { Some(transfer_policy::local_revision()?) };
        let cancel = self.state.lock().map_err(|_| REVOKED)?.0.child_token();
        let grant = ScopedDownloadGrant { inner: Arc::new(GrantInner {
            store: self.store.clone(), descriptor, discovery: discovery_id.map(|id| (self.discovery.clone(), id)),
            revision, cancel, revoked: CancellationToken::new(),
        }), book: None };
        grant.check().await?;
        let operation = async {
            let inventory = library::browse(&grant.inner.descriptor, grant.inner.cancel.child_token()).await?;
            let descriptor = &grant.inner.descriptor;
            if inventory.version != 1 || inventory.session_id != descriptor.session_id
                || inventory.peer_id != descriptor.peer_id || inventory.name.len() > 256 || inventory.books.len() > 10000
            { return Err("Invalid shared library session".into()); }
            let mut ids = HashSet::new();
            let mut books = Vec::with_capacity(inventory.books.len());
            for book in inventory.books {
                if book.id.is_empty() || book.id.len() > 256 || book.id == "." || book.id == ".."
                    || book.id.chars().any(char::is_control) || !ids.insert(book.id.clone())
                    || book.size == 0 || book.title.len() > 4096
                { return Err("Invalid shared library book".into()); }
                crate::acquisition::shared_file_name(&book.file_name)?;
                let archive_id = book.archive_id.as_deref().map(|id| uuid::Uuid::parse_str(id)
                    .map(|id| id.to_string()).map_err(|_| "Invalid shared library book")).transpose()?;
                books.push(SharedBook {
                    download_identity: download_identity(&descriptor.session_id, &descriptor.peer_id, &book.id),
                    id: book.id, title: book.title, file_name: book.file_name, size: book.size, archive_id,
                });
            }
            grant.check().await?;
            let books: Arc<[SharedBook]> = books.into();
            let result = SharedLibraryBrowse { receipt_id: uuid::Uuid::new_v4().to_string(),
                session_id: descriptor.session_id.clone(), name: inventory.name, books: books.clone() };
            let mut state = self.state.lock().map_err(|_| REVOKED)?;
            grant.require()?;
            let now = Instant::now();
            make_room(&mut state.1, now);
            if state.1.len() >= MAX_RECEIPTS {
                return Err("Too many shared libraries are open; close discovery and try again".into());
            }
            state.1.insert(result.receipt_id.clone(), Receipt { created: now, grant: grant.clone(), books });
            Ok(result)
        };
        let result = tokio::select! {
            biased;
            _ = grant.denied() => Err(REVOKED.into()),
            result = tokio::time::timeout(Duration::from_secs(30), operation) =>
                result.map_err(|_| "Shared library did not respond".to_string())?,
        };
        if result.is_ok() { grant.monitor(); }
        result
    }

    pub async fn download(&self, downloads: &DownloadManager, destination_dir: PathBuf, receipt_id: &str,
        book_id: &str, space_acknowledgement: Option<String>, replace_earlier: bool) -> Result<DownloadSnapshot, String>
    {
        let grant = self.receipt_grant(receipt_id, book_id)?;
        grant.check().await?;
        if let Some(id) = grant.book.as_ref().and_then(|book| book.archive_id.as_deref()) {
            let id = uuid::Uuid::parse_str(id).map_err(|_| "Invalid shared library book")?.to_string();
            if let Some(existing) = self.store.book(&id)? {
                return Err(if existing.available {
                    "This archive is already in your library. Open the existing copy instead."
                } else {
                    "This archive is already in your library, but its file is unavailable. Locate or remove that entry in Library before downloading it again."
                }.into());
            }
        }
        downloads.start_shared_library(grant, destination_dir, space_acknowledgement, replace_earlier).await
    }
}

impl SharedLibraryClient {
    /// The grant an open list holds for one of its books. How long the list
    /// has been open decides nothing here: the caller's `check()` does.
    fn receipt_grant(&self, receipt_id: &str, book_id: &str) -> Result<ScopedDownloadGrant, String> {
        let state = self.state.lock().map_err(|_| REVOKED)?;
        let receipt = state.1.get(receipt_id).ok_or(REVOKED)?;
        let book = receipt.books.iter().find(|book| book.id == book_id).ok_or("Book is not in this shared library")?;
        let mut grant = receipt.grant.clone();
        grant.book = Some(book.clone());
        Ok(grant)
    }
}

/// Keep the receipt table bounded without revoking a list somebody still has
/// open. A receipt whose grant was revoked can never admit anything again and
/// goes first. Only when the table is full do the oldest lists past the
/// retention age make way, so age matters under pressure and never by itself.
fn make_room(receipts: &mut HashMap<String, Receipt>, now: Instant) {
    receipts.retain(|_, receipt| !receipt.grant.retired());
    while receipts.len() >= MAX_RECEIPTS {
        let oldest = receipts.iter()
            .filter(|(_, receipt)| now.saturating_duration_since(receipt.created) >= RECEIPT_TTL)
            .min_by_key(|(_, receipt)| receipt.created).map(|(id, _)| id.clone());
        match oldest { Some(id) => { receipts.remove(&id); }, None => break }
    }
}

impl Drop for SharedLibraryClient {
    fn drop(&mut self) { self.invalidate(); }
}

struct GrantInner {
    store: Arc<Store>,
    descriptor: LibraryPeerDescriptor,
    discovery: Option<(LocalSwarmManager, String)>,
    revision: Option<u64>,
    cancel: CancellationToken,
    revoked: CancellationToken,
}

impl Drop for GrantInner {
    fn drop(&mut self) { self.cancel.cancel(); self.revoked.cancel(); }
}

#[derive(Clone)]
pub(crate) struct ScopedDownloadGrant {
    inner: Arc<GrantInner>,
    book: Option<SharedBook>,
}

impl ScopedDownloadGrant {
    /// Paused admissions also lose authority permanently when their selection disappears.
    fn monitor(&self) {
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                {
                    let Some(inner) = weak.upgrade() else { return; };
                    if (ScopedDownloadGrant { inner, book: None }).check().await.is_err() { return; }
                }
                tokio::time::sleep(IDLE_REVOCATION_POLL).await;
            }
        });
    }

    /// Authority this grant has permanently lost. Cheap flags only: the full
    /// proof is `check()`.
    fn retired(&self) -> bool {
        self.inner.cancel.is_cancelled() || self.inner.revoked.is_cancelled() || self.inner.descriptor.cancel.is_cancelled()
    }

    pub(crate) fn book(&self) -> Result<&SharedBook, String> { self.book.as_ref().ok_or_else(|| REVOKED.into()) }
    pub(crate) fn identity(&self) -> Result<String, String> { Ok(self.book()?.download_identity.clone()) }
    pub(crate) fn validate_url(&self, value: &str) -> Result<Url, String> {
        if value != self.identity()? { return Err(REVOKED.into()); }
        Url::parse(value).map_err(|_| REVOKED.into())
    }

    pub(crate) fn require(&self) -> Result<(), String> {
        let result = (|| {
            require_store(&self.inner.store)?;
            if self.inner.cancel.is_cancelled() || self.inner.revoked.is_cancelled() || self.inner.descriptor.cancel.is_cancelled() {
                return Err(REVOKED.into());
            }
            if let Some(revision) = self.inner.revision { transfer_policy::require_local_since(revision)?; }
            self.inner.descriptor.check()?;
            Ok(())
        })();
        if result.is_err() { self.inner.revoked.cancel(); }
        result
    }

    pub(crate) async fn check(&self) -> Result<(), String> {
        let result = async {
            self.require()?;
            if let Some((discovery, id)) = &self.inner.discovery {
                if let Some(current) = discovery.selected_library_peer(id).await? {
                    if !self.inner.descriptor.matches_peer(&current) { return Err(REVOKED.into()); }
                }
            }
            Ok(())
        }.await;
        if result.is_err() { self.inner.revoked.cancel(); }
        result
    }

    pub(crate) async fn denied(&self) {
        loop {
            if self.check().await.is_err() { return; }
            tokio::select! {
                biased;
                _ = self.inner.cancel.cancelled() => return,
                _ = self.inner.revoked.cancelled() => return,
                _ = self.inner.descriptor.cancel.cancelled() => return,
                _ = self.local_policy_denied() => {
                    self.inner.revoked.cancel(); return;
                },
                _ = tokio::time::sleep(Duration::from_millis(250)) => {},
            }
        }
    }

    async fn local_policy_denied(&self) {
        match self.inner.revision {
            Some(revision) => transfer_policy::local_denied_since(revision).await,
            None => std::future::pending::<()>().await,
        }
    }

    pub(crate) async fn request(&self, headers: &http::HeaderMap) -> Result<crate::transfer_response::TransferResponse, String> {
        self.check().await?;
        let response = tokio::select! {
            biased;
            _ = self.denied() => return Err(REVOKED.into()),
            result = library::open(&self.inner.descriptor, &self.book()?.id, headers.clone(), self.inner.cancel.child_token()) => result?,
        };
        self.check().await?;
        Ok(crate::transfer_response::TransferResponse::Quic(response))
    }
}

/// Lifecycle tests retain the same real grant owner without creating a network listener.
#[cfg(all(test, not(mobile)))]
pub(crate) fn test_download_grant(store: Arc<Store>, size: u64) -> (ScopedDownloadGrant, impl Fn() -> bool) {
    let session = "b".repeat(32);
    let peer = "a".repeat(32);
    let grant = ScopedDownloadGrant {
        inner: Arc::new(GrantInner {
            store, descriptor: LibraryPeerDescriptor {
                session_id: session.clone(), peer_id: peer.clone(), name: "Lifecycle library".into(),
                port: 4433, addresses: vec!["192.168.1.2".into()], native_services: Vec::new(), direct: false,
                route: None, cancel: CancellationToken::new(), authority: Default::default(),
            }, discovery: None, revision: Some(transfer_policy::local_revision().unwrap()),
            cancel: CancellationToken::new(), revoked: CancellationToken::new(),
        }),
        book: Some(SharedBook { id: "book".into(), title: "Lifecycle archive".into(), file_name: "fixture.zim".into(),
            size, archive_id: None, download_identity: download_identity(&session, &peer, "book") }),
    };
    grant.require().unwrap();
    let weak = Arc::downgrade(&grant.inner);
    (grant, move || weak.strong_count() != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_identity_binds_peer_session_and_exact_book_but_contains_no_route_authority() {
        let identity = download_identity("session", "peer", "book/with space");
        assert_eq!(identity, "kiwix-library://public/session/peer/book%2Fwith%20space");
        assert_ne!(identity, download_identity("session2", "peer", "book/with space"));
        assert_ne!(identity, download_identity("session", "peer2", "book/with space"));
        assert_ne!(identity, download_identity("session", "peer", "book"));
    }

    #[tokio::test]
    #[cfg(not(mobile))]
    async fn accepted_selection_survives_an_absent_scan_record_but_not_owner_retirement() {
        let root = std::env::temp_dir().join(format!("kiwix-public-scan-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(Store::new(root.clone()).unwrap());
        let discovery = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
        let (mut grant, probe) = test_download_grant(store.clone(), 128);
        drop(probe);
        Arc::get_mut(&mut grant.inner).unwrap().discovery = Some((discovery.clone(), "selected.local".into()));
        assert!(discovery.library_peer("selected.local").await.is_err(), "Fresh browse still requires a visible selected record");
        assert!(discovery.selected_library_peer("selected.local").await.unwrap().is_none());
        grant.check().await.unwrap();
        assert!(!grant.inner.revoked.is_cancelled(), "Stop finding is not Stop all");
        grant.inner.descriptor.cancel.cancel();
        assert!(grant.check().await.is_err());
        drop(grant); drop(discovery); drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(mobile))]
    fn receipt(grant: &ScopedDownloadGrant, created: Instant) -> Receipt {
        let mut held = grant.clone();
        let books: Arc<[SharedBook]> = vec![held.book.take().unwrap()].into();
        Receipt { created, grant: held, books }
    }

    /// A list left open for longer than the retention age is still the list
    /// the sender is offering. Download must work; only the grant decides.
    #[tokio::test]
    #[cfg(not(mobile))]
    async fn an_open_list_outlives_its_age_but_never_its_grant() {
        let root = std::env::temp_dir().join(format!("kiwix-public-receipt-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(Store::new(root.clone()).unwrap());
        let discovery = LocalSwarmManager::new(store.clone(), CancellationToken::new()).unwrap();
        let client = SharedLibraryClient::new(store.clone(), discovery.clone());
        let (grant, _) = test_download_grant(store.clone(), 128);
        let now = Instant::now();
        // As old as this machine's clock allows; a just-booted machine cannot
        // represent an instant before its own start.
        let old = now.checked_sub(RECEIPT_TTL * 2).unwrap_or(now);
        client.state.lock().unwrap().1.insert("open-list".into(), receipt(&grant, old));

        let admitted = client.receipt_grant("open-list", "book").unwrap();
        admitted.check().await.unwrap();
        assert_eq!(admitted.book().unwrap().id, "book");
        assert_eq!(client.receipt_grant("open-list", "other").err().unwrap(), "Book is not in this shared library");
        assert_eq!(client.receipt_grant("unknown", "book").err().unwrap(), REVOKED);

        // Another browse making room does not drop it either, however old.
        let later = now + RECEIPT_TTL * 4;
        make_room(&mut client.state.lock().unwrap().1, later);
        assert!(client.receipt_grant("open-list", "book").is_ok());
        // Under pressure the oldest lists past the retention age make way,
        // and a full table of recent lists is refused as before.
        {
            let mut state = client.state.lock().unwrap();
            for index in 1..MAX_RECEIPTS { state.1.insert(format!("recent-{index}"), receipt(&grant, later)); }
            assert_eq!(state.1.len(), MAX_RECEIPTS);
            make_room(&mut state.1, later);
            assert_eq!(state.1.len(), MAX_RECEIPTS - 1);
            assert!(!state.1.contains_key("open-list"));
            state.1.insert("recent-0".into(), receipt(&grant, later));
            make_room(&mut state.1, later);
            assert_eq!(state.1.len(), MAX_RECEIPTS);
        }

        // Authority is the grant: once its owner is gone the receipt admits
        // nothing, whatever its age, and stops occupying the table.
        grant.inner.descriptor.cancel.cancel();
        let revoked = client.receipt_grant("recent-1", "book").unwrap();
        assert_eq!(revoked.check().await.unwrap_err(), REVOKED);
        make_room(&mut client.state.lock().unwrap().1, later);
        assert!(client.state.lock().unwrap().1.is_empty());
        assert_eq!(client.receipt_grant("recent-1", "book").err().unwrap(), REVOKED);
        drop(revoked); drop(admitted); drop(grant); drop(client); drop(discovery); drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[cfg(not(mobile))]
    async fn selected_owner_cancellation_permanently_retires_a_paused_grant() {
        let root = std::env::temp_dir().join(format!("kiwix-public-grant-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(Store::new(root.clone()).unwrap());
        let (grant, _) = test_download_grant(store.clone(), 128);
        let token = grant.inner.descriptor.cancel.clone();
        grant.check().await.unwrap();
        token.cancel();
        assert!(grant.check().await.is_err());
        tokio::time::timeout(Duration::from_secs(1), grant.denied()).await.unwrap();
        assert!(grant.inner.revoked.is_cancelled());
        assert!(grant.validate_url("https://example.org/archive.zim").is_err());
        drop(grant); drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
