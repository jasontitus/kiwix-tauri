// SPDX-License-Identifier: GPL-3.0-or-later
//! Portable reading data only. Trust, transport, privacy gates and transactions belong to
//! the caller. Registers retain causal context; tombstones are never pruned implicitly.
//! `capture` runs at the local Store boundary for explicitly admitted categories,
//! continuing to track their edits and tombstones after sharing is disabled.
//! `apply` and `resolve` must run through the capture-suppressed Store transaction.
//! Bookmark roots use exact encoded article identity, not random local bookmark
//! IDs. Native projection bindings preserve local IDs while independent saves
//! converge. Conflicts commit to full causal revisions, including child records.
//! Reset retires all old replica identities before rebuilding visible data; a
//! forwarded pre-reset snapshot is rejected rather than resurrecting deletions.
use crate::store::{self, Book, Bookmark, Highlight, LegacyBookmarkBook, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const VERSION: u32 = 1;
pub const MAX_WIRE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 8_000;
const MAX_REPLICAS: usize = 128;
const MAX_VERSIONS: usize = 32;
const MAX_RETIRED_REPLICAS: usize = 1024;
const RETIRED: &str = "This snapshot predates a reading sync rebootstrap. Rebootstrap reading sync on the other device before reconnecting; stale data was not imported";
const FULL: &str = "Reading sync has reached its durable capacity. Rebootstrap your connected devices before adding more synchronized data; no deletion markers were discarded";
const ADMISSION_FULL: &str = "This category exceeds supported reading sync bounds. Leave reading positions or bookmarks off to sync the other category, or reduce local data before enabling. Your local profile was not changed";
type Clock = BTreeMap<String, u64>;

// Measure the complete encoding without allocating another multi-megabyte copy.
fn bounded_encoding(value: &impl Serialize) -> Result<(), String> {
    struct Size(usize);
    impl std::io::Write for Size {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.checked_add(bytes.len()).filter(|n| *n <= MAX_WIRE_BYTES)
                .ok_or_else(|| std::io::Error::other(FULL))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    serde_json::to_writer(Size(0), value).map_err(|_| FULL.to_string())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Categories {
    pub bookmarks: bool,
    pub positions: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Dot {
    replica: String,
    counter: u64,
}
impl Dot {
    fn id(&self) -> String { format!("{}:{}", self.replica, self.counter) }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Version<T> {
    dot: Dot,
    device_name: String,
    value: T,
}

/// Observed-remove multi-value register. Context includes retired values, not just
/// visible siblings, so replaying a pre-resolution snapshot cannot resurrect them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Register<T> {
    context: Clock,
    values: BTreeMap<String, Version<T>>,
}
impl<T> Default for Register<T> {
    fn default() -> Self { Self { context: Clock::new(), values: BTreeMap::new() } }
}
impl<T: Clone + PartialEq> Register<T> {
    fn value(&self) -> &T { &self.values.first_key_value().expect("validated nonempty register").1.value }
    fn set(&mut self, stamp: &Version<()>, value: T) {
        self.context.entry(stamp.dot.replica.clone()).and_modify(|n| *n = (*n).max(stamp.dot.counter)).or_insert(stamp.dot.counter);
        self.values.clear();
        self.values.insert(stamp.dot.id(), Version { dot: stamp.dot.clone(), device_name: stamp.device_name.clone(), value });
    }
    /// An ordinary editor displays one scalar, not the competing alternatives.
    /// Replacing that displayed scalar must not silently resolve its siblings.
    fn edit(&mut self, stamp: &Version<()>, value: T) {
        let visible = self.value().clone();
        if self.values.values().all(|v| v.value == visible) {
            self.set(stamp, value);
            return;
        }
        self.values.retain(|_, v| v.value != visible);
        self.context.entry(stamp.dot.replica.clone()).and_modify(|n| *n = (*n).max(stamp.dot.counter)).or_insert(stamp.dot.counter);
        self.values.insert(stamp.dot.id(), Version { dot: stamp.dot.clone(), device_name: stamp.device_name.clone(), value });
    }
    fn append(&mut self, stamp: &Version<()>, value: T) {
        if self.values.values().any(|v| v.value == value) { return; }
        self.context.entry(stamp.dot.replica.clone()).and_modify(|n| *n = (*n).max(stamp.dot.counter)).or_insert(stamp.dot.counter);
        self.values.insert(stamp.dot.id(), Version { dot: stamp.dot.clone(), device_name: stamp.device_name.clone(), value });
    }
    fn merge(&mut self, other: &Self) -> Result<(), String> {
        for (id, version) in &self.values {
            if let Some(incoming) = other.values.get(id) {
                if incoming != version { return Err("Reading sync contains an inconsistent causal event".into()); }
            }
        }
        self.values.retain(|id, v| other.values.contains_key(id) || !observed(&other.context, &v.dot));
        for (id, version) in &other.values {
            if self.values.contains_key(id) || !observed(&self.context, &version.dot) {
                self.values.insert(id.clone(), version.clone());
            }
        }
        for (replica, count) in &other.context {
            self.context.entry(replica.clone()).and_modify(|n| *n = (*n).max(*count)).or_insert(*count);
        }
        Ok(())
    }
}
fn observed(clock: &Clock, dot: &Dot) -> bool { clock.get(&dot.replica).copied().unwrap_or(0) >= dot.counter }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Anchor {
    document_path: String,
    exact: String,
    prefix: String,
    suffix: String,
    start: u32,
    end: u32,
}
impl Anchor {
    fn from_highlight(h: &Highlight) -> Self {
        Self { document_path: h.document_path.clone(), exact: h.exact.clone(), prefix: h.prefix.clone(), suffix: h.suffix.clone(), start: h.start, end: h.end }
    }
    fn highlight(&self, id: &str, comment: String) -> Highlight {
        Highlight { id: id.into(), document_path: self.document_path.clone(), exact: self.exact.clone(), prefix: self.prefix.clone(), suffix: self.suffix.clone(), start: self.start, end: self.end, comment }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HighlightRecord {
    deleted: bool,
    anchor: Register<Anchor>,
    comment: Register<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BookmarkRecord {
    deleted: bool,
    origin_ids: BTreeSet<String>,
    book_id: Register<String>,
    path: Register<String>,
    title: Register<String>,
    note: Register<String>,
    tags: Register<Vec<String>>,
    created_at: Register<String>,
    legacy_book: Register<Option<LegacyBookmarkBook>>,
    // An explicit re-add gets a new generation. A stale comment can never undo a
    // deletion in its original generation.
    highlights: BTreeMap<String, BTreeMap<String, HighlightRecord>>,
}
type Bookmarks = BTreeMap<String, BTreeMap<String, BookmarkRecord>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositionValue {
    book_id: String,
    path: String,
    title: String,
    scroll: f64,
}
type Positions = BTreeMap<String, BTreeMap<String, Register<PositionValue>>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingSync {
    pub replica_id: String,
    pub counter: u64,
    pub device_name: String,
    pub enabled: bool,
    pub categories: Categories,
    /// Admission is explicit and durable. Disabling consent later filters export,
    /// but must not stop capturing edits and deletion markers already admitted.
    #[serde(default)]
    pub tracked_categories: Categories,
    #[serde(default)]
    retired_replicas: BTreeSet<String>,
    // Native-only stable local IDs; never transmitted as projection authority.
    projection_ids: BTreeMap<String, String>,
    bookmarks: Bookmarks,
    positions: Positions,
}
impl Default for ReadingSync {
    fn default() -> Self {
        let replica_id = uuid::Uuid::new_v4().to_string();
        let device_name = format!("{} {}", std::env::consts::OS, &replica_id[..8]);
        Self { replica_id, counter: 0, device_name, enabled: false, categories: Categories::default(), tracked_categories: Categories::default(), retired_replicas: BTreeSet::new(), projection_ids: BTreeMap::new(), bookmarks: BTreeMap::new(), positions: BTreeMap::new() }
    }
}
impl ReadingSync {
    fn stamp(&mut self) -> Result<Version<()>, String> {
        self.counter = self.counter.checked_add(1).ok_or(FULL)?;
        Ok(Version { dot: Dot { replica: self.replica_id.clone(), counter: self.counter }, device_name: self.device_name.clone(), value: () })
    }
}

/// Deliberately excludes local replica counters, preferences, trust, paths,
/// archive installation principals, history timestamps and active tabs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSnapshot {
    version: u32,
    retired_replicas: BTreeSet<String>,
    bookmarks: Bookmarks,
    positions: Positions,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeSummary {
    pub changed: bool,
    pub conflicts: usize,
    pub positions: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alternative {
    pub id: String,
    pub device_name: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: String,
    pub title: String,
    pub field: String,
    pub alternatives: Vec<Alternative>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionCandidate {
    pub id: String,
    pub device_name: String,
    pub title: String,
    pub available: bool,
    pub book_id: String,
    pub path: String,
    pub scroll: f64,
}

/// Never decode percent escapes or normalize query/path spelling for identity.
/// `%23` is data; only an actual `#` starts the ignored fragment.
pub fn canonical_article_identity(book_id: &str, path: &str) -> String {
    let resource = path.split_once('#').map_or(path, |(resource, _)| resource);
    digest(&[book_id, resource])
}
fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts { hash.update((part.len() as u64).to_be_bytes()); hash.update(part.as_bytes()); }
    hex::encode(hash.finalize())
}
fn text(value: &str, max: usize, multiline: bool) -> Result<(), String> {
    if value.len() > max || value.chars().any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t'))) {
        Err("Reading sync contains invalid or oversized text".into())
    } else { Ok(()) }
}
fn identifier(value: &str) -> Result<(), String> {
    text(value, 512, false)?;
    if value.is_empty() { return Err("Reading sync contains an empty identifier".into()); }
    Ok(())
}
fn replica(value: &str) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| "Reading sync contains an invalid replica identity")?;
    if id.get_version_num() != 4 || id.to_string() != value { return Err("Reading sync contains an invalid replica identity".into()); }
    Ok(())
}
fn generation(value: &str) -> Result<(), String> {
    if value == "initial" { return Ok(()); }
    let (id, counter) = value.rsplit_once(':').ok_or("Reading sync contains an invalid generation")?;
    replica(id)?;
    let n = counter.parse::<u64>().map_err(|_| "Reading sync contains an invalid generation")?;
    if n == 0 || n.to_string() != counter { return Err("Reading sync contains an invalid generation".into()); }
    Ok(())
}
fn valid_birth(value: &str, context: &Clock) -> Result<(), String> {
    generation(value)?;
    if let Some((owner, counter)) = value.rsplit_once(':') {
        let counter = counter.parse::<u64>().map_err(|_| "Reading sync contains an invalid generation")?;
        if context.get(owner).copied().unwrap_or(0) < counter {
            return Err("Reading sync generation has no causal creation event".into());
        }
    }
    Ok(())
}
fn location(book_id: &str, path: &str) -> Result<(), String> {
    identifier(book_id)?;
    text(path, 16_384, false)?;
    let pathname = path.split(['?', '#']).next().unwrap_or("");
    if pathname.is_empty() || pathname.starts_with('/') || pathname.contains('\\') || pathname.split('/').next().is_some_and(|p| p.contains(':')) {
        return Err("Reading sync locations must be archive-relative references".into());
    }
    // Validate with the reader's parser, but retain the original encoded bytes.
    crate::server::decode_location_path(path)?;
    Ok(())
}
fn descriptor(value: &Option<LegacyBookmarkBook>) -> Result<(), String> {
    if let Some(b) = value {
        identifier(&b.id)?;
        for field in [&b.title, &b.name, &b.flavour, &b.language, &b.date] { text(field, 4096, false)?; }
    }
    Ok(())
}
fn valid_register<T: Clone + PartialEq>(reg: &Register<T>, check: impl Fn(&T) -> Result<(), String>) -> Result<(), String> {
    if reg.context.len() > MAX_REPLICAS || reg.values.len() > MAX_VERSIONS { return Err(FULL.into()); }
    if reg.values.is_empty() { return Err("Reading sync contains an empty causal register".into()); }
    for (id, counter) in &reg.context { replica(id)?; if *counter == 0 { return Err("Reading sync contains an invalid causal counter".into()); } }
    for (id, value) in &reg.values {
        replica(&value.dot.replica)?;
        if value.dot.counter == 0 || id != &value.dot.id() || !observed(&reg.context, &value.dot) { return Err("Reading sync contains an invalid causal event".into()); }
        text(&value.device_name, 128, false)?;
        check(&value.value)?;
    }
    Ok(())
}
fn valid_data<'a>(bookmarks: &'a Bookmarks, positions: &'a Positions, retired: &BTreeSet<String>) -> Result<BTreeSet<&'a str>, String> {
    if retired.len() > MAX_RETIRED_REPLICAS { return Err("Reading sync has reached its reset safety limit. Use a new profile; retired identities were not discarded".into()); }
    for id in retired { replica(id)?; }
    let mut count = 0usize;
    let mut all_replicas: BTreeSet<&str> = BTreeSet::new();
    for (id, generations) in bookmarks {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err("Reading sync contains an invalid canonical bookmark identity".into()); }
        if generations.is_empty() { return Err("Reading sync contains an empty bookmark generation set".into()); }
        if generations.values().filter(|b| !b.deleted).count() > MAX_VERSIONS { return Err(FULL.into()); }
        for (g, b) in generations {
            valid_birth(g, &b.book_id.context)?; count += 1;
            if b.origin_ids.is_empty() || b.origin_ids.len() > MAX_REPLICAS { return Err(FULL.into()); }
            for source_id in &b.origin_ids { identifier(source_id)?; }
            if let Some((owner, _)) = g.rsplit_once(':') { all_replicas.insert(owner); }
            valid_register(&b.book_id, |v| identifier(v))?;
            valid_register(&b.path, |v| location("archive", v))?;
            if b.book_id.values.values().any(|v| v.value != *b.book_id.value())
                || b.path.values.values().any(|v| canonical_article_identity(b.book_id.value(), &v.value) != *id) {
                return Err("Reading sync bookmark does not match its canonical article identity".into());
            }
            valid_register(&b.title, |v| text(v, 16_384, false))?;
            valid_register(&b.note, |v| text(v, 65_536, true))?;
            valid_register(&b.tags, |v| {
                if v.len() > 256 { return Err("Reading sync has too many tags".into()); }
                for tag in v { text(tag, 1024, false)?; }
                Ok(())
            })?;
            valid_register(&b.created_at, |v| crate::timestamps::parse(v).map(|_| ()).map_err(|_| "Reading sync contains an invalid creation timestamp".into()))?;
            valid_register(&b.legacy_book, descriptor)?;
            for (hid, highlights) in &b.highlights {
                if uuid::Uuid::parse_str(hid).ok().is_none_or(|id| id.to_string() != *hid) { return Err("Reading sync contains an invalid highlight identity".into()); }
                if highlights.is_empty() { return Err("Reading sync contains an empty highlight generation set".into()); }
                if highlights.values().filter(|h| !h.deleted).count() > MAX_VERSIONS { return Err(FULL.into()); }
                for (hg, h) in highlights {
                    valid_birth(hg, &h.anchor.context)?; count += 1;
                    if let Some((owner, _)) = hg.rsplit_once(':') { all_replicas.insert(owner); }
                    valid_register(&h.anchor, |a| {
                        location("archive", &a.document_path)?;
                        text(&a.exact, 65_536, true)?; text(&a.prefix, 512, true)?; text(&a.suffix, 512, true)?;
                        store::validate_highlights(&[a.highlight(hid, String::new())])
                    })?;
                    valid_register(&h.comment, |v| { text(v, 65_536, true)?; if v.encode_utf16().count() > 16_384 { return Err("Reading sync comment exceeds its allowed bounds".into()); } Ok(()) })?;
                }
            }
            for key in b.book_id.context.keys().chain(b.path.context.keys()).chain(b.title.context.keys()).chain(b.note.context.keys()).chain(b.tags.context.keys()).chain(b.created_at.context.keys()).chain(b.legacy_book.context.keys()) { all_replicas.insert(key); }
            for highlights in b.highlights.values() { for h in highlights.values() { all_replicas.extend(h.anchor.context.keys().map(String::as_str)); all_replicas.extend(h.comment.context.keys().map(String::as_str)); } }
        }
    }
    for (owner, entries) in positions {
        replica(owner)?; all_replicas.insert(owner.as_str());
        for (key, reg) in entries {
            count += 1;
            valid_register(reg, |p| {
                location(&p.book_id, &p.path)?; text(&p.title, 16_384, false)?;
                if !p.scroll.is_finite() || p.scroll < 0.0 { return Err("Reading sync requires finite nonnegative scroll positions".into()); }
                if key != &canonical_article_identity(&p.book_id, &p.path) { return Err("Reading sync contains an inconsistent article identity".into()); }
                Ok(())
            })?;
            if reg.values.values().any(|v| &v.dot.replica != owner) || reg.context.keys().any(|r| r != owner) { return Err("Reading sync position has the wrong replica owner".into()); }
        }
    }
    if all_replicas.iter().any(|id| retired.contains(*id)) { return Err(RETIRED.into()); }
    if count > MAX_RECORDS || all_replicas.len() > MAX_REPLICAS { return Err(FULL.into()); }
    Ok(all_replicas)
}
/// Decoding bookmark records may be tolerant only before any causal baseline or
/// deletion markers exist. A remote-only baseline can have local counter zero.
pub fn has_bookmark_history(sync: &ReadingSync) -> bool { !sync.bookmarks.is_empty() }

pub fn validate(sync: &ReadingSync) -> Result<(), String> {
    replica(&sync.replica_id)?; text(&sync.device_name, 128, false)?;
    if sync.retired_replicas.contains(&sync.replica_id) { return Err(RETIRED.into()); }
    let mut local_ids = BTreeSet::new();
    for (key, id) in &sync.projection_ids {
        identifier(id)?;
        if !sync.bookmarks.contains_key(key) || !local_ids.insert(id) { return Err("Reading sync has inconsistent local bookmark identities".into()); }
    }
    if sync.bookmarks.iter().any(|(key, gs)| live_bookmark(gs).is_some() && !sync.projection_ids.contains_key(key)) {
        return Err("Reading sync has no local bookmark projection identity".into());
    }
    let known = valid_data(&sync.bookmarks, &sync.positions, &sync.retired_replicas)?;
    // Reserve the local writer's slot, even before its first edit. Otherwise a
    // full remote vector could be accepted but could never be resolved locally.
    if known.len() + usize::from(!known.contains(sync.replica_id.as_str())) > MAX_REPLICAS { return Err(FULL.into()); }
    bounded_encoding(sync)?;
    validate_local_counter(sync)
}
fn validate_local_counter(sync: &ReadingSync) -> Result<(), String> {
    fn check<T>(r: &Register<T>, id: &str, n: u64) -> Result<(), String> {
        if r.context.get(id).copied().unwrap_or(0) > n { return Err("Reading sync replica counter rolled back; rebootstrap this device".into()); }
        Ok(())
    }
    for gs in sync.bookmarks.values() { for b in gs.values() {
        check(&b.book_id, &sync.replica_id, sync.counter)?; check(&b.path, &sync.replica_id, sync.counter)?;
        check(&b.title, &sync.replica_id, sync.counter)?; check(&b.note, &sync.replica_id, sync.counter)?;
        check(&b.tags, &sync.replica_id, sync.counter)?; check(&b.created_at, &sync.replica_id, sync.counter)?; check(&b.legacy_book, &sync.replica_id, sync.counter)?;
        for hs in b.highlights.values() { for h in hs.values() { check(&h.anchor, &sync.replica_id, sync.counter)?; check(&h.comment, &sync.replica_id, sync.counter)?; } }
    } }
    for ps in sync.positions.values() { for p in ps.values() { check(p, &sync.replica_id, sync.counter)?; } }
    Ok(())
}

fn single<T: Clone + PartialEq>(stamp: &Version<()>, value: T) -> Register<T> { let mut r = Register::default(); r.set(stamp, value); r }
fn new_highlight(h: &Highlight, stamp: &Version<()>) -> HighlightRecord {
    HighlightRecord { deleted: false, anchor: single(stamp, Anchor::from_highlight(h)), comment: single(stamp, h.comment.clone()) }
}
/// Library books by ID, built once per capture instead of scanned per bookmark.
type BookIndex<'a> = BTreeMap<&'a str, &'a Book>;

fn book_index(state: &State) -> BookIndex<'_> {
    let mut index = BTreeMap::new();
    for book in &state.books {
        // Keep the first row for an ID, matching the earlier linear `find`.
        index.entry(book.metadata.id.as_str()).or_insert(book);
    }
    index
}
fn enriched_descriptor(books: &BookIndex<'_>, b: &Bookmark) -> Option<LegacyBookmarkBook> {
    b.legacy_book.clone().or_else(|| books.get(b.book_id.as_str()).map(|book| LegacyBookmarkBook {
        id: book.metadata.id.clone(), title: book.metadata.title.clone(), name: book.metadata.name.clone(), flavour: String::new(), language: book.metadata.language.clone(), date: book.metadata.date.clone(),
    }))
}
fn portable_bookmark(books: &BookIndex<'_>, b: &Bookmark) -> Bookmark {
    let mut portable = b.clone();
    portable.legacy_book = enriched_descriptor(books, b);
    for h in &mut portable.highlights { h.id.make_ascii_lowercase(); }
    portable
}
fn new_bookmark(b: &Bookmark, stamp: &Version<()>) -> BookmarkRecord {
    BookmarkRecord { deleted: false, origin_ids: BTreeSet::from([b.id.clone()]), book_id: single(stamp, b.book_id.clone()), path: single(stamp, b.path.clone()), title: single(stamp, b.title.clone()), note: single(stamp, b.note.clone()), tags: single(stamp, b.tags.clone()), created_at: single(stamp, b.created_at.clone()), legacy_book: single(stamp, b.legacy_book.clone()), highlights: b.highlights.iter().map(|h| (h.id.to_ascii_lowercase(), BTreeMap::from([("initial".into(), new_highlight(h, stamp))]))).collect() }
}
fn live_bookmark(gs: &BTreeMap<String, BookmarkRecord>) -> Option<(&String, &BookmarkRecord)> { gs.iter().rev().find(|(_, b)| !b.deleted) }
fn live_highlight(gs: &BTreeMap<String, HighlightRecord>) -> Option<(&String, &HighlightRecord)> { gs.iter().rev().find(|(_, h)| !h.deleted) }
fn edit_bookmark(record: &mut BookmarkRecord, before: &Bookmark, after: &Bookmark, stamp: &Version<()>) {
    macro_rules! field { ($name:ident) => { if before.$name != after.$name { record.$name.edit(stamp, after.$name.clone()); } }; }
    field!(book_id); field!(path); field!(title); field!(note); field!(tags); field!(created_at); field!(legacy_book);
    for old in &before.highlights {
        if !after.highlights.iter().any(|h| h.id.eq_ignore_ascii_case(&old.id)) {
            if let Some(gs) = record.highlights.get_mut(&old.id.to_ascii_lowercase()) { for h in gs.values_mut() { h.deleted = true; } }
        }
    }
    for h in &after.highlights {
        let key = h.id.to_ascii_lowercase();
        let gs = record.highlights.entry(key).or_default();
        let old = before.highlights.iter().find(|old| old.id.eq_ignore_ascii_case(&h.id));
        let live = live_highlight(gs).map(|(g, _)| g.clone());
        match (old, live) {
            (Some(old), Some(g)) => {
                let record = gs.get_mut(&g).expect("selected highlight generation");
                if Anchor::from_highlight(old) != Anchor::from_highlight(h) { record.anchor.edit(stamp, Anchor::from_highlight(h)); }
                if old.comment != h.comment { record.comment.edit(stamp, h.comment.clone()); }
            }
            _ => { let g = if gs.is_empty() { "initial".into() } else { stamp.dot.id() }; gs.insert(g, new_highlight(h, stamp)); }
        }
    }
}

fn bookmark_groups(state: &State) -> Result<BTreeMap<String, Vec<Bookmark>>, String> {
    let mut ids = BTreeSet::new();
    let books = book_index(state);
    let mut groups: BTreeMap<String, Vec<Bookmark>> = BTreeMap::new();
    for b in &state.bookmarks {
        store::validate_bookmark(b)?;
        if !ids.insert(b.id.as_str()) { return Err("Reading sync contains duplicate local bookmark IDs".into()); }
        groups.entry(canonical_article_identity(&b.book_id, &b.path)).or_default().push(portable_bookmark(&books, b));
    }
    Ok(groups)
}
fn same_bookmark_content(a: &Bookmark, b: &Bookmark) -> bool {
    a.book_id == b.book_id && a.path == b.path && a.title == b.title && a.note == b.note
        && a.tags == b.tags && a.highlights == b.highlights && a.created_at == b.created_at && a.legacy_book == b.legacy_book
}
fn append_bookmark(record: &mut BookmarkRecord, b: &Bookmark, stamp: &Version<()>) {
    record.origin_ids.insert(b.id.clone());
    macro_rules! field { ($name:ident) => { record.$name.append(stamp, b.$name.clone()); }; }
    field!(book_id); field!(path); field!(title); field!(note); field!(tags); field!(created_at); field!(legacy_book);
    for h in &b.highlights {
        let gs = record.highlights.entry(h.id.clone()).or_default();
        if let Some(g) = live_highlight(gs).map(|(g, _)| g.clone()) {
            let existing = gs.get_mut(&g).expect("selected highlight generation");
            existing.anchor.append(stamp, Anchor::from_highlight(h));
            existing.comment.append(stamp, h.comment.clone());
        } else {
            let g = if gs.is_empty() { "initial".into() } else { stamp.dot.id() };
            gs.insert(g, new_highlight(h, stamp));
        }
    }
}
fn new_group(sync: &mut ReadingSync, group: &[Bookmark]) -> Result<(String, BookmarkRecord), String> {
    let stamp = sync.stamp()?;
    let mut record = new_bookmark(&group[0], &stamp);
    for b in &group[1..] { let stamp = sync.stamp()?; append_bookmark(&mut record, b, &stamp); }
    Ok((stamp.dot.id(), record))
}
/// Reverse index of `projection_ids` (local ID → keys currently claiming it), so
/// reassigning one local identity costs O(1) instead of scanning every entry.
type ProjectionOwners = BTreeMap<String, BTreeSet<String>>;

fn projection_owners(projection_ids: &BTreeMap<String, String>) -> ProjectionOwners {
    let mut owners: ProjectionOwners = BTreeMap::new();
    for (key, id) in projection_ids {
        owners.entry(id.clone()).or_default().insert(key.clone());
    }
    owners
}

/// Take `id` for `key`. A local ID may be referenced by several keys after an
/// article move, but only the current key may keep it: every other claimant is
/// dropped, exactly as the previous `retain(|k, v| k == key || v != id)` did.
fn claim_projection_id(
    projection_ids: &mut BTreeMap<String, String>,
    owners: &mut ProjectionOwners,
    key: &str,
    id: &str,
) {
    if let Some(previous) = projection_ids.get(key).and_then(|previous| owners.get_mut(previous)) {
        previous.remove(key);
    }
    if let Some(claimants) = owners.get(id) {
        for other in claimants.iter().filter(|other| other.as_str() != key) {
            projection_ids.remove(other.as_str());
        }
    }
    owners.insert(id.to_string(), BTreeSet::from([key.to_string()]));
    projection_ids.insert(key.to_string(), id.to_string());
}

fn ensure_projection_ids(sync: &mut ReadingSync) {
    let mut used: BTreeSet<String> = sync.projection_ids.iter()
        .filter(|(key, _)| sync.bookmarks.get(*key).and_then(live_bookmark).is_some())
        .map(|(_, id)| id.clone()).collect();
    let mut owners = projection_owners(&sync.projection_ids);
    for (key, gs) in &sync.bookmarks {
        let Some((_, record)) = live_bookmark(gs) else { continue; };
        if sync.projection_ids.contains_key(key) { continue; }
        let id = if let Some(id) = record.origin_ids.iter().find(|id| !used.contains(*id)) { id.clone() }
        else {
            let base = format!("reading-{key}");
            let mut candidate = base.clone();
            let mut suffix = 0;
            while used.contains(&candidate) { suffix += 1; candidate = format!("{base}-{suffix}"); }
            candidate
        };
        used.insert(id.clone());
        // Reuse a local ID carried through an article move, but never take an
        // identity from another live article or grant it native authority.
        claim_projection_id(&mut sync.projection_ids, &mut owners, key, &id);
    }
}

#[derive(PartialEq)]
struct PositionView<'a> { book_id: &'a str, path: &'a str, title: &'a str, scroll: f64 }
impl PositionView<'_> {
    fn matches(&self, value: &PositionValue) -> bool {
        self.book_id == value.book_id && self.path == value.path && self.title == value.title && self.scroll == value.scroll
    }
    fn owned(&self) -> PositionValue {
        PositionValue { book_id: self.book_id.into(), path: self.path.into(), title: self.title.into(), scroll: self.scroll }
    }
}
fn position_producers(state: &State) -> BTreeMap<(u8, &str, &str), PositionView<'_>> {
    let mut values = BTreeMap::new();
    for entry in &state.history {
        values.insert((0, "", entry.id.as_str()), PositionView { book_id: &entry.book_id, path: &entry.path, title: &entry.title, scroll: entry.scroll });
    }
    for tab in &state.tabs {
        if let Some(book) = &tab.book_id {
            if !tab.path.is_empty() { values.insert((1, "", tab.id.as_str()), PositionView { book_id: book, path: &tab.path, title: &tab.title, scroll: tab.scroll }); }
        }
    }
    for (window, session) in &state.windows {
        for tab in &session.tabs {
            if let Some(book) = &tab.book_id {
                if !tab.path.is_empty() { values.insert((2, window.as_str(), tab.id.as_str()), PositionView { book_id: book, path: &tab.path, title: &tab.title, scroll: tab.scroll }); }
            }
        }
    }
    values
}
fn capture_positions(before: &State, after: &State, sync: &mut ReadingSync, bootstrap: bool) -> Result<(), String> {
    // The normal snapshot restored on a privacy/read-only cutover is not a new
    // reading gesture. Runtime privacy is additionally gated by the native Store.
    if before.settings.private_mode || before.settings.read_only
        || after.settings.private_mode || after.settings.read_only { return Ok(()); }
    let previous = position_producers(before);
    let mut candidates = BTreeMap::new();
    for (producer, value) in position_producers(after) {
        let key = canonical_article_identity(value.book_id, value.path);
        let known = sync.positions.get(&sync.replica_id).is_some_and(|entries| entries.contains_key(&key));
        if previous.get(&producer) == Some(&value) && (!bootstrap || known) { continue; }
        // After first admission only touched producers compete. Lexical
        // window/tab IDs break ties, never a wall clock or maximum scroll.
        candidates.insert(key, value);
    }
    for (key, view) in candidates {
        if sync.positions.get(&sync.replica_id).and_then(|entries| entries.get(&key)).is_some_and(|r| view.matches(r.value())) { continue; }
        let stamp = sync.stamp()?;
        sync.positions.entry(stamp.dot.replica.clone()).or_default().entry(key).or_default().set(&stamp, view.owned());
    }
    Ok(())
}

