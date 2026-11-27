// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
use crate::store::HistoryEntry;

const ALL: Categories = Categories { bookmarks: true, positions: true };
const H1: &str = "00000000-0000-4000-8000-000000000001";
const H2: &str = "00000000-0000-4000-8000-000000000002";
const H3: &str = "00000000-0000-4000-8000-000000000003";
fn bookmark() -> Bookmark {
    Bookmark { id: "bookmark-one".into(), book_id: "archive-uuid".into(), path: "A/Encoded%2Farticle?q=%23#section".into(), title: "Article".into(), note: "Original note".into(), tags: vec!["original".into()], highlights: vec![highlight(H1)], created_at: "2026-11-02T00:00:00Z".into(), legacy_book: None }
}
fn bookmark_key() -> String { let b = bookmark(); canonical_article_identity(&b.book_id, &b.path) }
fn set_record_path(record: &mut BookmarkRecord, path: &str) {
    for version in record.path.values.values_mut() { version.value = path.into(); }
}
fn highlight(id: &str) -> Highlight {
    Highlight { id: id.into(), document_path: "A/Encoded%2Farticle?q=%23".into(), exact: "quote".into(), prefix: "before ".into(), suffix: " after".into(), start: 10, end: 15, comment: "Original comment".into() }
}
fn history(scroll: f64) -> HistoryEntry {
    HistoryEntry { id: "history-one".into(), book_id: "archive-uuid".into(), path: "A/Encoded%2Farticle?q=%23#section".into(), title: "Article".into(), visited_at: "2026-11-02T00:00:00Z".into(), scroll }
}
fn device(name: &str) -> State {
    let mut state = State::default();
    state.reading_sync.device_name = name.into();
    state.reading_sync.enabled = true;
    state.reading_sync.categories = ALL;
    state
}
fn edit(state: &mut State, change: impl FnOnce(&mut State)) {
    let before = state.clone();
    change(state);
    capture(&before, state).unwrap();
}
fn pair() -> (State, State) {
    let mut a = device("Laptop");
    edit(&mut a, |s| s.bookmarks.push(bookmark()));
    let mut b = device("Phone");
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    (a, b)
}
fn exchange(a: &mut State, b: &mut State) {
    let from_a = export(a, ALL).unwrap();
    let from_b = export(b, ALL).unwrap();
    apply(a, from_b, ALL).unwrap();
    apply(b, from_a, ALL).unwrap();
}

#[test]
fn concurrent_independent_fields_and_distinct_highlights_merge() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| { s.bookmarks[0].note = "Laptop note".into(); s.bookmarks[0].highlights.push(highlight(H2)); });
    edit(&mut b, |s| { s.bookmarks[0].title = "Phone title".into(); s.bookmarks[0].tags.push("phone".into()); s.bookmarks[0].highlights.push(highlight(H3)); });
    exchange(&mut a, &mut b);
    assert_eq!(a.bookmarks, b.bookmarks);
    let merged = &a.bookmarks[0];
    assert_eq!(merged.note, "Laptop note");
    assert_eq!(merged.title, "Phone title");
    assert!(merged.tags.contains(&"phone".into()));
    assert_eq!(merged.highlights.len(), 3);
    assert!(conflicts(&a).is_empty());
}

#[test]
fn concurrent_notes_and_comments_are_explicit_and_resolve_causally() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| { s.bookmarks[0].note = "Laptop note".into(); s.bookmarks[0].highlights[0].comment = "Laptop comment".into(); });
    edit(&mut b, |s| { s.bookmarks[0].note = "Phone note".into(); s.bookmarks[0].highlights[0].comment = "Phone comment".into(); });
    let stale_a = export(&a, ALL).unwrap();
    let stale_b = export(&b, ALL).unwrap();
    exchange(&mut a, &mut b);
    let pending = conflicts(&a);
    assert_eq!(pending.len(), 2);
    for conflict in pending {
        assert_eq!(conflict.alternatives.len(), 2);
        assert!(conflict.alternatives.iter().any(|v| v.device_name == "Laptop"));
        assert!(conflict.alternatives.iter().any(|v| v.device_name == "Phone"));
        let choice = conflict.alternatives.iter().find(|v| v.value.starts_with("Phone")).unwrap();
        // Display text is a human scalar, not a serialized JSON string.
        assert!(!choice.value.starts_with('"'));
        resolve(&mut a, &conflict.id, &choice.id).unwrap();
    }
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    apply(&mut a, stale_a, ALL).unwrap();
    apply(&mut a, stale_b, ALL).unwrap();
    assert!(conflicts(&a).is_empty());
    assert!(conflicts(&b).is_empty());
    assert_eq!(a.bookmarks[0].note, "Phone note");
    assert_eq!(a.bookmarks[0].highlights[0].comment, "Phone comment");
    assert_eq!(a.bookmarks, b.bookmarks);
}

#[test]
fn parent_deletion_dominates_stale_children_and_explicit_recreation_is_distinct() {
    let (mut a, mut b) = pair();
    let original = export(&a, ALL).unwrap();
    edit(&mut b, |s| { s.bookmarks[0].note = "Stale offline note".into(); s.bookmarks[0].highlights.push(highlight(H2)); });
    let stale_edit = export(&b, ALL).unwrap();
    edit(&mut a, |s| s.bookmarks.clear());
    apply(&mut a, stale_edit.clone(), ALL).unwrap();
    assert!(a.bookmarks.is_empty());
    edit(&mut a, |s| { let mut recreated = bookmark(); recreated.note = "Explicitly recreated".into(); recreated.highlights.clear(); s.bookmarks.push(recreated); });
    apply(&mut a, original, ALL).unwrap();
    apply(&mut a, stale_edit, ALL).unwrap();
    assert_eq!(a.bookmarks.len(), 1);
    assert_eq!(a.bookmarks[0].note, "Explicitly recreated");
    assert!(a.bookmarks[0].highlights.is_empty());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert_eq!(a.bookmarks, b.bookmarks);
}

