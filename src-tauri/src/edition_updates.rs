// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit full-archive edition cutover. Durable intent is evidence, never a
//! restored authorization; all mutating entry points require the service's
//! normal-writable/privacy/window admission lease and source-mutation gate.
#![forbid(unsafe_code)]
use crate::{
    acquisition::{DownloadJob, DownloadManager, ImportPublication},
    archive_source::{self, RetainedSource},
    catalog::CatalogEntry,
    imports::{ImportManager, ReadyImport},
    integrity::ContentIdentity,
    store::{new_id, new_installation_id, Book, State, Store, Tab},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionSelection {
    pub source_id: String,
    pub source_url: String,
    pub entry_id: String,
    pub catalog_generation: u64,
    pub entry: CatalogEntry,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionBinding {
    pub installation_id: String,
    pub source_id: String,
    pub source_url: String,
    pub entry_id: String,
    #[serde(default)]
    pub series: Option<crate::catalog::SeriesKey>,
}
impl EditionBinding {
    pub fn matches_series(&self, source_id: &str, series: &crate::catalog::SeriesKey) -> bool {
        self.source_id == source_id && self.series.as_ref() == Some(series)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionIntent {
    pub id: String,
    pub selection: EditionSelection,
    pub target: Book,
    pub original_identity: ContentIdentity,
    pub destination: String,
    pub download_directory: String,
    pub download_id: Option<String>,
    pub receipt: Option<String>,
    pub status: String,
    pub verified: Option<Book>,
    pub verified_identity: Option<ContentIdentity>,
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub lineage_associated: bool,
}

fn same_metadata(a: &crate::archive::ArchiveMetadata, b: &crate::archive::ArchiveMetadata) -> bool {
    a.id == b.id && a.title == b.title && a.description == b.description
        && a.language == b.language && a.creator == b.creator && a.publisher == b.publisher
        && a.date == b.date && a.name == b.name && a.tags == b.tags && a.size == b.size
        && a.article_count == b.article_count && a.media_count == b.media_count
        && a.has_fulltext_index == b.has_fulltext_index && a.has_title_index == b.has_title_index
        && a.main_path == b.main_path
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceTransition {
    pub revision: u64,
    pub transaction_id: String,
    pub from_book_id: String,
    pub to_book_id: String,
    pub from_installation_id: String,
    pub to_installation_id: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionTransaction {
    pub id: String,
    pub intent_id: String,
    pub original: Book,
    pub original_identity: ContentIdentity,
    pub replacement: Book,
    pub revision: u64,
    pub rolled_back_installation_id: Option<String>,
    #[serde(default)]
    pub rollback_book: Option<Book>,
    /// The person chose to stop keeping the superseded original: its file was
    /// deleted (or was already gone, or belongs to another app) and it no
    /// longer reserves its path, its provider grant or a rollback.
    #[serde(default)]
    pub original_discarded: bool,
}
/// What `edition_discard_original` did with the original's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiscardedFile {
    /// Deleted permanently.
    Deleted,
    /// Moved to the system Trash.
    Trashed,
    /// Nothing was at the original's location any more; nothing was deleted.
    Missing,
    /// The original belongs to another app's document provider, which Kiwix
    /// cannot delete from; only Kiwix's hold on it was released.
    External,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionPreparation {
    pub token: String,
    pub installed: Book,
    pub selection: EditionSelection,
    pub intent_id: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionStatus {
    #[serde(flatten)]
    pub intent: EditionIntent,
    pub needs_authorization: bool,
}
struct Decision {
    token: String,
    epoch: u64,
    window: String,
    target: Book,
    selection: EditionSelection,
    directory: String,
    source: RetainedSource,
    identity: ContentIdentity,
    intent_id: Option<String>,
    lineage_associated: bool,
}
pub(crate) struct RollbackPreparation {
    pub(crate) transaction: EditionTransaction,
    pub(crate) current: Book,
    epoch: u64,
    window: String,
    source: RetainedSource,
}
#[derive(Default)]
pub(crate) struct EditionManager {
    decisions: Mutex<HashMap<String, Decision>>,
    /// Serializes a rollback's commit with deleting a retained original, so
    /// neither can act on a transaction the other is changing.
    originals: Mutex<()>,
}

fn writable(state: &State) -> Result<(), String> {
    if state.settings.private_mode || state.settings.read_only {
        return Err("Edition changes require normal writable mode".into());
    }
    Ok(())
}
fn same_installation(a: &Book, b: &Book) -> bool {
    // Availability and the latest background integrity observation may change
    // while a full download runs; neither changes the installation capability.
    a.installation_id == b.installation_id && a.path == b.path && a.source_reference == b.source_reference
        && a.added_at == b.added_at && a.managed == b.managed
        && a.metadata.id == b.metadata.id && a.metadata.name == b.metadata.name
        && a.metadata.language == b.metadata.language && a.metadata.date == b.metadata.date
        && a.metadata.size == b.metadata.size && a.metadata.tags == b.metadata.tags
}
fn exact_target(state: &State, expected: &Book) -> Result<(), String> {
    if !state.books.iter().any(|book| same_installation(book, expected)) {
        return Err("The installed archive changed; select its edition again".into());
    }
    Ok(())
}
fn same_selection(a: &EditionSelection, b: &EditionSelection) -> bool {
    a.source_id == b.source_id && a.source_url == b.source_url
        && a.entry_id == b.entry_id && a.entry == b.entry
}
fn check_book(selection: &EditionSelection, book: &Book, lineage_associated: bool) -> Result<(), String> {
    let entry = &selection.entry;
    let same_series = crate::catalog::series_key(&book.metadata.name, &book.metadata.language, &book.metadata.tags)
        .zip(crate::catalog::series_key(&entry.name, &entry.language, &entry.tags))
        .is_some_and(|(actual, expected)| {
            crate::catalog::series_compatible(&actual, &expected)
                && (actual.flavour.is_some() && actual.flavour == expected.flavour || lineage_associated)
        });
    // OPDS issued timestamps and ZIM Date days express the same dated edition.
    // Unknown day precision (month-only values) must never be widened to a day.
    let same_date = book.metadata.date == entry.date
        || crate::catalog::edition_date(&book.metadata.date)
            .zip(crate::catalog::edition_date(&entry.date))
            .is_some_and(|(actual, expected)| actual == expected);
    // Atom IDs are not ZIM UUIDs. The pinned native acquisition, libzim
    // metadata and variant/date are the evidence; never invent UUID semantics.
    if book.source_reference.is_some() || book.metadata.name != entry.name
        || book.metadata.language != entry.language || !same_date
        || !same_series
    {
        return Err("Verified archive does not match the selected catalog edition".into());
    }
    if let Some(promised) = entry.id.strip_prefix("urn:uuid:") {
        let expected = uuid::Uuid::parse_str(promised).map_err(|_| "Catalog promises an invalid archive UUID")?;
        if uuid::Uuid::parse_str(&book.metadata.id).ok() != Some(expected) {
            return Err("Downloaded archive UUID does not match the catalog's explicit UUID promise".into());
        }
    }
    Ok(())
}
impl EditionManager {
    pub(crate) fn new() -> Self { Self::default() }

    pub(crate) fn selection_for_token(&self, token: &str) -> Result<EditionSelection, String> {
        self.decisions.lock().map_err(|_| "Edition authorization unavailable")?
            .get(token).map(|decision| decision.selection.clone())
            .ok_or_else(|| "Select this edition again before continuing".into())
    }

    pub(crate) fn retire_authorizations(&self) {
        if let Ok(mut decisions) = self.decisions.lock() { decisions.clear(); }
    }

    pub(crate) fn retire_window_authorizations(&self, window: &str) {
        if let Ok(mut decisions) = self.decisions.lock() {
            decisions.retain(|_, decision| decision.window != window);
        }
    }

    /// `selection` must come from CatalogIndex native lookup, never shell metadata.
    pub(crate) fn prepare(
        &self, store: &Store, epoch: u64, window: &str, old: &Book,
        selection: EditionSelection, association_confirmed: bool,
    ) -> Result<EditionPreparation, String> {
        let state = store.read()?;
        writable(&state)?;
        exact_target(&state, old)?;
        if selection.source_id.is_empty() || selection.entry_id.is_empty() || selection.source_url.is_empty()
            || selection.entry.url.is_empty() || selection.entry.name.is_empty()
        { return Err("Select an unambiguous native catalog entry first".into()); }
        if crate::catalog::source_id(&selection.source_url)? != selection.source_id
            || (selection.source_url != state.settings.catalog_url
                && !state.settings.opds_sources.contains(&selection.source_url))
        { return Err("Select a currently configured catalog source".into()); }
        let new_series = crate::catalog::series_key(&selection.entry.name, &selection.entry.language, &selection.entry.tags);
        let bound = new_series.as_ref().is_some_and(|series|
            state.edition_bindings.iter().any(|binding| binding.installation_id == old.installation_id
                && binding.matches_series(&selection.source_id, series)));
        if !bound && !association_confirmed {
            return Err("Confirm this archive's catalog association before preparing its edition".into());
        }
        let old_series = crate::catalog::series_key(&old.metadata.name, &old.metadata.language, &old.metadata.tags);
        if !old_series.as_ref().zip(new_series.as_ref()).is_some_and(|(old, new)|
            crate::catalog::series_compatible(old, new)
                && (old.flavour.is_some() && old.flavour == new.flavour || bound || association_confirmed))
            || !crate::catalog::edition_is_newer(&selection.entry.date, &old.metadata.date)
        { return Err("This catalog entry is not a comparable newer edition of the installed archive".into()); }
        let source = archive_source::retain(old)?;
        source.unchanged()?;
        let identity = source.identity()?;
        let mut decisions = self.decisions.lock().map_err(|_| "Edition authorization unavailable")?;
        let intent_id = store.update(|current| {
            writable(current)?;
            exact_target(current, old)?;
            if selection.source_url != current.settings.catalog_url
                && !current.settings.opds_sources.contains(&selection.source_url)
            { return Err("Select a currently configured catalog source".into()); }
            if current.settings.download_directory != state.settings.download_directory {
                return Err("Edition download directory changed. Select this edition again".into());
            }
            let existing = current.edition_intents.iter_mut().find(|intent|
                same_installation(&intent.target, old) && intent.original_identity == identity
                    && same_selection(&intent.selection, &selection)
                    && intent.download_directory == state.settings.download_directory
                    && matches!(intent.status.as_str(), "reserved" | "downloading" | "ready"));
            Ok(existing.map(|intent| {
                intent.selection = selection.clone();
                intent.lineage_associated |= bound || association_confirmed;
                intent.id.clone()
            }))
        })?;
        let token = new_id();
        let prepared = EditionPreparation {
            token: token.clone(), installed: old.clone(), selection: selection.clone(),
            intent_id: intent_id.clone(),
        };
        decisions.retain(|_, decision| decision.epoch == epoch && decision.window != window);
        decisions.insert(token.clone(), Decision {
            token, epoch, window: window.into(), target: old.clone(), selection,
            directory: state.settings.download_directory.clone(), source, identity, intent_id,
            lineage_associated: bound || association_confirmed,
        });
        Ok(prepared)
    }

    /// Must complete before DownloadManager::start is called. A receipt is
    /// edition-owned by its unique destination even if start completes before
    /// bind_download returns, or the process dies between those two operations.
    pub(crate) fn reserve(&self, store: &Store, token: &str, epoch: u64, window: &str) -> Result<EditionIntent, String> {
        let mut decisions = self.decisions.lock().map_err(|_| "Edition authorization unavailable")?;
        let decision = decisions.get_mut(token).ok_or("Select this edition again before downloading")?;
        authorize(decision, epoch, window)?;
        recheck_original(decision)?;
        std::fs::create_dir_all(&decision.directory).map_err(|error| format!("Opening edition download directory: {error}"))?;
        let directory = std::fs::canonicalize(&decision.directory).map_err(|error| error.to_string())?;
        let intent = store.update(|state| {
            validate_decision(decision, state)?;
            let series = crate::catalog::series_key(&decision.selection.entry.name,
                &decision.selection.entry.language, &decision.selection.entry.tags)
                .ok_or("Edition series changed. Select this edition again")?;
            let binding = EditionBinding {
                installation_id: decision.target.installation_id.clone(), source_id: decision.selection.source_id.clone(),
                source_url: decision.selection.source_url.clone(), entry_id: decision.selection.entry_id.clone(),
                series: Some(series),
            };
            if let Some(existing) = state.edition_bindings.iter_mut().find(|binding|
                binding.installation_id == decision.target.installation_id && binding.source_id == decision.selection.source_id)
            { *existing = binding; }
            else { state.edition_bindings.push(binding); }
            if let Some(id) = &decision.intent_id {
                let existing = state.edition_intents.iter_mut().find(|intent| &intent.id == id)
                    .ok_or("Edition intent is no longer available")?;
                if !same_installation(&existing.target, &decision.target)
                    || existing.original_identity != decision.identity
                    || !same_selection(&existing.selection, &decision.selection)
                    || existing.download_directory != decision.directory
                    || !matches!(existing.status.as_str(), "reserved" | "downloading" | "ready")
                { return Err("Edition intent changed. Select this edition again".into()); }
                existing.selection = decision.selection.clone();
                existing.lineage_associated |= decision.lineage_associated;
                return Ok(existing.clone());
            }
            if let Some(existing) = state.edition_intents.iter_mut().find(|intent|
                same_installation(&intent.target, &decision.target) && intent.original_identity == decision.identity
                    && same_selection(&intent.selection, &decision.selection)
                    && intent.download_directory == decision.directory
                    && matches!(intent.status.as_str(), "reserved" | "downloading" | "ready"))
            {
                existing.selection = decision.selection.clone();
                existing.lineage_associated |= decision.lineage_associated;
                return Ok(existing.clone());
            }
            let id = new_id();
            let destination = directory.join(format!("edition-{id}.zim"));
            let intent = EditionIntent {
                id, selection: decision.selection.clone(), target: decision.target.clone(),
                original_identity: decision.identity.clone(),
                destination: destination.to_string_lossy().into_owned(),
                download_directory: decision.directory.clone(), download_id: None,
                receipt: None, status: "reserved".into(), verified: None, verified_identity: None, transaction_id: None,
                lineage_associated: decision.lineage_associated,
            };
            state.edition_intents.push(intent.clone());
            Ok(intent)
        })?;
        decision.intent_id = Some(intent.id.clone());
        Ok(intent)
    }

    pub(crate) fn bind_download(&self, store: &Store, intent_id: &str, job: &DownloadJob) -> Result<(), String> {
        let state = store.read()?;
        writable(&state)?;
        if state.edition_intents.iter().any(|intent| intent.id == intent_id
            && intent.download_id.as_deref() == Some(job.id.as_str())
            && intent.destination == job.destination && intent.selection.entry.url == job.url
            && matches!(intent.status.as_str(), "downloading" | "ready"))
        { return Ok(()); }
        store.update(|state| {
            writable(state)?;
            let intent = state.edition_intents.iter_mut().find(|intent| intent.id == intent_id)
                .ok_or("Unknown edition intent")?;
            if job.destination != intent.destination || job.url != intent.selection.entry.url
                || intent.download_id.as_ref().is_some_and(|id| id != &job.id)
                || !matches!(intent.status.as_str(), "reserved" | "downloading" | "ready")
            { return Err("Download does not belong to this edition intent".into()); }
            intent.download_id = Some(job.id.clone());
            if intent.status != "ready" { intent.status = "downloading".into(); }
            Ok(())
        })
    }

    pub(crate) fn owns_receipt(state: &State, receipt: &str, paths: &[String]) -> bool {
        state.edition_intents.iter().any(|intent| {
            intent.receipt.as_deref() == Some(receipt)
                || intent.download_id.as_ref().is_some_and(|id|
                    receipt == id || receipt.strip_prefix(id).is_some_and(|suffix| suffix.starts_with(':')))
                || paths.iter().any(|path| path == &intent.destination)
        })
    }

    /// DownloadManager ownership is compared to the very descriptors libzim
    /// verified; a swapped file at the same pathname cannot become Ready.
    pub(crate) fn ready(
        &self, store: &Store, intent_id: &str, ready: &ReadyImport,
        imports: &ImportManager, downloads: &DownloadManager,
    ) -> Result<EditionIntent, String> {
        let result = imports.with_ready_sources(ready, |books, sources, publication| {
            let state = store.read()?;
            writable(&state)?;
            let expected = state.edition_intents.iter().find(|intent| intent.id == intent_id)
                .ok_or("Unknown edition intent")?;
            let (id, publication) = match verify_ready(expected, ready.receipt(), books, sources, publication) {
                Ok(verified) => verified,
                Err(error) => return Ok(Err(error)),
            };
            // Compare the worker's exact publication proof, not a persisted
            // Ready identity. Hold its producer's ownership through the write.
            match downloads.with_import_publication(id, &sources[0], publication, || {
                Ok((|| {
                    let identity = sources[0].identity()?;
                    if expected.status == "ready" && expected.receipt.as_deref() == Some(ready.receipt())
                        && expected.verified.as_ref() == Some(&books[0])
                        && expected.verified_identity.as_ref() == Some(&identity)
                    { return Ok(expected.clone()); }
                    store.update(|state| {
                        writable(state)?;
                        ready.sources_unchanged()?;
                        let intent = state.edition_intents.iter_mut().find(|intent| intent.id == intent_id)
                            .ok_or("Unknown edition intent")?;
                        if &*intent != expected { return Err("Edition intent changed during verification".into()); }
                        intent.verified = Some(books[0].clone());
                        intent.verified_identity = Some(identity);
                        intent.receipt = Some(ready.receipt().into());
                        intent.status = "ready".into();
                        Ok(intent.clone())
                    })
                })())
            }) {
                Ok(recorded) => recorded.map(Ok),
                Err(error) => Ok(Err(error)),
            }
        })?;
        match result {
            Ok(intent) => Ok(intent),
            Err(error) => {
                imports.fail_ready(ready, &error);
                Err(error)
            }
        }
    }

    pub(crate) fn status(&self, store: &Store, epoch: u64, window: &str) -> Result<Vec<EditionStatus>, String> {
        let state = store.read()?;
        writable(&state)?;
        let decisions = self.decisions.lock().map_err(|_| "Edition authorization unavailable")?;
        Ok(state.edition_intents.iter().map(|intent| {
            let authorized = decisions.values().any(|decision| decision.epoch == epoch
                && decision.window == window && decision.intent_id.as_deref() == Some(&intent.id)
                && validate_decision(decision, &state).is_ok());
            EditionStatus { needs_authorization: !authorized, intent: intent.clone() }
        }).collect())
    }

    /// Service must finish source/sharing/reader admission before this short
    /// transaction and fence the predecessor even on committed Store errors.
    /// Only after observing the durable transaction may it ack the import.
    // Security/durability boundary: keep the independent decision scope, pinned
    // import authority and download publication authority explicit at commit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit(
        &self, store: &Store, intent_id: &str, token: &str, epoch: u64, window: &str,
        imports: &ImportManager, ready: &ReadyImport, downloads: &DownloadManager,
    ) -> Result<EditionTransaction, String> {
        let decisions = self.decisions.lock().map_err(|_| "Edition authorization unavailable")?;
        let decision = decisions.get(token).ok_or("Select this edition again before replacing")?;
        authorize(decision, epoch, window)?;
        if decision.intent_id.as_deref() != Some(intent_id) { return Err("Edition authorization belongs to another download".into()); }
        recheck_original(decision)?;
        imports.with_ready_sources(ready, |books, sources, publication| {
            let state = store.read()?;
            validate_decision(decision, &state)?;
            let expected = state.edition_intents.iter().find(|intent| intent.id == intent_id)
                .ok_or("Unknown edition intent")?.clone();
            let (id, publication) = verify_ready(&expected, ready.receipt(), books, sources, publication)?;
            if expected.status != "ready" || expected.receipt.as_deref() != Some(ready.receipt())
                || expected.verified_identity.as_ref() != Some(&sources[0].identity()?)
            { return Err("Verify this edition before replacing the installed archive".into()); }
            downloads.with_import_publication(id, &sources[0], publication, || {
                store.update(|state| {
                    validate_decision(decision, state)?;
                    ready.sources_unchanged()?;
                    let intent = state.edition_intents.iter().find(|intent| intent.id == intent_id)
                        .ok_or("Unknown edition intent")?.clone();
                    if intent != expected {
                        return Err("Edition intent changed before commit".into());
                    }
                    state.replace_edition(&intent, &books[0], ready.receipt())
                })
            })
        })
    }

    pub(crate) fn cancel(&self, store: &Store, intent_id: &str) -> Result<(), String> {
        store.update(|state| {
            writable(state)?;
            let intent = state.edition_intents.iter_mut().find(|intent| intent.id == intent_id)
                .ok_or("Unknown edition intent")?;
            if intent.status == "committed" { return Err("A committed edition requires explicit rollback".into()); }
            intent.status = "cancelled".into();
            Ok(())
        })
    }

    /// Run on a blocking worker: this validates the retained original using
    /// libzim, including every split part/provider descriptor, without deleting
    /// either edition or forgetting its persistent provider capability.
    pub(crate) fn prepare_rollback(
        &self, store: &Store, transaction_id: &str, expected_installation: &str,
        epoch: u64, window: &str,
    ) -> Result<RollbackPreparation, String> {
        let state = store.read()?;
        writable(&state)?;
        let transaction = state.edition_transactions.iter().find(|entry| entry.id == transaction_id)
            .ok_or("Unknown edition transaction")?.clone();
        if transaction.rolled_back_installation_id.is_some() { return Err("This edition was already rolled back".into()); }
        if transaction.original_discarded { return Err(ORIGINAL_DISCARDED.into()); }
        let current = state.books.iter().find(|book| book.installation_id == expected_installation
            && book.installation_id == transaction.replacement.installation_id)
            .ok_or("The installed edition changed; rollback cannot overwrite it")?.clone();
        let source = archive_source::retain(&transaction.original)
            .map_err(|error| format!("Original archive is unavailable; reconnect its storage or provider: {error}"))?;
        if source.identity()? != transaction.original_identity {
            return Err("The retained original archive changed; rollback was not authorized".into());
        }
        let archive = source.open()?;
        if archive.metadata()?.id != transaction.original.metadata.id || !archive.check_integrity()? {
            return Err("The original archive failed verification; neither edition was changed".into());
        }
        source.unchanged()?;
        Ok(RollbackPreparation { transaction, current, epoch, window: window.into(), source })
    }

    pub(crate) fn rollback(
        &self, store: &Store, prepared: &RollbackPreparation, epoch: u64, window: &str,
    ) -> Result<EditionTransaction, String> {
        if prepared.epoch != epoch || prepared.window != window {
            return Err("Edition rollback authorization expired; choose rollback again".into());
        }
        let _originals = self.originals.lock().map_err(|_| "Edition authorization unavailable")?;
        prepared.source.unchanged()?;
        if prepared.source.identity()? != prepared.transaction.original_identity {
            return Err("Original archive changed after rollback verification".into());
        }
        store.update(|state| {
            writable(state)?;
            exact_target(state, &prepared.current)?;
            state.rollback_edition(&prepared.transaction)?;
            state.edition_transactions.iter().find(|entry| entry.id == prepared.transaction.id)
                .cloned().ok_or_else(|| "Rollback transaction disappeared".into())
        })
    }

    /// Delete the superseded original of a committed edition and close its
    /// transaction. Run on a blocking worker: a Trash on another volume copies.
    /// A rollback cannot commit between the checks here and the deletion.
    ///
    /// The file is removed first and the transaction is marked afterwards: an
    /// interruption in between leaves an open transaction whose original is
    /// missing, which the next call closes. The opposite order could release
    /// the original's reservation while its file still exists.
    pub(crate) fn discard_original(
        &self, store: &Store, transaction_id: &str, trash: bool,
    ) -> Result<(EditionTransaction, DiscardedFile), String> {
        let _originals = self.originals.lock().map_err(|_| "Edition authorization unavailable")?;
        let state = store.read()?;
        writable(&state)?;
        let transaction = state.discardable_original(transaction_id)?.clone();
        drop(state);
        let file = remove_original(&transaction, trash)?;
        let transaction = store.update(|state| {
            writable(state)?;
            state.discard_edition_original(&transaction)
        })?;
        Ok((transaction, file))
    }
}
const ORIGINAL_DISCARDED: &str = "The original of this edition was deleted, so it can no longer be rolled back";
/// Re-retain and identity-check the original as rollback does, then delete it
/// through the library's own deletion path: the exact part cohort the archive
/// declares, staged by exclusive rename, never a path sweep. Unlike rollback
/// this does not checksum the content; a damaged original stays deletable.
fn remove_original(transaction: &EditionTransaction, trash: bool) -> Result<DiscardedFile, String> {
    let original = &transaction.original;
    if original.source_reference.is_some() {
        return Ok(DiscardedFile::External);
    }
    // Only an absent name counts as gone. Any other failure is reported, so
    // an unreadable folder is never mistaken for a deleted file.
    match std::fs::symlink_metadata(&original.path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(DiscardedFile::Missing),
        Err(error) => return Err(format!("The original archive could not be checked, so it was not deleted: {error}")),
        Ok(_) => {}
    }
    let source = archive_source::retain(original)
        .map_err(|error| format!("The original archive could not be checked, so it was not deleted: {error}"))?;
    if source.identity()? != transaction.original_identity {
        return Err("The retained original archive changed since this update, so Kiwix did not delete it. Delete the file yourself if you no longer need it, then choose this again".into());
    }
    source.unchanged()?;
    // Windows cannot move a file out from under these handles.
    drop(source);
    crate::library_files::delete_book(original, trash)?;
    Ok(if trash { DiscardedFile::Trashed } else { DiscardedFile::Deleted })
}
fn authorize(decision: &Decision, epoch: u64, window: &str) -> Result<(), String> {
    if decision.epoch != epoch || decision.window != window || decision.token.is_empty() {
        return Err("Edition authorization expired; select this edition again".into());
    }
    Ok(())
}
/// `RetainedSource::unchanged` proves the path still names the pinned inode and
/// compares size/mtime, but mtime is restorable. Recompute the full content
/// identity, which includes Unix change time, before reserving or committing so
/// a rewrite that restores timestamps cannot inherit the original's authority.
/// A zero or degenerate Unix change time is not proof and refuses the operation;
/// Windows exposes creation/modification time rather than change time, so there
/// a deliberately restored mtime still leaves only the retained read handles as
/// proof (the recheck still rejects replacement, growth and ordinary rewrites).
fn recheck_original(decision: &Decision) -> Result<(), String> {
    decision.source.unchanged()?;
    if !change_times_provable(&decision.source)? {
        return Err("The installed archive's change time is unavailable; select this edition again".into());
    }
    if decision.source.identity()? != decision.identity {
        return Err("The installed archive changed; select this edition again".into());
    }
    Ok(())
}

#[cfg(unix)]
fn change_times_provable(source: &RetainedSource) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    for (_, file) in source.files() {
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        let nanos = metadata.ctime_nsec();
        if !(0..1_000_000_000).contains(&nanos) || (metadata.ctime() == 0 && nanos == 0) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(not(unix))]
fn change_times_provable(_source: &RetainedSource) -> Result<bool, String> {
    Ok(true)
}
fn validate_decision(decision: &Decision, state: &State) -> Result<(), String> {
    writable(state)?;
    exact_target(state, &decision.target)?;
    if decision.directory != state.settings.download_directory {
        return Err("Download location changed; select this edition again".into());
    }
    if decision.selection.source_url != state.settings.catalog_url
        && !state.settings.opds_sources.contains(&decision.selection.source_url)
    { return Err("Catalog source changed; select this edition again".into()); }
    Ok(())
}
fn verify_ready<'a, 'b>(
    intent: &'a EditionIntent, receipt: &str, books: &[Book], sources: &[RetainedSource],
    publication: Option<&'b ImportPublication>,
) -> Result<(&'a str, &'b ImportPublication), String> {
    if books.len() != 1 || sources.len() != 1 || books[0].path != intent.destination
        || !matches!(intent.status.as_str(), "reserved" | "downloading" | "ready")
    { return Err("Verified import does not belong to this edition intent".into()); }
    let id = intent.download_id.as_deref().ok_or("Bind the edition download before verification")?;
    if receipt != id && !receipt.strip_prefix(id).is_some_and(|suffix| suffix.starts_with(':')) {
        return Err("Edition receipt belongs to another acquisition".into());
    }
    let publication = publication.filter(|proof| proof.destination() == intent.destination)
        .ok_or("Edition import has no verified download publication")?;
    check_book(&intent.selection, &books[0], intent.lineage_associated)?;
    Ok((id, publication))
}

impl State {
    /// Called only after a native relocation has verified the same content.
    /// Reconcile the current capability, never the retained rollback source.
    pub(crate) fn reconcile_edition_relocation(&mut self, before: &Book, after: &Book) -> Result<(), String> {
        if before.installation_id != after.installation_id
            || !same_metadata(&before.metadata, &after.metadata) || before.added_at != after.added_at
            || before.integrity != after.integrity
        { return Err("Archive relocation changed its installation or content metadata".into()); }
        let matches_prior = |expected: &Book| {
            expected.path == before.path && expected.source_reference == before.source_reference
                && expected.managed == before.managed && expected.added_at == before.added_at
                && same_metadata(&expected.metadata, &before.metadata)
        };
        // Validate every matching expectation before mutating any transaction.
        for transaction in &self.edition_transactions {
            for expected in std::iter::once(&transaction.replacement).chain(transaction.rollback_book.iter()) {
                if expected.installation_id == before.installation_id && !matches_prior(expected) {
                    return Err("Edition relocation no longer matches its transaction".into());
                }
            }
        }
        for transaction in &mut self.edition_transactions {
            for expected in std::iter::once(&mut transaction.replacement).chain(transaction.rollback_book.iter_mut()) {
                if expected.installation_id == before.installation_id {
                    expected.path.clone_from(&after.path);
                    expected.source_reference.clone_from(&after.source_reference);
                    expected.managed = after.managed;
                    expected.available = after.available;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn replace_edition(&mut self, intent: &EditionIntent, verified: &Book, receipt: &str) -> Result<EditionTransaction, String> {
        writable(self)?;
        exact_target(self, &intent.target)?;
        if self.consumed_acquisitions.contains(receipt) { return Err("Edition acquisition was already consumed".into()); }
        if intent.status != "ready" || intent.receipt.as_deref() != Some(receipt)
            || intent.verified.as_ref() != Some(verified)
            || !self.edition_intents.iter().any(|saved| saved == intent)
        { return Err("Edition cutover requires its exact durable verified reservation".into()); }
        check_book(&intent.selection, verified, intent.lineage_associated)?;
        if self.books.iter().any(|book| book.installation_id != intent.target.installation_id
            && (book.metadata.id == verified.metadata.id || book.path == verified.path)) {
            return Err("The verified edition is already installed separately; no archive was overwritten".into());
        }
        let mut replacement = verified.clone();
        replacement.installation_id = new_installation_id();
        replacement.integrity = None;
        replacement.added_at.clone_from(&intent.target.added_at);
        let id = new_id();
        let revision = self.transition_book(&intent.target, &replacement, &id)?;
        self.edition_bindings.push(EditionBinding {
            installation_id: replacement.installation_id.clone(), source_id: intent.selection.source_id.clone(),
            source_url: intent.selection.source_url.clone(), entry_id: intent.selection.entry_id.clone(),
            series: crate::catalog::series_key(&intent.selection.entry.name,
                &intent.selection.entry.language, &intent.selection.entry.tags),
        });
        let transaction = EditionTransaction {
            id: id.clone(), intent_id: intent.id.clone(), original: intent.target.clone(),
            original_identity: intent.original_identity.clone(), replacement,
            revision, rolled_back_installation_id: None, rollback_book: None,
            original_discarded: false,
        };
        self.consumed_acquisitions.insert(receipt.into());
        let durable = self.edition_intents.iter_mut().find(|entry| entry.id == intent.id)
            .ok_or("Edition reservation disappeared")?;
        durable.status = "committed".into();
        durable.transaction_id = Some(id);
        self.edition_transactions.push(transaction.clone());
        Ok(transaction)
    }

    /// A book that is the discarded original of an edition update. Its file
    /// is gone or released, so it reserves nothing: not its path for a
    /// download or an import, and not its provider grant.
    pub(crate) fn original_discarded(&self, book: &Book) -> bool {
        self.edition_transactions.iter().any(|transaction| transaction.original_discarded
            && transaction.original.installation_id == book.installation_id)
    }

    /// The open transaction whose original may be deleted. A rolled-back
    /// transaction's original is the installed archive again, and nothing the
    /// library currently lists may be deleted through this path.
    fn discardable_original(&self, transaction_id: &str) -> Result<&EditionTransaction, String> {
        let transaction = self.edition_transactions.iter().find(|entry| entry.id == transaction_id)
            .ok_or("Unknown edition transaction")?;
        if transaction.rolled_back_installation_id.is_some() {
            return Err("This edition was rolled back, so its original is the installed archive. Remove it from the library instead".into());
        }
        if transaction.original_discarded { return Err("The original of this edition was already deleted".into()); }
        let original = &transaction.original;
        if self.books.iter().any(|book| book.installation_id == original.installation_id
            || book.path == original.path
            || book.source_reference.as_ref().zip(original.source_reference.as_ref())
                .is_some_and(|(installed, retained)| installed.reference == retained.reference)) {
            return Err("The original of this edition is in the library. Remove it from the library instead".into());
        }
        Ok(transaction)
    }

    pub(crate) fn discard_edition_original(&mut self, expected: &EditionTransaction) -> Result<EditionTransaction, String> {
        let current = self.discardable_original(&expected.id)?;
        if current.original != expected.original || current.original_identity != expected.original_identity {
            return Err("Edition transaction changed; its original was not released".into());
        }
        let transaction = self.edition_transactions.iter_mut().find(|entry| entry.id == expected.id)
            .ok_or("Edition transaction is no longer available")?;
        transaction.original_discarded = true;
        Ok(transaction.clone())
    }

    pub(crate) fn rollback_edition(&mut self, expected: &EditionTransaction) -> Result<Book, String> {
        let transaction = self.edition_transactions.iter().find(|entry| entry.id == expected.id)
            .ok_or("Edition transaction is no longer available")?;
        if transaction.original_discarded { return Err(ORIGINAL_DISCARDED.into()); }
        if transaction.rolled_back_installation_id.is_some()
            || transaction.replacement != expected.replacement || transaction.original != expected.original {
            return Err("Edition transaction changed; rollback was not authorized".into());
        }
        exact_target(self, &expected.replacement)?;
        if self.books.iter().any(|book| book.installation_id != expected.replacement.installation_id
            && (book.metadata.id == expected.original.metadata.id || book.path == expected.original.path)) {
            return Err("The original edition is already installed separately".into());
        }
        let mut restored = expected.original.clone();
        restored.installation_id = new_installation_id();
        restored.available = true;
        restored.integrity = None;
        let revision = self.transition_book(&expected.replacement, &restored, &expected.id)?;
        if let Some(binding) = self.edition_bindings.iter().find(|binding|
            binding.installation_id == expected.original.installation_id).cloned() {
            self.edition_bindings.push(EditionBinding { installation_id: restored.installation_id.clone(), ..binding });
        }
        let transaction = self.edition_transactions.iter_mut().find(|entry| entry.id == expected.id).unwrap();
        transaction.rolled_back_installation_id = Some(restored.installation_id.clone());
        transaction.rollback_book = Some(restored.clone());
        transaction.revision = revision;
        Ok(restored)
    }

    fn transition_book(&mut self, old: &Book, new: &Book, transaction_id: &str) -> Result<u64, String> {
        let revision = self.reference_revision.checked_add(1).ok_or("Reference revision exhausted")?;
        let index = self.books.iter().position(|book| same_installation(book, old)).ok_or("Archive installation changed")?;
        self.books[index] = new.clone();
        for bookmark in &mut self.bookmarks {
            if bookmark.book_id == old.metadata.id { bookmark.book_id.clone_from(&new.metadata.id); }
        }
        for entry in &mut self.history {
            if entry.book_id == old.metadata.id { entry.book_id.clone_from(&new.metadata.id); }
        }
        remap_tabs(&mut self.tabs, &old.metadata.id, &new.metadata.id);
        for window in self.windows.values_mut() { remap_tabs(&mut window.tabs, &old.metadata.id, &new.metadata.id); }
        if old.metadata.id != new.metadata.id {
            if let Some(zoom) = self.settings.book_zoom.remove(&old.metadata.id) {
                self.settings.book_zoom.insert(new.metadata.id.clone(), zoom);
            }
        }
        self.reader_storage.remove(&old.installation_id);
        self.settings.sharing_book_ids.retain(|id| id != &old.metadata.id && id != &new.metadata.id);
        self.settings.remap_pinned_book(&old.metadata.id, &new.metadata.id);
        self.reference_revision = revision;
        self.reference_transitions.push(ReferenceTransition {
            revision, transaction_id: transaction_id.into(), from_book_id: old.metadata.id.clone(),
            to_book_id: new.metadata.id.clone(), from_installation_id: old.installation_id.clone(),
            to_installation_id: new.installation_id.clone(),
        });
        Ok(revision)
    }

    /// Translate a delayed shell save/previous-session restore from its observed
    /// revision, in order. Replaying only transitions newer than that revision
    /// avoids applying a forward+rollback cycle twice to an already current tab.
    pub fn translate_reference_tabs(&self, tabs: &mut [Tab], revision: u64) -> Result<(), String> {
        if revision > self.reference_revision { return Err("Session reference revision is newer than this profile".into()); }
        for transition in self.reference_transitions.iter().filter(|entry| entry.revision > revision) {
            remap_tabs(tabs, &transition.from_book_id, &transition.to_book_id);
        }
        Ok(())
    }
}
fn remap_tabs(tabs: &mut [Tab], old: &str, new: &str) {
    for tab in tabs {
        if tab.book_id.as_deref() == Some(old) { tab.book_id = Some(new.into()); }
        for location in tab.back.iter_mut().chain(&mut tab.forward) {
            if location.book_id == old { location.book_id = new.into(); }
        }
    }
}

pub(crate) fn validate_state(state: &State) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    let mut bindings = std::collections::HashSet::new();
    for binding in &state.edition_bindings {
        if uuid::Uuid::parse_str(&binding.installation_id).is_err()
            || binding.source_id.is_empty() || binding.source_url.is_empty() || binding.entry_id.is_empty()
            || !bindings.insert((&binding.installation_id, &binding.source_id))
        { return Err("Saved catalog association is invalid".into()); }
    }
    for intent in &state.edition_intents {
        if uuid::Uuid::parse_str(&intent.id).is_err() || !ids.insert(&intent.id)
            || intent.destination.is_empty() || intent.destination.contains('\0')
            || intent.download_directory.is_empty() || intent.selection.source_id.is_empty()
            || intent.selection.entry_id.is_empty()
            || !matches!(intent.status.as_str(), "reserved" | "downloading" | "ready" | "committed" | "cancelled")
        { return Err("Saved edition intent is invalid; preserve the profile and restore its backup".into()); }
        validate_retained_book(&intent.target)?;
        if let Some(book) = &intent.verified { validate_retained_book(book)?; }
    }
    // Index the intent-to-transaction association once: the transaction loop
    // would otherwise rescan every intent per transaction.
    let committed: std::collections::HashMap<_, _> = state.edition_intents.iter()
        .map(|intent| (intent.id.as_str(), intent.transaction_id.as_deref()))
        .collect();
    let mut transaction_ids = std::collections::HashSet::new();
    for transaction in &state.edition_transactions {
        if !transaction_ids.insert(&transaction.id) || transaction.id.is_empty()
            || transaction.original.installation_id == transaction.replacement.installation_id
            || transaction.revision == 0 || transaction.revision > state.reference_revision
            || committed.get(transaction.intent_id.as_str()).copied().flatten() != Some(transaction.id.as_str())
        { return Err("Saved edition transaction is invalid".into()); }
        validate_retained_book(&transaction.original)?;
        validate_retained_book(&transaction.replacement)?;
        if let Some(book) = &transaction.rollback_book { validate_retained_book(book)?; }
    }
    let mut previous = 0u64;
    for transition in &state.reference_transitions {
        if transition.revision != previous.checked_add(1).ok_or("Invalid reference revision")?
            || transition.from_book_id.is_empty() || transition.to_book_id.is_empty()
            || transition.from_installation_id == transition.to_installation_id
            || !transaction_ids.contains(&transition.transaction_id)
        { return Err("Saved edition reference-transition history is invalid".into()); }
        previous = transition.revision;
    }
    if previous != state.reference_revision { return Err("Saved reference revision has missing transitions".into()); }
    Ok(())
}
fn validate_retained_book(book: &Book) -> Result<(), String> {
    if book.source_reference.as_ref().is_some_and(|source| !source.persistent) {
        return Err("Session-only provider references cannot be retained by an edition transaction".into());
    }
    let mut state = State::default();
    state.books.push(book.clone());
    // Validate ordinary Book invariants without recursively validating edition records.
    state.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{archive::ArchiveMetadata, store::{Bookmark, Highlight, HistoryEntry, TabLocation, WindowSession}};

    fn book(id: &str, date: &str) -> Book {
        Book {
            metadata: ArchiveMetadata {
                id: id.into(), title: "Encyclopedia".into(), description: String::new(),
                language: "eng".into(), creator: String::new(), publisher: String::new(),
                date: date.into(), name: "encyclopedia_eng".into(), tags: vec!["_flavour:maxi".into()],
                size: 512, article_count: 1, media_count: 0, has_fulltext_index: true,
                has_title_index: true, main_path: "A/Main".into(),
            },
            installation_id: new_installation_id(), path: format!("/archives/{id}.zim"),
            source_reference: None, added_at: "2025-01-01T00:00:00Z".into(),
            available: true, managed: false, integrity: None,
        }
    }
    fn setup(same_uuid: bool) -> (State, EditionIntent, Book) {
        let old = book("old", "2025-01");
        let mut new = book(if same_uuid { "old" } else { "new" }, "2026-01");
        new.path = "/archives/edition-output.zim".into();
        let selection = EditionSelection {
            source_id: "source".into(), source_url: "https://example.org/catalog".into(),
            entry_id: "opaque-publisher-edition".into(), catalog_generation: 3,
            entry: CatalogEntry {
                id: "opaque-publisher-edition".into(), title: new.metadata.title.clone(),
                description: String::new(), language: new.metadata.language.clone(),
                creator: String::new(), publisher: String::new(), date: new.metadata.date.clone(),
                name: new.metadata.name.clone(), tags: new.metadata.tags.clone(), size: 512,
                article_count: 1, media_count: 0, url: "https://example.org/new.zim".into(),
                thumbnail_url: None, torrent_url: None, torrent_resolver_url: None,
            },
        };
        let intent = EditionIntent {
            id: new_id(), selection, target: old.clone(),
            original_identity: serde_json::from_value(serde_json::json!([])).unwrap(),
            destination: new.path.clone(), download_directory: "/archives".into(),
            download_id: Some("download".into()), receipt: Some("download".into()),
            status: "ready".into(), verified: Some(new.clone()),
            verified_identity: Some(serde_json::from_value(serde_json::json!([])).unwrap()),
            transaction_id: None,
            lineage_associated: true,
        };
        let location = TabLocation { book_id: "old".into(), path: "A/Previous?q=1#part".into(), title: "Previous".into(), scroll: 8.0 };
        let tab = Tab {
            restricted: false,
            id: "tab".into(), book_id: Some("old".into()), path: "A/Current#fragment".into(),
            title: "Current".into(), scroll: 42.0, back: vec![location.clone()], forward: vec![location],
        };
        let mut state = State::default();
        state.books.push(old.clone());
        state.edition_intents.push(intent.clone());
        state.bookmarks.push(Bookmark {
            id: new_id(), book_id: "old".into(), path: "A/Current#fragment".into(),
            title: "Bookmark".into(), note: "Retain my note".into(), tags: vec!["personal".into()],
            highlights: vec![Highlight { id: new_id(), document_path: "A/Current".into(), exact: "quote".into(),
                prefix: String::new(), suffix: String::new(), start: 0, end: 5, comment: "My comment".into() }],
            created_at: "2025-01-01T00:00:00Z".into(), legacy_book: None,
        });
        state.history.push(HistoryEntry { id: "history".into(), book_id: "old".into(), path: "A/Current".into(),
            title: "History".into(), visited_at: "2025-01-01T00:00:00Z".into(), scroll: 10.0 });
        state.tabs.push(tab.clone());
        state.active_tab = Some("tab".into());
        state.windows.insert("main".into(), WindowSession { tabs: vec![tab], active_tab: Some("tab".into()), ..WindowSession::default() });
        state.settings.book_zoom.insert("old".into(), 1.5);
        state.settings.sharing_book_ids.push("old".into());
        state.settings.pinned_book_ids.push("old".into());
        state.reader_storage.insert(old.installation_id.clone(), serde_json::json!({"cookie": "private"}));
        (state, intent, new)
    }

    #[tokio::test]
    async fn edition_ready_rechecks_its_download_even_with_a_saved_verified_identity() {
        use sha2::{Digest, Sha256};
        use std::{fs, time::Duration};

        let root = std::env::temp_dir().join(format!("kiwix-edition-publication-{}", new_id()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let store = std::sync::Arc::new(Store::new(root.join("profile")).unwrap());
        let download_state = root.join("downloads");
        fs::create_dir(&download_state).unwrap();
        let path = root.join("edition.zim");
        let bytes = include_bytes!("../tests/fixtures/small.zim");
        fs::write(&path, bytes).unwrap();
        let path_str = path.to_str().unwrap();
        let book = crate::imports::load_book(path_str, true, false).unwrap();
        let (_, mut intent, _) = setup(false);
        let id = new_id();
        intent.destination = path_str.to_owned();
        intent.download_id = Some(id.clone());
        intent.status = "downloading".into();
        intent.receipt = None;
        intent.verified = None;
        intent.verified_identity = None;
        intent.selection.entry.name = book.metadata.name;
        intent.selection.entry.language = book.metadata.language;
        intent.selection.entry.date = book.metadata.date;
        intent.selection.entry.tags = book.metadata.tags;
        store.update(|state| {
            state.edition_intents.push(intent.clone());
            Ok(())
        }).unwrap();

        let ownership = crate::torrent::CleanupEntry::retain(&path, &fs::File::open(&path).unwrap()).unwrap();
        let digest = hex::encode(Sha256::digest(bytes));
        let journal = serde_json::json!({ (id.clone()): {
            "id": &id, "url": &intent.selection.entry.url, "fileName": "edition.zim",
            "destination": path_str, "status": "complete", "downloaded": bytes.len(),
            "total": bytes.len(), "speed": 0,
            "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z",
            "publication": {"size": bytes.len(), "sha256": digest},
            "publishedOwnership": ownership,
        }});
        fs::write(download_state.join("downloads.json"), serde_json::to_vec(&journal).unwrap()).unwrap();
        let downloads = DownloadManager::new(download_state, store.clone(), Default::default()).unwrap();
        let imports = ImportManager::new().unwrap();
        imports.enqueue_publication(&id, vec![path_str.to_owned()], downloads.import_publication(&id)).unwrap();
        let ready = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(ready) = imports.ready().into_iter().next() { break ready; }
                imports.changed().notified().await;
            }
        }).await.unwrap();

        let editions = EditionManager::new();
        let admitted = editions.ready(&store, &intent.id, &ready, &imports, &downloads).unwrap();
        assert!(admitted.verified_identity.is_some());
        downloads.remove_terminal(&id).unwrap();
        assert!(editions.ready(&store, &intent.id, &ready, &imports, &downloads).is_err());
        assert_eq!(imports.status(&id).unwrap().state, crate::imports::ImportState::Failed);
        assert_eq!(fs::read(&path).unwrap().as_slice(), bytes);
        imports.shutdown();
        drop(ready);
        drop(downloads);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relocation_preserves_retained_provider_and_rejects_substituted_expectations() {
        let (mut state, intent, verified) = setup(false);
        let mut transaction = state.replace_edition(&intent, &verified, "download").unwrap();
        let provider = crate::provider_reference::ProviderReference {
            reference: new_id(), display_name: "original.zim".into(), persistent: true,
        };
        transaction.original.path = provider.path_marker();
        transaction.original.source_reference = Some(provider.clone());
        state.edition_transactions[0] = transaction.clone();
        let before = transaction.replacement.clone();
        let mut after = before.clone();
        after.path = "/new-root/replacement.zim".into();
        after.managed = true;
        after.available = true;
        let saved = state.edition_transactions.clone();
        let mut forged = before.clone();
        forged.path = "/substituted/prior.zim".into();
        assert!(state.reconcile_edition_relocation(&forged, &after).is_err());
        assert_eq!(state.edition_transactions, saved);
        let mut fresh_principal = after.clone();
        fresh_principal.installation_id = new_installation_id();
        assert!(state.reconcile_edition_relocation(&before, &fresh_principal).is_err());
        assert_eq!(state.edition_transactions, saved);
        state.reconcile_edition_relocation(&before, &after).unwrap();
        let updated = &state.edition_transactions[0];
        assert_eq!(updated.replacement, after);
        assert_eq!(updated.original, transaction.original);
        assert_eq!(updated.original.source_reference, Some(provider));
        assert_eq!(updated.original_identity, transaction.original_identity);
        assert_eq!(updated.rollback_book, transaction.rollback_book);
        let mut rollback = transaction.original.clone();
        rollback.installation_id = new_installation_id();
        state.edition_transactions[0].rollback_book = Some(rollback.clone());
        let mut relocated_rollback = rollback.clone();
        relocated_rollback.path = "/new-root/restored.zim".into();
        relocated_rollback.source_reference = None;
        relocated_rollback.managed = true;
        state.reconcile_edition_relocation(&rollback, &relocated_rollback).unwrap();
        assert_eq!(state.edition_transactions[0].rollback_book, Some(relocated_rollback));
        assert_eq!(state.edition_transactions[0].original, transaction.original);
        assert_eq!(state.edition_transactions[0].original_identity, transaction.original_identity);
    }

    #[test]
    fn replacement_and_rollback_preserve_newer_reading_state_without_transferring_authority() {
        let (mut state, intent, verified) = setup(false);
        let original_principal = intent.target.installation_id.clone();
        let original_highlights = state.bookmarks[0].highlights.clone();
        let transaction = state.replace_edition(&intent, &verified, "download").unwrap();
        state.validate().unwrap();
        assert_eq!(state.books[0].added_at, intent.target.added_at);
        assert_ne!(state.books[0].installation_id, original_principal);
        assert_ne!(state.books[0].installation_id, verified.installation_id);
        assert_eq!(state.bookmarks[0].book_id, "new");
        assert_eq!(state.bookmarks[0].highlights, original_highlights);
        assert_eq!(state.settings.book_zoom.get("new"), Some(&1.5));
        assert!(state.settings.sharing_book_ids.is_empty());
        assert_eq!(state.settings.pinned_book_ids, ["new"]);
        assert!(state.reader_storage.is_empty());
        assert_eq!(state.tabs[0].back[0].book_id, "new");
        assert_eq!(state.windows["main"].tabs[0].forward[0].book_id, "new");
        state.bookmarks[0].note = "Edited after replacing".into();
        state.bookmarks[0].highlights[0].comment = "Edited highlight after replacing".into();
        state.history.push(HistoryEntry { id: "new-history".into(), book_id: "new".into(), path: "A/New".into(),
            title: "New reading".into(), visited_at: "2026-01-01T00:00:00Z".into(), scroll: 22.0 });
        state.settings.book_zoom.insert("new".into(), 2.0);
        let restored = state.rollback_edition(&transaction).unwrap();
        state.validate().unwrap();
        assert_ne!(restored.installation_id, original_principal);
        assert_ne!(restored.installation_id, transaction.replacement.installation_id);
        assert_eq!(state.bookmarks[0].note, "Edited after replacing");
        assert_eq!(state.bookmarks[0].highlights[0].comment, "Edited highlight after replacing");
        assert!(state.history.iter().all(|entry| entry.book_id == "old"));
        assert_eq!(state.settings.book_zoom.get("old"), Some(&2.0));
        assert_eq!(state.settings.pinned_book_ids, ["old"]);
        assert_eq!(state.reference_revision, 2);
        assert_eq!(state.reference_transitions.iter().map(|entry| entry.revision).collect::<Vec<_>>(), [1, 2]);
        assert!(state.rollback_edition(&transaction).is_err());
    }

    #[test]
    fn ordered_delayed_session_translation_handles_forward_rollback_and_same_uuid() {
        let (mut state, intent, verified) = setup(false);
        let mut before = state.tabs.clone();
        let committed = state.replace_edition(&intent, &verified, "download").unwrap();
        let mut during = state.tabs.clone();
        state.rollback_edition(&committed).unwrap();
        state.translate_reference_tabs(&mut before, 0).unwrap();
        state.translate_reference_tabs(&mut during, 1).unwrap();
        assert_eq!(before[0].book_id.as_deref(), Some("old"));
        assert_eq!(during[0].back[0].book_id, "old");
        assert_eq!(during[0].path, "A/Current#fragment");
        assert_eq!(during[0].scroll, 42.0);
        assert!(state.translate_reference_tabs(&mut before, 3).is_err());
        let (mut state, intent, verified) = setup(true);
        let committed = state.replace_edition(&intent, &verified, "download").unwrap();
        assert_eq!(state.tabs[0].book_id.as_deref(), Some("old"));
        assert_ne!(committed.original.installation_id, committed.replacement.installation_id);
        assert_eq!(state.reference_revision, 1);
    }

    #[test]
    fn reserved_destination_excludes_early_and_restarted_receipts_from_ordinary_import() {
        let (mut state, _, _) = setup(false);
        state.edition_intents[0].status = "reserved".into();
        state.edition_intents[0].download_id = None;
        state.edition_intents[0].receipt = None;
        let destination = state.edition_intents[0].destination.clone();
        assert!(EditionManager::owns_receipt(&state, "not-yet-bound", std::slice::from_ref(&destination)));
        let saved = serde_json::to_vec(&state).unwrap();
        let restored: State = serde_json::from_slice(&saved).unwrap();
        assert!(EditionManager::owns_receipt(&restored, "after-restart", &[destination]));
        assert!(!EditionManager::owns_receipt(&restored, "foreign", &["/archives/unrelated.zim".into()]));
        assert!(EditionManager::new().decisions.lock().unwrap().is_empty());
    }

    #[test]
    fn durable_cutover_is_atomic_and_survives_restart_without_reauthorizing() {
        let directory = std::env::temp_dir().join(format!("kiwix-edition-state-{}", new_id()));
        let store = Store::new(directory.clone()).unwrap();
        let (initial, intent, verified) = setup(false);
        store.update(|state| { *state = initial; Ok(()) }).unwrap();
        let bytes = std::fs::read(directory.join("state.json")).unwrap();
        let failed: Result<(), String> = store.update(|state| {
            state.replace_edition(&intent, &verified, "download")?;
            Err("Injected precommit failure".into())
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(directory.join("state.json")).unwrap(), bytes);
        assert_eq!(store.read().unwrap().books[0].metadata.id, "old");
        let transaction = store.update(|state| state.replace_edition(&intent, &verified, "download")).unwrap();
        drop(store);
        let store = Store::new(directory.clone()).unwrap();
        let restarted = store.read().unwrap();
        assert_eq!(restarted.edition_transactions[0], transaction);
        assert!(restarted.consumed_acquisitions.contains("download"));
        assert_eq!(restarted.reference_revision, 1);
        assert_eq!(restarted.books[0].metadata.id, "new");
        assert!(EditionManager::new().decisions.lock().unwrap().is_empty());
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_saved_intent_fails_closed_without_rewriting_profile() {
        let directory = std::env::temp_dir().join(format!("kiwix-edition-corrupt-{}", new_id()));
        let store = Store::new(directory.clone()).unwrap();
        let (state, _, _) = setup(false);
        store.update(|saved| { *saved = state; Ok(()) }).unwrap();
        drop(store);
        let path = directory.join("state.json");
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["editionIntents"] = serde_json::json!({"corrupt": "not-an-array"});
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(Store::new(directory.clone()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pending_and_retained_paths_are_excluded_except_exact_http_artifact() {
        let directory = std::env::temp_dir().join(format!("kiwix-edition-reserved-{}", new_id()));
        let store = Store::new(directory.clone()).unwrap();
        let (state, intent, verified) = setup(false);
        store.update(|saved| { *saved = state; Ok(()) }).unwrap();
        let candidate = std::path::Path::new(&intent.destination);
        assert!(store.with_acquisition_destination(candidate, || Ok(())).is_err());
        assert!(store.with_http_acquisition_destinations(&[candidate], "https://example.org/foreign.zim", || Ok(())).is_err());
        assert!(store.with_http_acquisition_destinations(&[candidate], &intent.selection.entry.url, || Ok(())).is_ok());
        store.update(|state| { state.edition_intents[0].status = "cancelled".into(); Ok(()) }).unwrap();
        assert!(store.with_acquisition_destination(candidate, || Ok(())).is_err());
        store.update(|state| { state.edition_intents[0].status = "ready".into(); Ok(()) }).unwrap();
        let transaction = store.update(|state| state.replace_edition(&intent, &verified, "download")).unwrap();
        assert!(store.with_acquisition_destination(std::path::Path::new(&transaction.original.path), || Ok(())).is_err());
        assert!(store.with_http_acquisition_destinations(&[candidate], &intent.selection.entry.url, || Ok(())).is_err());
        store.update(|state| state.rollback_edition(&transaction)).unwrap();
        assert!(store.with_acquisition_destination(candidate, || Ok(())).is_err());
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn artifact_checks_use_raw_uuid_promises_and_exact_metadata_variant() {
        let (_, intent, mut verified) = setup(false);
        let mut selection = intent.selection;
        let promised = new_id();
        selection.entry_id = "source-scoped-entry-hash".into();
        selection.entry.id = format!("urn:uuid:{promised}");
        verified.metadata.id = new_id();
        assert!(check_book(&selection, &verified, false).is_err());
        verified.metadata.id = promised.clone();
        assert!(check_book(&selection, &verified, false).is_ok());
        // A bare Atom ID that happens to parse as a UUID is still opaque.
        selection.entry.id = promised;
        verified.metadata.id = new_id();
        assert!(check_book(&selection, &verified, false).is_ok());
        verified.metadata.tags = vec!["_flavour:mini".into()];
        assert!(check_book(&selection, &verified, true).is_err());
        verified.metadata.tags = selection.entry.tags.clone();
        verified.metadata.date = "2024-01".into();
        assert!(check_book(&selection, &verified, false).is_err());
        verified.metadata.date = "2026-01-15".into();
        selection.entry.date = "2026-01-15T00:00:00Z".into();
        assert!(check_book(&selection, &verified, false).is_ok());
        selection.entry.date = "2026-01-16T00:00:00Z".into();
        assert!(check_book(&selection, &verified, false).is_err());
        selection.entry.date = "2026-01".into();
        assert!(check_book(&selection, &verified, false).is_err());
    }

    #[test]
    fn real_catalog_shape_requires_association_for_missing_archive_flavour() {
        let (_, intent, mut verified) = setup(false);
        let mut selection = intent.selection;
        let variants = vec!["_pictures:yes".into(), "_videos:no".into(), "_details:yes".into(), "_ftindex:yes".into()];
        verified.metadata.tags = variants.clone();
        verified.metadata.date = "2026-10-16".into();
        selection.entry.tags = variants;
        selection.entry.tags.push("_flavour:maxi".into());
        selection.entry.date = "2026-10-16T00:00:00Z".into();
        assert!(check_book(&selection, &verified, false).is_err());
        assert!(check_book(&selection, &verified, true).is_ok());
        verified.metadata.tags.push("_flavour:mini".into());
        assert!(check_book(&selection, &verified, true).is_err());
        verified.metadata.tags.pop();
        verified.metadata.tags.retain(|tag| tag != "_pictures:yes");
        assert!(check_book(&selection, &verified, true).is_err());
    }

    #[test]
    fn selection_reauthorization_ignores_generation_but_not_native_artifact_changes() {
        let (_, intent, _) = setup(false);
        let mut refreshed = intent.selection.clone();
        refreshed.catalog_generation += 1;
        assert!(same_selection(&intent.selection, &refreshed));
        refreshed.entry.url = "https://example.org/substituted.zim".into();
        assert!(!same_selection(&intent.selection, &refreshed));
    }
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    #[test]
    fn durable_reservation_survives_restart_but_window_and_policy_authority_do_not() {
        let directory = std::env::temp_dir().join(format!("kiwix-edition-intent-{}", new_id()));
        let store = Store::new(directory.clone()).unwrap();
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let mut old = crate::imports::load_book(fixture.to_str().unwrap(), false, false).unwrap();
        old.metadata.name = "encyclopedia_eng".into();
        old.metadata.language = "eng".into();
        old.metadata.date = "2025-01".into();
        old.metadata.tags = vec!["_flavour:maxi".into()];
        let root = "https://example.org/catalog";
        let selection = EditionSelection {
            source_id: crate::catalog::source_id(root).unwrap(), source_url: root.into(),
            entry_id: "opaque-newer-entry".into(), catalog_generation: 1,
            entry: CatalogEntry {
                id: "opaque-newer-entry".into(), title: old.metadata.title.clone(),
                description: String::new(), language: old.metadata.language.clone(),
                creator: String::new(), publisher: String::new(), date: "2026-01".into(),
                name: old.metadata.name.clone(), tags: old.metadata.tags.clone(), size: old.metadata.size,
                article_count: 1, media_count: 0, url: "https://example.org/new.zim".into(),
                thumbnail_url: None, torrent_url: None, torrent_resolver_url: None,
            },
        };
        // Real ZIM metadata may omit the flavour inferred by the catalog parser.
        old.metadata.tags.clear();
        store.update(|state| {
            state.books.push(old.clone());
            state.settings.catalog_url = root.into();
            state.settings.download_directory = directory.join("archives").to_string_lossy().into_owned();
            Ok(())
        }).unwrap();
        let manager = EditionManager::new();
        assert!(manager.prepare(&store, 7, "main", &old, selection.clone(), false).is_err());
        let prepared = manager.prepare(&store, 7, "main", &old, selection.clone(), true).unwrap();
        assert!(manager.reserve(&store, &prepared.token, 8, "main").is_err());
        assert!(manager.reserve(&store, &prepared.token, 7, "other-window").is_err());
        let reserved = manager.reserve(&store, &prepared.token, 7, "main").unwrap();
        assert!(reserved.lineage_associated);
        let mut different_flavour = selection.clone();
        different_flavour.entry.tags = vec!["_flavour:mini".into()];
        assert!(manager.prepare(&store, 7, "main", &old, different_flavour, false).is_err());
        let binding = store.read().unwrap().edition_bindings[0].clone();
        let selected_series = crate::catalog::series_key(&selection.entry.name,
            &selection.entry.language, &selection.entry.tags).unwrap();
        assert!(binding.matches_series(&selection.source_id, &selected_series));
        let mut legacy_binding = binding;
        legacy_binding.series = None;
        assert!(!legacy_binding.matches_series(&selection.source_id, &selected_series));
        assert!(std::path::Path::new(&reserved.destination).file_name().unwrap().to_string_lossy().starts_with("edition-"));
        assert!(!std::path::Path::new(&reserved.destination).exists());
        assert!(store.read().unwrap().edition_bindings.iter().any(|binding| binding.installation_id == old.installation_id));
        assert_eq!(manager.reserve(&store, &prepared.token, 7, "main").unwrap().id, reserved.id);
        store.update(|state| {
            let intent = state.edition_intents.iter_mut().find(|intent| intent.id == reserved.id).unwrap();
            intent.status = "ready".into();
            intent.download_id = Some("retained-transfer".into());
            intent.receipt = Some("retained-transfer".into());
            let mut verified = old.clone();
            verified.path = intent.destination.clone();
            verified.metadata.date = selection.entry.date.clone();
            intent.verified = Some(verified);
            intent.verified_identity = Some(intent.original_identity.clone());
            Ok(())
        }).unwrap();
        let retained = store.read().unwrap().edition_intents[0].clone();
        drop(manager);
        drop(store);
        let store = Store::new(directory.clone()).unwrap();
        let manager = EditionManager::new();
        assert!(manager.status(&store, 7, "main").unwrap()[0].needs_authorization);
        assert!(manager.reserve(&store, &prepared.token, 7, "main").is_err());
        let mut refreshed = selection;
        refreshed.catalog_generation += 1;
        let fresh = manager.prepare(&store, 7, "main", &old, refreshed, false).unwrap();
        assert_eq!(fresh.intent_id.as_deref(), Some(reserved.id.as_str()));
        assert_eq!(fresh.selection.catalog_generation, retained.selection.catalog_generation + 1);
        let saved = store.read().unwrap().edition_intents[0].clone();
        assert_eq!(saved.selection.catalog_generation, fresh.selection.catalog_generation);
        assert_eq!(saved.status, retained.status);
        assert_eq!(saved.download_id, retained.download_id);
        assert_eq!(saved.receipt, retained.receipt);
        assert_eq!(saved.destination, retained.destination);
        assert_eq!(serde_json::to_value(&saved.verified).unwrap(), serde_json::to_value(&retained.verified).unwrap());
        let mut substituted = fresh.selection.clone();
        substituted.entry.url = "https://example.org/substituted.zim".into();
        let changed = manager.prepare(&store, 7, "main", &old, substituted, false).unwrap();
        assert!(changed.intent_id.is_none());
        assert_eq!(serde_json::to_value(store.read().unwrap().edition_intents).unwrap(),
            serde_json::to_value(vec![saved]).unwrap());
        let fresh = manager.prepare(&store, 7, "main", &old, fresh.selection, false).unwrap();
        assert_eq!(store.read().unwrap().edition_intents.len(), 1);
        assert_eq!(manager.reserve(&store, &fresh.token, 7, "main").unwrap().id, reserved.id);
        let other = manager.prepare(&store, 7, "other-window", &old, fresh.selection.clone(), false).unwrap();
        manager.retire_window_authorizations("main");
        assert!(manager.reserve(&store, &fresh.token, 7, "main").is_err());
        assert_eq!(manager.reserve(&store, &other.token, 7, "other-window").unwrap().id, reserved.id);
        manager.retire_authorizations();
        assert!(manager.reserve(&store, &fresh.token, 7, "main").is_err());
        assert!(manager.reserve(&store, &other.token, 7, "other-window").is_err());
        drop(manager);
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reserve_rejects_an_original_rewritten_after_prepare() {
        use std::{fs, io::{Seek, SeekFrom, Write}};
        let directory = std::env::temp_dir().join(format!("kiwix-edition-original-{}", new_id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("original.zim");
        fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
            &source,
        ).unwrap();
        let mut old = crate::imports::load_book(source.to_str().unwrap(), false, false).unwrap();
        old.metadata.name = "encyclopedia_eng".into();
        old.metadata.language = "eng".into();
        old.metadata.date = "2025-01".into();
        old.metadata.tags = vec!["_flavour:maxi".into()];
        let root = "https://example.org/catalog";
        let selection = EditionSelection {
            source_id: crate::catalog::source_id(root).unwrap(), source_url: root.into(),
            entry_id: "opaque-newer-entry".into(), catalog_generation: 1,
            entry: CatalogEntry {
                id: "opaque-newer-entry".into(), title: old.metadata.title.clone(),
                description: String::new(), language: old.metadata.language.clone(),
                creator: String::new(), publisher: String::new(), date: "2026-01".into(),
                name: old.metadata.name.clone(), tags: old.metadata.tags.clone(), size: old.metadata.size,
                article_count: 1, media_count: 0, url: "https://example.org/new.zim".into(),
                thumbnail_url: None, torrent_url: None, torrent_resolver_url: None,
            },
        };
        // Real ZIM metadata may omit the flavour inferred by the catalog parser.
        old.metadata.tags.clear();
        let store = Store::new(directory.join("profile")).unwrap();
        store.update(|state| {
            state.books.push(old.clone());
            state.settings.catalog_url = root.into();
            state.settings.download_directory = directory.join("archives").to_string_lossy().into_owned();
            Ok(())
        }).unwrap();
        let manager = EditionManager::new();
        let prepared = manager.prepare(&store, 7, "main", &old, selection, true).unwrap();
        // Rewrite the original in place without changing its length, then put
        // mtime back. The stamp-only path still accepts it; the full content
        // identity (which includes ctime) must not.
        let modified = fs::metadata(&source).unwrap().modified().unwrap();
        let mut file = fs::OpenOptions::new().write(true).open(&source).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"changed!").unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::OpenOptions::new().write(true).open(&source).unwrap()
            .set_modified(modified).unwrap();
        {
            let decisions = manager.decisions.lock().unwrap();
            let decision = decisions.get(&prepared.token).unwrap();
            assert!(
                decision.source.unchanged().is_ok(),
                "the stamp check cannot see a rewrite whose mtime was restored"
            );
            assert!(recheck_original(decision).is_err());
        }
        let error = manager.reserve(&store, &prepared.token, 7, "main").unwrap_err();
        assert!(error.contains("installed archive changed"), "{error}");
        assert!(store.read().unwrap().edition_intents.is_empty());
        drop(manager);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_discarded_original_closes_its_transaction_and_refuses_rollback() {
        let (mut state, intent, verified) = setup(false);
        let transaction = state.replace_edition(&intent, &verified, "download").unwrap();
        assert!(!transaction.original_discarded);
        assert!(!state.original_discarded(&intent.target));
        // A profile written before this field existed loads as "not discarded".
        let mut saved = serde_json::to_value(&transaction).unwrap();
        assert_eq!(saved["originalDiscarded"], false);
        saved.as_object_mut().unwrap().remove("originalDiscarded");
        assert_eq!(serde_json::from_value::<EditionTransaction>(saved).unwrap(), transaction);

        let discarded = state.discard_edition_original(&transaction).unwrap();
        assert!(discarded.original_discarded);
        assert!(state.original_discarded(&intent.target));
        assert!(!state.original_discarded(&state.books[0]));
        state.validate().unwrap();
        // The current edition and every reference transition are untouched.
        assert_eq!(state.books[0], transaction.replacement);
        assert_eq!(state.reference_revision, transaction.revision);
        let error = state.rollback_edition(&discarded).unwrap_err();
        assert!(error.contains("was deleted") && error.contains("rolled back"), "{error}");
        assert_eq!(state.books[0], transaction.replacement);
        assert!(state.discard_edition_original(&transaction).unwrap_err().contains("already deleted"));
    }

    #[test]
    fn an_original_the_library_lists_again_is_never_discarded() {
        // After a rollback the original is the installed archive.
        let (mut state, intent, verified) = setup(false);
        let transaction = state.replace_edition(&intent, &verified, "download").unwrap();
        state.rollback_edition(&transaction).unwrap();
        let rolled_back = state.edition_transactions[0].clone();
        let error = state.discard_edition_original(&rolled_back).unwrap_err();
        assert!(error.contains("rolled back"), "{error}");
        assert!(!state.edition_transactions[0].original_discarded);

        // A library row at the original's location, however it got there.
        let (mut state, intent, verified) = setup(false);
        let transaction = state.replace_edition(&intent, &verified, "download").unwrap();
        let mut listed = book("other", "2024-01");
        listed.path.clone_from(&transaction.original.path);
        state.books.push(listed);
        let error = state.discard_edition_original(&transaction).unwrap_err();
        assert!(error.contains("is in the library"), "{error}");
        assert!(!state.edition_transactions[0].original_discarded);

        // A transaction whose original was swapped is not the one confirmed.
        let (mut state, intent, verified) = setup(false);
        let transaction = state.replace_edition(&intent, &verified, "download").unwrap();
        let mut forged = transaction.clone();
        forged.original.path = "/archives/another.zim".into();
        assert!(state.discard_edition_original(&forged).is_err());
        assert!(!state.edition_transactions[0].original_discarded);
    }

    /// The whole command against real files: the superseded original is
    /// deleted through the library's own deletion path, its name becomes
    /// available again, and rollback says why it is gone.
    #[cfg(desktop)]
    #[tokio::test]
    async fn discarding_a_retained_original_deletes_it_and_frees_its_name() {
        use serde_json::json;
        use std::{fs, path::Path};
        let root = std::env::temp_dir().join(format!("kiwix-edition-discard-{}", new_id()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let library = root.join("library");
        fs::create_dir(&library).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let service = crate::service::Service::new(root.join("profile"), false, false).unwrap();
        let generation = service.launch_generation();
        let commit = |name: &str| {
            let original_path = library.join(format!("{name}-original.zim"));
            let replacement_path = library.join(format!("{name}-replacement.zim"));
            fs::copy(&fixture, &original_path).unwrap();
            fs::copy(&fixture, &replacement_path).unwrap();
            let mut original = crate::imports::load_book(original_path.to_str().unwrap(), false, false).unwrap();
            original.metadata.date = "2024-01".into();
            let mut replacement = original.clone();
            replacement.path = replacement_path.to_string_lossy().into_owned();
            replacement.metadata.id = new_id();
            replacement.metadata.date = "2025-01".into();
            replacement.installation_id = new_installation_id();
            let entry: CatalogEntry = serde_json::from_value(json!({
                "id":format!("urn:uuid:{}", replacement.metadata.id),
                "title":replacement.metadata.title,"description":replacement.metadata.description,
                "language":replacement.metadata.language,"creator":replacement.metadata.creator,
                "publisher":replacement.metadata.publisher,"date":replacement.metadata.date,
                "name":replacement.metadata.name,"tags":replacement.metadata.tags,
                "size":replacement.metadata.size,"articleCount":replacement.metadata.article_count,
                "mediaCount":replacement.metadata.media_count,"url":format!("https://example.org/{name}.zim"),
                "thumbnailUrl":null,"torrentUrl":null
            })).unwrap();
            let receipt = format!("{name}-job");
            let intent = EditionIntent {
                id: new_id(),
                selection: EditionSelection {
                    source_id: "a".repeat(64), source_url: "https://example.org/catalog".into(),
                    entry_id: "b".repeat(64), catalog_generation: 1, entry,
                },
                lineage_associated: true,
                target: original.clone(),
                original_identity: archive_source::retain(&original).unwrap().identity().unwrap(),
                destination: replacement.path.clone(), download_directory: library.to_string_lossy().into_owned(),
                download_id: Some(receipt.clone()), receipt: Some(receipt.clone()),
                status: "ready".into(), verified: Some(replacement.clone()),
                verified_identity: None, transaction_id: None,
            };
            let transaction = service.store.update(|state| {
                state.settings.trash_deleted_files = false;
                state.books.retain(|book| book.metadata.id != original.metadata.id);
                state.books.push(original.clone());
                state.edition_intents.push(intent.clone());
                state.replace_edition(&intent, &replacement, &receipt)
            }).unwrap();
            // Only the transaction under test may claim this archive's UUID.
            service.store.update(|state| {
                state.books.retain(|book| book.installation_id != transaction.replacement.installation_id);
                Ok(())
            }).unwrap();
            (original_path, replacement_path, transaction)
        };
        let reserved = |path: &Path| service.store
            .with_acquisition_destination(path, || -> Result<(), String> { Ok(()) }).is_err();
        let import = |path: &Path| service.clone().dispatch("import_archives", json!({
            "paths":[path.to_string_lossy()],"privacyGeneration":generation
        }));
        let discard = |id: &str| service.clone().dispatch("edition_discard_original", json!({
            "transactionId":id,"privacyGeneration":generation
        }));

        let (original_path, replacement_path, transaction) = commit("first");
        // While the original is retained its name is reserved and not importable.
        assert!(reserved(&original_path));
        let refused = import(&original_path).await.unwrap();
        assert!(refused["errors"].to_string().contains("edition decision"), "{refused}");

        // A deletion that fails changes nothing: here the Trash is refused under test.
        service.store.update(|state| { state.settings.trash_deleted_files = true; Ok(()) }).unwrap();
        let error = discard(&transaction.id).await.unwrap_err();
        assert!(error.contains("Moving archive to Trash"), "{error}");
        assert_eq!(fs::read(&original_path).unwrap(), fs::read(&fixture).unwrap());
        assert!(!service.store.read().unwrap().edition_transactions[0].original_discarded);
        assert!(reserved(&original_path));
        service.store.update(|state| { state.settings.trash_deleted_files = false; Ok(()) }).unwrap();

        // An original that changed since the update is not the one retained.
        let saved = fs::read(&original_path).unwrap();
        fs::rename(&original_path, library.join("aside.zim")).unwrap();
        fs::write(&original_path, &saved).unwrap();
        let error = discard(&transaction.id).await.unwrap_err();
        assert!(error.contains("changed since this update"), "{error}");
        assert_eq!(fs::read(&original_path).unwrap(), saved);
        fs::remove_file(&original_path).unwrap();
        fs::rename(library.join("aside.zim"), &original_path).unwrap();
        // Renaming changed the inode's change time, so recorded and current
        // identity now differ for good: the person deletes such a file, and
        // the command then only closes the transaction.
        assert!(discard(&transaction.id).await.unwrap_err().contains("changed since this update"));
        assert!(original_path.exists());

        // The ordinary case, on a second untouched transaction.
        let (second_original, second_replacement, second) = commit("second");
        let reply = discard(&second.id).await.unwrap();
        assert_eq!(reply["originalFile"], "deleted", "{reply}");
        assert_eq!(reply["transaction"]["id"], second.id.as_str());
        assert_eq!(reply["transaction"]["originalDiscarded"], true);
        assert!(!second_original.exists());
        assert_eq!(fs::read(&second_replacement).unwrap(), fs::read(&fixture).unwrap());
        let mut names: Vec<_> = fs::read_dir(&library).unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["first-original.zim", "first-replacement.zim", "second-replacement.zim"],
            "only the discarded original may be gone, and no staging folder may remain");
        // Rollback and a second discard are refused, each saying why.
        let error = service.clone().dispatch("edition_rollback", json!({
            "transactionId":second.id,"expectedInstallationId":second.replacement.installation_id,
            "privacyGeneration":generation
        })).await.unwrap_err();
        assert!(error.contains("was deleted"), "{error}");
        assert!(discard(&second.id).await.unwrap_err().contains("already deleted"));
        // Its name can be published to and imported from again.
        assert!(!reserved(&second_original));
        fs::copy(&fixture, &second_original).unwrap();
        let readmitted = import(&second_original).await.unwrap();
        assert_eq!(readmitted["errors"], json!([]), "{readmitted}");
        assert_eq!(readmitted["books"][0]["path"], second_original.to_string_lossy().as_ref());
        // The retained original of the other transaction is still protected.
        assert!(reserved(&original_path));

        // A file the person already deleted leaves only the record to close.
        fs::remove_file(&original_path).unwrap();
        let reply = discard(&transaction.id).await.unwrap();
        assert_eq!(reply["originalFile"], "missing", "{reply}");
        assert_eq!(reply["transaction"]["originalDiscarded"], true);
        assert!(replacement_path.is_file());
        assert!(!reserved(&original_path));

        // The decision is durable.
        service.shutdown().await.unwrap();
        drop(service);
        let store = Store::new(root.join("profile")).unwrap();
        assert!(store.read().unwrap().edition_transactions.iter().all(|saved| saved.original_discarded));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_discarded_provider_original_gives_up_its_capability() {
        let directory = std::env::temp_dir().join(format!("kiwix-edition-provider-{}", new_id()));
        std::fs::create_dir(&directory).unwrap();
        let store = Store::new(directory.clone()).unwrap();
        let (_, mut intent, verified) = setup(false);
        let provider = crate::provider_reference::ProviderReference {
            reference: new_id(), display_name: "original.zim".into(), persistent: true,
        };
        intent.target.path = provider.path_marker();
        intent.target.source_reference = Some(provider.clone());
        let transaction = store.update(|state| {
            state.books.push(intent.target.clone());
            state.edition_intents.push(intent.clone());
            state.replace_edition(&intent, &verified, "download")
        }).unwrap();
        // Both the transaction and its committed intent retain the grant.
        assert!(store.retains_provider_reference(&provider.reference).unwrap());
        // Kiwix cannot delete from another app's provider: nothing is deleted,
        // and the reply says so instead of reporting a deletion.
        let (discarded, file) = EditionManager::new().discard_original(&store, &transaction.id, false).unwrap();
        assert_eq!(file, DiscardedFile::External);
        assert!(discarded.original_discarded);
        assert!(!store.retains_provider_reference(&provider.reference).unwrap());
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