/// Explicitly enabled category consent admits local data atomically. Once a
/// category is tracked, disabling export does not stop capturing its mutations.
/// History removal, visitedAt-only touches and retention never delete positions.
pub fn capture(before: &State, after: &mut State) -> Result<(), String> {
    if after.settings.private_mode || after.settings.read_only { return Ok(()); }
    let mut sync = after.reading_sync.clone();
    let admitting_bookmarks = !sync.tracked_categories.bookmarks && sync.enabled && sync.categories.bookmarks;
    let admitting_positions = !sync.tracked_categories.positions && sync.enabled && sync.categories.positions;
    sync.tracked_categories.bookmarks |= sync.enabled && sync.categories.bookmarks;
    sync.tracked_categories.positions |= sync.enabled && sync.categories.positions;
    if !sync.tracked_categories.bookmarks && !sync.tracked_categories.positions { return Ok(()); }
    if sync.tracked_categories.bookmarks {
        let before_groups = bookmark_groups(before)?;
        let after_groups = bookmark_groups(after)?;
        // Seed before diffing: even a first-transaction deletion leaves a tombstone.
        let mut projected: BTreeSet<String> = sync.projection_ids.values().cloned().collect();
        for (key, group) in &before_groups {
            if !sync.bookmarks.contains_key(key) && !group.iter().any(|b| projected.contains(&b.id)) {
                let (_, record) = new_group(&mut sync, group)?;
                sync.bookmarks.insert(key.clone(), BTreeMap::from([("initial".into(), record)]));
                sync.projection_ids.insert(key.clone(), group[0].id.clone());
                projected.insert(group[0].id.clone());
            }
        }
        ensure_projection_ids(&mut sync);
        // A bookmark removed from the live profile keeps its tombstone so a stale
        // snapshot cannot resurrect it. Keys absent from `after` cannot be edited
        // below, so this runs before the baseline is materialized.
        for (key, gs) in &mut sync.bookmarks {
            if !after_groups.contains_key(key.as_str()) && live_bookmark(gs).is_some() {
                for b in gs.values_mut() { b.deleted = true; }
            }
        }
        // Durable baseline captures explicit bookmark writes deferred while private.
        // Only articles this transaction still holds need their pre-edit content.
        let mut baseline: BTreeMap<&str, Bookmark> = BTreeMap::new();
        for key in after_groups.keys() {
            if let Some(bookmark) = projected_bookmark(&sync, key)? { baseline.insert(key.as_str(), bookmark); }
        }
        let mut owners = projection_owners(&sync.projection_ids);
        for (key, group) in &after_groups {
            let old = baseline.get(key.as_str());
            let primary = old.and_then(|old| group.iter().find(|b| b.id == old.id)).unwrap_or(&group[0]);
            // IDs remain native-local even when independent pre-pair saves converge.
            claim_projection_id(&mut sync.projection_ids, &mut owners, key, &primary.id);
            let live = sync.bookmarks.get(key).and_then(live_bookmark).map(|(g, _)| g.clone());
            if let (Some(old), Some(g)) = (old, live) {
                if !same_bookmark_content(old, primary) {
                    let stamp = sync.stamp()?;
                    edit_bookmark(sync.bookmarks.get_mut(key).expect("existing bookmark").get_mut(&g).expect("existing generation"), old, primary, &stamp);
                }
                for extra in group.iter().filter(|b| b.id != primary.id) {
                    let stamp = sync.stamp()?;
                    append_bookmark(sync.bookmarks.get_mut(key).expect("existing bookmark").get_mut(&g).expect("existing generation"), extra, &stamp);
                }
            } else {
                let (birth, record) = new_group(&mut sync, group)?;
                let gs = sync.bookmarks.entry(key.clone()).or_default();
                let g = if gs.is_empty() { "initial".into() } else { birth };
                gs.insert(g, record);
            }
        }
    }
    if sync.tracked_categories.positions { capture_positions(before, after, &mut sync, admitting_positions)?; }
    ensure_projection_ids(&mut sync);
    // One validation per mutation: the pre-mutation document is covered by the
    // same rules, and every edit above only appends to already-valid registers.
    validate(&sync).map_err(|error| {
        if (admitting_bookmarks || admitting_positions) && error == FULL { ADMISSION_FULL.to_string() } else { error }
    })?;
    // Keep the materialized scalar aligned with the durable conflict projection.
    let bookmarks = if sync.tracked_categories.bookmarks { Some(projected_bookmarks(&sync)?) } else { None };
    after.reading_sync = sync;
    if let Some(bookmarks) = bookmarks { after.bookmarks = bookmarks; }
    Ok(())
}