#[test]
fn highlight_deletion_and_recreation_reject_stale_comments() {
    let (mut a, mut b) = pair();
    edit(&mut b, |s| s.bookmarks[0].highlights[0].comment = "Stale comment".into());
    let stale = export(&b, ALL).unwrap();
    edit(&mut a, |s| s.bookmarks[0].highlights.clear());
    apply(&mut a, stale.clone(), ALL).unwrap();
    assert!(a.bookmarks[0].highlights.is_empty());
    edit(&mut a, |s| { let mut h = highlight(H1); h.comment = "Recreated comment".into(); s.bookmarks[0].highlights.push(h); });
    apply(&mut a, stale, ALL).unwrap();
    assert_eq!(a.bookmarks[0].highlights[0].comment, "Recreated comment");
    assert!(conflicts(&a).is_empty());
}

#[test]
fn concurrent_recreations_are_not_silently_hidden() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| s.bookmarks.clear());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    edit(&mut a, |s| { let mut r = bookmark(); r.note = "Laptop recreation".into(); s.bookmarks.push(r); });
    edit(&mut b, |s| { let mut r = bookmark(); r.note = "Phone recreation".into(); s.bookmarks.push(r); });
    exchange(&mut a, &mut b);
    let pending = conflicts(&a);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].field, "recreation");
    assert_eq!(pending[0].alternatives.len(), 2);
    let choice = pending[0].alternatives.iter().find(|v| v.value.contains("Phone recreation")).unwrap();
    resolve(&mut a, &pending[0].id, &choice.id).unwrap();
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert!(conflicts(&b).is_empty());
    assert_eq!(b.bookmarks[0].note, "Phone recreation");
}

#[test]
fn duplicate_delivery_restart_and_empty_capture_do_not_echo() {
    let (a, mut b) = pair();
    let snapshot = export(&a, ALL).unwrap();
    let counter = b.reading_sync.counter;
    assert!(!apply(&mut b, snapshot.clone(), ALL).unwrap().changed);
    let durable = serde_json::to_vec(&b.reading_sync).unwrap();
    b.reading_sync = serde_json::from_slice(&durable).unwrap();
    assert!(!apply(&mut b, snapshot, ALL).unwrap().changed);
    let before = export(&b, ALL).unwrap();
    edit(&mut b, |_| {});
    assert_eq!(counter, b.reading_sync.counter);
    assert_eq!(before, export(&b, ALL).unwrap());
}

#[test]
fn initial_empty_capture_seeds_and_first_transaction_deletion_leaves_tombstone() {
    let mut a = device("Laptop");
    a.bookmarks.push(bookmark());
    edit(&mut a, |_| {});
    assert_eq!(a.reading_sync.bookmarks.len(), 1);
    let mut b = device("Phone");
    b.bookmarks.push(bookmark());
    edit(&mut b, |s| s.bookmarks.clear());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert!(b.bookmarks.is_empty());
}

#[test]
fn empty_normal_capture_records_deferred_explicit_bookmark_writes() {
    let (mut a, mut b) = pair();
    // Native Store skips capture during private mode. An explicit bookmark save
    // remains a durable local bookmark; no private history is passed on resume.
    a.bookmarks[0].note = "Explicit private bookmark save".into();
    edit(&mut a, |_| {});
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert_eq!(b.bookmarks[0].note, "Explicit private bookmark save");
    a.bookmarks.clear();
    edit(&mut a, |_| {});
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert!(b.bookmarks.is_empty());
}

#[test]
fn encoded_article_identity_preserves_full_resource_semantics() {
    let key = |path| canonical_article_identity("book", path);
    assert_eq!(key("A/a?q=%23#one"), key("A/a?q=%23#two"));
    assert_ne!(key("A/a%2Fb"), key("A/a/b"));
    assert_ne!(key("A/a%23b"), key("A/a#b"));
    assert_ne!(key("A/a?q=a%26b"), key("A/a?q=a&b"));
    assert_ne!(key("A/a?q=1&b=2"), key("A/a?b=2&q=1"));
    assert_ne!(key("A/a%2fb"), key("A/a%2Fb"));
    assert_ne!(key("A/A"), key("A/a"));
    assert_ne!(key("A/a"), canonical_article_identity("other-book", "A/a"));
}

#[test]
fn positions_are_per_device_and_scroll_edits_not_visited_at_or_maximum() {
    let mut a = device("Laptop");
    edit(&mut a, |s| s.history.push(history(900.0)));
    let mut b = device("Phone");
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert!(b.history.is_empty());
    edit(&mut b, |s| s.history.push(history(12.0)));
    exchange(&mut a, &mut b);
    assert_eq!(positions(&a).len(), 2);
    assert_eq!(a.history[0].scroll, 900.0);
    let before = export(&a, ALL).unwrap();
    let counter = a.reading_sync.counter;
    edit(&mut a, |s| s.history[0].visited_at = "2099-01-01T00:00:00Z".into());
    assert_eq!(a.reading_sync.counter, counter);
    assert_eq!(before, export(&a, ALL).unwrap());
    edit(&mut a, |s| s.history[0].scroll = 3.0);
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    let candidate = positions(&b).into_iter().find(|p| p.device_name == "Laptop").unwrap();
    assert_eq!(candidate.scroll, 3.0);
    assert!(!candidate.available);
    assert_eq!(b.history[0].scroll, 12.0);
    edit(&mut a, |s| s.history.clear());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert_eq!(positions(&b).len(), 2);
}

