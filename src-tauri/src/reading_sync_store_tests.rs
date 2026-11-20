// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
struct Profile(PathBuf);
impl Profile {
    fn new() -> Self { Self(std::env::temp_dir().join(format!("kiwix-sync-store-{}", Uuid::new_v4()))) }
    fn initialized(&self) -> Store { Store::new(self.0.clone()).unwrap() }
}
impl Drop for Profile { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn bookmark() -> Bookmark {
    Bookmark { id: Uuid::new_v4().to_string(), book_id: "missing".into(), path: "A/Article?q=%2F#section".into(), title: "Preserve this reading list".into(), note: "owned personal note".into(), tags: vec![], highlights: vec![], created_at: now(), legacy_book: None }
}
#[test]
fn existing_history_above_sync_bound_keeps_ordinary_writes_and_explicit_category_recovery() {
    let profile = Profile::new(); drop(profile.initialized());
    let mut legacy = State::default(); legacy.bookmarks.push(bookmark());
    legacy.history = (0..8_001).map(|index| HistoryEntry { id: Uuid::new_v4().to_string(), book_id: "missing".into(), path: format!("A/History{index}"), title: format!("History {index}"), visited_at: now(), scroll: index as f64 }).collect();
    // A pre-sync profile can legitimately contain 10,000 visits.
    let mut saved = serde_json::to_value(&legacy).unwrap(); saved.as_object_mut().unwrap().remove("readingSync"); saved.as_object_mut().unwrap().remove("privateSync");
    fs::write(profile.0.join("state.json"), serde_json::to_vec(&saved).unwrap()).unwrap();
    let store = profile.initialized();
    store.update(|state| { state.settings.show_search_snippets = false; state.bookmarks[0].note = "ordinary edit still persists".into(); Ok(()) }).unwrap();
    assert_eq!(store.read().unwrap().history.len(), 8_001);
    assert_eq!(store.read().unwrap().reading_sync.counter, 0, "no implicit bootstrap before category consent");
    let before = fs::read(profile.0.join("state.json")).unwrap();
    let rejected = store.update(|state| { state.reading_sync.enabled = true; state.reading_sync.categories = crate::reading_data::Categories { bookmarks: true, positions: true }; Ok(()) });
    assert!(rejected.is_err(), "oversized initial admission must be explicit and atomic");
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), before);
    assert!(!store.read().unwrap().reading_sync.enabled);
    // Recover by consenting to the bounded category, without deleting local history.
    store.update(|state| { state.reading_sync.enabled = true; state.reading_sync.categories = crate::reading_data::Categories { bookmarks: true, positions: false }; Ok(()) }).unwrap();
    let state = store.read().unwrap(); assert_eq!(state.history.len(), 8_001); assert_eq!(state.bookmarks[0].note, "ordinary edit still persists");
    assert!(state.reading_sync.counter > 0); assert!(crate::reading_data::positions(&state).is_empty());
}
#[test]
fn actual_mobile_loader_never_replaces_malformed_causal_or_trust_metadata() {
    let profile = Profile::new(); drop(profile.initialized());
    let path = profile.0.join("state.json");
    for key in ["readingSync", "privateSync"] {
        let mut value = serde_json::to_value(State::default()).unwrap(); value["bookmarks"] = serde_json::json!([bookmark()]); value[key] = serde_json::json!({"damaged":true});
        let bytes = serde_json::to_vec(&value).unwrap(); fs::write(&path, &bytes).unwrap();
        assert!(load_mobile_state(&profile.0, &path, false, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!fs::read_dir(&profile.0).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("state.unreadable-")));
    }
    let truncated = br#"{"version":2,"readingSync":{"replicaId":"unterminated"#;
    fs::write(&path, truncated).unwrap();
    assert!(load_mobile_state(&profile.0, &path, false, None).is_err()); assert_eq!(fs::read(&path).unwrap(), truncated);
}
#[test]
fn materialized_bookmark_recovery_cannot_generate_a_causal_deletion() {
    let profile = Profile::new(); let store = profile.initialized();
    store.update(|state| { state.reading_sync.enabled = true; state.reading_sync.categories.bookmarks = true; state.bookmarks.push(bookmark()); Ok(()) }).unwrap();
    let original = store.read().unwrap(); let wire = crate::reading_data::export(&original, crate::reading_data::Categories { bookmarks: true, positions: false }).unwrap();
    let mut healthy_peer = State::default(); crate::reading_data::apply(&mut healthy_peer, wire, crate::reading_data::Categories { bookmarks: true, positions: false }).unwrap();
    drop(store);
    let mut damaged = serde_json::to_value(&original).unwrap(); damaged["bookmarks"][0]["path"] = serde_json::json!(17);
    let bytes = serde_json::to_vec(&damaged).unwrap(); let path = profile.0.join("state.json"); fs::write(&path, &bytes).unwrap();
    assert!(state_from_value(damaged).is_err());
    assert!(load_mobile_state(&profile.0, &path, false, None).is_err());
    assert!(Store::new(profile.0.clone()).is_err(), "no Store is admitted that could capture the missing row as a deletion");
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(healthy_peer.bookmarks[0].note, original.bookmarks[0].note);
}