/// Explicit user-confirmed reset only. The caller must revoke all peer trust in
/// the same durable transaction and close every old transfer before committing.
/// Both sides must reset when either still carries retired causal identities.
/// Retirement barriers are forwarded and never silently pruned.
pub fn rebootstrap(state: &mut State) -> Result<(), String> { rebootstrap_inner(state, false) }

/// Explicitly confirmed capacity recovery: retire the old causal identity and
/// forget all sync position candidates, without deleting history or changing
/// live tabs/bookmarks. The caller revokes every peer in this same transaction.
/// Unchanged history/session producers stay forgotten until a new reading edit.
pub fn rebootstrap_forgetting_positions(state: &mut State) -> Result<(), String> {
    rebootstrap_inner(state, true)
}
fn rebootstrap_inner(state: &mut State, forget_positions: bool) -> Result<(), String> {
    if !conflicts(state).is_empty() {
        return Err("Resolve all reading conflicts before rebootstrap so no alternatives are discarded".into());
    }
    let old = &state.reading_sync;
    let mut retired = old.retired_replicas.clone();
    retired.extend(valid_data(&old.bookmarks, &old.positions, &old.retired_replicas)?.into_iter().map(str::to_owned));
    retired.insert(old.replica_id.clone());
    if retired.len() > MAX_RETIRED_REPLICAS {
        return Err("Reading sync has reached its reset safety limit. Use a new profile; retired identities were not discarded".into());
    }
    let mut seed = State::default();
    // Reset rebuilds only categories already admitted, not a pending selection.
    seed.reading_sync.enabled = false;
    seed.reading_sync.categories = old.categories;
    seed.reading_sync.tracked_categories = old.tracked_categories;
    seed.reading_sync.device_name = old.device_name.clone();
    seed.reading_sync.retired_replicas = retired;
    if old.tracked_categories.bookmarks {
        let books = book_index(state);
        seed.bookmarks = state.bookmarks.iter().map(|b| portable_bookmark(&books, b)).collect();
    }
    if old.tracked_categories.positions { seed.history = state.history.clone(); }
    // Normal reset and history retention preserve all own saved positions.
    // Only the separately confirmed capacity-recovery path forgets them.
    if let Some(entries) = old.positions.get(&old.replica_id).filter(|_| old.tracked_categories.positions && !forget_positions) {
        let owner = seed.reading_sync.replica_id.clone();
        for (key, reg) in entries {
            let mut replacement = Register::default();
            for version in reg.values.values() {
                let stamp = seed.reading_sync.stamp()?;
                replacement.context.insert(owner.clone(), stamp.dot.counter);
                replacement.values.insert(stamp.dot.id(), Version { dot: stamp.dot, device_name: version.device_name.clone(), value: version.value.clone() });
            }
            seed.reading_sync.positions.entry(owner.clone()).or_default().insert(key.clone(), replacement);
        }
    }
    capture(state, &mut seed)?;
    seed.reading_sync.enabled = old.enabled;
    validate(&seed.reading_sync)?;
    state.reading_sync = seed.reading_sync;
    Ok(())
}