#[test]
fn disabled_categories_are_filters_not_deletions() {
    let (mut a, mut b) = pair();
    a.reading_sync.enabled = false;
    a.reading_sync.categories = Categories::default();
    assert!(!a.reading_sync.enabled);
    edit(&mut a, |s| s.bookmarks[0].note = "Captured while disabled".into());
    let filtered = export(&a, Categories::default()).unwrap();
    assert!(filtered.bookmarks.is_empty());
    apply(&mut b, filtered, ALL).unwrap();
    assert_eq!(b.bookmarks[0].note, "Original note");
    apply(&mut b, export(&a, ALL).unwrap(), Categories::default()).unwrap();
    assert_eq!(b.bookmarks[0].note, "Original note");
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert_eq!(b.bookmarks[0].note, "Captured while disabled");
}

#[test]
fn malformed_projection_is_rejected_atomically() {
    let (a, mut b) = pair();
    let before = serde_json::to_value(&b).unwrap();
    let mut snapshot = export(&a, ALL).unwrap();
    let record = snapshot.bookmarks.get_mut(&bookmark_key()).unwrap().get_mut("initial").unwrap();
    record.path.values.first_entry().unwrap().get_mut().value = "file:///private/profile/state.json".into();
    assert!(apply(&mut b, snapshot, ALL).is_err());
    assert_eq!(serde_json::to_value(&b).unwrap(), before);
    let mut snapshot = export(&a, ALL).unwrap();
    snapshot.bookmarks.get_mut(&bookmark_key()).unwrap().get_mut("initial").unwrap().highlights.get_mut(H1).unwrap().get_mut("initial").unwrap().anchor.values.first_entry().unwrap().get_mut().value.end = 16;
    assert!(apply(&mut b, snapshot, ALL).is_err());
    assert_eq!(serde_json::to_value(&b).unwrap(), before);
    let mut wire = serde_json::to_value(export(&a, ALL).unwrap()).unwrap();
    wire.as_object_mut().unwrap().insert("readerStorage".into(), serde_json::json!({"secret": "not portable"}));
    assert!(serde_json::from_value::<WireSnapshot>(wire).is_err());
}

#[test]
fn own_replica_counter_cannot_be_advanced_by_remote_state() {
    let (a, mut b) = pair();
    let mut snapshot = export(&a, ALL).unwrap();
    let record = snapshot.bookmarks.get_mut(&bookmark_key()).unwrap().get_mut("initial").unwrap();
    let stamp = Version { dot: Dot { replica: b.reading_sync.replica_id.clone(), counter: b.reading_sync.counter + 1 }, device_name: "Forged".into(), value: () };
    record.note.set(&stamp, "Spoofed future local event".into());
    let before = b.reading_sync.clone();
    assert!(apply(&mut b, snapshot, ALL).is_err());
    assert_eq!(b.reading_sync, before);
}

#[test]
fn capacity_failure_never_prunes_tombstones() {
    let (mut a, _) = pair();
    let tombstone = a.reading_sync.bookmarks[&bookmark_key()]["initial"].clone();
    for n in 0..MAX_RECORDS {
        let mut deleted = tombstone.clone();
        deleted.deleted = true;
        let path = format!("A/deleted-{n}");
        set_record_path(&mut deleted, &path);
        a.reading_sync.bookmarks.insert(canonical_article_identity("archive-uuid", &path), BTreeMap::from([("initial".into(), deleted)]));
    }
    let retained = a.reading_sync.bookmarks.len();
    assert!(validate(&a.reading_sync).unwrap_err().contains("Rebootstrap"));
    assert_eq!(a.reading_sync.bookmarks.len(), retained);
}

#[test]
fn ordinary_edit_of_visible_conflict_does_not_discard_hidden_alternatives() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| { s.bookmarks[0].note = "Laptop note".into(); s.bookmarks[0].highlights[0].comment = "Laptop comment".into(); });
    edit(&mut b, |s| { s.bookmarks[0].note = "Phone note".into(); s.bookmarks[0].highlights[0].comment = "Phone comment".into(); });
    exchange(&mut a, &mut b);
    let old_note = a.bookmarks[0].note.clone();
    let old_comment = a.bookmarks[0].highlights[0].comment.clone();
    let hidden_note = conflicts(&a).into_iter().find(|c| c.field == "note").unwrap().alternatives.into_iter().find(|v| v.value != old_note).unwrap().value;
    let hidden_comment = conflicts(&a).into_iter().find(|c| c.field == "comment").unwrap().alternatives.into_iter().find(|v| v.value != old_comment).unwrap().value;
    edit(&mut a, |s| { s.bookmarks[0].note = "Ordinary edited note".into(); s.bookmarks[0].highlights[0].comment = "Ordinary edited comment".into(); });
    let counter = a.reading_sync.counter;
    edit(&mut a, |_| {});
    assert_eq!(a.reading_sync.counter, counter);
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    for state in [&a, &b] {
        let pending = conflicts(state);
        let note = pending.iter().find(|c| c.field == "note").unwrap();
        assert!(note.alternatives.iter().any(|v| v.value == hidden_note));
        assert!(note.alternatives.iter().any(|v| v.value == "Ordinary edited note"));
        let comment = pending.iter().find(|c| c.field == "comment").unwrap();
        assert!(comment.alternatives.iter().any(|v| v.value == hidden_comment));
        assert!(comment.alternatives.iter().any(|v| v.value == "Ordinary edited comment"));
    }
}

#[test]
fn reset_retires_stale_payloads_and_preserves_evicted_own_positions() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| s.history.push(history(41.0)));
    edit(&mut b, |s| s.history.push(history(7.0)));
    exchange(&mut a, &mut b);
    edit(&mut a, |s| { s.history.clear(); s.bookmarks.clear(); });
    let stale = export(&b, ALL).unwrap();
    let old_replica = a.reading_sync.replica_id.clone();
    a.reading_sync.enabled = true;
    a.reading_sync.categories = ALL;
    rebootstrap(&mut a).unwrap();
    assert_ne!(a.reading_sync.replica_id, old_replica);
    assert!(a.reading_sync.retired_replicas.contains(&old_replica));
    assert!(a.reading_sync.enabled);
    assert_eq!(a.reading_sync.categories, ALL);
    assert!(a.bookmarks.is_empty());
    assert!(a.history.is_empty());
    assert_eq!(positions(&a).len(), 1);
    assert_eq!(positions(&a)[0].device_name, "Laptop");
    assert_eq!(positions(&a)[0].scroll, 41.0);
    let before = a.reading_sync.clone();
    assert!(apply(&mut a, stale.clone(), ALL).unwrap_err().contains("rebootstrap"));
    assert_eq!(a.reading_sync, before);
    // The barrier travels with the full snapshot, including through a new peer.
    let mut c = device("New device");
    apply(&mut c, export(&a, ALL).unwrap(), ALL).unwrap();
    assert!(apply(&mut c, stale, ALL).is_err());
    // Merely receiving the reset must not discard the old peer's reading data.
    let before_b = serde_json::to_value(&b).unwrap();
    assert!(apply(&mut b, export(&a, ALL).unwrap(), ALL).is_err());
    assert_eq!(serde_json::to_value(&b).unwrap(), before_b);
}

#[test]
fn reset_rejects_unresolved_alternatives_without_mutation() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| s.bookmarks[0].note = "Laptop note".into());
    edit(&mut b, |s| s.bookmarks[0].note = "Phone note".into());
    exchange(&mut a, &mut b);
    let before = a.reading_sync.clone();
    assert!(rebootstrap(&mut a).unwrap_err().contains("Resolve"));
    assert_eq!(a.reading_sync, before);
}

#[test]
fn exact_byte_capacity_still_allows_conflict_resolution_then_reset() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| s.bookmarks[0].note = "Laptop note".into());
    edit(&mut b, |s| s.bookmarks[0].note = "Phone note".into());
    exchange(&mut a, &mut b);
    // Exercise the worst source-label growth case: selecting existing content
    // must not replace its short source label with this long resolver label.
    a.reading_sync.device_name = "x".repeat(128);
    let mut filler = a.reading_sync.bookmarks[&bookmark_key()]["initial"].clone();
    filler.deleted = true;
    let first = filler.note.values.first_key_value().unwrap().0.clone();
    filler.note.values.retain(|id, _| id == &first);
    filler.note.values.first_entry().unwrap().get_mut().value.clear();
    set_record_path(&mut filler, "A/Ballast0000");
    let empty_cost = serde_json::to_vec(&BTreeMap::from([("initial", &filler)])).unwrap().len()
        + 64 + 4; // canonical hex key + quotes + colon + comma
    let mut size = serde_json::to_vec(&a.reading_sync).unwrap().len();
    let mut index = 0;
    while MAX_WIRE_BYTES - size >= empty_cost {
        let length = (MAX_WIRE_BYTES - size - empty_cost).min(65_536);
        filler.note.values.first_entry().unwrap().get_mut().value = "x".repeat(length);
        let path = format!("A/Ballast{index:04}");
        set_record_path(&mut filler, &path);
        let id = canonical_article_identity("archive-uuid", &path);
        a.reading_sync.bookmarks.insert(id, BTreeMap::from([("initial".into(), filler.clone())]));
        size += empty_cost + length;
        index += 1;
    }
    assert_eq!(size, MAX_WIRE_BYTES);
    assert_eq!(serde_json::to_vec(&a.reading_sync).unwrap().len(), MAX_WIRE_BYTES);
    validate(&a.reading_sync).unwrap();
    let pending = conflicts(&a);
    assert_eq!(pending.len(), 1);
    resolve(&mut a, &pending[0].id, &pending[0].alternatives[0].id).unwrap();
    assert!(conflicts(&a).is_empty());
    rebootstrap(&mut a).unwrap();
    validate(&a.reading_sync).unwrap();
    assert_eq!(a.bookmarks.len(), 1);
    assert_eq!(a.reading_sync.bookmarks.len(), 1);
}