pub fn export(state: &State, categories: Categories) -> Result<WireSnapshot, String> {
    validate(&state.reading_sync)?;
    let snapshot = WireSnapshot { version: VERSION, retired_replicas: state.reading_sync.retired_replicas.clone(), bookmarks: if categories.bookmarks { state.reading_sync.bookmarks.clone() } else { BTreeMap::new() }, positions: if categories.positions { state.reading_sync.positions.clone() } else { BTreeMap::new() } };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}
fn validate_snapshot(snapshot: &WireSnapshot) -> Result<(), String> {
    if snapshot.version != VERSION { return Err("Unsupported reading sync snapshot version".into()); }
    valid_data(&snapshot.bookmarks, &snapshot.positions, &snapshot.retired_replicas)?;
    bounded_encoding(snapshot)
}
fn merge_bookmark(b: &mut BookmarkRecord, other: &BookmarkRecord) -> Result<(), String> {
    b.deleted |= other.deleted;
    b.origin_ids.extend(other.origin_ids.iter().cloned());
    b.book_id.merge(&other.book_id)?; b.path.merge(&other.path)?; b.title.merge(&other.title)?; b.note.merge(&other.note)?;
    b.tags.merge(&other.tags)?; b.created_at.merge(&other.created_at)?; b.legacy_book.merge(&other.legacy_book)?;
    for (id, gs) in &other.highlights {
        let local = b.highlights.entry(id.clone()).or_default();
        for (g, h) in gs {
            if let Some(existing) = local.get_mut(g) { existing.deleted |= h.deleted; existing.anchor.merge(&h.anchor)?; existing.comment.merge(&h.comment)?; }
            else { local.insert(g.clone(), h.clone()); }
        }
    }
    Ok(())
}
fn projected_bookmark(sync: &ReadingSync, id: &str) -> Result<Option<Bookmark>, String> {
    let Some(gs) = sync.bookmarks.get(id) else { return Ok(None) };
    let Some((_, b)) = live_bookmark(gs) else { return Ok(None) };
    let mut highlights = Vec::new();
    for (hid, hgs) in &b.highlights {
        if let Some((_, h)) = live_highlight(hgs) { highlights.push(h.anchor.value().highlight(hid, h.comment.value().clone())); }
    }
    let bookmark = Bookmark { id: sync.projection_ids.get(id).ok_or("Reading sync has no local bookmark projection identity")?.clone(), book_id: b.book_id.value().clone(), path: b.path.value().clone(), title: b.title.value().clone(), note: b.note.value().clone(), tags: b.tags.value().clone(), highlights, created_at: b.created_at.value().clone(), legacy_book: b.legacy_book.value().clone() };
    store::validate_bookmark(&bookmark)?;
    Ok(Some(bookmark))
}
fn projected_bookmarks(sync: &ReadingSync) -> Result<Vec<Bookmark>, String> {
    let mut result = Vec::new();
    for id in sync.bookmarks.keys() {
        if let Some(bookmark) = projected_bookmark(sync, id)? { result.push(bookmark); }
    }
    Ok(result)
}