#[test]
fn independent_bookmark_ids_converge_by_encoded_article_and_keep_local_ids() {
    let mut a = device("Laptop");
    let mut b = device("Phone");
    edit(&mut a, |s| {
        let mut saved = bookmark();
        saved.id = "local-laptop-id".into();
        saved.note = "Laptop note".into();
        saved.highlights[0].comment = "Laptop comment".into();
        saved.highlights.push(highlight(H2));
        s.bookmarks.push(saved);
    });
    edit(&mut b, |s| {
        let mut saved = bookmark();
        saved.id = "local-phone-id".into();
        saved.note = "Phone note".into();
        saved.highlights[0].comment = "Phone comment".into();
        saved.highlights.push(highlight(H3));
        s.bookmarks.push(saved);
    });
    let stale = export(&b, ALL).unwrap();
    exchange(&mut a, &mut b);
    assert_eq!(a.bookmarks.len(), 1);
    assert_eq!(b.bookmarks.len(), 1);
    assert_eq!(a.bookmarks[0].id, "local-laptop-id");
    assert_eq!(b.bookmarks[0].id, "local-phone-id");
    assert_eq!(a.bookmarks[0].highlights.len(), 3);
    assert_eq!(conflicts(&a).len(), 2);
    assert_eq!(export(&a, ALL).unwrap(), export(&b, ALL).unwrap());
    edit(&mut a, |s| s.bookmarks.clear());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert!(b.bookmarks.is_empty());
    edit(&mut b, |s| {
        let mut recreated = bookmark();
        recreated.id = "new-phone-id".into();
        recreated.note = "Explicit new generation".into();
        s.bookmarks.push(recreated);
    });
    apply(&mut b, stale, ALL).unwrap();
    assert_eq!(b.bookmarks[0].id, "new-phone-id");
    assert_eq!(b.bookmarks[0].note, "Explicit new generation");
}

#[test]
fn canonical_fragment_duplicates_preserve_notes_but_encoded_resources_stay_distinct() {
    let mut a = device("Laptop");
    edit(&mut a, |s| {
        let mut first = bookmark();
        first.id = "fragment-one".into();
        first.path = "A/a%2Fb?q=%23#one".into();
        first.note = "First fragment note".into();
        let mut second = first.clone();
        second.id = "fragment-two".into();
        second.path = "A/a%2Fb?q=%23#two".into();
        second.note = "Second fragment note".into();
        let mut distinct = first.clone();
        distinct.id = "literal-slash".into();
        distinct.path = "A/a/b?q=%23#one".into();
        s.bookmarks.extend([first, second, distinct]);
    });
    assert_eq!(a.bookmarks.len(), 2);
    let note = conflicts(&a).into_iter().find(|c| c.field == "note").unwrap();
    assert!(note.alternatives.iter().any(|v| v.value == "First fragment note"));
    assert!(note.alternatives.iter().any(|v| v.value == "Second fragment note"));
    let counter = a.reading_sync.counter;
    edit(&mut a, |_| {});
    assert_eq!(a.reading_sync.counter, counter);
}

#[test]
fn moving_article_retires_old_resource_without_stealing_live_bookmark_ids() {
    let (mut a, mut b) = pair();
    let stale = export(&a, ALL).unwrap();
    edit(&mut a, |s| s.bookmarks[0].path = "A/Moved?q=1".into());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    assert_eq!(a.bookmarks[0].id, "bookmark-one");
    assert_eq!(b.bookmarks[0].id, "bookmark-one");
    assert_eq!(b.bookmarks[0].path, "A/Moved?q=1");
    apply(&mut b, stale, ALL).unwrap();
    assert_eq!(b.bookmarks.len(), 1);
    assert_eq!(b.bookmarks[0].path, "A/Moved?q=1");
}

#[test]
fn malformed_encoded_bookmark_position_and_anchor_paths_reject_atomically() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| s.history.push(history(20.0)));
    let before = serde_json::to_value(&b).unwrap();
    for path in ["A/%ZZ.html", "A/trailing%", "A/%FF.html"] {
        let mut snapshot = export(&a, ALL).unwrap();
        let mut gs = snapshot.bookmarks.remove(&bookmark_key()).unwrap();
        set_record_path(gs.get_mut("initial").unwrap(), path);
        snapshot.bookmarks.insert(canonical_article_identity("archive-uuid", path), gs);
        assert!(apply(&mut b, snapshot, ALL).is_err());
        assert_eq!(serde_json::to_value(&b).unwrap(), before);

        let mut snapshot = export(&a, ALL).unwrap();
        let positions = snapshot.positions.get_mut(&a.reading_sync.replica_id).unwrap();
        let (_, mut position) = positions.pop_first().unwrap();
        for v in position.values.values_mut() { v.value.path = path.into(); }
        positions.insert(canonical_article_identity("archive-uuid", path), position);
        assert!(apply(&mut b, snapshot, ALL).is_err());
        assert_eq!(serde_json::to_value(&b).unwrap(), before);

        let mut snapshot = export(&a, ALL).unwrap();
        let anchor = &mut snapshot.bookmarks.get_mut(&bookmark_key()).unwrap().get_mut("initial").unwrap().highlights.get_mut(H1).unwrap().get_mut("initial").unwrap().anchor;
        for v in anchor.values.values_mut() { v.value.document_path = path.into(); }
        assert!(apply(&mut b, snapshot, ALL).is_err());
        assert_eq!(serde_json::to_value(&b).unwrap(), before);
    }
}

#[test]
fn recreation_choice_revisions_include_notes_and_child_highlights() {
    for change_child in [false, true] {
        let (mut a, mut b) = pair();
        edit(&mut a, |s| s.bookmarks.clear());
        apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
        edit(&mut a, |s| { let mut saved = bookmark(); saved.note = "Laptop recreation".into(); s.bookmarks.push(saved); });
        edit(&mut b, |s| { let mut saved = bookmark(); saved.note = "Phone recreation".into(); s.bookmarks.push(saved); });
        exchange(&mut a, &mut b);
        let old = conflicts(&a).into_iter().find(|c| c.field == "recreation").unwrap();
        edit(&mut b, |s| {
            if change_child { s.bookmarks[0].highlights.push(highlight(H2)); }
            else { s.bookmarks[0].note = "Edited after presentation".into(); }
        });
        apply(&mut a, export(&b, ALL).unwrap(), ALL).unwrap();
        let fresh = conflicts(&a).into_iter().find(|c| c.field == "recreation").unwrap();
        assert_ne!(old.id, fresh.id);
        assert_ne!(old.alternatives.iter().map(|v| &v.id).collect::<Vec<_>>(), fresh.alternatives.iter().map(|v| &v.id).collect::<Vec<_>>());
        let retained = a.reading_sync.clone();
        assert!(resolve(&mut a, &old.id, &old.alternatives[0].id).is_err());
        assert_eq!(a.reading_sync, retained);
        resolve(&mut a, &fresh.id, &fresh.alternatives[0].id).unwrap();
    }
}

#[test]
fn highlight_recreation_choice_is_invalidated_by_later_comment_edit() {
    let (mut a, mut b) = pair();
    edit(&mut a, |s| s.bookmarks[0].highlights.clear());
    apply(&mut b, export(&a, ALL).unwrap(), ALL).unwrap();
    edit(&mut a, |s| { let mut h = highlight(H1); h.comment = "Laptop recreation".into(); s.bookmarks[0].highlights.push(h); });
    edit(&mut b, |s| { let mut h = highlight(H1); h.comment = "Phone recreation".into(); s.bookmarks[0].highlights.push(h); });
    exchange(&mut a, &mut b);
    let old = conflicts(&a).into_iter().find(|c| c.field == "highlightRecreation").unwrap();
    edit(&mut b, |s| s.bookmarks[0].highlights[0].comment = "Edited after presentation".into());
    apply(&mut a, export(&b, ALL).unwrap(), ALL).unwrap();
    let fresh = conflicts(&a).into_iter().find(|c| c.field == "highlightRecreation").unwrap();
    assert_ne!(old.id, fresh.id);
    let retained = a.reading_sync.clone();
    assert!(resolve(&mut a, &old.id, &old.alternatives[0].id).is_err());
    assert_eq!(a.reading_sync, retained);
    resolve(&mut a, &fresh.id, &fresh.alternatives[0].id).unwrap();
}

fn tab(id: &str, scroll: f64) -> store::Tab {
    store::Tab { restricted: false, id: id.into(), book_id: Some("archive-uuid".into()), path: "A/Encoded%2Farticle?q=%23#section".into(), title: "Article".into(), scroll, back: Vec::new(), forward: Vec::new() }
}

#[test]
fn tab_scroll_without_history_and_window_producers_capture_only_touched_positions() {
    let mut a = device("Laptop");
    edit(&mut a, |s| s.tabs.push(tab("native-root-tab", 20.0)));
    assert!(a.history.is_empty());
    assert_eq!(positions(&a)[0].scroll, 20.0);
    edit(&mut a, |s| s.tabs[0].scroll = 4.0);
    assert_eq!(positions(&a)[0].scroll, 4.0);
    edit(&mut a, |s| {
        let mut first = store::WindowSession::default();
        first.tabs.push(tab("native-first-tab", 5.0));
        let mut last = store::WindowSession::default();
        last.tabs.push(tab("native-last-tab", 90.0));
        s.windows.insert("a-window".into(), first);
        s.windows.insert("z-window".into(), last);
    });
    assert_eq!(positions(&a)[0].scroll, 90.0);
    edit(&mut a, |s| s.windows.get_mut("a-window").unwrap().tabs[0].scroll = 2.0);
    assert_eq!(positions(&a)[0].scroll, 2.0);
    edit(&mut a, |s| {
        s.history.push(history(1000.0));
        s.windows.get_mut("a-window").unwrap().tabs[0].scroll = 1.0;
    });
    assert_eq!(positions(&a)[0].scroll, 1.0);
    let wire = serde_json::to_string(&export(&a, ALL).unwrap()).unwrap();
    assert!(!wire.contains("native-first-tab"));
    assert!(!wire.contains("a-window"));
    edit(&mut a, |s| { s.history.clear(); s.tabs.clear(); s.windows.clear(); });
    assert_eq!(positions(&a)[0].scroll, 1.0);
    rebootstrap(&mut a).unwrap();
    assert_eq!(positions(&a)[0].scroll, 1.0);
}

#[test]
fn private_session_projections_never_become_reading_position_edits() {
    let mut normal = device("Laptop");
    edit(&mut normal, |s| s.tabs.push(tab("native-tab", 11.0)));
    let durable = normal.reading_sync.clone();
    let mut private = normal.clone();
    private.settings.private_mode = true;
    private.tabs[0].scroll = 999.0;
    let before = private.clone();
    capture(&before, &mut private).unwrap();
    assert_eq!(private.reading_sync, durable);
    // Store restores persisted browsing before calling capture on exit.
    capture(&private, &mut normal).unwrap();
    assert_eq!(normal.reading_sync, durable);
    assert_eq!(positions(&normal)[0].scroll, 11.0);
}