/// Validates the entire snapshot before changing the caller's transaction state.
/// Does not touch local history, tabs, live reader scroll or local op counters.
pub fn apply(state: &mut State, snapshot: WireSnapshot, categories: Categories) -> Result<MergeSummary, String> {
    validate_snapshot(&snapshot)?;
    let mut sync = state.reading_sync.clone();
    sync.retired_replicas.extend(snapshot.retired_replicas);
    sync.tracked_categories.bookmarks |= categories.bookmarks;
    sync.tracked_categories.positions |= categories.positions;
    if categories.bookmarks {
        for (id, gs) in snapshot.bookmarks {
            let local = sync.bookmarks.entry(id).or_default();
            for (g, b) in gs { if let Some(existing) = local.get_mut(&g) { merge_bookmark(existing, &b)?; } else { local.insert(g, b); } }
        }
    }
    if categories.positions {
        for (owner, entries) in snapshot.positions {
            let local = sync.positions.entry(owner).or_default();
            for (key, value) in entries { if let Some(existing) = local.get_mut(&key) { existing.merge(&value)?; } else { local.insert(key, value); } }
        }
    }
    ensure_projection_ids(&mut sync);
    validate(&sync)?;
    let projected = if categories.bookmarks { Some(projected_bookmarks(&sync)?) } else { None };
    let changed = sync != state.reading_sync;
    state.reading_sync = sync;
    if let Some(bookmarks) = projected { state.bookmarks = bookmarks; }
    Ok(MergeSummary { changed, conflicts: conflicts(state).len(), positions: positions(state).len() })
}