fn oversized_unadmitted_profile() -> State {
    let mut state = State::default();
    state.bookmarks.push(bookmark());
    state.history = (0..10_000).map(|index| {
        let mut entry = history(1.0);
        entry.id = format!("history-{index}");
        entry.path = format!("A/Article-{index}");
        entry
    }).collect();
    state
}

#[test]
fn unadmitted_legacy_data_does_not_bootstrap_or_block_normal_updates() {
    let mut state = oversized_unadmitted_profile();
    let initial = state.reading_sync.clone();
    edit(&mut state, |s| s.bookmarks[0].note = "Ordinary local note".into());
    assert_eq!(state.reading_sync, initial);
    assert_eq!(state.history.len(), 10_000);
    assert_eq!(state.bookmarks[0].note, "Ordinary local note");
    // Enabled without a category, or a selected category while disabled, is
    // not admission and must not inspect all local history.
    edit(&mut state, |s| s.reading_sync.enabled = true);
    assert_eq!(state.reading_sync.tracked_categories, Categories::default());
    edit(&mut state, |s| {
        s.reading_sync.enabled = false;
        s.reading_sync.categories = ALL;
    });
    assert_eq!(state.reading_sync.tracked_categories, Categories::default());
    assert!(state.reading_sync.bookmarks.is_empty());
    assert!(state.reading_sync.positions.is_empty());
}

#[test]
fn oversized_position_admission_is_atomic_and_bookmark_only_recovery_succeeds() {
    let mut state = oversized_unadmitted_profile();
    let original = serde_json::to_value(&state).unwrap();
    let mut rejected = state.clone();
    rejected.reading_sync.enabled = true;
    rejected.reading_sync.categories = Categories { bookmarks: false, positions: true };
    assert!(capture(&state, &mut rejected).unwrap_err().contains("Leave reading positions or bookmarks off"));
    assert_eq!(rejected.reading_sync.tracked_categories, Categories::default());
    assert_eq!(rejected.reading_sync.counter, 0);
    assert!(rejected.reading_sync.positions.is_empty());
    assert_eq!(serde_json::to_value(&state).unwrap(), original);

    edit(&mut state, |s| {
        s.reading_sync.enabled = true;
        s.reading_sync.categories = Categories { bookmarks: true, positions: false };
    });
    assert_eq!(state.reading_sync.tracked_categories, Categories { bookmarks: true, positions: false });
    assert_eq!(state.reading_sync.bookmarks.len(), 1);
    assert!(state.reading_sync.positions.is_empty());
    assert_eq!(state.history.len(), 10_000);
    edit(&mut state, |s| {
        s.reading_sync.enabled = false;
        s.reading_sync.categories = Categories::default();
        s.bookmarks.clear();
    });
    assert!(state.reading_sync.bookmarks[&bookmark_key()]["initial"].deleted);
    assert!(state.reading_sync.tracked_categories.bookmarks);
    assert!(!state.reading_sync.tracked_categories.positions);
    rebootstrap(&mut state).unwrap();
    assert!(state.reading_sync.tracked_categories.bookmarks);
    assert!(!state.reading_sync.tracked_categories.positions);
    assert_eq!(state.history.len(), 10_000);
    assert!(positions(&state).is_empty());
}

#[test]
fn negotiated_apply_admits_only_its_categories_and_empty_capture_does_not_echo() {
    let mut source = device("Laptop");
    edit(&mut source, |s| s.bookmarks.push(bookmark()));
    let bookmarks = Categories { bookmarks: true, positions: false };
    let mut receiver = oversized_unadmitted_profile();
    receiver.bookmarks.clear();
    apply(&mut receiver, export(&source, bookmarks).unwrap(), bookmarks).unwrap();
    assert!(!receiver.reading_sync.enabled);
    assert_eq!(receiver.reading_sync.tracked_categories, bookmarks);
    let before = export(&receiver, bookmarks).unwrap();
    let counter = receiver.reading_sync.counter;
    edit(&mut receiver, |_| {});
    assert_eq!(receiver.reading_sync.counter, counter);
    assert_eq!(export(&receiver, bookmarks).unwrap(), before);
    assert_eq!(receiver.history.len(), 10_000);
}

#[test]
fn admission_defaults_false_on_load_and_reset_does_not_admit_pending_categories() {
    let mut state = oversized_unadmitted_profile();
    let mut encoded = serde_json::to_value(&state.reading_sync).unwrap();
    encoded.as_object_mut().unwrap().remove("trackedCategories");
    let loaded: ReadingSync = serde_json::from_value(encoded).unwrap();
    assert_eq!(loaded.tracked_categories, Categories::default());
    state.reading_sync.enabled = true;
    state.reading_sync.categories = ALL;
    rebootstrap(&mut state).unwrap();
    assert!(state.reading_sync.enabled);
    assert_eq!(state.reading_sync.categories, ALL);
    assert_eq!(state.reading_sync.tracked_categories, Categories::default());
    assert!(state.reading_sync.bookmarks.is_empty());
    assert!(state.reading_sync.positions.is_empty());
    assert_eq!(state.history.len(), 10_000);
    assert_eq!(state.bookmarks.len(), 1);
}

#[test]
fn explicit_position_reset_recovers_live_capacity_without_replaying_history() {
    let mut state = device("Laptop");
    edit(&mut state, |s| {
        s.history = (0..MAX_RECORDS).map(|index| {
            let mut entry = history(1.0);
            entry.id = format!("history-{index}");
            entry.path = format!("A/Article-{index}");
            entry
        }).collect();
        s.tabs.push(tab("current-reader", 1.0));
        // Use a resource already in history, keeping exactly MAX_RECORDS keys.
        s.tabs[0].path = "A/Article-0".into();
    });
    assert_eq!(positions(&state).len(), MAX_RECORDS);
    let old_snapshot = export(&state, ALL).unwrap();
    let original_replica = state.reading_sync.replica_id.clone();
    let mut rejected = state.clone();
    let mut new_visit = history(3.0);
    new_visit.id = "history-new-resource".into();
    new_visit.path = "A/New-resource".into();
    rejected.history.push(new_visit.clone());
    assert!(capture(&state, &mut rejected).unwrap_err().contains("Rebootstrap"));
    assert_eq!(positions(&state).len(), MAX_RECORDS);

    let history_before = serde_json::to_value(&state.history).unwrap();
    let tabs_before = serde_json::to_value(&state.tabs).unwrap();
    rebootstrap_forgetting_positions(&mut state).unwrap();
    assert_ne!(state.reading_sync.replica_id, original_replica);
    assert!(state.reading_sync.retired_replicas.contains(&original_replica));
    assert!(positions(&state).is_empty());
    assert_eq!(serde_json::to_value(&state.history).unwrap(), history_before);
    assert_eq!(serde_json::to_value(&state.tabs).unwrap(), tabs_before);
    edit(&mut state, |_| {});
    assert!(positions(&state).is_empty());

    edit(&mut state, |s| s.history.push(new_visit));
    assert_eq!(positions(&state).len(), 1);
    assert_eq!(positions(&state)[0].path, "A/New-resource");
    let retained = state.reading_sync.clone();
    assert!(apply(&mut state, old_snapshot, ALL).is_err());
    assert_eq!(state.reading_sync, retained);
    edit(&mut state, |s| s.tabs[0].scroll = 17.0);
    assert_eq!(positions(&state).len(), 2);
    assert!(positions(&state).iter().any(|p| p.path == "A/Article-0" && p.scroll == 17.0));
}

#[test]
fn projection_claim_drops_a_stale_entry_that_shares_the_local_id() {
    let (mut a, _) = pair();
    let key = bookmark_key();
    // An article move leaves the same local ID claimed by a key with no live record.
    a.reading_sync.projection_ids.insert("archive-uuid/A/Retired".into(), "bookmark-one".into());
    edit(&mut a, |s| s.bookmarks[0].note = "Edited".into());
    assert_eq!(a.reading_sync.projection_ids.get(&key).map(String::as_str), Some("bookmark-one"));
    assert!(!a.reading_sync.projection_ids.contains_key("archive-uuid/A/Retired"));
    assert_eq!(a.reading_sync.projection_ids.len(), 1);
    assert_eq!(a.bookmarks[0].id, "bookmark-one");
}

#[test]
fn projection_claim_keeps_one_owner_per_local_id() {
    let mut a = device("Laptop");
    let mut second = bookmark();
    second.id = "bookmark-two".into();
    second.path = "A/Second".into();
    edit(&mut a, |s| { s.bookmarks.push(bookmark()); s.bookmarks.push(second.clone()); });
    let first_key = bookmark_key();
    let second_key = canonical_article_identity(&second.book_id, &second.path);
    assert_ne!(first_key, second_key);
    assert_eq!(a.reading_sync.projection_ids[&first_key], "bookmark-one");
    assert_eq!(a.reading_sync.projection_ids[&second_key], "bookmark-two");
    // A merge can carry one local ID under two articles. The current key keeps it
    // and the other claimant is reassigned, so no local ID ends up owned twice.
    a.reading_sync.projection_ids.insert(second_key.clone(), "bookmark-one".into());
    edit(&mut a, |s| s.bookmarks.iter_mut().find(|b| b.id == "bookmark-one").unwrap().note = "Edited".into());
    assert_eq!(a.reading_sync.projection_ids[&first_key], "bookmark-one");
    assert_eq!(a.reading_sync.projection_ids[&second_key], "bookmark-two");
    let mut ids: Vec<&str> = a.reading_sync.projection_ids.values().map(String::as_str).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), a.reading_sync.projection_ids.len());
    assert_eq!(a.bookmarks.iter().filter(|b| b.id == "bookmark-one").count(), 1);
    assert_eq!(a.bookmarks.iter().filter(|b| b.id == "bookmark-two").count(), 1);
    assert_eq!(a.bookmarks.iter().find(|b| b.id == "bookmark-one").unwrap().note, "Edited");
}

#[test]
fn unrelated_store_mutations_succeed_even_when_sync_positions_are_at_capacity() {
    let mut state = device("Laptop");
    edit(&mut state, |s| {
        s.history = (0..MAX_RECORDS)
            .map(|index| {
                let mut entry = history(1.0);
                entry.id = format!("history-{index}");
                entry.path = format!("A/Article-{index}");
                entry
            })
            .collect();
    });
    assert_eq!(positions(&state).len(), MAX_RECORDS);
    let before = state.clone();
    state.settings.theme = "dark".into();
    capture(&before, &mut state).unwrap();
    assert_eq!(state.settings.theme, "dark");
    assert_eq!(positions(&state).len(), MAX_RECORDS);
}