fn revision(parts: &[&str], value: &impl Serialize) -> String {
    let mut hash = Sha256::new();
    for part in parts { hash.update((part.len() as u64).to_be_bytes()); hash.update(part.as_bytes()); }
    serde_json::to_writer(crate::integrity::Sha256Writer(&mut hash), value)
        .expect("validated portable values serialize to an infallible hash sink");
    hex::encode(hash.finalize())
}
fn add_conflict<T: Clone + PartialEq + Serialize>(out: &mut Vec<Conflict>, id: &str, g: &str, title: &str, field: &str, reg: &Register<T>, display: impl Fn(&T) -> String) {
    let mut unique: Vec<&T> = Vec::new();
    for v in reg.values.values() { if !unique.contains(&&v.value) { unique.push(&v.value); } }
    if unique.len() < 2 { return; }
    let label = if field.starts_with("highlight:") { field.rsplit(':').next().unwrap_or(field) } else { field };
    out.push(Conflict { id: revision(&[id, g, field], reg), title: title.into(), field: label.into(), alternatives: reg.values.values().map(|v| Alternative { id: v.dot.id(), device_name: v.device_name.clone(), value: display(&v.value) }).collect() });
}
pub fn conflicts(state: &State) -> Vec<Conflict> {
    let mut out = Vec::new();
    for (id, gs) in &state.reading_sync.bookmarks {
        let Some((g, b)) = live_bookmark(gs) else { continue; };
        let title = b.title.value();
        let recreations: Vec<_> = gs.iter().filter(|(_, r)| !r.deleted).collect();
        if recreations.len() > 1 {
            out.push(Conflict { id: revision(&[id, "generations", "recreation"], gs), title: title.clone(), field: "recreation".into(), alternatives: recreations.into_iter().map(|(generation, r)| Alternative {
                id: revision(&[generation], r), device_name: r.note.values.first_key_value().expect("validated note").1.device_name.clone(), value: format!("{}\n{}", r.title.value(), r.note.value()),
            }).collect() });
            continue;
        }
        add_conflict(&mut out, id, g, title, "bookId", &b.book_id, Clone::clone);
        add_conflict(&mut out, id, g, title, "path", &b.path, Clone::clone);
        add_conflict(&mut out, id, g, title, "title", &b.title, Clone::clone);
        add_conflict(&mut out, id, g, title, "note", &b.note, Clone::clone);
        add_conflict(&mut out, id, g, title, "tags", &b.tags, |v| v.join(", "));
        add_conflict(&mut out, id, g, title, "createdAt", &b.created_at, Clone::clone);
        add_conflict(&mut out, id, g, title, "legacyBook", &b.legacy_book, |v| serde_json::to_string(v).unwrap_or_default());
        for (hid, hgs) in &b.highlights {
            let recreations: Vec<_> = hgs.iter().filter(|(_, r)| !r.deleted).collect();
            if recreations.len() > 1 {
                out.push(Conflict { id: revision(&[id, g, hid, "recreation"], hgs), title: title.clone(), field: "highlightRecreation".into(), alternatives: recreations.into_iter().map(|(generation, r)| Alternative {
                    id: revision(&[generation], r), device_name: r.comment.values.first_key_value().expect("validated comment").1.device_name.clone(), value: format!("{}\n{}", r.anchor.value().exact, r.comment.value()),
                }).collect() });
                continue;
            }
            if let Some((hg, h)) = live_highlight(hgs) {
                add_conflict(&mut out, id, g, title, &format!("highlight:{hid}:{hg}:comment"), &h.comment, Clone::clone);
                add_conflict(&mut out, id, g, title, &format!("highlight:{hid}:{hg}:anchor"), &h.anchor, |v| serde_json::to_string(v).unwrap_or_default());
            }
        }
    }
    out
}
fn resolve_register<T: Clone + PartialEq>(reg: &mut Register<T>, alternative: &str, stamp: &Version<()>) -> Result<(), String> {
    let chosen = reg.values.get(alternative).ok_or("The selected conflict alternative no longer exists")?;
    let value = chosen.value.clone();
    // Resolution chooses existing authored content. Keep its source label,
    // rather than growing a capacity-full register with a longer resolver name.
    let mut resolution = stamp.clone();
    resolution.device_name = chosen.device_name.clone();
    reg.set(&resolution, value);
    Ok(())
}
pub fn resolve(state: &mut State, conflict: &str, alternative: &str) -> Result<(), String> {
    let selected = conflicts(state).into_iter().find(|c| c.id == conflict).ok_or("The reading conflict no longer exists")?;
    if !selected.alternatives.iter().any(|v| v.id == alternative) { return Err("The selected conflict alternative no longer exists".into()); }
    let mut sync = state.reading_sync.clone();
    let stamp = sync.stamp()?;
    let mut found = false;
    for (id, gs) in &mut sync.bookmarks {
        if selected.field == "recreation" && revision(&[id, "generations", "recreation"], gs) == conflict {
            let chosen = gs.iter().find(|(g, b)| !b.deleted && revision(&[g], *b) == alternative).map(|(g, _)| g.clone()).ok_or("The selected recreation no longer exists")?;
            for (g, b) in gs { if g != &chosen { b.deleted = true; } }
            found = true;
            continue;
        }
        for (g, b) in gs {
            if b.deleted { continue; }
            macro_rules! field {
                ($name:ident, $wire:literal) => {
                    if selected.field == $wire && revision(&[id, g, $wire], &b.$name) == conflict {
                        resolve_register(&mut b.$name, alternative, &stamp)?;
                        found = true;
                    }
                };
            }
            field!(book_id, "bookId"); field!(path, "path"); field!(title, "title");
            field!(note, "note"); field!(tags, "tags"); field!(created_at, "createdAt"); field!(legacy_book, "legacyBook");
            for (hid, hgs) in &mut b.highlights {
                if selected.field == "highlightRecreation" && revision(&[id, g, hid, "recreation"], hgs) == conflict {
                    let chosen = hgs.iter().find(|(hg, h)| !h.deleted && revision(&[hg], *h) == alternative).map(|(hg, _)| hg.clone()).ok_or("The selected recreation no longer exists")?;
                    for (hg, h) in hgs { if hg != &chosen { h.deleted = true; } }
                    found = true;
                    continue;
                }
                for (hg, h) in hgs {
                    if h.deleted { continue; }
                    if selected.field == "comment" && revision(&[id, g, &format!("highlight:{hid}:{hg}:comment")], &h.comment) == conflict {
                        resolve_register(&mut h.comment, alternative, &stamp)?;
                        found = true;
                    }
                    if selected.field == "anchor" && revision(&[id, g, &format!("highlight:{hid}:{hg}:anchor")], &h.anchor) == conflict {
                        resolve_register(&mut h.anchor, alternative, &stamp)?;
                        found = true;
                    }
                }
            }
        }
    }
    if !found { return Err("The reading conflict no longer exists".into()); }
    validate(&sync)?;
    let bookmarks = projected_bookmarks(&sync)?;
    state.reading_sync = sync;
    state.bookmarks = bookmarks;
    Ok(())
}
pub fn positions(state: &State) -> Vec<PositionCandidate> {
    let available: BTreeSet<&str> = state.books.iter().filter(|b| b.available).map(|b| b.metadata.id.as_str()).collect();
    let mut out = Vec::new();
    for (owner, entries) in &state.reading_sync.positions {
        for (key, reg) in entries {
            for v in reg.values.values() {
                let p = &v.value;
                out.push(PositionCandidate { id: digest(&[owner, key, &v.dot.id()]), device_name: v.device_name.clone(), title: p.title.clone(), available: available.contains(p.book_id.as_str()), book_id: p.book_id.clone(), path: p.path.clone(), scroll: p.scroll });
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "reading_data_tests.rs"]
mod tests;
